import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  GaugeIcon,
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
import { ErrorState, LoadingState } from "@/components/ui/states";
import { StatusBadge } from "@/components/ui/status-badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import { useServiceStore, type ServiceDiscoveryEntry } from "@/stores/serviceStore";
import type { ServerProfile, ServiceDiscovery, ServiceState } from "@/types/contracts";

type ServiceStateBadge = {
  label: string;
  variant: "success" | "warning" | "danger" | "outline";
};

const serviceStateBadges: Record<ServiceState, ServiceStateBadge> = {
  active: { label: "Active", variant: "success" },
  inactive: { label: "Inactive", variant: "outline" },
  failed: { label: "Failed", variant: "danger" },
  unknown: { label: "Unknown", variant: "warning" },
};

function formatSampleTime(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeValue(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function discoveryBadge(
  connected: boolean,
  entry: ServiceDiscoveryEntry | undefined,
): { label: string; variant: "success" | "warning" | "info" | "outline" } {
  const discovery = entry?.discovery;

  if (!discovery) {
    return entry?.pending
      ? { label: "Discovering", variant: "info" }
      : { label: connected ? "Not discovered" : "Disconnected", variant: "outline" };
  }
  if (entry?.pending) {
    return { label: "Refreshing", variant: "info" };
  }
  if (!connected) {
    return { label: "Read-only", variant: "outline" };
  }
  if (discovery.status === "unsupported") {
    return { label: "Unsupported", variant: "warning" };
  }
  if (discovery.status === "unavailable") {
    return { label: "Unavailable", variant: "warning" };
  }

  return { label: "Discovered", variant: "success" };
}

function stateCounts(discovery: ServiceDiscovery | null) {
  const counts: Record<ServiceState, number> = {
    active: 0,
    inactive: 0,
    failed: 0,
    unknown: 0,
  };
  for (const service of discovery?.services ?? []) {
    counts[service.state] += 1;
  }
  return counts;
}

function ServicesToolbar({
  servers,
  selected,
  onSelect,
  connected,
  entry,
  onRefresh,
}: {
  servers: ServerProfile[];
  selected: ServerProfile;
  onSelect: (serverId: string) => void;
  connected: boolean;
  entry: ServiceDiscoveryEntry | undefined;
  onRefresh: () => void;
}) {
  const pending = entry?.pending ?? false;
  const badge = discoveryBadge(connected, entry);

  return (
    <section
      aria-label="Service discovery controls"
      className="rounded-md border border-border bg-card px-3 py-3"
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 flex-1">
          <label
            htmlFor="services-server"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Server
          </label>
          <Select value={selected.id} onValueChange={onSelect}>
            <SelectTrigger id="services-server" className="mt-1 max-w-md">
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
              {connected
                ? "Read-only systemd discovery"
                : "Connect this server to discover its actual units"}
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
            aria-label="Refresh discovered services"
          >
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {pending ? "Discovering..." : "Refresh now"}
          </Button>
        </div>
      </div>
    </section>
  );
}

function ServiceList({ discovery }: { discovery: ServiceDiscovery }) {
  const counts = stateCounts(discovery);

  if (discovery.services.length === 0) {
    return (
      <EmptyState
        icon={<GaugeIcon className="size-4" aria-hidden="true" />}
        title="No systemd services discovered"
        description="The server reported systemd but did not list any service units. ServerDesk never adds common service names that the server did not report."
      />
    );
  }

  return (
    <section
      aria-label="Discovered systemd services"
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-3 py-2.5">
        <div className="min-w-0">
          <h2 className="text-[13px] font-semibold text-foreground">Discovered systemd units</h2>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            {discovery.services.length} unit{discovery.services.length === 1 ? "" : "s"} reported by
            systemctl
            {discovery.truncated ? " · output was truncated at the safety limit" : ""}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {(["active", "inactive", "failed", "unknown"] as const).map((state) => (
            <Badge key={state} variant={serviceStateBadges[state].variant}>
              {serviceStateBadges[state].label} {counts[state]}
            </Badge>
          ))}
        </div>
      </div>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Unit</TableHead>
            <TableHead>State</TableHead>
            <TableHead>Active / sub</TableHead>
            <TableHead>Load</TableHead>
            <TableHead>Description</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {discovery.services.map((service) => (
            <TableRow key={service.name} data-service-state={service.state}>
              <TableCell className="font-mono text-[12px] font-medium">{service.name}</TableCell>
              <TableCell>
                <Badge variant={serviceStateBadges[service.state].variant}>
                  {serviceStateBadges[service.state].label}
                </Badge>
              </TableCell>
              <TableCell className="font-mono text-[11px] text-muted-foreground">
                {service.activeState}
                {service.subState ? ` / ${service.subState}` : ""}
              </TableCell>
              <TableCell className="text-[11px] text-muted-foreground">
                {service.loadState ?? "Unknown"}
              </TableCell>
              <TableCell className="max-w-[48ch] text-[12px] text-muted-foreground">
                {service.description ?? "No description reported"}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>

      <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border px-3 py-2 text-[10px] text-muted-foreground">
        <span>
          Sampled{" "}
          <time dateTime={dateTimeValue(discovery.sampledAt)}>
            {formatSampleTime(discovery.sampledAt)}
          </time>{" "}
          in {discovery.durationMs} ms.
        </span>
        <span>Read-only discovery. Start, stop, and restart actions are not enabled yet.</span>
      </div>
    </section>
  );
}

function ServicesPage() {
  const servers = useServerStore((state) => state.servers);
  const status = useServerStore((state) => state.status);
  const error = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const connections = useConnectionStore((state) => state.connections);
  const lastActiveServerId = usePreferencesStore((state) => state.preferences.lastActiveServerId);
  const refresh = useServiceStore((state) => state.refresh);
  const clearError = useServiceStore((state) => state.clearError);

  const [selectedId, setSelectedId] = useState<string | null>(lastActiveServerId);

  const selected =
    servers.find((server) => server.id === selectedId) ??
    servers.find((server) => server.id === lastActiveServerId) ??
    servers[0] ??
    null;
  const selectedServerId = selected?.id ?? null;
  const connection = selected ? connections[selected.id] : undefined;
  const connected = connection?.state === "connected";
  const entry = useServiceStore((state) => (selected ? state.entries[selected.id] : undefined));
  const discovery = entry?.discovery ?? null;

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

  useEffect(() => {
    if (!connected || !selectedServerId) {
      return;
    }
    if (entry?.pending || entry?.discovery || entry?.error) {
      return;
    }

    void refresh(selectedServerId);
  }, [connected, selectedServerId, entry?.pending, entry?.discovery, entry?.error, refresh]);

  return (
    <div className="space-y-6">
      <PageHeader
        title="Services"
        description="Discover the systemd service units that actually exist on the target machine, including versioned PHP-FPM units and unlisted custom services."
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
              Service discovery needs a live SSH session. Open the desktop build and connect a
              server to inspect its real systemd units. No service names or states are fabricated in
              browser preview.
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
          icon={<GaugeIcon className="size-4" aria-hidden="true" />}
          title="No server selected"
          description="Add a server profile and connect it to discover its actual systemd services."
          action={
            <Button asChild size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      ) : null}

      {selected ? (
        <>
          <ServicesToolbar
            servers={servers}
            selected={selected}
            onSelect={setSelectedId}
            connected={connected}
            entry={entry}
            onRefresh={() => void refresh(selected.id)}
          />

          {entry?.error ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Could not discover services</AlertTitle>
                <AlertDescription>
                  {entry.error}
                  {discovery
                    ? " The last successful discovery stays visible with its original timestamp. No service states were invented."
                    : null}
                </AlertDescription>
              </div>
              <Button variant="ghost" size="sm" onClick={() => clearError(selected.id)}>
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {discovery?.status === "unsupported" ? (
            <Alert variant="warning">
              <InfoIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
              <div>
                <AlertTitle>Systemd is not available</AlertTitle>
                <AlertDescription>
                  {discovery.detail ??
                    "This server does not run systemd, so service discovery cannot list units."}
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {discovery?.status === "unavailable" ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>Systemd discovery failed</AlertTitle>
                <AlertDescription>
                  {discovery.detail ??
                    "The remote systemd command failed. Try again while the server is connected."}
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {entry?.pending && !discovery ? <LoadingState rows={5} /> : null}

          {discovery?.status === "available" ? <ServiceList discovery={discovery} /> : null}

          {!entry?.pending && !discovery && !entry?.error ? (
            <EmptyState
              icon={<GaugeIcon className="size-4" aria-hidden="true" />}
              title={connected ? "No discovery yet" : "Service discovery is paused"}
              description={
                connected
                  ? "Refresh to query systemd on the selected server."
                  : "Connect this server to query its systemd units. Service discovery never assumes that common units exist."
              }
              action={
                connected ? (
                  <Button size="sm" onClick={() => void refresh(selected.id)}>
                    Discover services
                  </Button>
                ) : undefined
              }
            />
          ) : null}
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

export { ServicesPage };
