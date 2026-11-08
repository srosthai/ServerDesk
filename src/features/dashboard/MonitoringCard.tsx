import type { ReactNode } from "react";
import {
  ArrowClockwiseIcon,
  ChartLineIcon,
  CircleNotchIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useMonitoringPolling } from "@/hooks/useMonitoringPolling";
import { MIN_MONITORING_INTERVAL_SECONDS } from "@/lib/preferences";
import { cn } from "@/lib/utils";
import { useMonitoringStore } from "@/stores/monitoringStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import type {
  CpuMetrics,
  FilesystemMetrics,
  MemoryMetrics,
  MetricGroup,
  MetricStatus,
  MonitoringSnapshot,
  NetworkInterfaceMetrics,
} from "@/types/contracts";

type MonitoringCardProps = {
  serverId: string;
  connected: boolean;
};

type SnapshotBadge = {
  label: string;
  variant: "success" | "warning" | "info" | "outline";
};

const groupStatusConfig: Record<
  MetricStatus,
  { label: string; variant: "success" | "warning" | "outline" }
> = {
  available: { label: "Available", variant: "success" },
  unsupported: { label: "Unsupported", variant: "outline" },
  unavailable: { label: "Unavailable", variant: "warning" },
};

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "Unavailable";
  }

  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let value = bytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex++;
  }

  const digits = value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[unitIndex]}`;
}

function formatPercent(value: number) {
  return Number.isFinite(value) ? `${value.toFixed(1)}%` : "Unavailable";
}

function formatLoad(value: number) {
  return Number.isFinite(value) ? value.toFixed(2) : "Unavailable";
}

function formatCount(value: number) {
  return Number.isFinite(value) ? new Intl.NumberFormat().format(value) : "Unavailable";
}

function formatSampleTime(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeValue(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function overallBadge(
  connected: boolean,
  pending: boolean,
  stale: boolean,
  hasSnapshot: boolean,
): SnapshotBadge {
  if (!hasSnapshot) {
    return pending
      ? { label: "Collecting", variant: "info" }
      : { label: "No sample", variant: "outline" };
  }
  if (!connected) {
    return { label: "Paused", variant: "warning" };
  }
  if (stale) {
    return { label: pending ? "Stale · refreshing" : "Stale", variant: "warning" };
  }
  if (pending) {
    return { label: "Refreshing", variant: "info" };
  }

  return { label: "Current", variant: "success" };
}

function MetricValue({ label, value, unit }: { label: string; value: string; unit?: string }) {
  return (
    <div>
      <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
        {label}
      </dt>
      <dd className="mt-0.5 font-mono text-[12px] text-foreground">
        {value}
        {unit ? (
          <span className="ml-1 font-sans text-[10px] text-muted-foreground">{unit}</span>
        ) : null}
      </dd>
    </div>
  );
}

function MetricGroupCard<T>({
  title,
  group,
  className,
  children,
}: {
  title: string;
  group: MetricGroup<T>;
  className?: string;
  children: (data: T) => ReactNode;
}) {
  const status = groupStatusConfig[group.status];

  return (
    <article className={cn("rounded-md border border-border bg-background p-3", className)}>
      <header className="flex items-center justify-between gap-2">
        <h4 className="text-[12px] font-semibold text-foreground">{title}</h4>
        <Badge variant={status.variant}>{status.label}</Badge>
      </header>

      <div className="mt-2">
        {group.status === "available" && group.data !== null ? (
          children(group.data)
        ) : (
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {group.detail ??
              (group.status === "unsupported"
                ? "The remote system does not expose this metric."
                : "This metric is unavailable.")}
          </p>
        )}
      </div>

      <footer className="mt-3 border-t border-border/70 pt-2 text-[10px] text-muted-foreground">
        Sampled{" "}
        <time dateTime={dateTimeValue(group.sampledAt)}>{formatSampleTime(group.sampledAt)}</time>
      </footer>
    </article>
  );
}

function CpuContent({ data }: { data: CpuMetrics }) {
  const hasCpuCount = data.logicalCpus !== null && data.logicalCpus > 0;
  const capacityPercent = hasCpuCount ? (data.load1m / data.logicalCpus!) * 100 : null;
  const barWidth = capacityPercent === null ? 0 : Math.min(100, Math.max(0, capacityPercent));

  return (
    <>
      <div className="grid grid-cols-3 gap-2">
        <MetricValue label="1 min" value={formatLoad(data.load1m)} unit="load avg" />
        <MetricValue label="5 min" value={formatLoad(data.load5m)} unit="load avg" />
        <MetricValue label="15 min" value={formatLoad(data.load15m)} unit="load avg" />
      </div>
      {capacityPercent !== null ? (
        <>
          <div className="mt-3 flex items-baseline justify-between gap-2">
            <span className="text-[10px] text-muted-foreground">
              1 min load relative to {formatCount(data.logicalCpus!)} logical CPUs
            </span>
            <span className="shrink-0 font-mono text-[11px] text-foreground">
              {formatPercent(capacityPercent)}
            </span>
          </div>
          <div
            role="progressbar"
            aria-label="Load average relative to logical CPUs"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(barWidth)}
            className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted"
          >
            <div className="h-full rounded-full bg-primary" style={{ width: `${barWidth}%` }} />
          </div>
          <p className="mt-1.5 text-[10px] text-muted-foreground">
            {formatCount(data.logicalCpus!)} logical CPUs available.
          </p>
          <p className="mt-1 text-[10px] text-muted-foreground">
            Load average is a run-queue measure, not CPU utilization; it can exceed 100% of the
            available cores.
          </p>
        </>
      ) : (
        <p className="mt-2 text-[11px] text-muted-foreground">
          Logical CPU count unavailable, so relative load cannot be calculated.
        </p>
      )}
    </>
  );
}

function MemoryContent({ data }: { data: MemoryMetrics }) {
  const usedPercent = Number.isFinite(data.usedPercent) ? data.usedPercent : 0;
  const barWidth = Math.min(100, Math.max(0, usedPercent));

  return (
    <>
      <div className="flex items-end justify-between gap-3">
        <div>
          <p className="text-[22px] font-semibold leading-none text-foreground">
            {formatPercent(data.usedPercent)}
          </p>
          <p className="mt-1 text-[10px] text-muted-foreground">memory used</p>
        </div>
      </div>
      <div
        role="progressbar"
        aria-label="Memory used"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(barWidth)}
        className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
      >
        <div className="h-full rounded-full bg-primary" style={{ width: `${barWidth}%` }} />
      </div>
      <dl className="mt-3 grid grid-cols-3 gap-2">
        <MetricValue label="Total" value={formatBytes(data.totalBytes)} />
        <MetricValue label="Used" value={formatBytes(data.usedBytes)} />
        <MetricValue label="Available" value={formatBytes(data.availableBytes)} />
      </dl>
    </>
  );
}

function FilesystemContent({ data }: { data: FilesystemMetrics[] }) {
  const top = [...data].sort((left, right) => right.totalBytes - left.totalBytes);
  const visible = top.slice(0, 4);

  return (
    <div className="space-y-3">
      {visible.map((filesystem, index) => {
        const barWidth = Math.min(100, Math.max(0, filesystem.usedPercent));

        return (
          <div key={`${filesystem.mount}-${index}`}>
            <div className="flex items-baseline justify-between gap-2">
              <code className="truncate font-mono text-[11px] text-foreground">
                {filesystem.mount}
              </code>
              <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                {formatPercent(filesystem.usedPercent)}
              </span>
            </div>
            <div
              role="progressbar"
              aria-label={`Disk used at ${filesystem.mount}`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(barWidth)}
              className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted"
            >
              <div className="h-full rounded-full bg-primary" style={{ width: `${barWidth}%` }} />
            </div>
            <dl className="mt-1 grid grid-cols-3 gap-2">
              <MetricValue label="Total" value={formatBytes(filesystem.totalBytes)} />
              <MetricValue label="Used" value={formatBytes(filesystem.usedBytes)} />
              <MetricValue label="Available" value={formatBytes(filesystem.availableBytes)} />
            </dl>
            {filesystem.filesystem ? (
              <p className="mt-1 truncate font-mono text-[10px] text-muted-foreground">
                {filesystem.filesystem}
              </p>
            ) : null}
          </div>
        );
      })}
      {top.length > visible.length ? (
        <p className="text-[10px] text-muted-foreground">
          + {top.length - visible.length} more mounted filesystems not shown.
        </p>
      ) : null}
    </div>
  );
}

function NetworkContent({ data }: { data: NetworkInterfaceMetrics[] }) {
  const visible = data.slice(0, 4);

  return (
    <div className="space-y-3">
      {visible.map((network) => (
        <div key={network.interface}>
          <code className="font-mono text-[11px] text-foreground">{network.interface}</code>
          <dl className="mt-1 grid grid-cols-2 gap-2">
            <MetricValue label="Received" value={formatBytes(network.receivedBytes)} />
            <MetricValue label="Transmitted" value={formatBytes(network.transmittedBytes)} />
          </dl>
          <p className="mt-1 text-[10px] text-muted-foreground">
            {formatCount(network.receivedPackets)} RX packets ·{" "}
            {formatCount(network.transmittedPackets)} TX packets
          </p>
        </div>
      ))}
      {data.length > visible.length ? (
        <p className="text-[10px] text-muted-foreground">
          + {data.length - visible.length} more interfaces not shown.
        </p>
      ) : null}
    </div>
  );
}

function SnapshotContent({
  snapshot,
  intervalSeconds,
}: {
  snapshot: MonitoringSnapshot;
  intervalSeconds: number;
}) {
  return (
    <>
      <div className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
        <span>
          Last successful sample{" "}
          <time dateTime={dateTimeValue(snapshot.sampledAt)}>
            {formatSampleTime(snapshot.sampledAt)}
          </time>
        </span>
        <span aria-hidden="true">·</span>
        <span>Completed in {snapshot.durationMs} ms</span>
        <span aria-hidden="true">·</span>
        <span>Refreshes every {intervalSeconds} s while connected</span>
      </div>

      <div className="mt-3 grid gap-3 xl:grid-cols-2">
        <MetricGroupCard title="CPU" group={snapshot.cpu}>
          {(data: CpuMetrics) => <CpuContent data={data} />}
        </MetricGroupCard>
        <MetricGroupCard title="Memory" group={snapshot.memory}>
          {(data: MemoryMetrics) => <MemoryContent data={data} />}
        </MetricGroupCard>
        <MetricGroupCard title="Disk" group={snapshot.filesystems} className="xl:col-span-2">
          {(data: FilesystemMetrics[]) => <FilesystemContent data={data} />}
        </MetricGroupCard>
        <MetricGroupCard title="Network" group={snapshot.network} className="xl:col-span-2">
          {(data: NetworkInterfaceMetrics[]) => <NetworkContent data={data} />}
        </MetricGroupCard>
      </div>
    </>
  );
}

function MonitoringCard({ serverId, connected }: MonitoringCardProps) {
  const entry = useMonitoringStore((state) => state.entries[serverId]);
  const refresh = useMonitoringStore((state) => state.refresh);
  const clearError = useMonitoringStore((state) => state.clearError);
  const intervalSeconds = usePreferencesStore(
    (state) => state.preferences.monitoringIntervalSeconds,
  );

  useMonitoringPolling(serverId, connected);

  const snapshot = entry?.snapshot ?? null;
  const stale = entry?.stale ?? false;
  const pending = entry?.pending ?? false;
  const error = entry?.error ?? null;
  const safeIntervalSeconds = Math.max(MIN_MONITORING_INTERVAL_SECONDS, intervalSeconds);
  const badge = overallBadge(connected, pending, stale, snapshot !== null);

  return (
    <section aria-label="Resource monitoring">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h3 className="flex items-center gap-1.5 text-[13px] font-semibold text-foreground">
            <ChartLineIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            Resource monitoring
          </h3>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            Lightweight read-only snapshots of CPU load, memory, filesystems, and network counters.
            Every group keeps its own sample time.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge variant={badge.variant}>{badge.label}</Badge>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void refresh(serverId)}
            disabled={!connected || pending}
            aria-label="Refresh monitoring"
          >
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {pending ? "Refreshing..." : "Refresh"}
          </Button>
        </div>
      </div>

      {error ? (
        <Alert variant="destructive" className="mt-3 py-2.5">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div className="flex-1">
            <AlertTitle>Could not refresh monitoring</AlertTitle>
            <AlertDescription>
              {error}
              {snapshot
                ? " The last snapshot is kept but marked stale. Values below are not live."
                : null}
            </AlertDescription>
          </div>
          <Button variant="ghost" size="sm" onClick={() => clearError(serverId)}>
            Dismiss
          </Button>
        </Alert>
      ) : null}

      {!connected && snapshot ? (
        <Alert variant="warning" className="mt-3 py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Monitoring paused</AlertTitle>
            <AlertDescription>
              Polling stopped because the server is disconnected. The values below are retained from
              the last successful sample and are marked stale.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {snapshot ? (
        <SnapshotContent snapshot={snapshot} intervalSeconds={safeIntervalSeconds} />
      ) : pending ? (
        <div
          className="mt-3 flex items-center gap-2 text-[11px] text-muted-foreground"
          aria-busy="true"
        >
          <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
          Collecting the first monitoring snapshot...
        </div>
      ) : (
        <p className="mt-3 rounded-md border border-dashed border-border px-3 py-2 text-[11px] text-muted-foreground">
          {connected
            ? "No monitoring snapshot has been collected yet. Use Refresh to try again."
            : "Connect the server to collect CPU, memory, filesystem, and network snapshots. Polling never runs while the server is disconnected."}
        </p>
      )}
    </section>
  );
}

export { MonitoringCard };
