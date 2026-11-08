import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  CaretRightIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  ClockCounterClockwiseIcon,
  FolderOpenIcon,
  FolderSimpleIcon,
  InfoIcon,
  MagnifyingGlassIcon,
  PlayIcon,
  QuestionIcon,
  TerminalWindowIcon,
  WarningCircleIcon,
  XCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
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
  ARTISAN_ACTIONS,
  artisanActionConfig,
  artisanCommand,
  artisanTranscript,
} from "@/features/projects/artisanActions";
import { normalizeProjectPath } from "@/features/projects/projectPaths";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import {
  projectActionKey,
  useProjectStore,
  type ProjectActionEntry,
  type ProjectEntry,
} from "@/stores/projectStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ArtisanActionName,
  ProjectArtisanRun,
  ProjectOperationStatus,
  ProjectRecord,
  ProjectScanResult,
  RemoteDirectoryListing,
  ServerProfile,
} from "@/types/contracts";

const DEFAULT_REMOTE_PATH = "/var/www";

const operationStatusConfig: Record<
  ProjectOperationStatus,
  {
    label: string;
    variant: "success" | "danger" | "warning";
    Icon: typeof CheckCircleIcon;
  }
> = {
  succeeded: { label: "Succeeded", variant: "success", Icon: CheckCircleIcon },
  failed: { label: "Failed", variant: "danger", Icon: XCircleIcon },
  unknown: { label: "Unknown", variant: "warning", Icon: QuestionIcon },
};

function formatTimestamp(unixSeconds: number) {
  const date = new Date(unixSeconds * 1000);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeValue(unixSeconds: number) {
  const date = new Date(unixSeconds * 1000);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function remoteLink(pathname: string, project: ProjectRecord, remotePath: string) {
  const params = new URLSearchParams({
    server: project.serverId,
    path: remotePath,
  });
  return `${pathname}?${params.toString()}`;
}

function OperationStatusBadge({ status }: { status: ProjectOperationStatus }) {
  const config = operationStatusConfig[status];
  return (
    <Badge variant={config.variant}>
      <config.Icon className="size-3" aria-hidden="true" />
      {config.label}
    </Badge>
  );
}

function ArtisanResultPanel({ run, onDismiss }: { run: ProjectArtisanRun; onDismiss: () => void }) {
  const result = run.result;
  const config = artisanActionConfig[result.action];
  const outcome = result.succeeded
    ? { label: "Succeeded", variant: "success" as const }
    : { label: "Failed", variant: "danger" as const };

  return (
    <section
      aria-label={`${config.label} result`}
      className="mt-2 rounded-md border border-border bg-muted/20 px-2.5 py-2.5"
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h4 className="text-[12px] font-semibold text-foreground">{config.label} result</h4>
            <Badge variant={outcome.variant}>
              {result.succeeded ? (
                <CheckCircleIcon className="size-3" aria-hidden="true" />
              ) : (
                <XCircleIcon className="size-3" aria-hidden="true" />
              )}
              {outcome.label}
            </Badge>
          </div>
          <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">
            {result.command}
          </p>
        </div>
        <Button variant="ghost" size="sm" className="shrink-0" onClick={onDismiss}>
          Dismiss
        </Button>
      </div>

      <dl className="mt-2 grid grid-cols-2 gap-x-3 gap-y-1.5 rounded-sm border border-border bg-card px-2.5 py-2 text-[10px] sm:grid-cols-4">
        <div>
          <dt className="text-muted-foreground">Working directory</dt>
          <dd className="break-all font-mono text-foreground">{result.targetPath}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Exit status</dt>
          <dd className="font-mono text-foreground">
            {result.exitCode === null ? "No exit status" : `Exit ${result.exitCode.toString()}`}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Duration</dt>
          <dd className="font-mono text-foreground">{result.durationMs} ms</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Local history</dt>
          <dd className="text-foreground">{run.historyRecorded ? "Recorded" : "Not recorded"}</dd>
        </div>
      </dl>

      {!run.historyRecorded ? (
        <Alert variant="warning" className="mt-2 py-2">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Action ran but history was not saved</AlertTitle>
            <AlertDescription>
              {run.historyError ?? "The local operation history could not be updated."}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {result.secretsRedacted ? (
        <Alert variant="info" className="mt-2 py-2">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Secret-like output was redacted</AlertTitle>
            <AlertDescription>
              ServerDesk replaced environment-style lines, secret assignments, tokens, or
              private-key blocks before showing or storing this output.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {result.outputTruncated ? (
        <Alert variant="warning" className="mt-2 py-2">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Output was shortened</AlertTitle>
            <AlertDescription>
              Only the bounded portion of stdout or stderr is shown. The exact exit status above is
              still the command result.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      <OutputPanel
        title="Command output"
        content={artisanTranscript(result)}
        meta={
          result.exitCode === null
            ? "no exit status"
            : `exit ${result.exitCode.toString()} · ${result.durationMs.toString()} ms`
        }
        className="mt-2"
        maxHeightClassName="max-h-64"
      />
    </section>
  );
}

function ArtisanActionsSection({
  project,
  connected,
  actionEntry,
  onRequestAction,
  onDismissResult,
}: {
  project: ProjectRecord;
  connected: boolean;
  actionEntry: ProjectActionEntry | undefined;
  onRequestAction: (action: ArtisanActionName) => void;
  onDismissResult: () => void;
}) {
  const pending = actionEntry?.pending ?? null;

  return (
    <section
      aria-label={`Artisan actions for ${project.name}`}
      className="border-t border-border px-3.5 py-3"
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <TerminalWindowIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            <h3 className="text-[12px] font-semibold text-foreground">Artisan actions</h3>
          </div>
          <p className="mt-1 text-[11px] text-muted-foreground">
            Fixed commands only. Every action runs with{" "}
            <span className="break-all font-mono text-foreground">{project.remotePath}</span> as its
            working directory. ServerDesk never accepts custom command text.
          </p>
        </div>
        <Badge variant="outline">{ARTISAN_ACTIONS.length} fixed actions</Badge>
      </div>

      {!connected ? (
        <p className="mt-2 text-[11px] text-warning">
          Connect this server to run Artisan actions. Saved project data stays visible offline.
        </p>
      ) : null}

      <div className="mt-2 grid gap-1.5 xl:grid-cols-2">
        {ARTISAN_ACTIONS.map((config) => {
          const actionPending = pending === config.action;
          return (
            <div
              key={config.action}
              className="rounded-sm border border-border bg-card px-2.5 py-2"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-[12px] font-medium text-foreground">{config.label}</span>
                    {config.destructive ? (
                      <Badge variant="warning">Confirm</Badge>
                    ) : (
                      <Badge variant="outline">Read-only</Badge>
                    )}
                  </div>
                  <p className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                    {config.description}
                  </p>
                </div>
                <Button
                  type="button"
                  variant={config.destructive ? "secondary" : "default"}
                  size="sm"
                  className="h-7 shrink-0 gap-1 px-2 text-[11px]"
                  aria-label={`Run ${config.label}`}
                  title={`${config.label} in ${project.remotePath}`}
                  disabled={!connected || Boolean(pending)}
                  onClick={() => onRequestAction(config.action)}
                >
                  {actionPending ? (
                    <CircleNotchIcon className="size-3 animate-spin" aria-hidden="true" />
                  ) : (
                    <PlayIcon className="size-3" aria-hidden="true" />
                  )}
                  {actionPending ? "Running..." : config.destructive ? "Review" : "Run"}
                </Button>
              </div>
              <code className="mt-1.5 block break-all rounded-sm bg-muted/45 px-2 py-1 font-mono text-[10px] text-muted-foreground">
                {artisanCommand(project.remotePath, config.action)}
              </code>
            </div>
          );
        })}
      </div>

      {actionEntry?.error ? (
        <Alert variant="destructive" className="mt-2">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div className="flex-1">
            <AlertTitle>Artisan action could not run</AlertTitle>
            <AlertDescription>
              <span className="block">{actionEntry.error}</span>
              {actionEntry.errorDetails ? (
                <span className="mt-1 block whitespace-pre-wrap font-mono text-[10px]">
                  {actionEntry.errorDetails}
                </span>
              ) : null}
            </AlertDescription>
          </div>
          <Button variant="ghost" size="sm" onClick={onDismissResult}>
            Dismiss
          </Button>
        </Alert>
      ) : null}

      {actionEntry?.run ? (
        <ArtisanResultPanel run={actionEntry.run} onDismiss={onDismissResult} />
      ) : null}
    </section>
  );
}

function ProjectCard({
  project,
  connected,
  actionEntry,
  onRequestAction,
  onDismissResult,
}: {
  project: ProjectRecord;
  connected: boolean;
  actionEntry: ProjectActionEntry | undefined;
  onRequestAction: (action: ArtisanActionName) => void;
  onDismissResult: () => void;
}) {
  const terminalPath = remoteLink("/terminal", project, project.remotePath);
  const logPath = remoteLink("/logs", project, project.logPath);

  return (
    <section
      aria-label={`Project ${project.name}`}
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border px-3.5 py-3">
        <div className="flex min-w-0 items-start gap-2.5">
          <div className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-md border border-border bg-muted/50 text-muted-foreground">
            <FolderSimpleIcon className="size-4" aria-hidden="true" />
          </div>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="truncate text-[14px] font-semibold text-foreground">{project.name}</h2>
              <Badge variant="success">Laravel</Badge>
            </div>
            <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
              {project.remotePath}
            </p>
          </div>
        </div>
        <p className="text-[11px] text-muted-foreground">
          Detected{" "}
          <time dateTime={dateTimeValue(project.detectedAt)}>
            {formatTimestamp(project.detectedAt)}
          </time>
        </p>
      </div>

      <dl className="grid gap-px bg-border sm:grid-cols-2 xl:grid-cols-4">
        <div className="bg-card px-3.5 py-2.5">
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Laravel version
          </dt>
          <dd className="mt-1 font-mono text-[12px] text-foreground">
            {project.laravelVersion ?? "Not detected"}
          </dd>
        </div>
        <div className="bg-card px-3.5 py-2.5">
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            PHP version
          </dt>
          <dd className="mt-1 font-mono text-[12px] text-foreground">
            {project.phpVersion ?? "Not detected"}
          </dd>
        </div>
        <div className="bg-card px-3.5 py-2.5">
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Queue hint
          </dt>
          <dd className="mt-1 text-[11px] leading-5 text-muted-foreground">
            {project.queueHint ?? "Not detected"}
          </dd>
        </div>
        <div className="bg-card px-3.5 py-2.5">
          <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Scheduler hint
          </dt>
          <dd className="mt-1 text-[11px] leading-5 text-muted-foreground">
            {project.schedulerHint ?? "Not detected"}
          </dd>
        </div>
      </dl>

      <div className="border-t border-border px-3.5 py-3">
        <div className="flex items-center gap-1.5">
          <ClockCounterClockwiseIcon
            className="size-3.5 text-muted-foreground"
            aria-hidden="true"
          />
          <h3 className="text-[12px] font-semibold text-foreground">Recent operation status</h3>
        </div>
        {project.recentOperations.length === 0 ? (
          <p className="mt-1.5 text-[11px] text-muted-foreground">
            No operations have been recorded for this project yet.
          </p>
        ) : (
          <ul className="mt-2 space-y-1.5">
            {project.recentOperations.map((operation) => (
              <li
                key={operation.id}
                className="flex flex-wrap items-start justify-between gap-2 rounded-sm border border-border bg-muted/25 px-2.5 py-2"
              >
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-[11px] font-medium text-foreground">
                      {operation.operation}
                    </span>
                    <OperationStatusBadge status={operation.status} />
                  </div>
                  <p className="mt-0.5 text-[11px] text-muted-foreground">
                    {operation.safeSummary}
                  </p>
                </div>
                <div className="text-right text-[10px] text-muted-foreground">
                  <div>
                    Exit{" "}
                    <span className="font-mono">
                      {operation.exitCode === null ? "unknown" : operation.exitCode.toString()}
                    </span>
                  </div>
                  <div>
                    <time dateTime={dateTimeValue(operation.startedAt)}>
                      {formatTimestamp(operation.startedAt)}
                    </time>{" "}
                    · {operation.durationMs} ms
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

      <ArtisanActionsSection
        project={project}
        connected={connected}
        actionEntry={actionEntry}
        onRequestAction={onRequestAction}
        onDismissResult={onDismissResult}
      />

      <div className="flex flex-wrap items-end justify-between gap-3 border-t border-border bg-muted/20 px-3.5 py-3">
        <div className="min-w-0">
          <p className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Project log path
          </p>
          <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
            {project.logPath}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button asChild variant="secondary" size="sm">
            <Link to={terminalPath} title={`Open terminal at ${project.remotePath}`}>
              <TerminalWindowIcon className="size-3.5" aria-hidden="true" />
              Open terminal
            </Link>
          </Button>
          <Button asChild variant="secondary" size="sm">
            <Link to={logPath} title={`Open log at ${project.logPath}`}>
              <FolderOpenIcon className="size-3.5" aria-hidden="true" />
              Open Laravel log
            </Link>
          </Button>
        </div>
      </div>
    </section>
  );
}

function DetectionPanel({
  servers,
  selected,
  connected,
  pathInput,
  pathError,
  entry,
  scanTarget,
  onSelectServer,
  onPathChange,
  onPathBlur,
  onBrowse,
  onScan,
}: {
  servers: ServerProfile[];
  selected: ServerProfile;
  connected: boolean;
  pathInput: string;
  pathError: string | null;
  entry: ProjectEntry | undefined;
  scanTarget: "directory" | "children" | null;
  onSelectServer: (serverId: string) => void;
  onPathChange: (value: string) => void;
  onPathBlur: () => void;
  onBrowse: () => void;
  onScan: (includeChildren: boolean) => void;
}) {
  const normalized = normalizeProjectPath(pathInput);
  const browsing = entry?.browsing ?? false;
  const scanning = entry?.scanning ?? false;
  const busy = browsing || scanning;
  const canRun = connected && Boolean(normalized) && !busy;

  return (
    <section
      aria-label="Project detection controls"
      className="rounded-md border border-border bg-card px-3.5 py-3"
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 flex-1">
          <label
            htmlFor="projects-server"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Server
          </label>
          <Select value={selected.id} onValueChange={onSelectServer}>
            <SelectTrigger id="projects-server" className="mt-1 max-w-md">
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
                ? "Browse and scan run only on the path you choose"
                : "Connect this server to browse or scan remote directories"}
            </span>
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <StatusBadge state={connected ? "connected" : "disconnected"} />
          <Badge variant={normalized ? "info" : "warning"}>
            {normalized ? "Absolute path" : "Path required"}
          </Badge>
        </div>
      </div>

      <div className="mt-3 border-t border-border pt-3">
        <label
          htmlFor="project-remote-path"
          className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
        >
          Remote directory
        </label>
        <div className="mt-1 flex flex-col gap-2 md:flex-row md:items-center">
          <Input
            id="project-remote-path"
            value={pathInput}
            onChange={(event) => onPathChange(event.target.value)}
            onBlur={onPathBlur}
            spellCheck={false}
            autoComplete="off"
            placeholder="/var/www"
            aria-invalid={pathError ? true : undefined}
            aria-describedby="project-remote-path-help"
            className="font-mono text-[12px] md:max-w-xl"
          />
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={!canRun}
              onClick={onBrowse}
            >
              {browsing ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <FolderOpenIcon className="size-3.5" aria-hidden="true" />
              )}
              {browsing ? "Browsing..." : "Browse"}
            </Button>
            <Button type="button" size="sm" disabled={!canRun} onClick={() => onScan(false)}>
              {scanning && scanTarget === "directory" ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <MagnifyingGlassIcon className="size-3.5" aria-hidden="true" />
              )}
              {scanning && scanTarget === "directory" ? "Scanning..." : "Scan directory"}
            </Button>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={!canRun}
              onClick={() => onScan(true)}
            >
              {scanning && scanTarget === "children" ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
              )}
              {scanning && scanTarget === "children" ? "Scanning..." : "Scan subdirectories"}
            </Button>
          </div>
        </div>
        <div id="project-remote-path-help" className="mt-1.5 space-y-1">
          <p className="break-all font-mono text-[10px] text-muted-foreground">
            Target: {normalized ?? (pathInput.trim() || "/var/www")}
          </p>
          {pathError ? (
            <p className="text-[11px] text-destructive" role="alert">
              {pathError}
            </p>
          ) : (
            <p className="text-[11px] text-muted-foreground">
              Paths must be absolute and normalized. Browse lists only immediate child directories;
              scanning subdirectories checks only one level below the selected path. ServerDesk
              never scans the whole server automatically.
            </p>
          )}
        </div>
      </div>
    </section>
  );
}

function BrowseResults({
  listing,
  onUsePath,
  onDismiss,
}: {
  listing: RemoteDirectoryListing;
  onUsePath: (path: string) => void;
  onDismiss: () => void;
}) {
  return (
    <section
      aria-label="Remote directory browse results"
      className="rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-start justify-between gap-2 border-b border-border px-3.5 py-2.5">
        <div>
          <h2 className="text-[13px] font-semibold text-foreground">Immediate subdirectories</h2>
          <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">
            {listing.path}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Badge variant="outline">{listing.directories.length} directories</Badge>
          <Button variant="ghost" size="sm" onClick={onDismiss}>
            Dismiss
          </Button>
        </div>
      </div>
      {listing.directories.length === 0 ? (
        <p className="px-3.5 py-4 text-[12px] text-muted-foreground">
          This directory has no readable immediate child directories.
        </p>
      ) : (
        <ul className="divide-y divide-border">
          {listing.directories.map((directory) => (
            <li
              key={directory.path}
              className="flex flex-wrap items-center justify-between gap-2 px-3.5 py-2"
            >
              <div className="min-w-0">
                <span className="block text-[12px] font-medium text-foreground">
                  {directory.name}
                </span>
                <span className="block break-all font-mono text-[10px] text-muted-foreground">
                  {directory.path}
                </span>
              </div>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                className="h-7 gap-1 px-2 text-[11px]"
                onClick={() => onUsePath(directory.path)}
                aria-label={`Use path ${directory.path}`}
              >
                Use path
                <CaretRightIcon className="size-3" aria-hidden="true" />
              </Button>
            </li>
          ))}
        </ul>
      )}
      {listing.truncated ? (
        <p className="border-t border-border px-3.5 py-2 text-[11px] text-warning">
          Output was truncated at the safety limit. Only the listed directories can be selected.
        </p>
      ) : null}
      <p className="border-t border-border px-3.5 py-2 text-[10px] text-muted-foreground">
        Browsed in {listing.durationMs} ms ·{" "}
        <time dateTime={dateTimeValue(listing.sampledAt)}>
          {formatTimestamp(listing.sampledAt)}
        </time>
      </p>
    </section>
  );
}

function ScanSummary({ scan }: { scan: ProjectScanResult }) {
  const detected = scan.projects.length;
  const scope = scan.includeChildren
    ? "the selected directory and its immediate subdirectories"
    : "the selected directory";

  return (
    <Alert variant={detected > 0 ? "success" : "warning"} aria-live="polite">
      <InfoIcon
        className={`mt-0.5 size-4 shrink-0 ${detected > 0 ? "text-success" : "text-warning"}`}
        aria-hidden="true"
      />
      <div>
        <AlertTitle>
          {detected > 0
            ? `${detected.toString()} Laravel project${detected === 1 ? "" : "s"} detected`
            : "No Laravel project found"}
        </AlertTitle>
        <AlertDescription>
          Scanned {scan.scannedPaths.toString()} path{scan.scannedPaths === 1 ? "" : "s"} in {scope}
          . <span className="font-mono">{scan.path}</span>.{" "}
          {detected > 0
            ? `${detected.toString()} project record${detected === 1 ? "" : "s"} saved.`
            : "The directory was rejected because it is missing at least one required indicator: artisan, composer.json, bootstrap/, or storage/."}{" "}
          {scan.rejectedPaths > 0
            ? `${scan.rejectedPaths.toString()} path${scan.rejectedPaths === 1 ? "" : "s"} rejected as non-Laravel.`
            : null}
          {scan.truncated
            ? " Results were truncated at the safety limit; rescan a narrower path to continue."
            : null}
        </AlertDescription>
      </div>
    </Alert>
  );
}

function ProjectsPage() {
  const servers = useServerStore((state) => state.servers);
  const status = useServerStore((state) => state.status);
  const serverError = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const connections = useConnectionStore((state) => state.connections);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const lastActiveServerId = usePreferencesStore((state) => state.preferences.lastActiveServerId);
  const fetchProjects = useProjectStore((state) => state.fetchProjects);
  const browse = useProjectStore((state) => state.browse);
  const scan = useProjectStore((state) => state.scan);
  const runAction = useProjectStore((state) => state.runAction);
  const actions = useProjectStore((state) => state.actions);
  const clearActionResult = useProjectStore((state) => state.clearActionResult);
  const clearScan = useProjectStore((state) => state.clearScan);
  const clearBrowse = useProjectStore((state) => state.clearBrowse);
  const clearError = useProjectStore((state) => state.clearError);

  const [selectedId, setSelectedId] = useState<string | null>(lastActiveServerId);
  const [pathInput, setPathInput] = useState(DEFAULT_REMOTE_PATH);
  const [pathError, setPathError] = useState<string | null>(null);
  const [scanTarget, setScanTarget] = useState<"directory" | "children" | null>(null);
  const [pendingAction, setPendingAction] = useState<{
    project: ProjectRecord;
    action: ArtisanActionName;
  } | null>(null);

  const selected =
    servers.find((server) => server.id === selectedId) ??
    servers.find((server) => server.id === lastActiveServerId) ??
    servers[0] ??
    null;
  const selectedServerId = selected?.id ?? null;
  const connected = selected ? connections[selected.id]?.state === "connected" : false;
  const entry = useProjectStore((state) => (selected ? state.entries[selected.id] : undefined));
  const projects = entry?.projects ?? [];

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
    if (!selectedServerId || !isTauriRuntime()) {
      return;
    }
    void fetchProjects(selectedServerId);
  }, [selectedServerId, fetchProjects]);

  function selectServer(serverId: string) {
    setSelectedId(serverId);
    setPathInput(DEFAULT_REMOTE_PATH);
    setPathError(null);
    setScanTarget(null);
    setPendingAction(null);
  }

  function requestAction(project: ProjectRecord, action: ArtisanActionName) {
    if (!connected) {
      return;
    }
    if (artisanActionConfig[action].destructive) {
      setPendingAction({ project, action });
      return;
    }

    void runAction(project.serverId, project.id, action, false);
  }

  function validatePath(): string | null {
    const normalized = normalizeProjectPath(pathInput);
    if (!normalized) {
      setPathError(
        "Enter an absolute, normalized remote path such as /var/www/storefront. Parent segments and control characters are rejected.",
      );
      return null;
    }

    setPathInput(normalized);
    setPathError(null);
    return normalized;
  }

  function handlePathBlur() {
    const normalized = normalizeProjectPath(pathInput);
    if (normalized) {
      setPathInput(normalized);
      setPathError(null);
    } else if (pathInput.trim()) {
      setPathError("Enter an absolute, normalized remote path.");
    }
  }

  function handleBrowse() {
    if (!selectedServerId || !connected) {
      return;
    }
    const path = validatePath();
    if (!path) {
      return;
    }
    void browse(selectedServerId, path);
  }

  function handleScan(includeChildren: boolean) {
    if (!selectedServerId || !connected) {
      return;
    }
    const path = validatePath();
    if (!path) {
      return;
    }

    setScanTarget(includeChildren ? "children" : "directory");
    void scan(selectedServerId, path, includeChildren).finally(() => {
      setScanTarget(null);
    });
  }

  function useDirectory(path: string) {
    setPathInput(path);
    setPathError(null);
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Projects"
        description="Browse to a remote directory or scan selected locations for Laravel projects. Detection never scans the whole server automatically."
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
              Remote project detection requires a live SSH session. Open the desktop build and
              connect a server to browse and scan selected directories. No project paths or versions
              are fabricated in browser preview.
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
          icon={<FolderSimpleIcon className="size-4" aria-hidden="true" />}
          title="No server selected"
          description="Add a server profile and connect it, then browse to a selected remote directory or scan one level of subdirectories for Laravel projects."
          action={
            <Button asChild size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      ) : null}

      {selected ? (
        <>
          <DetectionPanel
            servers={servers}
            selected={selected}
            connected={connected}
            pathInput={pathInput}
            pathError={pathError}
            entry={entry}
            scanTarget={scanTarget}
            onSelectServer={selectServer}
            onPathChange={(value) => {
              setPathInput(value);
              if (pathError) {
                setPathError(null);
              }
            }}
            onPathBlur={handlePathBlur}
            onBrowse={handleBrowse}
            onScan={handleScan}
          />

          {entry?.browseError ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Could not browse the selected directory</AlertTitle>
                <AlertDescription>{entry.browseError}</AlertDescription>
              </div>
              <Button variant="ghost" size="sm" onClick={() => clearBrowse(selected.id)}>
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {entry?.listing ? (
            <BrowseResults
              listing={entry.listing}
              onUsePath={useDirectory}
              onDismiss={() => clearBrowse(selected.id)}
            />
          ) : null}

          {entry?.scanError ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Could not scan the selected directory</AlertTitle>
                <AlertDescription>{entry.scanError}</AlertDescription>
              </div>
              <Button variant="ghost" size="sm" onClick={() => clearScan(selected.id)}>
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {entry?.scan ? <ScanSummary scan={entry.scan} /> : null}

          {entry?.error ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Could not load saved projects</AlertTitle>
                <AlertDescription>{entry.error}</AlertDescription>
              </div>
              <Button variant="ghost" size="sm" onClick={() => clearError(selected.id)}>
                Dismiss
              </Button>
            </Alert>
          ) : null}

          <section aria-label="Saved Laravel projects" className="space-y-3">
            <div className="flex flex-wrap items-end justify-between gap-2">
              <div>
                <h2 className="text-[15px] font-semibold tracking-[-0.01em] text-foreground">
                  Saved projects
                </h2>
                <p className="mt-0.5 text-[11px] text-muted-foreground">
                  Stored locally with server, name, remote path, type, versions, hints, and
                  detection time. Records persist across restarts.
                </p>
              </div>
              <Badge variant="outline">
                {projects.length} project{projects.length === 1 ? "" : "s"}
              </Badge>
            </div>

            {entry?.loading && projects.length === 0 ? <LoadingState rows={3} /> : null}

            {!entry?.loading && projects.length === 0 ? (
              <EmptyState
                icon={<FolderSimpleIcon className="size-4" aria-hidden="true" />}
                title={connected ? "No saved projects for this server" : "No saved projects yet"}
                description={
                  connected
                    ? "Browse to a selected directory or scan it for Laravel indicators. Detection only runs when you choose a path."
                    : "Saved projects remain available offline. Connect this server to browse or scan additional directories."
                }
              />
            ) : null}

            {projects.map((project) => (
              <ProjectCard
                key={project.id}
                project={project}
                connected={connected}
                actionEntry={actions[projectActionKey(project.serverId, project.id)]}
                onRequestAction={(action) => requestAction(project, action)}
                onDismissResult={() => clearActionResult(project.serverId, project.id)}
              />
            ))}
          </section>

          <ConfirmDialog
            open={pendingAction !== null}
            onOpenChange={(open) => {
              if (!open) {
                setPendingAction(null);
              }
            }}
            title={
              pendingAction
                ? `${artisanActionConfig[pendingAction.action].label} on ${pendingAction.project.name}?`
                : "Confirm Artisan action"
            }
            description={
              pendingAction
                ? (artisanActionConfig[pendingAction.action].confirmation ??
                  "Review the target project path and exact command before running this action.")
                : ""
            }
            confirmLabel="Run command"
            details={
              pendingAction
                ? [
                    {
                      label: "Server",
                      value: `${selected.name} (${selected.username}@${selected.host}:${selected.port})`,
                    },
                    { label: "Project", value: pendingAction.project.name },
                    { label: "Target path", value: pendingAction.project.remotePath },
                    {
                      label: "Command",
                      value: artisanCommand(pendingAction.project.remotePath, pendingAction.action),
                    },
                  ]
                : undefined
            }
            pending={
              pendingAction
                ? Boolean(
                    actions[
                      projectActionKey(pendingAction.project.serverId, pendingAction.project.id)
                    ]?.pending,
                  )
                : false
            }
            onConfirm={() => {
              if (!pendingAction) {
                return;
              }
              void runAction(
                pendingAction.project.serverId,
                pendingAction.project.id,
                pendingAction.action,
                true,
              );
              setPendingAction(null);
            }}
          />
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

export { ProjectsPage };
