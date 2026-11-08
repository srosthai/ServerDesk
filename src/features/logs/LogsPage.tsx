import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  FileTextIcon,
  InfoIcon,
  ListMagnifyingGlassIcon,
  MagnifyingGlassIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { OutputPanel } from "@/components/ui/output-panel";
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
  filteredLogText,
  filterLogLines,
  formatByteSize,
  type FilteredLog,
} from "@/features/logs/logView";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { logEntryKey, useLogStore } from "@/stores/logStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useProjectStore } from "@/stores/projectStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  LogSourceKind,
  LogTail,
  LogTailStatus,
  ServerProfile,
  SystemLogSource,
} from "@/types/contracts";

const LINE_LIMITS = [100, 200, 300, 500, 1_000, 2_000] as const;
const DEFAULT_LINE_LIMIT = 200;

type LogOption = {
  path: string;
  label: string;
  description: string;
  kind: LogSourceKind;
};

function formatRetrievedAt(retrievedAt: number) {
  const date = new Date(retrievedAt);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeValue(retrievedAt: number) {
  const date = new Date(retrievedAt);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function logFileName(path: string) {
  const name = path.split("/").filter(Boolean).pop();
  return name ?? path;
}

type StatusBadgeConfig = {
  label: string;
  variant: "success" | "warning" | "danger" | "info" | "outline";
};

const statusBadges: Record<LogTailStatus, StatusBadgeConfig> = {
  available: { label: "Available", variant: "success" },
  missing: { label: "Missing", variant: "warning" },
  not_file: { label: "Not a file", variant: "warning" },
  unreadable: { label: "Unreadable", variant: "danger" },
  unavailable: { label: "Unavailable", variant: "danger" },
};

function tailBadge(connected: boolean, pending: boolean, tail: LogTail | null): StatusBadgeConfig {
  if (pending) {
    return { label: tail ? "Refreshing" : "Reading", variant: "info" };
  }
  if (!tail) {
    return { label: connected ? "No tail yet" : "Disconnected", variant: "outline" };
  }
  return statusBadges[tail.status];
}

function LogsToolbar({
  servers,
  selected,
  connected,
  sources,
  sourcesPending,
  sourcesError,
  projectsLoading,
  options,
  pathInput,
  pathError,
  lineLimit,
  pending,
  tail,
  onSelectServer,
  onSelectSource,
  onPathChange,
  onLineLimitChange,
  onOpen,
  onRefresh,
}: {
  servers: ServerProfile[];
  selected: ServerProfile;
  connected: boolean;
  sources: SystemLogSource[];
  sourcesPending: boolean;
  sourcesError: string | null;
  projectsLoading: boolean;
  options: LogOption[];
  pathInput: string;
  pathError: string | null;
  lineLimit: number;
  pending: boolean;
  tail: LogTail | null;
  onSelectServer: (serverId: string) => void;
  onSelectSource: (path: string) => void;
  onPathChange: (value: string) => void;
  onLineLimitChange: (value: string) => void;
  onOpen: () => void;
  onRefresh: () => void;
}) {
  const badge = tailBadge(connected, pending, tail);
  const sourceValue = options.some((option) => option.path === pathInput.trim())
    ? pathInput.trim()
    : "";
  const canOpen = connected && pathInput.trim().length > 0 && !pending;

  return (
    <section
      aria-label="Log viewer controls"
      className="rounded-md border border-border bg-card px-3 py-3"
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 flex-1">
          <label
            htmlFor="logs-server"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Server
          </label>
          <Select value={selected.id} onValueChange={onSelectServer}>
            <SelectTrigger id="logs-server" className="mt-1 max-w-md">
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
                ? "Read-only tails from approved log locations"
                : "Connect this server to read a bounded log tail"}
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
            disabled={!connected || pending || pathInput.trim().length === 0}
            aria-label="Refresh log tail"
          >
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {pending ? "Reading..." : "Refresh"}
          </Button>
        </div>
      </div>

      <div className="mt-3 grid gap-2 border-t border-border pt-3 xl:grid-cols-[minmax(0,1.1fr)_minmax(0,1.4fr)_8rem_auto] xl:items-end">
        <div className="min-w-0">
          <label
            htmlFor="logs-source"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Common log
          </label>
          <Select value={sourceValue} onValueChange={onSelectSource}>
            <SelectTrigger id="logs-source" className="mt-1">
              <SelectValue placeholder="Choose a system or Laravel log" />
            </SelectTrigger>
            <SelectContent>
              {options.length === 0 ? (
                <SelectItem value="__no_sources__" disabled>
                  {sourcesPending || projectsLoading ? "Loading log sources..." : "No saved logs"}
                </SelectItem>
              ) : null}
              {options.map((option) => (
                <SelectItem key={option.path} value={option.path}>
                  {option.label} · {option.path}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="min-w-0">
          <label
            htmlFor="logs-path"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Remote log path
          </label>
          <Input
            id="logs-path"
            value={pathInput}
            onChange={(event) => onPathChange(event.target.value)}
            spellCheck={false}
            autoComplete="off"
            placeholder="/var/www/storefront/storage/logs/laravel.log"
            aria-invalid={pathError ? true : undefined}
            className="mt-1 font-mono text-[12px]"
          />
        </div>

        <div>
          <label
            htmlFor="logs-lines"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Tail lines
          </label>
          <Select value={lineLimit.toString()} onValueChange={onLineLimitChange}>
            <SelectTrigger id="logs-lines" className="mt-1">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {LINE_LIMITS.map((limit) => (
                <SelectItem key={limit} value={limit.toString()}>
                  Last {limit.toLocaleString()} lines
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <Button
          type="button"
          size="sm"
          className="h-9 xl:mb-0"
          onClick={onOpen}
          disabled={!canOpen}
        >
          {pending ? (
            <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
          ) : (
            <FileTextIcon className="size-3.5" aria-hidden="true" />
          )}
          {pending ? "Reading..." : "Open log"}
        </Button>
      </div>

      <div className="mt-2 space-y-1">
        <p className="break-all font-mono text-[10px] text-muted-foreground">
          Target: {pathInput.trim() || "No log path selected"} · max {lineLimit.toLocaleString()}{" "}
          lines and 64 KiB per request
        </p>
        {pathError ? (
          <p className="text-[11px] text-destructive" role="alert">
            {pathError}
          </p>
        ) : null}
        {sourcesError ? (
          <p className="text-[11px] text-warning" role="alert">
            System log list could not be loaded: {sourcesError}. A saved Laravel log or a direct
            path still works.
          </p>
        ) : null}
        <p className="text-[11px] text-muted-foreground">
          Only common system logs and Laravel storage/logs files are approved. Relative paths,
          parent segments, symlinks that escape those locations, and arbitrary files are rejected
          before a remote read.
        </p>
      </div>
      {tail ? (
        <p className="mt-2 border-t border-border pt-2 text-[10px] text-muted-foreground">
          Showing {logFileName(tail.requestedPath)} retrieved{" "}
          <time dateTime={dateTimeValue(tail.retrievedAt)}>
            {formatRetrievedAt(tail.retrievedAt)}
          </time>
          . Refresh replaces this tail with a new bounded read; filtering stays local.
        </p>
      ) : null}
      {sources.length === 0 && sourcesPending ? (
        <span className="sr-only">Loading the common system log list.</span>
      ) : null}
    </section>
  );
}

function LogStatusAlerts({ tail }: { tail: LogTail }) {
  return (
    <>
      {tail.status === "missing" ? (
        <Alert variant="warning">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Log file not found</AlertTitle>
            <AlertDescription>
              {tail.detail ??
                "The selected log path does not exist on this server right now. It may have been rotated or removed."}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {tail.status === "not_file" ? (
        <Alert variant="warning">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Log path is not a regular file</AlertTitle>
            <AlertDescription>
              {tail.detail ??
                "The selected path exists but is not a regular file, so it cannot be shown as a log tail."}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {tail.status === "unreadable" ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>Log file is not readable</AlertTitle>
            <AlertDescription>
              {tail.detail ??
                "The SSH account cannot read this log file. ServerDesk does not use sudo to read logs."}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {tail.status === "unavailable" ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>Could not read the log file</AlertTitle>
            <AlertDescription>
              {tail.detail ??
                "The remote log command failed. The server may be under pressure or the file may be changing."}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {tail.rotated ? (
        <Alert variant="warning">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Log rotated during retrieval</AlertTitle>
            <AlertDescription>
              {tail.rotationDetail ??
                "The log file changed while ServerDesk was reading it. This tail may not be a continuous view; refresh to read the current file."}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {tail.truncated ? (
        <Alert variant="warning">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Tail was truncated at the safety limit</AlertTitle>
            <AlertDescription>
              {tail.truncatedBytes
                ? `Only the last ${formatByteSize(tail.byteLimit)} of the file were requested.`
                : ""}
              {tail.truncatedBytes && tail.truncatedLines ? " " : ""}
              {tail.truncatedLines
                ? `Only the last ${tail.lineLimit.toLocaleString()} lines are kept.`
                : ""}
              {tail.partialFirstLine
                ? " The first partial line was dropped so the displayed tail starts on a line boundary."
                : ""}{" "}
              Narrow the selection or choose a smaller line limit if you need older entries.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}
    </>
  );
}

function LogMetadataCard({ tail, pending }: { tail: LogTail; pending: boolean }) {
  return (
    <section
      aria-label="Log retrieval details"
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border px-3.5 py-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-[13px] font-semibold text-foreground">
              {logFileName(tail.requestedPath)}
            </h2>
            <Badge variant={statusBadges[tail.status].variant}>
              {statusBadges[tail.status].label}
            </Badge>
            <Badge variant={tail.truncated ? "warning" : "outline"}>
              {tail.truncated ? "Truncated" : "Complete tail"}
            </Badge>
            {tail.rotated ? <Badge variant="warning">Rotated</Badge> : null}
            {pending ? <Badge variant="info">Refreshing</Badge> : null}
            <Badge variant="secondary">
              {tail.sourceKind === "laravel" ? "Laravel log" : "System log"}
            </Badge>
          </div>
          <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
            {tail.requestedPath}
          </p>
          {tail.resolvedPath && tail.resolvedPath !== tail.requestedPath ? (
            <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">
              Resolved to {tail.resolvedPath}
            </p>
          ) : null}
        </div>
        <p className="text-[11px] text-muted-foreground">
          Retrieved{" "}
          <time dateTime={dateTimeValue(tail.retrievedAt)}>
            {formatRetrievedAt(tail.retrievedAt)}
          </time>
        </p>
      </div>

      <dl className="grid gap-px bg-border sm:grid-cols-2 xl:grid-cols-4">
        <div className="bg-card px-3.5 py-2.5">
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            File size
          </dt>
          <dd className="mt-1 font-mono text-[12px] text-foreground">
            {formatByteSize(tail.fileSizeBytes)}
          </dd>
        </div>
        <div className="bg-card px-3.5 py-2.5">
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Retrieved
          </dt>
          <dd className="mt-1 font-mono text-[12px] text-foreground">
            {tail.returnedLines.toLocaleString()} lines · {formatByteSize(tail.returnedBytes)}
          </dd>
        </div>
        <div className="bg-card px-3.5 py-2.5">
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Request limits
          </dt>
          <dd className="mt-1 font-mono text-[12px] text-foreground">
            {tail.lineLimit.toLocaleString()} lines · {formatByteSize(tail.byteLimit)}
          </dd>
        </div>
        <div className="bg-card px-3.5 py-2.5">
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Duration
          </dt>
          <dd className="mt-1 font-mono text-[12px] text-foreground">{tail.durationMs} ms</dd>
        </div>
      </dl>
    </section>
  );
}

function LogViewer({
  tail,
  filtered,
  filter,
  onFilter,
  onClearFilter,
}: {
  tail: LogTail;
  filtered: FilteredLog;
  filter: string;
  onFilter: (value: string) => void;
  onClearFilter: () => void;
}) {
  const content = filteredLogText(filtered);
  const emptyText =
    filtered.totalLines === 0
      ? tail.truncatedBytes
        ? "No complete line fits in the 64 KiB tail limit. One log entry is probably very large."
        : "The log file is empty."
      : filtered.matchedLines === 0
        ? "No lines match this filter."
        : "No output.";

  return (
    <section aria-label="Bounded log output" className="space-y-2">
      <div className="flex flex-wrap items-end justify-between gap-2">
        <div className="min-w-0">
          <h2 className="text-[13px] font-semibold text-foreground">Bounded log tail</h2>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            Filtering is local to the retrieved tail and never downloads the whole file again.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative w-full min-w-[220px] max-w-sm">
            <label htmlFor="log-filter" className="sr-only">
              Filter log text
            </label>
            <MagnifyingGlassIcon
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <Input
              id="log-filter"
              type="search"
              value={filter}
              onChange={(event) => onFilter(event.target.value)}
              placeholder="Filter displayed lines"
              className="h-8 pl-8 text-[12px]"
              disabled={tail.status !== "available"}
            />
          </div>
          {filter.trim() ? (
            <Button type="button" variant="ghost" size="sm" onClick={onClearFilter}>
              Clear filter
            </Button>
          ) : null}
          <p className="text-[11px] text-muted-foreground" aria-live="polite">
            {filtered.matchedLines.toLocaleString()} of {filtered.totalLines.toLocaleString()} lines
          </p>
        </div>
      </div>

      <OutputPanel
        title="Log lines"
        content={content}
        emptyText={emptyText}
        meta={`${filtered.matchedLines.toLocaleString()} lines · ${formatByteSize(tail.returnedBytes)}`}
        maxHeightClassName="max-h-[34rem]"
      />
    </section>
  );
}

function LogsPage() {
  const [searchParams] = useSearchParams();
  const linkedServerId = searchParams.get("server");
  const linkedPath = searchParams.get("path")?.trim() ?? "";

  const servers = useServerStore((state) => state.servers);
  const status = useServerStore((state) => state.status);
  const serverError = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const connections = useConnectionStore((state) => state.connections);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const lastActiveServerId = usePreferencesStore((state) => state.preferences.lastActiveServerId);
  const sources = useLogStore((state) => state.sources);
  const sourcesPending = useLogStore((state) => state.sourcesPending);
  const sourcesError = useLogStore((state) => state.sourcesError);
  const fetchSources = useLogStore((state) => state.fetchSources);
  const readLog = useLogStore((state) => state.read);
  const clearError = useLogStore((state) => state.clearError);
  const fetchProjects = useProjectStore((state) => state.fetchProjects);

  const [selectedId, setSelectedId] = useState<string | null>(linkedServerId);
  const [pathInput, setPathInput] = useState(linkedPath);
  const [pathError, setPathError] = useState<string | null>(null);
  const [lineLimit, setLineLimit] = useState(DEFAULT_LINE_LIMIT);
  const [filter, setFilter] = useState("");
  const autoReadRef = useRef<string | null>(null);

  const selected =
    servers.find((server) => server.id === selectedId) ??
    servers.find((server) => server.id === lastActiveServerId) ??
    servers[0] ??
    null;
  const selectedServerId = selected?.id ?? null;
  const connected = selected ? connections[selected.id]?.state === "connected" : false;
  const projectEntry = useProjectStore((state) =>
    selected ? state.entries[selected.id] : undefined,
  );
  const projects = useMemo(() => projectEntry?.projects ?? [], [projectEntry?.projects]);
  const trimmedPath = pathInput.trim();
  const entry = useLogStore((state) =>
    selectedServerId ? state.entries[logEntryKey(selectedServerId, trimmedPath)] : undefined,
  );
  const tail = entry?.tail ?? null;

  const options = useMemo<LogOption[]>(() => {
    const byPath = new Map<string, LogOption>();
    for (const source of sources) {
      byPath.set(source.path, {
        path: source.path,
        label: source.label,
        description: source.description,
        kind: "system",
      });
    }
    for (const project of projects) {
      if (!byPath.has(project.logPath)) {
        byPath.set(project.logPath, {
          path: project.logPath,
          label: `Laravel · ${project.name}`,
          description: project.remotePath,
          kind: "laravel",
        });
      }
    }
    return [...byPath.values()].sort((left, right) => left.label.localeCompare(right.label));
  }, [sources, projects]);

  const filtered = useMemo(
    () => filterLogLines(tail?.content ?? "", filter),
    [tail?.content, filter],
  );

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
    if (!sources.length && !sourcesPending && !sourcesError) {
      void fetchSources();
    }
  }, [sources.length, sourcesPending, sourcesError, fetchSources]);

  useEffect(() => {
    if (selectedServerId && isTauriRuntime()) {
      void fetchProjects(selectedServerId);
    }
  }, [selectedServerId, fetchProjects]);

  useEffect(() => {
    if (!connected || !selectedServerId || !linkedPath) {
      return;
    }
    if (pathInput.trim() && pathInput.trim() !== linkedPath) {
      return;
    }

    const key = `${selectedServerId}:${linkedPath}`;
    if (autoReadRef.current === key) {
      return;
    }

    autoReadRef.current = key;
    setPathInput(linkedPath);
    setPathError(null);
    setFilter("");
    void readLog(selectedServerId, linkedPath, lineLimit);
  }, [connected, selectedServerId, linkedPath, pathInput, lineLimit, readLog]);

  function selectServer(serverId: string) {
    setSelectedId(serverId);
    setPathError(null);
    setFilter("");
  }

  function requestRead(targetPath: string, targetLines = lineLimit) {
    if (!selectedServerId || !connected) {
      return;
    }
    const trimmed = targetPath.trim();
    if (!trimmed) {
      setPathError("Enter a remote log path.");
      return;
    }
    if (!trimmed.startsWith("/")) {
      setPathError("Enter an absolute remote log path such as /var/log/syslog.");
      return;
    }

    setPathError(null);
    setFilter("");
    void readLog(selectedServerId, trimmed, targetLines);
  }

  function selectSource(path: string) {
    setPathInput(path);
    requestRead(path);
  }

  function changeLineLimit(value: string) {
    const next = Number(value);
    if (!Number.isFinite(next)) {
      return;
    }
    setLineLimit(next);
    if (tail && selectedServerId && connected && trimmedPath) {
      void readLog(selectedServerId, trimmedPath, next);
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Logs"
        description="Read bounded tails of common system logs and Laravel logs without downloading whole files. Every tail shows its remote path, retrieval time, limits, and truncation state."
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
              Log tails need a live SSH session. Open the desktop build and connect a server to read
              approved system or Laravel logs. No log content is fabricated in browser preview.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {status === "loading" && servers.length === 0 ? <LoadingState rows={4} /> : null}

      {status === "error" && servers.length === 0 ? (
        <ErrorState
          message={serverError ?? "Could not load server profiles."}
          onRetry={fetchServers}
        />
      ) : null}

      {status !== "loading" && servers.length === 0 ? (
        <EmptyState
          icon={<ListMagnifyingGlassIcon className="size-4" aria-hidden="true" />}
          title="No server selected"
          description="Add a server profile and connect it to inspect recent system and Laravel logs with bounded reads."
          action={
            <Button asChild size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      ) : null}

      {selected ? (
        <>
          <LogsToolbar
            servers={servers}
            selected={selected}
            connected={connected}
            sources={sources}
            sourcesPending={sourcesPending}
            sourcesError={sourcesError}
            projectsLoading={projectEntry?.loading ?? false}
            options={options}
            pathInput={pathInput}
            pathError={pathError}
            lineLimit={lineLimit}
            pending={entry?.pending ?? false}
            tail={tail}
            onSelectServer={selectServer}
            onSelectSource={selectSource}
            onPathChange={(value) => {
              setPathInput(value);
              if (pathError) {
                setPathError(null);
              }
            }}
            onLineLimitChange={changeLineLimit}
            onOpen={() => requestRead(pathInput)}
            onRefresh={() => requestRead(pathInput)}
          />

          {!connected && tail ? (
            <Alert variant="info">
              <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
              <div>
                <AlertTitle>Showing the last retrieved tail</AlertTitle>
                <AlertDescription>
                  The connection is not active, so this tail is not live. Its original retrieval
                  time stays visible; reconnect to refresh it.
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {entry?.error ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Could not read this log</AlertTitle>
                <AlertDescription>
                  <span className="block">{entry.error}</span>
                  {entry.errorDetails ? (
                    <span className="mt-1 block whitespace-pre-wrap font-mono text-[10px]">
                      {entry.errorDetails}
                    </span>
                  ) : null}
                  {tail
                    ? " The last successful tail stays visible with its original retrieval time."
                    : null}
                </AlertDescription>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => clearError(selected.id, trimmedPath)}
              >
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {entry?.pending && !tail ? <LoadingState rows={5} /> : null}

          {tail ? (
            <>
              <LogStatusAlerts tail={tail} />
              <LogMetadataCard tail={tail} pending={entry?.pending ?? false} />
              {tail.status === "available" ? (
                <LogViewer
                  tail={tail}
                  filtered={filtered}
                  filter={filter}
                  onFilter={setFilter}
                  onClearFilter={() => setFilter("")}
                />
              ) : null}
            </>
          ) : null}

          {!entry?.pending && !tail && !entry?.error ? (
            <EmptyState
              icon={<ListMagnifyingGlassIcon className="size-4" aria-hidden="true" />}
              title={connected ? "No log tail yet" : "Log reading is paused"}
              description={
                connected
                  ? "Choose a common log, open a saved Laravel log, or enter an approved remote path. ServerDesk reads only a bounded tail."
                  : "Connect this server to read logs. Saved Laravel projects and their log paths remain visible offline."
              }
              action={
                connected && trimmedPath ? (
                  <Button size="sm" onClick={() => requestRead(pathInput)}>
                    Read log tail
                  </Button>
                ) : undefined
              }
            />
          ) : null}

          <p className="text-[10px] text-muted-foreground">
            Log content stays in memory only. ServerDesk does not store raw log lines in SQLite,
            history, or telemetry.
          </p>
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
            <AlertDescription>{serverError}</AlertDescription>
          </div>
        </Alert>
      ) : null}
    </div>
  );
}

export { LogsPage };
