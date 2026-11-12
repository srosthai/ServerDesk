import {
  ArrowClockwiseIcon,
  ArrowRightIcon,
  CircleNotchIcon,
  DatabaseIcon,
  HardDrivesIcon,
  LinkBreakIcon,
  PencilSimpleIcon,
  PlugsIcon,
  PulseIcon,
  TrashIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { StatusBadge } from "@/components/ui/status-badge";
import {
  DATABASE_ENGINE_LABELS,
  databaseConnectionStateLabel,
  formatDatabaseEndpoint,
  formatLatency,
} from "@/features/databases/databaseProfileForm";
import { describeConnectionError } from "@/lib/connectionErrors";
import { isTauriRuntime } from "@/lib/tauri";
import type {
  DatabaseConnectionSnapshot,
  DatabaseConnectionState,
  DatabaseEngine,
  DatabaseProfile,
  DatabaseTestResult,
  DatabaseTunnelSnapshot,
  ServerProfile,
} from "@/types/contracts";

type DatabaseConnectionListProps = {
  profiles: DatabaseProfile[];
  servers: ServerProfile[];
  snapshots: Record<string, DatabaseConnectionSnapshot>;
  testResults: Record<string, DatabaseTestResult>;
  busy: Record<string, boolean>;
  selectedProfileId: string | null;
  sshConnected: boolean;
  onOpen: (profile: DatabaseProfile) => void;
  onConnect: (profile: DatabaseProfile) => void;
  onReconnect: (profile: DatabaseProfile) => void;
  onDisconnect: (profile: DatabaseProfile) => void;
  onTest: (profile: DatabaseProfile) => void;
  onEdit: (profile: DatabaseProfile) => void;
  onDelete: (profile: DatabaseProfile) => void;
  onAdd: () => void;
};

const engineAccent: Record<DatabaseEngine, string> = {
  mysql: "bg-warning",
  postgresql: "bg-info",
};

function EngineBadge({ engine }: { engine: DatabaseEngine }) {
  return (
    <Badge variant="outline" className="gap-1.5">
      <span aria-hidden="true" className={`size-1.5 rounded-full ${engineAccent[engine]}`} />
      {DATABASE_ENGINE_LABELS[engine]}
    </Badge>
  );
}

function tunnelLabel(tunnel: DatabaseTunnelSnapshot | undefined): string {
  if (!tunnel) {
    return "No tunnel";
  }

  switch (tunnel.state) {
    case "active":
      return "Tunnel active";
    case "opening":
      return "Tunnel opening";
    default:
      return "Tunnel closed";
  }
}

function tunnelVariant(tunnel: DatabaseTunnelSnapshot | undefined) {
  if (!tunnel) {
    return "outline" as const;
  }
  if (tunnel.state === "active") {
    return "success" as const;
  }
  if (tunnel.state === "opening") {
    return "warning" as const;
  }
  return "outline" as const;
}

function snapshotState(snapshot: DatabaseConnectionSnapshot | undefined): DatabaseConnectionState {
  return snapshot?.state ?? "disconnected";
}

function RuntimeStat({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">{label}</dt>
      <dd className="mt-0.5 truncate font-mono text-[11px] text-foreground" title={title ?? value}>
        {value}
      </dd>
    </div>
  );
}

function DatabaseConnectionList({
  profiles,
  servers,
  snapshots,
  testResults,
  busy,
  selectedProfileId,
  sshConnected,
  onOpen,
  onConnect,
  onReconnect,
  onDisconnect,
  onTest,
  onEdit,
  onDelete,
  onAdd,
}: DatabaseConnectionListProps) {
  if (profiles.length === 0) {
    return (
      <EmptyState
        icon={<DatabaseIcon className="size-4" aria-hidden="true" />}
        title="No database connections yet"
        description="Add a MySQL or PostgreSQL profile to open a loopback tunnel through the active server's SSH session."
        action={
          <Button size="sm" onClick={onAdd} disabled={!isTauriRuntime()}>
            <PlugsIcon className="size-3.5" aria-hidden="true" />
            Add your first connection
          </Button>
        }
      />
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-[13px] font-semibold tracking-[-0.01em] text-foreground">
            Saved connections
          </h2>
          <p className="text-[11px] text-muted-foreground">
            {profiles.length} profile{profiles.length === 1 ? "" : "s"} for this server. Passwords
            stay in the OS credential store.
          </p>
        </div>
        <Badge variant="outline">{profiles.length}</Badge>
      </div>

      <ul className="space-y-2.5">
        {profiles.map((profile) => {
          const snapshot = snapshots[profile.id];
          const testResult = testResults[profile.id];
          const state = snapshotState(snapshot);
          const isBusy = busy[profile.id] ?? false;
          const connectionError = snapshot?.error ? describeConnectionError(snapshot.error) : null;
          const server = servers.find((entry) => entry.id === profile.serverId);
          const endpoint = formatDatabaseEndpoint(profile.host, profile.port, profile.socket);
          const version = snapshot?.version?.version ?? testResult?.version.version;
          const rawVersion = snapshot?.version?.raw ?? testResult?.version.raw;
          const activeDatabase = snapshot?.databaseName ?? profile.databaseName;
          const tunnel = snapshot?.tunnel;
          const canConnect = state === "disconnected";
          const canReconnect = state !== "disconnected";
          const selected = profile.id === selectedProfileId;

          return (
            <li
              key={profile.id}
              aria-current={selected ? "true" : undefined}
              className={`overflow-hidden rounded-md border bg-card transition-colors ${
                selected ? "border-primary/45 ring-1 ring-primary/20" : "border-border"
              }`}
            >
              <div className="flex flex-col gap-3 px-3.5 py-3 2xl:flex-row 2xl:items-start 2xl:justify-between">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <button
                      type="button"
                      onClick={() => onOpen(profile)}
                      aria-pressed={selected}
                      className="min-w-0 rounded-sm text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                    >
                      <span className="block truncate text-[13px] font-semibold text-foreground">
                        {profile.name}
                      </span>
                    </button>
                    <EngineBadge engine={profile.engine} />
                    <StatusBadge state={state} label={databaseConnectionStateLabel(state)} />
                    <Badge variant={tunnelVariant(tunnel)}>{tunnelLabel(tunnel)}</Badge>
                    {profile.readOnly ? <Badge variant="warning">Read-only</Badge> : null}
                  </div>

                  <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-2 lg:grid-cols-4">
                    <div className="min-w-0">
                      <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                        Server
                      </dt>
                      <dd
                        className="mt-0.5 flex min-w-0 items-center gap-1 text-[11px] text-foreground"
                        title={server ? `${server.name} · ${server.host}` : "Unknown server"}
                      >
                        <HardDrivesIcon
                          className="size-3 shrink-0 text-muted-foreground"
                          aria-hidden="true"
                        />
                        <span className="truncate">{server?.name ?? "Unknown server"}</span>
                      </dd>
                    </div>
                    <RuntimeStat label="Endpoint" value={endpoint} />
                    <RuntimeStat
                      label="Engine version"
                      value={version ?? "Not measured"}
                      title={rawVersion ?? version ?? "Run Test connection to read the version."}
                    />
                    <RuntimeStat
                      label="Latency"
                      value={formatLatency(testResult?.latencyMs)}
                      title={
                        testResult
                          ? `Last test at ${new Date(testResult.testedAt).toLocaleString()}`
                          : "Run Test connection to measure latency."
                      }
                    />
                  </dl>

                  <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-2 border-t border-border pt-2 lg:grid-cols-4">
                    <RuntimeStat label="Active database" value={activeDatabase ?? "No default"} />
                    <RuntimeStat
                      label="Tunnel"
                      value={
                        tunnel?.state === "active" && tunnel.localPort
                          ? `127.0.0.1:${tunnel.localPort}`
                          : tunnelLabel(tunnel)
                      }
                      title={
                        tunnel
                          ? `${tunnel.localHost}:${tunnel.localPort ?? "?"} to ${tunnel.remoteHost}:${tunnel.remotePort}`
                          : "No loopback tunnel is open."
                      }
                    />
                    <RuntimeStat
                      label="Last used"
                      value={
                        profile.lastUsedAt
                          ? new Date(profile.lastUsedAt).toLocaleString()
                          : "Not used yet"
                      }
                    />
                    <RuntimeStat label="SSL mode" value={profile.sslMode ?? "Engine default"} />
                  </dl>

                  {snapshot?.error && connectionError ? (
                    <div className="mt-2.5 rounded-md border border-destructive/30 bg-destructive/8 px-3 py-2">
                      <p className="flex items-center gap-1.5 text-[11px] font-medium text-destructive">
                        <WarningCircleIcon className="size-3.5" aria-hidden="true" />
                        {connectionError.title}
                      </p>
                      <p className="mt-1 text-[11px] leading-5 text-foreground">
                        {snapshot.error.message}
                      </p>
                      <p className="mt-0.5 text-[10px] leading-4 text-muted-foreground">
                        {snapshot.error.details ?? connectionError.action}
                      </p>
                    </div>
                  ) : null}
                </div>

                <div className="flex shrink-0 flex-wrap items-center gap-1 2xl:max-w-64 2xl:justify-end">
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => onOpen(profile)}
                    title="Open this connection in a workspace tab"
                    aria-label={`Open ${profile.name} workspace`}
                  >
                    Open
                    <ArrowRightIcon className="size-3.5" aria-hidden="true" />
                  </Button>
                  {canConnect ? (
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => onConnect(profile)}
                      disabled={isBusy}
                      title={
                        sshConnected
                          ? "Connect through the SSH tunnel"
                          : "Connect the server over SSH first"
                      }
                      aria-label={`Connect ${profile.name}`}
                    >
                      {isBusy ? (
                        <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
                      ) : (
                        <PlugsIcon className="size-3.5" aria-hidden="true" />
                      )}
                      Connect
                    </Button>
                  ) : null}
                  {canReconnect ? (
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => onReconnect(profile)}
                      disabled={isBusy}
                      title="Close the current session and connect again"
                      aria-label={`Reconnect ${profile.name}`}
                    >
                      <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
                      Reconnect
                    </Button>
                  ) : null}
                  {state !== "disconnected" ? (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => onDisconnect(profile)}
                      disabled={isBusy}
                      title="Close the database session and tunnel"
                      aria-label={`Disconnect ${profile.name}`}
                    >
                      <LinkBreakIcon className="size-3.5" aria-hidden="true" />
                      Disconnect
                    </Button>
                  ) : null}
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    onClick={() => onTest(profile)}
                    disabled={isBusy}
                    title="Test the saved profile"
                    aria-label={`Test ${profile.name}`}
                  >
                    <PulseIcon className="size-3.5" aria-hidden="true" />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    onClick={() => onEdit(profile)}
                    disabled={isBusy}
                    title="Edit profile"
                    aria-label={`Edit ${profile.name}`}
                  >
                    <PencilSimpleIcon className="size-3.5" aria-hidden="true" />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    onClick={() => onDelete(profile)}
                    disabled={isBusy}
                    title="Delete profile"
                    aria-label={`Delete ${profile.name}`}
                  >
                    <TrashIcon className="size-3.5" aria-hidden="true" />
                  </Button>
                </div>
              </div>

              {testResult ? (
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-border bg-muted/20 px-3.5 py-1.5 text-[10px] text-muted-foreground">
                  <span className="inline-flex items-center gap-1">
                    <ArrowRightIcon className="size-3" aria-hidden="true" />
                    Last test passed in {formatLatency(testResult.latencyMs)}
                  </span>
                  <span aria-hidden="true">·</span>
                  <span>{testResult.version.raw || testResult.version.version}</span>
                  <span aria-hidden="true">·</span>
                  <span>{testResult.databaseName ?? "No default database"}</span>
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export { DatabaseConnectionList, EngineBadge };
