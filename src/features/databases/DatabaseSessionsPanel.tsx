import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  InfoIcon,
  StopIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import {
  DatabaseSessionActionDialog,
  type DatabaseSessionIntent,
} from "@/features/databases/DatabaseSessionDialogs";
import {
  SESSION_POLL_INTERVAL_MS,
  SESSION_POLL_MAX_TICKS,
  sessionActionCapability,
  sessionActionLabel,
  sessionApplicationLabel,
  sessionCapabilitiesFor,
  sessionDatabaseLabel,
  sessionElapsedLabel,
  sessionHostLabel,
  sessionPollingExhausted,
  sessionQueryText,
  sessionStateLabel,
  sessionWaitEventLabel,
  type SessionCapabilities,
} from "@/features/databases/databaseSessionsModel";
import { cn } from "@/lib/utils";
import { useDatabaseSessionStore } from "@/stores/databaseSessionStore";
import type {
  CapabilityFlags,
  DatabaseAction,
  DatabaseEngine,
  DatabaseSessionActionKind,
  DatabaseSessionSummary,
} from "@/types/contracts";

type DatabaseSessionsPanelProps = {
  profileId: string | null;
  profileName: string;
  engine: DatabaseEngine;
  version?: string | null;
  connected: boolean;
  readOnly: boolean;
  actions?: DatabaseAction[];
  flags?: CapabilityFlags;
};

function UnavailableText({ children }: { children: string }) {
  return <span className="text-muted-foreground/70">{children}</span>;
}

function MetadataCell({ value, title }: { value: string; title?: string }) {
  if (value === "Unavailable") {
    return <UnavailableText>{value}</UnavailableText>;
  }
  return (
    <span className="text-foreground" title={title}>
      {value}
    </span>
  );
}

function actionDisabledReason(
  action: DatabaseSessionActionKind,
  session: DatabaseSessionSummary,
  capabilities: SessionCapabilities,
  pending: boolean,
): string | null {
  const decision = capabilities[sessionActionCapability(action)];
  if (pending) {
    return "Another session action is still running.";
  }
  if (session.isCurrent) {
    return "ServerDesk will not target its own database connection.";
  }
  if (!decision.enabled) {
    return decision.reason ?? `${sessionActionLabel(action)} is unavailable.`;
  }
  if (action === "cancel_query" && session.query.status !== "available") {
    return "This session is not running a query.";
  }
  return null;
}

function DatabaseSessionsPanel({
  profileId,
  profileName,
  engine,
  version,
  connected,
  readOnly,
  actions,
  flags,
}: DatabaseSessionsPanelProps) {
  const entry = useDatabaseSessionStore((state) =>
    profileId ? state.entries[profileId] : undefined,
  );
  const fetchSessions = useDatabaseSessionStore((state) => state.fetchSessions);
  const setRunningOnly = useDatabaseSessionStore((state) => state.setRunningOnly);
  const refresh = useDatabaseSessionStore((state) => state.refresh);
  const executeAction = useDatabaseSessionStore((state) => state.executeAction);

  const [intent, setIntent] = useState<DatabaseSessionIntent | null>(null);
  const [autoRefresh, setAutoRefresh] = useState(false);
  const pollTicks = useRef(0);

  const capabilities = useMemo(
    () =>
      sessionCapabilitiesFor({
        engine,
        version,
        connected,
        readOnly,
        actions,
        flags,
      }),
    [actions, connected, engine, flags, readOnly, version],
  );

  const sessions = entry?.sessions ?? [];
  const runningOnly = entry?.runningOnly ?? false;
  const loading = entry?.status === "loading" || entry?.status === "idle";
  const refreshing = entry?.refreshing ?? false;
  const actionPending = entry?.actionStatus === "running";
  const lastResult = entry?.lastResult ?? null;
  const error = entry?.error ?? null;

  useEffect(() => {
    if (profileId && connected && capabilities.browse.enabled) {
      void fetchSessions(profileId);
    }
  }, [capabilities.browse.enabled, connected, fetchSessions, profileId]);

  useEffect(() => {
    pollTicks.current = 0;
  }, [profileId, runningOnly]);

  useEffect(() => {
    if (!autoRefresh || !profileId || !connected || !capabilities.browse.enabled) {
      return;
    }
    const interval = window.setInterval(() => {
      if (typeof document !== "undefined" && document.visibilityState !== "visible") {
        return;
      }
      pollTicks.current += 1;
      if (sessionPollingExhausted(pollTicks.current)) {
        setAutoRefresh(false);
        return;
      }
      void refresh(profileId);
    }, SESSION_POLL_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [autoRefresh, capabilities.browse.enabled, connected, profileId, refresh]);

  if (!connected) {
    return (
      <Alert variant="warning">
        <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div>
          <AlertTitle>Connection required</AlertTitle>
          <AlertDescription>
            Connect {profileName} before listing active sessions or running queries.
          </AlertDescription>
        </div>
      </Alert>
    );
  }

  if (!capabilities.browse.enabled) {
    return (
      <Alert variant="warning">
        <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div>
          <AlertTitle>Session monitoring is unavailable</AlertTitle>
          <AlertDescription>
            {capabilities.browse.reason ??
              "This engine or server version does not expose active sessions."}
          </AlertDescription>
        </div>
      </Alert>
    );
  }

  function openIntent(action: DatabaseSessionActionKind, session: DatabaseSessionSummary): void {
    setIntent({ action, session });
  }

  async function handleConfirm(confirmation: string): Promise<void> {
    if (!intent || !profileId) {
      return;
    }
    try {
      await executeAction(profileId, {
        sessionId: intent.session.id,
        action: intent.action,
        expectedUser: intent.session.user,
        expectedDatabase:
          intent.session.database.status === "available" ? intent.session.database.value : null,
        confirmation,
      });
      setIntent(null);
    } catch {
      // The store keeps a typed error and the dialog renders it. Keep the
      // dialog open so the operator can retry or cancel deliberately.
    }
  }

  function renderActionButton(
    action: DatabaseSessionActionKind,
    session: DatabaseSessionSummary,
  ): ReactNode {
    const reason = actionDisabledReason(action, session, capabilities, actionPending);
    const isCancel = action === "cancel_query";
    return (
      <Button
        key={action}
        type="button"
        variant={isCancel ? "secondary" : "destructive"}
        size="sm"
        className="h-6 px-2 text-[10px]"
        disabled={reason !== null}
        title={reason ?? undefined}
        onClick={() => openIntent(action, session)}
      >
        {isCancel ? (
          <XIcon className="size-3" aria-hidden="true" />
        ) : (
          <StopIcon className="size-3" aria-hidden="true" />
        )}
        {sessionActionLabel(action)}
      </Button>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-1.5">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="h-7 px-2 text-[11px]"
            onClick={() => profileId && void refresh(profileId)}
            disabled={refreshing || loading}
          >
            {refreshing ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {refreshing ? "Refreshing..." : "Refresh"}
          </Button>
          <Button
            type="button"
            variant={autoRefresh ? "default" : "secondary"}
            size="sm"
            className="h-7 px-2 text-[11px]"
            aria-pressed={autoRefresh}
            onClick={() => {
              pollTicks.current = 0;
              setAutoRefresh((current) => !current);
            }}
          >
            Auto-refresh {autoRefresh ? "on" : "off"}
          </Button>
          <Button
            type="button"
            variant={runningOnly ? "default" : "secondary"}
            size="sm"
            className="h-7 px-2 text-[11px]"
            aria-pressed={runningOnly}
            onClick={() => profileId && void setRunningOnly(profileId, !runningOnly)}
          >
            Running only
          </Button>
          {autoRefresh ? (
            <span className="text-[10px] text-muted-foreground">
              5 s interval · stops after {SESSION_POLL_MAX_TICKS} refreshes
            </span>
          ) : null}
        </div>
        <div className="text-[10px] text-muted-foreground">
          {entry?.lastRefreshedAt
            ? `Last refreshed ${new Date(entry.lastRefreshedAt).toLocaleTimeString()}`
            : "Not refreshed yet"}
        </div>
      </div>

      {readOnly ? (
        <Alert variant="warning" className="py-2.5">
          <InfoIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div>
            <AlertTitle>Read-only profile</AlertTitle>
            <AlertDescription>
              Sessions can be inspected, but cancelling queries and terminating sessions are
              disabled for this connection profile.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {error ? (
        <Alert variant="warning" className="py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div>
            <AlertTitle>The session list could not be refreshed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </div>
        </Alert>
      ) : null}

      {lastResult ? (
        <Alert variant={lastResult.verified ? "success" : "warning"} className="py-2.5">
          {lastResult.verified ? (
            <InfoIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          ) : (
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          )}
          <div>
            <AlertTitle>
              {lastResult.verified
                ? `${sessionActionLabel(lastResult.action)} verified`
                : `${sessionActionLabel(lastResult.action)} finished but was not verified`}
            </AlertTitle>
            <AlertDescription>
              {lastResult.verified
                ? lastResult.verification
                : (lastResult.verificationError ??
                  `${lastResult.verification} Refresh the list and review the session before retrying.`)}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      <Alert variant="info" className="py-2.5">
        <InfoIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div>
          <AlertTitle>Visibility depends on database privileges</AlertTitle>
          <AlertDescription>
            MySQL accounts without PROCESS and PostgreSQL accounts without pg_read_all_stats may see
            only their own sessions. Query text is redacted before it is displayed.
          </AlertDescription>
        </div>
      </Alert>

      {loading && sessions.length === 0 ? (
        <div className="space-y-2">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-3/4" />
        </div>
      ) : sessions.length === 0 ? (
        <EmptyState
          icon={<InfoIcon className="size-4" aria-hidden="true" />}
          title={runningOnly ? "No running queries" : "No active sessions"}
          description={
            runningOnly
              ? "No session currently reports a running query that this account can see."
              : "The database server did not report any active sessions visible to this account."
          }
        />
      ) : (
        <div className="overflow-x-auto rounded-md border border-border">
          <table className="w-full min-w-[980px] text-left text-[11px]">
            <thead className="bg-muted/40 text-muted-foreground">
              <tr>
                <th className="px-2 py-1.5 font-medium">Session</th>
                <th className="px-2 py-1.5 font-medium">Account</th>
                <th className="px-2 py-1.5 font-medium">Client</th>
                <th className="px-2 py-1.5 font-medium">Database</th>
                <th className="px-2 py-1.5 font-medium">Application</th>
                <th className="px-2 py-1.5 font-medium">State</th>
                <th className="px-2 py-1.5 font-medium">Elapsed</th>
                <th className="px-2 py-1.5 font-medium">Query</th>
                <th className="px-2 py-1.5 font-medium">Actions</th>
              </tr>
            </thead>
            <tbody>
              {sessions.map((session) => {
                const query = session.query.status === "available" ? session.query.value : null;
                const queryAvailable = query !== null;
                return (
                  <tr
                    key={`${session.id}:${session.user}`}
                    className="border-t border-border align-top"
                  >
                    <td className="px-2 py-1.5 font-mono text-foreground">
                      <span className="flex flex-wrap items-center gap-1">
                        {session.id}
                        {session.isCurrent ? <Badge variant="outline">Current</Badge> : null}
                      </span>
                      {session.backendType.status === "available" ? (
                        <span className="block text-[10px] text-muted-foreground">
                          {session.backendType.value}
                        </span>
                      ) : null}
                    </td>
                    <td className="px-2 py-1.5 font-mono text-foreground">{session.user}</td>
                    <td className="px-2 py-1.5 font-mono">
                      <MetadataCell value={sessionHostLabel(session)} />
                      {session.connectedAt.status === "available" ? (
                        <span className="block text-[10px] text-muted-foreground">
                          since {session.connectedAt.value}
                        </span>
                      ) : null}
                    </td>
                    <td className="px-2 py-1.5 font-mono">
                      <MetadataCell value={sessionDatabaseLabel(session)} />
                    </td>
                    <td className="px-2 py-1.5">
                      <MetadataCell value={sessionApplicationLabel(session)} />
                    </td>
                    <td className="px-2 py-1.5">
                      <MetadataCell value={sessionStateLabel(session)} />
                      {session.waitEvent.status === "available" ? (
                        <span className="block text-[10px] text-muted-foreground">
                          wait: {session.waitEvent.value}
                        </span>
                      ) : (
                        <span className="block text-[10px] text-muted-foreground/70">
                          wait: {sessionWaitEventLabel(session)}
                        </span>
                      )}
                    </td>
                    <td
                      className={cn(
                        "px-2 py-1.5 font-mono",
                        queryAvailable ? "text-foreground" : "text-muted-foreground/70",
                      )}
                    >
                      {sessionElapsedLabel(session)}
                    </td>
                    <td className="max-w-[360px] px-2 py-1.5">
                      <span
                        className={cn(
                          "block break-words font-mono",
                          queryAvailable
                            ? "line-clamp-3 text-foreground"
                            : "text-muted-foreground/70",
                        )}
                        title={query?.text}
                      >
                        {sessionQueryText(session)}
                      </span>
                      {query && (query.redacted || query.truncated) ? (
                        <span className="mt-1 flex flex-wrap gap-1">
                          {query.redacted ? <Badge variant="warning">Redacted</Badge> : null}
                          {query.truncated ? <Badge variant="outline">Truncated</Badge> : null}
                        </span>
                      ) : null}
                    </td>
                    <td className="px-2 py-1.5">
                      <div className="flex flex-wrap gap-1">
                        {renderActionButton("cancel_query", session)}
                        {renderActionButton("terminate_session", session)}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      <p className="text-[10px] text-muted-foreground">
        Fields the selected engine does not expose are shown as Unavailable. Elapsed time is only
        shown where the engine reports a running-query start time or duration.
      </p>

      <DatabaseSessionActionDialog
        key={intent ? `${intent.action}:${intent.session.id}` : "none"}
        intent={intent}
        pending={actionPending}
        error={entry?.actionError ?? null}
        onOpenChange={(open) => {
          if (!open) {
            setIntent(null);
          }
        }}
        onConfirm={(confirmation) => void handleConfirm(confirmation)}
      />
    </div>
  );
}

export { DatabaseSessionsPanel };
