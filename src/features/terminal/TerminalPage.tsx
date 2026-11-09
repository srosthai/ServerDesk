import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
  CircleNotchIcon,
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
import {
  measureTerminalSize,
  stripAnsi,
  terminalKeySequence,
} from "@/features/terminal/terminalView";
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

function sessionBadge(session: TerminalSessionState) {
  if (session.status === "open") {
    return <Badge variant="success">Live</Badge>;
  }
  const reason = session.exitReason ? exitLabels[session.exitReason] : "Exited";
  const code =
    session.exitReason === "remote-exit" && session.exitCode !== null
      ? ` · code ${session.exitCode}`
      : "";
  return <Badge variant="outline">{`${reason}${code}`}</Badge>;
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
  const outputRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (serverStatus === "idle") {
      void fetchServers();
    }
  }, [fetchServers, serverStatus]);

  const selected = servers.find((server) => server.id === selectedId) ?? servers[0] ?? null;
  const connected = selected ? connections[selected.id]?.state === "connected" : false;
  const serverSessions = useMemo(
    () =>
      Object.values(sessions)
        .filter((session) => session.info.serverId === selected?.id)
        .sort((left, right) => left.info.openedAt - right.info.openedAt),
    [selected?.id, sessions],
  );
  const activeSessionId =
    activeByServer[selected?.id ?? ""] ?? serverSessions[0]?.info.sessionId ?? null;
  const activeSession = activeSessionId ? sessions[activeSessionId] : null;
  const preview = useMemo(
    () => (activeSession ? stripAnsi(activeSession.output) : ""),
    [activeSession],
  );

  useEffect(() => {
    const element = outputRef.current;
    if (element) {
      element.scrollTop = element.scrollHeight;
    }
  }, [preview]);

  useEffect(() => {
    if (activeSession?.status === "open") {
      inputRef.current?.focus();
    }
  }, [activeSession?.status, activeSessionId]);

  useEffect(() => {
    const element = outputRef.current;
    if (!element || !activeSessionId || activeSession?.status !== "open") {
      return;
    }

    let lastSize = "";
    const resize = () => {
      const size = measureTerminalSize(element.clientWidth, element.clientHeight);
      const key = `${size.cols}x${size.rows}`;
      if (key === lastSize) {
        return;
      }
      lastSize = key;
      void resizeSession(activeSessionId, size);
    };

    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(element);
    return () => observer.disconnect();
  }, [activeSession?.status, activeSessionId, resizeSession]);

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
    await closeSession(sessionId);
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
        description="Open a dedicated PTY-backed shell on a connected server. Resize events and ANSI output are forwarded to the remote terminal."
      />

      <TerminalToolbar
        servers={servers}
        selected={selected}
        connected={connected}
        opening={selected ? Boolean(openingByServer[selected.id]) : false}
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

      {selected && !connected ? (
        <Alert variant="info">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div>
            <AlertTitle>Server is not connected</AlertTitle>
            <AlertDescription>
              Connect to {selected.name} before opening a terminal. Terminal sessions share the
              managed SSH connection but use independent channels.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {selected && (connected || serverSessions.length > 0) ? (
        <section className="overflow-hidden rounded-lg border border-border bg-card">
          <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
            <TerminalWindowIcon className="size-4 text-muted-foreground" aria-hidden="true" />
            <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
              {serverSessions.length === 0 ? (
                <span className="text-[12px] text-muted-foreground">No open sessions</span>
              ) : (
                serverSessions.map((session, index) => {
                  const active = session.info.sessionId === activeSessionId;
                  return (
                    <button
                      key={session.info.sessionId}
                      type="button"
                      className={
                        active
                          ? "rounded-sm border border-primary/35 bg-primary/10 px-2 py-1 text-[11px] font-medium text-foreground"
                          : "rounded-sm border border-transparent px-2 py-1 text-[11px] text-muted-foreground hover:border-border hover:text-foreground"
                      }
                      onClick={() => handleSelectSession(session.info.sessionId)}
                    >
                      {`${index + 1}: ${session.info.cols}×${session.info.rows}`}
                    </button>
                  );
                })
              )}
            </div>
            {activeSession ? sessionBadge(activeSession) : null}
            {activeSession ? (
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Close terminal session"
                onClick={() => {
                  void handleCloseSession(activeSession.info.sessionId);
                }}
              >
                <XIcon aria-hidden="true" />
              </Button>
            ) : null}
          </div>

          {activeSession ? (
            <>
              <div
                ref={outputRef}
                className="h-[360px] overflow-auto bg-[#07100c] px-4 py-3 font-mono text-[12px] leading-[18px] text-[#d7e4dc]"
                role="log"
                aria-label="Terminal output"
                aria-live="polite"
              >
                {activeSession.output.length === 0 ? (
                  <p className="text-[#7f9188]">
                    Waiting for output from {selected.username}@{selected.host}…
                  </p>
                ) : (
                  <pre className="m-0 whitespace-pre-wrap break-words font-mono">{preview}</pre>
                )}
              </div>

              {activeSession.outputTruncated ? (
                <p className="border-t border-border px-4 py-2 text-[11px] text-muted-foreground">
                  The visible preview is capped. The full xterm.js scrollback arrives in #27.
                </p>
              ) : null}

              <div className="border-t border-border bg-background/40 px-3 py-2">
                <label htmlFor="terminal-input" className="sr-only">
                  Terminal input
                </label>
                <textarea
                  id="terminal-input"
                  ref={inputRef}
                  aria-label="Terminal input"
                  className="h-16 w-full resize-none rounded-md border border-input bg-[#07100c] px-3 py-2 font-mono text-[12px] leading-[18px] text-[#d7e4dc] outline-none placeholder:text-[#7f9188] focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-60"
                  value=""
                  readOnly
                  spellCheck={false}
                  autoComplete="off"
                  placeholder={
                    activeSession.status === "open"
                      ? "Type directly into the remote shell. Ctrl, Alt, arrows, and paste are forwarded."
                      : "This terminal session has ended."
                  }
                  disabled={activeSession.status !== "open"}
                  onKeyDown={(event) => {
                    const sequence = terminalKeySequence(event);
                    if (sequence !== null) {
                      event.preventDefault();
                      void writeInput(activeSession.info.sessionId, sequence);
                    }
                  }}
                  onPaste={(event) => {
                    const text = event.clipboardData.getData("text");
                    if (text.length > 0) {
                      event.preventDefault();
                      void writeInput(activeSession.info.sessionId, text);
                    }
                  }}
                />
              </div>

              {activeSession.status === "exited" ? (
                <div className="border-t border-border px-4 py-3 text-[12px] text-muted-foreground">
                  <div className="flex flex-wrap items-center gap-2">
                    <span>
                      {activeSession.exitReason
                        ? exitLabels[activeSession.exitReason]
                        : "Terminal session ended"}
                      {activeSession.exitReason === "remote-exit" && activeSession.exitCode !== null
                        ? ` with exit code ${activeSession.exitCode}.`
                        : "."}
                    </span>
                    {activeSession.message ? <span>{activeSession.message}</span> : null}
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => dismissSession(activeSession.info.sessionId)}
                    >
                      Dismiss
                    </Button>
                  </div>
                </div>
              ) : null}

              {activeSession.error ? (
                <div className="border-t border-border px-4 py-2 text-[12px] text-destructive">
                  {activeSession.error}
                </div>
              ) : null}
            </>
          ) : (
            <div className="flex h-[360px] items-center justify-center bg-[#07100c] px-6">
              <div className="max-w-[46ch] text-center">
                <p className="text-[13px] text-[#d7e4dc]">No terminal session is open.</p>
                <p className="mt-1 text-[12px] leading-5 text-[#7f9188]">
                  Open a terminal above. Each session gets its own SSH channel, so multiple shells
                  can run against the same server.
                </p>
              </div>
            </div>
          )}
        </section>
      ) : null}

      <p className="text-[11px] leading-5 text-muted-foreground">
        The plain-text preview strips ANSI control sequences for readability. Raw ANSI bytes and
        colors are preserved end to end for the xterm.js renderer in #27. Terminal input is never
        written to application state or logs.
      </p>
    </div>
  );
}

export { TerminalPage };
