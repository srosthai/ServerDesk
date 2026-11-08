import { useState } from "react";
import {
  CircleNotchIcon,
  HeartbeatIcon,
  LinkBreakIcon,
  PlugsIcon,
  TerminalWindowIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { OutputPanel } from "@/components/ui/output-panel";
import { StatusBadge } from "@/components/ui/status-badge";
import { HostKeyVerificationDialog } from "@/features/servers/HostKeyDialogs";
import { MonitoringCard } from "@/features/dashboard/MonitoringCard";
import { ServerIdentityCard } from "@/features/dashboard/ServerIdentityCard";
import { describeConnectionError } from "@/lib/connectionErrors";
import { MIN_MONITORING_INTERVAL_SECONDS } from "@/lib/preferences";
import { isTauriRuntime } from "@/lib/tauri";
import { remoteCommands } from "@/services/remote";
import { useConnectionStore } from "@/stores/connectionStore";
import { useMonitoringStore } from "@/stores/monitoringStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useRemoteCommandStore } from "@/stores/remoteCommandStore";
import { useSystemIdentityStore } from "@/stores/systemIdentityStore";
import type { RemoteCommandName, RemoteCommandResult, ServerProfile } from "@/types/contracts";

type ServerInspectionPanelProps = {
  server: ServerProfile;
};

const commandDescriptions: Record<RemoteCommandName, string> = {
  hostname: "Read the remote host name.",
  uptime: "Read remote uptime and load average.",
};

function formatDuration(durationMs: number) {
  return durationMs < 1000 ? `${durationMs} ms` : `${(durationMs / 1000).toFixed(1)} s`;
}

function formatSampleTime(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeValue(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function OverviewSummary({ serverId, connected }: { serverId: string; connected: boolean }) {
  const monitoringEntry = useMonitoringStore((state) => state.entries[serverId]);
  const identity = useSystemIdentityStore((state) => state.identities[serverId]);
  const identityStale = useSystemIdentityStore((state) => state.stale[serverId] ?? false);
  const identityPending = useSystemIdentityStore((state) => state.pending[serverId] ?? false);
  const refreshIdentity = useSystemIdentityStore((state) => state.refresh);
  const refreshMonitoring = useMonitoringStore((state) => state.refresh);
  const intervalSeconds = usePreferencesStore(
    (state) => state.preferences.monitoringIntervalSeconds,
  );

  const monitoringPending = monitoringEntry?.pending ?? false;
  const monitoringStale = monitoringEntry?.stale ?? false;
  const pending = identityPending || monitoringPending;
  const sampleTimes = [identity?.sampledAt, monitoringEntry?.snapshot?.sampledAt].filter(
    (value): value is number => typeof value === "number" && Number.isFinite(value),
  );
  const lastSampleAt = sampleTimes.length > 0 ? Math.max(...sampleTimes) : null;
  const stale = monitoringStale || identityStale || (!connected && lastSampleAt !== null);
  const safeIntervalSeconds = Math.max(MIN_MONITORING_INTERVAL_SECONDS, intervalSeconds);

  const state = !connected
    ? lastSampleAt === null
      ? { label: "Unavailable", variant: "outline" as const }
      : { label: "Stale", variant: "warning" as const }
    : stale
      ? { label: pending ? "Stale · refreshing" : "Stale", variant: "warning" as const }
      : pending
        ? {
            label: lastSampleAt === null ? "Collecting" : "Refreshing",
            variant: "info" as const,
          }
        : lastSampleAt === null
          ? { label: "No sample", variant: "outline" as const }
          : { label: "Live", variant: "success" as const };

  async function handleRefresh() {
    await Promise.all([refreshIdentity(serverId), refreshMonitoring(serverId)]);
  }

  return (
    <div className="flex flex-col gap-3 rounded-md border border-border bg-muted/25 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
      <dl className="grid flex-1 grid-cols-2 gap-x-6 gap-y-2 md:grid-cols-3">
        <div>
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Data freshness
          </dt>
          <dd className="mt-0.5" aria-live="polite">
            <Badge variant={state.variant}>{state.label}</Badge>
          </dd>
        </div>
        <div>
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Last successful sample
          </dt>
          <dd className="mt-1 font-mono text-[11px] text-foreground">
            {lastSampleAt === null ? (
              <span className="font-sans text-muted-foreground">No successful sample yet</span>
            ) : (
              <time dateTime={dateTimeValue(lastSampleAt)}>{formatSampleTime(lastSampleAt)}</time>
            )}
          </dd>
        </div>
        <div>
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Auto refresh
          </dt>
          <dd className="mt-1 text-[11px] text-foreground">
            {connected ? `Every ${safeIntervalSeconds} s` : "Paused while disconnected"}
          </dd>
        </div>
      </dl>
      <Button
        variant="secondary"
        size="sm"
        onClick={() => void handleRefresh()}
        disabled={!connected || pending}
        aria-label="Refresh server overview"
        className="shrink-0"
      >
        {pending ? (
          <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
        ) : (
          <HeartbeatIcon className="size-3.5" aria-hidden="true" />
        )}
        {pending ? "Refreshing..." : "Refresh all"}
      </Button>
    </div>
  );
}

function RemoteCommandResultCard({
  result,
  pending,
}: {
  result: RemoteCommandResult;
  pending: boolean;
}) {
  return (
    <article
      aria-label={`${result.command} result`}
      className="overflow-hidden rounded-md border border-border bg-background"
    >
      <header className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
        <code className="font-mono text-[12px] font-medium text-foreground">{result.template}</code>
        <Badge variant={result.succeeded ? "success" : "danger"}>
          {result.exitCode === null ? "No exit status" : `Exit ${result.exitCode}`}
        </Badge>
        <span className="text-[11px] text-muted-foreground">
          {formatDuration(result.durationMs)}
        </span>
        {pending ? (
          <span className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <CircleNotchIcon className="size-3 animate-spin" aria-hidden="true" />
            Running again
          </span>
        ) : null}
      </header>
      <div className="space-y-2 p-3">
        <OutputPanel
          title="stdout"
          content={result.stdout}
          emptyText="(empty)"
          maxHeightClassName="max-h-40"
        />
        <OutputPanel
          title="stderr"
          content={result.stderr}
          emptyText="(empty)"
          maxHeightClassName="max-h-40"
        />
      </div>
    </article>
  );
}

function ServerInspectionPanel({ server }: ServerInspectionPanelProps) {
  const connection = useConnectionStore((state) => state.connections[server.id]);
  const connectionBusy = useConnectionStore((state) => state.busy[server.id] ?? false);
  const connectionError = useConnectionStore((state) => state.error);
  const connect = useConnectionStore((state) => state.connect);
  const disconnect = useConnectionStore((state) => state.disconnect);
  const trustHostKey = useConnectionStore((state) => state.trustHostKey);
  const rejectHostKey = useConnectionStore((state) => state.rejectHostKey);
  const clearConnectionError = useConnectionStore((state) => state.clearError);

  const results = useRemoteCommandStore((state) => state.results[server.id]);
  const pendingCommand = useRemoteCommandStore((state) => state.pending[server.id] ?? null);
  const remoteError = useRemoteCommandStore((state) => state.error);
  const runCommand = useRemoteCommandStore((state) => state.run);
  const clearServer = useRemoteCommandStore((state) => state.clearServer);
  const clearRemoteError = useRemoteCommandStore((state) => state.clearError);
  const clearIdentity = useSystemIdentityStore((state) => state.clearServer);

  const [dismissedConnectionError, setDismissedConnectionError] = useState(false);

  const state = connection?.state ?? "disconnected";
  const connected = state === "connected";
  const canConnect = state === "disconnected" || state === "error";
  const pendingHostKey =
    state === "host-key-verification" && connection?.hostKey ? connection.hostKey : null;
  const connectionErrorPresentation = connection?.error
    ? describeConnectionError(connection.error)
    : null;

  async function handleConnect() {
    setDismissedConnectionError(false);
    await connect(server.id);
  }

  async function handleDisconnect() {
    const disconnected = await disconnect(server.id);
    if (disconnected) {
      clearServer(server.id);
      clearIdentity(server.id);
    }
  }

  async function handleRun(command: RemoteCommandName) {
    await runCommand(server.id, command);
  }

  return (
    <>
      <section className="overflow-hidden rounded-md border border-border bg-card">
        <div className="flex flex-col gap-4 p-4 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0 space-y-1.5">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="truncate text-[15px] font-semibold text-foreground">{server.name}</h2>
              <StatusBadge state={state} />
            </div>
            <p className="break-all font-mono text-[12px] text-muted-foreground">
              {server.username}@{server.host}:{server.port}
            </p>
            <p className="text-[11px] text-muted-foreground">
              {state === "connected"
                ? "The authenticated SSH session is ready for fixed remote commands."
                : "Connect to verify the host key, authenticate, and open a session."}
            </p>
          </div>

          <div className="flex shrink-0 flex-wrap gap-2">
            {!isTauriRuntime() ? (
              <Badge variant="outline">Desktop only</Badge>
            ) : canConnect ? (
              <Button
                size="sm"
                onClick={() => void handleConnect()}
                disabled={connectionBusy}
                aria-label={`Connect ${server.name}`}
              >
                {connectionBusy ? (
                  <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
                ) : (
                  <PlugsIcon className="size-3.5" aria-hidden="true" />
                )}
                {connectionBusy ? "Connecting..." : "Connect"}
              </Button>
            ) : (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void handleDisconnect()}
                disabled={connectionBusy}
                aria-label={`Disconnect ${server.name}`}
              >
                <LinkBreakIcon className="size-3.5" aria-hidden="true" />
                {state === "connected" ? "Disconnect" : "Cancel connection"}
              </Button>
            )}
          </div>
        </div>

        {connection?.error && connectionErrorPresentation ? (
          <div className="px-4 pb-4">
            <Alert variant="destructive" className="py-2.5">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>{connectionErrorPresentation.title}</AlertTitle>
                <AlertDescription>
                  <p>{connection.error.message}</p>
                  {connection.error.details ? (
                    <p className="mt-1">{connection.error.details}</p>
                  ) : null}
                  <p className="mt-1">{connectionErrorPresentation.action}</p>
                </AlertDescription>
              </div>
            </Alert>
          </div>
        ) : null}

        {!connection?.error && connectionError && !dismissedConnectionError ? (
          <div className="px-4 pb-4">
            <Alert variant="destructive" className="py-2.5">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Connection action failed</AlertTitle>
                <AlertDescription>{connectionError}</AlertDescription>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setDismissedConnectionError(true);
                  clearConnectionError();
                }}
              >
                Dismiss
              </Button>
            </Alert>
          </div>
        ) : null}

        <div className="border-t border-border px-4 py-3">
          <OverviewSummary serverId={server.id} connected={connected} />
        </div>

        <div className="border-t border-border px-4 py-4">
          <ServerIdentityCard serverId={server.id} connected={connected} />
        </div>

        <div className="border-t border-border px-4 py-4">
          <MonitoringCard serverId={server.id} connected={connected} />
        </div>

        <div className="border-t border-border px-4 py-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div>
              <h3 className="flex items-center gap-1.5 text-[13px] font-semibold text-foreground">
                <TerminalWindowIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
                Remote commands
              </h3>
              <p className="mt-0.5 text-[11px] text-muted-foreground">
                Fixed, read-only commands run over the authenticated SSH session. No shell input is
                built from user text.
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              {remoteCommands.map((command) => (
                <Button
                  key={command}
                  variant="secondary"
                  size="sm"
                  onClick={() => void handleRun(command)}
                  disabled={!connected || pendingCommand !== null}
                  title={commandDescriptions[command]}
                  aria-label={`Run ${command}`}
                >
                  {pendingCommand === command ? (
                    <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
                  ) : (
                    <TerminalWindowIcon className="size-3.5" aria-hidden="true" />
                  )}
                  {pendingCommand === command ? `Running ${command}...` : `Run ${command}`}
                </Button>
              ))}
            </div>
          </div>

          {remoteError ? (
            <Alert variant="destructive" className="mt-3 py-2.5">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Remote command failed</AlertTitle>
                <AlertDescription>{remoteError}</AlertDescription>
              </div>
              <Button variant="ghost" size="sm" onClick={clearRemoteError}>
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {!connected ? (
            <p className="mt-3 rounded-md border border-dashed border-border px-3 py-2 text-[11px] text-muted-foreground">
              Connect the server to run hostname and uptime. The buttons stay disabled until the
              authenticated session is ready.
            </p>
          ) : null}

          {results ? (
            <div className="mt-3 grid gap-3 xl:grid-cols-2">
              {remoteCommands.map((command) => {
                const result = results[command];
                if (!result) {
                  return null;
                }
                return (
                  <RemoteCommandResultCard
                    key={command}
                    result={result}
                    pending={pendingCommand === command}
                  />
                );
              })}
            </div>
          ) : connected ? (
            <p className="mt-3 text-[11px] text-muted-foreground">
              Run hostname or uptime to see real remote stdout, stderr, and the exit code.
            </p>
          ) : null}
        </div>
      </section>

      {pendingHostKey ? (
        <HostKeyVerificationDialog
          serverName={server.name}
          host={`${server.host}:${server.port}`}
          hostKey={pendingHostKey}
          pending={connectionBusy}
          onTrust={() =>
            void trustHostKey(server.id, pendingHostKey.keyType, pendingHostKey.fingerprint)
          }
          onReject={() =>
            void rejectHostKey(server.id, pendingHostKey.keyType, pendingHostKey.fingerprint)
          }
        />
      ) : null}
    </>
  );
}

export { ServerInspectionPanel };
