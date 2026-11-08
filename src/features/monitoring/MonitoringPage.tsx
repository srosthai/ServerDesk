import { lazy, Suspense, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  ChartLineIcon,
  CircleNotchIcon,
  HardDrivesIcon,
  InfoIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorState, LoadingState } from "@/components/ui/states";
import { StatusBadge } from "@/components/ui/status-badge";
import { latestSuccessfulSample } from "@/features/monitoring/chartData";
import { useMonitoringPolling } from "@/hooks/useMonitoringPolling";
import { MIN_MONITORING_INTERVAL_SECONDS } from "@/lib/preferences";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { useMonitoringStore, type MonitoringEntry } from "@/stores/monitoringStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type { ServerProfile } from "@/types/contracts";
import { cn } from "@/lib/utils";

// The ECharts chunk is requested only after this page renders and a usable
// sample exists. It is never part of the initial application bundle.
const MonitoringCharts = lazy(() => import("@/features/monitoring/MonitoringCharts"));

type MonitoringStateBadge = {
  label: string;
  variant: "success" | "warning" | "info" | "outline";
};

function formatSampleTime(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeValue(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function monitoringBadge(
  connected: boolean,
  entry: MonitoringEntry | undefined,
  staleByAge: boolean,
): MonitoringStateBadge {
  const pending = entry?.pending ?? false;
  const hasSnapshot = entry?.snapshot != null;

  if (!hasSnapshot) {
    return pending
      ? { label: "Collecting", variant: "info" }
      : { label: "No sample", variant: "outline" };
  }
  if (!connected) {
    return { label: "Paused · stale", variant: "warning" };
  }
  if (entry?.stale || staleByAge) {
    return { label: pending ? "Stale · refreshing" : "Stale", variant: "warning" };
  }
  if (pending) {
    return { label: "Refreshing", variant: "info" };
  }

  return { label: "Live", variant: "success" };
}

function ChartSkeletonGrid() {
  return (
    <div className="grid min-w-0 gap-4 xl:grid-cols-2" aria-busy="true" aria-label="Loading charts">
      {["CPU load", "Memory usage", "Disk usage"].map((title) => (
        <div
          key={title}
          className={cn(
            "flex min-w-0 flex-col overflow-hidden rounded-md border border-border bg-card",
            title === "Disk usage" && "xl:col-span-2",
          )}
        >
          <div className="flex min-h-[68px] flex-wrap items-start justify-between gap-3 border-b border-border px-3 py-2.5">
            <div className="min-w-0 space-y-1">
              <p className="text-[12px] font-semibold text-foreground">{title}</p>
              <Skeleton className="h-2.5 w-[240px] max-w-full" />
            </div>
            <div className="space-y-1 text-right">
              <Skeleton className="ml-auto h-3 w-14" />
              <Skeleton className="ml-auto h-2.5 w-32" />
            </div>
          </div>
          <div className="h-[220px] w-full px-2 pt-2">
            <Skeleton className="h-[212px] w-full" />
          </div>
          <div className="border-t border-border px-3 py-2">
            <Skeleton className="h-2.5 w-[68%]" />
          </div>
        </div>
      ))}
    </div>
  );
}

function MonitoringToolbar({
  servers,
  selected,
  onSelect,
  connected,
  entry,
  intervalSeconds,
  staleByAge,
  onRefresh,
}: {
  servers: ServerProfile[];
  selected: ServerProfile;
  onSelect: (serverId: string) => void;
  connected: boolean;
  entry: MonitoringEntry | undefined;
  intervalSeconds: number;
  staleByAge: boolean;
  onRefresh: () => void;
}) {
  const pending = entry?.pending ?? false;
  const badge = monitoringBadge(connected, entry, staleByAge);
  const latest = latestSuccessfulSample(entry?.history ?? []);

  return (
    <section
      aria-label="Monitoring controls"
      className="rounded-md border border-border bg-card px-3 py-3"
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 flex-1">
          <label
            htmlFor="monitoring-server"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Server
          </label>
          <Select value={selected.id} onValueChange={onSelect}>
            <SelectTrigger id="monitoring-server" className="mt-1 max-w-md">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {servers.map((server) => (
                <SelectItem key={server.id} value={server.id}>
                  {server.name} · {server.username}@{server.host}:{server.port}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-[10px] text-muted-foreground">
            <span>
              {selected.username}@{selected.host}:{selected.port}
            </span>
            <span aria-hidden="true">·</span>
            <span>
              {connected ? "Polling active" : "Polling paused while disconnected"} · every{" "}
              {intervalSeconds} s
            </span>
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <StatusBadge state={connected ? "connected" : "disconnected"} />
          <Badge variant={badge.variant}>{badge.label}</Badge>
          <Button
            variant="secondary"
            size="sm"
            onClick={onRefresh}
            disabled={!connected || pending}
            aria-label="Refresh monitoring charts"
          >
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {pending ? "Refreshing..." : "Refresh now"}
          </Button>
        </div>
      </div>

      <dl className="mt-3 grid grid-cols-2 gap-x-6 gap-y-2 border-t border-border pt-3 md:grid-cols-4">
        <div>
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Latest sample
          </dt>
          <dd className="mt-0.5 font-mono text-[11px] text-foreground">
            {latest?.snapshot ? (
              <time dateTime={dateTimeValue(latest.snapshot.sampledAt)}>
                {formatSampleTime(latest.snapshot.sampledAt)}
              </time>
            ) : (
              <span className="font-sans text-muted-foreground">No successful sample yet</span>
            )}
          </dd>
        </div>
        <div>
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Samples retained
          </dt>
          <dd className="mt-0.5 text-[11px] text-foreground">
            {(entry?.history ?? []).filter((sample) => sample.snapshot !== null).length} of{" "}
            {(entry?.history ?? []).length} attempts
          </dd>
        </div>
        <div>
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Freshness
          </dt>
          <dd className="mt-0.5 text-[11px] text-foreground">
            {connected ? `Evaluated every ${intervalSeconds} s` : "Stale until reconnected"}
          </dd>
        </div>
        <div>
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Gap policy
          </dt>
          <dd className="mt-0.5 text-[11px] text-foreground">Missing samples break the line</dd>
        </div>
      </dl>
    </section>
  );
}

function MonitoringPage() {
  const servers = useServerStore((state) => state.servers);
  const status = useServerStore((state) => state.status);
  const error = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const connections = useConnectionStore((state) => state.connections);
  const intervalPreference = usePreferencesStore(
    (state) => state.preferences.monitoringIntervalSeconds,
  );
  const lastActiveServerId = usePreferencesStore((state) => state.preferences.lastActiveServerId);
  const refresh = useMonitoringStore((state) => state.refresh);
  const clearError = useMonitoringStore((state) => state.clearError);

  const [selectedId, setSelectedId] = useState<string | null>(lastActiveServerId);
  const [now, setNow] = useState<number | null>(null);

  const selected =
    servers.find((server) => server.id === selectedId) ??
    servers.find((server) => server.id === lastActiveServerId) ??
    servers[0] ??
    null;
  const connection = selected ? connections[selected.id] : undefined;
  const connected = connection?.state === "connected";
  const entry = useMonitoringStore((state) => (selected ? state.entries[selected.id] : undefined));
  const intervalSeconds = Math.max(MIN_MONITORING_INTERVAL_SECONDS, intervalPreference);
  const history = entry?.history ?? [];
  const latest = latestSuccessfulSample(history);
  const latestSampledAt = latest?.snapshot?.sampledAt ?? null;
  const sampleAgeMs =
    latestSampledAt === null || now === null ? null : Math.max(0, now - latestSampledAt);
  const staleByAge =
    connected && sampleAgeMs !== null && sampleAgeMs > Math.max(intervalSeconds * 1000 * 2, 15_000);
  const hasSnapshot = history.some((sample) => sample.snapshot !== null);

  useMonitoringPolling(selected?.id ?? null, connected);

  useEffect(() => {
    const updateNow = () => setNow(Date.now());
    updateNow();
    const timer = window.setInterval(updateNow, Math.max(5_000, intervalSeconds * 1000));

    return () => {
      window.clearInterval(timer);
    };
  }, [intervalSeconds]);

  useEffect(() => {
    if (status === "idle") {
      void fetchServers();
    }
  }, [status, fetchServers]);

  useEffect(() => {
    if (status === "ready" && servers.length > 0) {
      void refreshConnectionStates(servers.map((server) => server.id));
    }
  }, [status, servers, refreshConnectionStates]);

  function handleSelect(serverId: string) {
    setSelectedId(serverId);
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Monitoring"
        description="Session-local CPU load, memory, and disk trends from the same lightweight snapshots used by the Overview dashboard."
        actions={
          <Button asChild variant="secondary" size="sm">
            <Link to="/servers">Open servers</Link>
          </Button>
        }
      />

      {!isTauriRuntime() ? (
        <Alert variant="info">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Web preview</AlertTitle>
            <AlertDescription>
              Charts need live SSH monitoring snapshots. Open the desktop build and connect a server
              to collect real samples. No values are fabricated in browser preview.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {status === "loading" && servers.length === 0 ? <LoadingState rows={4} /> : null}

      {status === "error" && servers.length === 0 ? (
        <ErrorState message={error ?? "Could not load server profiles."} onRetry={fetchServers} />
      ) : null}

      {status !== "loading" && servers.length === 0 ? (
        <EmptyState
          icon={<ChartLineIcon className="size-4" aria-hidden="true" />}
          title="No server selected"
          description="Add a server profile and connect it to collect monitoring samples with honest timestamps."
          action={
            <Button asChild size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      ) : null}

      {selected ? (
        <>
          <MonitoringToolbar
            servers={servers}
            selected={selected}
            onSelect={handleSelect}
            connected={connected}
            entry={entry}
            intervalSeconds={intervalSeconds}
            staleByAge={staleByAge}
            onRefresh={() => void refresh(selected.id)}
          />

          {entry?.error ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Could not refresh monitoring</AlertTitle>
                <AlertDescription>
                  {entry.error}
                  {entry.snapshot
                    ? " The retained samples stay visible with their original timestamps and are marked stale. No new values are invented."
                    : null}
                </AlertDescription>
              </div>
              <Button variant="ghost" size="sm" onClick={() => clearError(selected.id)}>
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {!connected && history.length > 0 ? (
            <Alert variant="warning">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-warning"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>Monitoring paused</AlertTitle>
                <AlertDescription>
                  Polling stopped because the server is disconnected. Retained samples keep their
                  real timestamps, the newest value is marked stale, and the pause appears as a gap
                  when collection resumes.
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {hasSnapshot ? (
            <Suspense fallback={<ChartSkeletonGrid />}>
              <MonitoringCharts
                history={history}
                intervalSeconds={intervalSeconds}
                stale={entry?.stale ?? staleByAge}
              />
            </Suspense>
          ) : (
            <section
              aria-label="Monitoring charts pending"
              className="rounded-md border border-dashed border-border bg-card/35 px-5 py-6"
            >
              <div className="flex items-center gap-2 text-[13px] font-medium text-foreground">
                {entry?.pending ? (
                  <CircleNotchIcon className="size-4 animate-spin text-muted-foreground" />
                ) : (
                  <ChartLineIcon className="size-4 text-muted-foreground" />
                )}
                {entry?.pending ? "Collecting the first chart sample..." : "No chart samples yet"}
              </div>
              <p className="mt-2 max-w-[68ch] text-[12px] leading-relaxed text-muted-foreground">
                {connected
                  ? "The first successful snapshot will create CPU, memory, and disk charts. Failed attempts are kept as gaps, never as interpolated values."
                  : "Connect this server to collect chart samples. Retained history remains available after a disconnect, but polling never runs while disconnected."}
              </p>
            </section>
          )}

          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-border pt-3 text-[10px] text-muted-foreground">
            <span className="flex items-center gap-1.5">
              <HardDrivesIcon className="size-3" aria-hidden="true" />
              History is kept per server for this application session.
            </span>
            <span aria-hidden="true">·</span>
            <span>Up to 180 attempt markers are retained; older points roll off.</span>
          </div>
        </>
      ) : null}

      {status === "error" && servers.length > 0 ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>Could not refresh server profiles</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </div>
        </Alert>
      ) : null}
    </div>
  );
}

export { MonitoringPage };
