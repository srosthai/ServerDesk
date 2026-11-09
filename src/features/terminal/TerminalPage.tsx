import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  LinkBreakIcon,
  PlusIcon,
  TerminalWindowIcon,
  WarningCircleIcon,
  XIcon,
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
import { TerminalView } from "@/features/terminal/TerminalView";
import { cn } from "@/lib/utils";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { useServerStore } from "@/stores/serverStore";
import { useTerminalStore, type TerminalSessionState } from "@/stores/terminalStore";
import type { ServerProfile, TerminalExitReason } from "@/types/contracts";

const exitLabels: Record<TerminalExitReason, string> = {
  "remote-exit": "Remote process exited",
  closed: "Session closed",
  disconnected: "Connection lost",
  error: "Terminal error",
};

const exitShortLabels: Record<TerminalExitReason, string> = {
  "remote-exit": "Exited",
  closed: "Closed",
  disconnected: "Disconnected",
  error: "Error",
};

function exitDescription(session: TerminalSessionState): string {
  const reason = session.exitReason ? exitLabels[session.exitReason] : "Terminal session ended";
  if (session.exitReason === "remote-exit" && session.exitCode !== null) {
    return `${reason} with exit code ${session.exitCode}.`;
  }
  return `${reason}.`;
}

function sessionBadge(session: TerminalSessionState, connected: boolean) {
  if (session.status === "open") {
    return connected ? (
      <Badge variant="success">Live</Badge>
    ) : (
      <Badge variant="warning">Offline</Badge>
    );
  }

  const reason = session.exitReason ? exitLabels[session.exitReason] : "Exited";
  const code =
    session.exitReason === "remote-exit" && session.exitCode !== null
      ? ` · code ${session.exitCode}`
      : "";
  const variant =
    session.exitReason === "error" || session.exitReason === "disconnected" ? "warning" : "outline";

  return <Badge variant={variant}>{`${reason}${code}`}</Badge>;
}

function TerminalToolbar({
  servers,
  selected,
  connected,
  opening,
  onSelect,
  onOpen,
}: {
  servers: ServerProfile[];
  selected: ServerProfile | null;
  connected: boolean;
  opening: boolean;
  onSelect: (serverId: string) => void;
  onOpen: () => void;
}) {
  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
      <div className="w-full sm:w-64">
        <label
          htmlFor="terminal-server"
          className="mb-1 block text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground"
        >
          Server
        </label>
        <Select value={selected?.id ?? ""} onValueChange={onSelect} disabled={servers.length === 0}>
          <SelectTrigger id="terminal-server" aria-label="Terminal server">
            <SelectValue placeholder="Select a server" />
          </SelectTrigger>
          <SelectContent>
            {servers.map((server) => (
              <SelectItem key={server.id} value={server.id}>
                {server.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <Button onClick={onOpen} disabled={!selected || !connected || opening}>
        {opening ? (
          <CircleNotchIcon className="animate-spin" aria-hidden="true" />
        ) : (
          <TerminalWindowIcon aria-hidden="true" />
        )}
        {opening ? "Opening" : "Open terminal"}
      </Button>
    </div>
  );
}

function TerminalPage() {
  const servers = useServerStore((state) => state.servers);
  const serverStatus = useServerStore((state) => state.status);
  const fetchServers = useServerStore((state) => state.fetchServers);

  const connections = useConnectionStore((state) => state.connections);
  const connectionBusy = useConnectionStore((state) => state.busy);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const connect = useConnectionStore((state) => state.connect);

  const sessions = useTerminalStore((state) => state.sessions);
  const openingByServer = useTerminalStore((state) => state.openingByServer);
  const activeByServer = useTerminalStore((state) => state.activeByServer);
  const error = useTerminalStore((state) => state.error);
  const openSession = useTerminalStore((state) => state.open);
  const writeInput = useTerminalStore((state) => state.write);
  const resizeSession = useTerminalStore((state) => state.resize);
  const closeSession = useTerminalStore((state) => state.close);
  const selectSession = useTerminalStore((state) => state.select);
  const dismissSession = useTerminalStore((state) => state.dismiss);
  const clearError = useTerminalStore((state) => state.clearError);

  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    if (serverStatus === "idle") {
      void fetchServers();
    }
  }, [fetchServers, serverStatus]);

  useEffect(() => {
    if (serverStatus === "ready" && servers.length > 0) {
      void refreshConnectionStates(servers.map((server) => server.id));
    }
  }, [refreshConnectionStates, serverStatus, servers]);

  const selected = servers.find((server) => server.id === selectedId) ?? servers[0] ?? null;
  const connection = selected ? connections[selected.id] : undefined;
  const connectionState = connection?.state ?? "disconnected";
  const connected = connectionState === "connected";
  const busy = selected ? Boolean(connectionBusy[selected.id]) : false;
  const opening = selected ? Boolean(openingByServer[selected.id]) : false;

  const serverSessions = useMemo(
    () =>
      Object.values(sessions)
        .filter((session) => session.info.serverId === selected?.id)
        .sort((left, right) => left.info.openedAt - right.info.openedAt),
    [selected?.id, sessions],
  );
  const preferredSessionId = activeByServer[selected?.id ?? ""] ?? null;
  const activeSessionId = serverSessions.some(
    (session) => session.info.sessionId === preferredSessionId,
  )
    ? preferredSessionId
    : (serverSessions[0]?.info.sessionId ?? null);
  const activeSession = activeSessionId ? sessions[activeSessionId] : null;
  const connectionLost = Boolean(selected && !connected && serverSessions.length > 0);
  const connectionError = connection?.error?.message ?? null;

  const handleOpen = async () => {
    if (!selected) {
      return;
    }
    await openSession(selected.id);
  };

  const handleSelectSession = (sessionId: string) => {
    if (selected) {
      selectSession(selected.id, sessionId);
    }
  };

  const handleCloseSession = async (sessionId: string) => {
    const session = useTerminalStore.getState().sessions[sessionId];
    if (session?.status === "open") {
      const closed = await closeSession(sessionId);
      if (!closed) {
        return;
      }
    }
    dismissSession(sessionId);
  };

  const handleReconnect = async () => {
    if (!selected) {
      return;
    }
    await connect(selected.id);
  };

  if (!isTauriRuntime()) {
    return (
      <div className="space-y-6">
        <PageHeader
          title="Terminal"
          description="A full interactive shell over the selected SSH session."
        />
        <EmptyState
          title="Desktop application required"
          description="Interactive SSH terminals run in the ServerDesk desktop application, not the browser preview."
          action={
            <Button asChild variant="secondary" size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <PageHeader
        title="Terminal"
        description="Open dedicated xterm.js shells on a connected server. ANSI colors, cursor movement, selection, and scrollback are rendered locally while input and resize events are forwarded to the remote PTY."
      />

      <TerminalToolbar
        servers={servers}
        selected={selected}
        connected={connected}
        opening={opening}
        onSelect={setSelectedId}
        onOpen={() => {
          void handleOpen();
        }}
      />

      {error ? (
        <Alert variant="destructive">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <AlertTitle>Terminal request failed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </div>
          <Button variant="ghost" size="icon-sm" aria-label="Dismiss error" onClick={clearError}>
            <XIcon aria-hidden="true" />
          </Button>
        </Alert>
      ) : null}

      {serverStatus === "loading" && servers.length === 0 ? <LoadingState rows={3} /> : null}
      {serverStatus === "error" && servers.length === 0 ? (
        <ErrorState
          title="Servers could not be loaded"
          message="ServerDesk could not read the local server profiles."
          onRetry={() => {
            void fetchServers();
          }}
        />
      ) : null}

      {servers.length === 0 && serverStatus === "ready" ? (
        <EmptyState
          title="No server profiles yet"
          description="Add a server profile and connect before opening an interactive terminal."
          action={
            <Button asChild variant="secondary" size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      ) : null}

      {connectionLost && selected ? (
        <Alert variant="warning">
          <LinkBreakIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <AlertTitle>
              {connectionState === "reconnecting" ? "Reconnecting" : "Connection lost"}
            </AlertTitle>
            <AlertDescription>
              {connectionError ??
                `${selected.name} is not connected. Existing shells can no longer receive input. Reconnect deliberately before opening another shell.`}
            </AlertDescription>
          </div>
          <Button
            variant="secondary"
            size="sm"
            disabled={
              busy || connectionState === "connecting" || connectionState === "reconnecting"
            }
            onClick={() => {
              void handleReconnect();
            }}
          >
            {busy ? (
              <CircleNotchIcon className="animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon aria-hidden="true" />
            )}
            Reconnect
          </Button>
        </Alert>
      ) : null}

      {selected && !connected && !connectionLost ? (
        <Alert variant="info">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <AlertTitle>Server is not connected</AlertTitle>
            <AlertDescription>
              {connectionError ??
                `Connect to ${selected.name} before opening a terminal. Terminal sessions share the managed SSH connection but use independent channels.`}
            </AlertDescription>
          </div>
          <Button
            variant="secondary"
            size="sm"
            disabled={
              busy || connectionState === "connecting" || connectionState === "reconnecting"
            }
            onClick={() => {
              void handleReconnect();
            }}
          >
            {busy ? (
              <CircleNotchIcon className="animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon aria-hidden="true" />
            )}
            {connectionState === "reconnecting" ? "Reconnecting" : "Connect"}
          </Button>
        </Alert>
      ) : null}

      {selected && (connected || serverSessions.length > 0) ? (
        <section className="overflow-hidden rounded-xl border border-border bg-card shadow-sm">
          <header className="flex flex-wrap items-center gap-3 border-b border-border bg-muted/30 px-4 py-3">
            <div className="grid size-9 shrink-0 place-items-center rounded-md border border-border bg-card text-muted-foreground">
              <TerminalWindowIcon className="size-4" aria-hidden="true" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <h2 className="truncate text-[14px] font-semibold tracking-[-0.01em]">
                  {selected.name}
                </h2>
                <StatusBadge state={connectionState} />
                {activeSession ? sessionBadge(activeSession, connected) : null}
              </div>
              <p className="truncate font-mono text-[11px] leading-5 text-muted-foreground">
                {selected.username}@{selected.host}:{selected.port}
                {activeSession ? ` · ${activeSession.info.term}` : ""}
              </p>
            </div>
            <div className="text-right text-[11px] leading-5 text-muted-foreground">
              <div>
                {serverSessions.length} {serverSessions.length === 1 ? "shell" : "shells"}
              </div>
              <div className="font-mono text-[10px]">
                {activeSession
                  ? `${activeSession.info.cols}×${activeSession.info.rows} cols/rows`
                  : "No session"}
              </div>
            </div>
          </header>

          <div className="flex items-stretch border-b border-[#1b241f] bg-[#060907]">
            <div
              role="tablist"
              aria-label={`Terminal sessions for ${selected.name}`}
              className="flex min-w-0 flex-1 overflow-x-auto"
            >
              {serverSessions.length === 0 ? (
                <span className="flex items-center px-4 py-2 text-[11px] text-[#8b9a91]">
                  No open sessions
                </span>
              ) : (
                serverSessions.map((session, index) => {
                  const active = session.info.sessionId === activeSessionId;
                  const label = `Shell ${index + 1}`;
                  return (
                    <div
                      key={session.info.sessionId}
                      className={cn(
                        "flex shrink-0 items-stretch border-r border-white/10",
                        active ? "bg-[#000000] shadow-[inset_0_2px_0_#ff8a4c]" : "hover:bg-white/5",
                      )}
                    >
                      <button
                        type="button"
                        role="tab"
                        id={`terminal-tab-${session.info.sessionId}`}
                        aria-selected={active}
                        aria-controls={`terminal-panel-${session.info.sessionId}`}
                        tabIndex={active ? 0 : -1}
                        className="flex min-w-[138px] items-center gap-2 px-3 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-[#ff8a4c]/60"
                        onClick={() => handleSelectSession(session.info.sessionId)}
                      >
                        <span
                          aria-hidden="true"
                          className={cn(
                            "size-1.5 shrink-0 rounded-full",
                            session.status === "open"
                              ? connected
                                ? "bg-[#63d471]"
                                : "bg-[#f0b04a]"
                              : session.exitReason === "error" ||
                                  session.exitReason === "disconnected"
                                ? "bg-[#f0b04a]"
                                : "bg-[#7d8d84]",
                          )}
                        />
                        <span className="min-w-0">
                          <span
                            className={cn(
                              "block truncate text-[11px] font-medium",
                              active ? "text-[#f1f5f2]" : "text-[#dce6df]",
                            )}
                          >
                            {label}
                          </span>
                          <span className="block font-mono text-[10px] leading-4 text-[#8b9a91]">
                            {session.status === "open"
                              ? `${session.info.cols}×${session.info.rows}`
                              : session.exitReason
                                ? exitShortLabels[session.exitReason]
                                : "Exited"}
                          </span>
                        </span>
                      </button>
                      <button
                        type="button"
                        aria-label={`Close ${label}`}
                        className="my-1 mr-1 grid w-6 place-items-center rounded-sm text-[#8b9a91] outline-none transition-colors hover:bg-white/10 hover:text-[#f1f5f2] focus-visible:ring-2 focus-visible:ring-[#ff8a4c]/60"
                        onClick={() => {
                          void handleCloseSession(session.info.sessionId);
                        }}
                      >
                        <XIcon className="size-3" aria-hidden="true" />
                      </button>
                    </div>
                  );
                })
              )}
            </div>
            <button
              type="button"
              className="flex shrink-0 items-center gap-1.5 border-l border-white/10 px-3 text-[11px] font-medium text-[#dce6df] outline-none transition-colors hover:bg-white/10 hover:text-[#f1f5f2] focus-visible:ring-2 focus-visible:ring-[#ff8a4c]/60 disabled:pointer-events-none disabled:opacity-40"
              disabled={!connected || opening}
              onClick={() => {
                void handleOpen();
              }}
              title={
                connected ? "Open another shell on this server" : "Reconnect before opening a shell"
              }
            >
              {opening ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <PlusIcon className="size-3.5" aria-hidden="true" />
              )}
              New shell
            </button>
          </div>

          <div className="relative h-[clamp(320px,52dvh,560px)]">
            {serverSessions.length === 0 ? (
              <div className="flex h-full items-center justify-center bg-[#000000] px-6">
                <div className="max-w-[48ch] text-center">
                  <DiagnosticMark />
                  <p className="mt-4 text-[13px] text-[#eaf1ec]">No terminal session is open.</p>
                  <p className="mt-1 text-[12px] leading-5 text-[#8b9a91]">
                    Open a terminal to start a shell as {selected.username} on {selected.host}. Each
                    tab owns an independent SSH channel and scrollback buffer.
                  </p>
                  <Button
                    variant="secondary"
                    size="sm"
                    className="mt-4"
                    disabled={!connected || opening}
                    onClick={() => {
                      void handleOpen();
                    }}
                  >
                    <PlusIcon aria-hidden="true" />
                    Open terminal
                  </Button>
                </div>
              </div>
            ) : (
              serverSessions.map((session) => {
                const active = session.info.sessionId === activeSessionId;
                return (
                  <TerminalView
                    key={session.info.sessionId}
                    id={`terminal-panel-${session.info.sessionId}`}
                    labelledBy={`terminal-tab-${session.info.sessionId}`}
                    session={session}
                    active={active}
                    inputEnabled={session.status === "open" && connected}
                    onInput={(data) => {
                      void writeInput(session.info.sessionId, data);
                    }}
                    onResize={(size) => {
                      void resizeSession(session.info.sessionId, size);
                    }}
                  />
                );
              })
            )}
          </div>

          {activeSession && activeSession.status === "exited" ? (
            <div className="flex flex-wrap items-center gap-2 border-t border-[#1b241f] bg-[#060907] px-4 py-2.5 text-[12px] text-[#a2b0a7]">
              <span className="size-1.5 rounded-full bg-[#f0b04a]" aria-hidden="true" />
              <span className="min-w-0 flex-1">
                {exitDescription(activeSession)}
                {activeSession.message ? ` ${activeSession.message}` : ""}
              </span>
              <Button
                variant="outline"
                size="sm"
                className="border-white/15 bg-transparent text-[#f1f5f2] hover:border-white/25 hover:bg-white/10"
                disabled={!connected || opening}
                onClick={() => {
                  void handleOpen();
                }}
              >
                Open new shell
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="text-[#a2b0a7] hover:bg-white/10 hover:text-[#f1f5f2]"
                onClick={() => dismissSession(activeSession.info.sessionId)}
              >
                Dismiss
              </Button>
            </div>
          ) : null}

          {activeSession?.error ? (
            <div
              className="border-t border-[#1b241f] bg-[#060907] px-4 py-2 text-[12px] text-[#ff6b6b]"
              role="alert"
            >
              {activeSession.error}
            </div>
          ) : null}
        </section>
      ) : null}

      <p className="max-w-[90ch] text-[11px] leading-5 text-muted-foreground">
        The terminal renders full ANSI colors, cursor movement, mouse selection, and a 5,000-line
        scrollback locally. Copy uses Ctrl+Shift+C, paste uses Ctrl+Shift+V, and select all uses
        Ctrl+Shift+A. Terminal input is forwarded directly and never stored in application state or
        logs.
      </p>
    </div>
  );
}

function DiagnosticMark() {
  return (
    <div className="mx-auto flex w-fit items-center gap-1.5" aria-hidden="true">
      <span className="font-mono text-[11px] text-[#63d471]">deploy@server</span>
      <span className="font-mono text-[11px] text-[#8b9a91]">$</span>
      <span className="h-3.5 w-1.5 animate-pulse bg-[#ff8a4c]" />
    </div>
  );
}

export { TerminalPage };
