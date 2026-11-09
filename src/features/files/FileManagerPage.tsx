import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  ArrowUpIcon,
  CaretRightIcon,
  CircleNotchIcon,
  FileIcon,
  FilesIcon,
  FolderOpenIcon,
  FolderSimpleIcon,
  InfoIcon,
  LinkIcon,
  ListMagnifyingGlassIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
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
import {
  formatFileSize,
  normalizeRemotePath,
  remoteBreadcrumbs,
  remoteParentPath,
} from "@/lib/remotePaths";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { useFileStore, type FileBrowserEntry } from "@/stores/fileStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  RemoteEntryType,
  RemoteFileEntry,
  RemoteFileListing,
  ServerProfile,
} from "@/types/contracts";

const DEFAULT_ROOT = "/";

const typeLabels: Record<RemoteEntryType, string> = {
  directory: "Directory",
  file: "File",
  symlink: "Symlink",
  other: "Other",
};

const typeVariants: Record<RemoteEntryType, "info" | "outline" | "warning" | "secondary"> = {
  directory: "info",
  file: "outline",
  symlink: "warning",
  other: "secondary",
};

function formatModifiedAt(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) {
    return "Unavailable";
  }
  const date = new Date(seconds * 1000);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function modifiedDateTimeValue(seconds: number | null): string | undefined {
  if (seconds === null || !Number.isFinite(seconds)) {
    return undefined;
  }
  const date = new Date(seconds * 1000);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function formatSampledAt(milliseconds: number): string {
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeFromMillis(milliseconds: number): string | undefined {
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function EntryIcon({ entry }: { entry: RemoteFileEntry }) {
  if (entry.entryType === "directory") {
    return <FolderSimpleIcon className="size-4 text-muted-foreground" aria-hidden="true" />;
  }
  if (entry.entryType === "symlink") {
    return <LinkIcon className="size-4 text-muted-foreground" aria-hidden="true" />;
  }
  return <FileIcon className="size-4 text-muted-foreground" aria-hidden="true" />;
}

function canOpenEntry(entry: RemoteFileEntry): boolean {
  if (entry.entryType === "directory") {
    return true;
  }
  return (
    entry.entryType === "symlink" && !entry.brokenSymlink && entry.symlinkEscapesRoot === false
  );
}

function entryBlockedReason(entry: RemoteFileEntry): string | null {
  if (entry.brokenSymlink) {
    return "This symbolic link points to a missing target, so ServerDesk will not follow it.";
  }
  if (entry.symlinkEscapesRoot === true) {
    return "This symbolic link resolves outside the selected root, so navigation is blocked.";
  }
  if (entry.entryType === "symlink" && entry.symlinkEscapesRoot !== false) {
    return "The target of this symbolic link could not be verified, so navigation is blocked.";
  }
  return null;
}

function SymlinkDetail({ entry }: { entry: RemoteFileEntry }) {
  if (entry.entryType !== "symlink") {
    return null;
  }

  return (
    <span className="mt-0.5 flex flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
      <span className="break-all font-mono">→ {entry.symlinkTarget ?? "target unavailable"}</span>
      {entry.brokenSymlink ? (
        <Badge variant="warning">Broken link</Badge>
      ) : entry.symlinkEscapesRoot === true ? (
        <Badge variant="danger">Outside root</Badge>
      ) : entry.symlinkEscapesRoot === false ? (
        <Badge variant="outline">Inside root</Badge>
      ) : (
        <Badge variant="warning">Unverified</Badge>
      )}
    </span>
  );
}

function FileBrowserToolbar({
  servers,
  selected,
  connected,
  pathInput,
  pathError,
  entry,
  onSelectServer,
  onPathChange,
  onBrowse,
  onRefresh,
  onUp,
}: {
  servers: ServerProfile[];
  selected: ServerProfile;
  connected: boolean;
  pathInput: string;
  pathError: string | null;
  entry: FileBrowserEntry | undefined;
  onSelectServer: (serverId: string) => void;
  onPathChange: (value: string) => void;
  onBrowse: () => void;
  onRefresh: () => void;
  onUp: () => void;
}) {
  const pending = entry?.pending ?? false;
  const listing = entry?.listing ?? null;
  const root = entry?.root ?? null;
  const current = entry?.path ?? null;
  const parent = root && current ? remoteParentPath(root, current) : null;
  const normalizedInput = normalizeRemotePath(pathInput);

  return (
    <section
      aria-label="File browser controls"
      className="rounded-md border border-border bg-card px-3.5 py-3"
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-end xl:justify-between">
        <div className="min-w-0 flex-1">
          <label
            htmlFor="files-server"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Server
          </label>
          <Select value={selected.id} onValueChange={onSelectServer}>
            <SelectTrigger id="files-server" className="mt-1 max-w-md">
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
                ? "SFTP over the authenticated SSH connection, on the same port"
                : "Connect this server to browse remote files"}
            </span>
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <StatusBadge state={connected ? "connected" : "disconnected"} />
          <Badge variant={normalizedInput ? "info" : "warning"}>
            {normalizedInput ? "Absolute root" : "Root required"}
          </Badge>
          <Button
            variant="secondary"
            size="sm"
            onClick={onUp}
            disabled={!connected || pending || !parent}
          >
            <ArrowUpIcon className="size-3.5" aria-hidden="true" />
            Up
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={onRefresh}
            disabled={!connected || pending || !current}
            aria-label="Refresh directory listing"
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

      <div className="mt-3 grid gap-2 border-t border-border pt-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
        <div className="min-w-0">
          <label
            htmlFor="files-root"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Selected root directory
          </label>
          <Input
            id="files-root"
            value={pathInput}
            onChange={(event) => onPathChange(event.target.value)}
            spellCheck={false}
            autoComplete="off"
            placeholder="/var/www"
            aria-invalid={pathError ? true : undefined}
            aria-describedby="files-root-help"
            className="mt-1 font-mono text-[12px]"
          />
        </div>
        <Button
          type="button"
          size="sm"
          className="h-9"
          onClick={onBrowse}
          disabled={!connected || pending}
        >
          {pending ? (
            <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
          ) : (
            <FolderOpenIcon className="size-3.5" aria-hidden="true" />
          )}
          {pending ? "Browsing..." : listing ? "Browse new root" : "Browse"}
        </Button>
      </div>

      <div id="files-root-help" className="mt-1.5 space-y-1">
        <p className="break-all font-mono text-[10px] text-muted-foreground">
          Target root: {normalizedInput ?? (pathInput.trim() || DEFAULT_ROOT)}
        </p>
        {pathError ? (
          <p className="text-[11px] text-destructive" role="alert">
            {pathError}
          </p>
        ) : (
          <p className="text-[11px] text-muted-foreground">
            Only the selected root and its descendants can be browsed. Parent segments, relative
            paths, and symlinks that resolve outside the root are rejected before a listing is
            returned.
          </p>
        )}
      </div>
    </section>
  );
}

function Breadcrumbs({
  listing,
  onNavigate,
}: {
  listing: RemoteFileListing;
  onNavigate: (path: string) => void;
}) {
  const crumbs = remoteBreadcrumbs(listing.root, listing.path);
  return (
    <nav aria-label="Current remote directory" className="flex flex-wrap items-center gap-1">
      {crumbs.map((crumb, index) => {
        const isCurrent = index === crumbs.length - 1;
        return (
          <span key={crumb.path} className="flex items-center gap-1">
            {index > 0 ? (
              <CaretRightIcon className="size-3 text-muted-foreground" aria-hidden="true" />
            ) : null}
            {isCurrent ? (
              <span
                aria-current="page"
                className="max-w-[16rem] truncate font-mono text-[11px] text-foreground"
                title={crumb.path}
              >
                {crumb.name}
              </span>
            ) : (
              <button
                type="button"
                onClick={() => onNavigate(crumb.path)}
                className="max-w-[12rem] truncate rounded-sm font-mono text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
                title={crumb.path}
              >
                {crumb.name}
              </button>
            )}
          </span>
        );
      })}
    </nav>
  );
}

function DirectoryListing({
  listing,
  pending,
  onNavigate,
  onOpenEntry,
}: {
  listing: RemoteFileListing;
  pending: boolean;
  onNavigate: (path: string) => void;
  onOpenEntry: (entry: RemoteFileEntry) => void;
}) {
  const directoryCount = listing.entries.filter((entry) => entry.entryType === "directory").length;

  return (
    <section
      aria-label="Remote directory listing"
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border px-3.5 py-3">
        <div className="min-w-0">
          <Breadcrumbs listing={listing} onNavigate={onNavigate} />
          <p className="mt-1 break-all font-mono text-[10px] text-muted-foreground">
            {listing.path}
          </p>
          {listing.resolvedPath !== listing.path ? (
            <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">
              Resolved to {listing.resolvedPath}
            </p>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="outline">
            {listing.entries.length} {listing.entries.length === 1 ? "entry" : "entries"}
          </Badge>
          <Badge variant="secondary">
            {directoryCount} {directoryCount === 1 ? "directory" : "directories"}
          </Badge>
          {listing.truncated ? <Badge variant="warning">Truncated</Badge> : null}
          {pending ? <Badge variant="info">Refreshing</Badge> : null}
        </div>
      </div>

      {listing.entries.length === 0 ? (
        <div className="px-3.5 py-8 text-center">
          <p className="text-[13px] font-medium text-foreground">This directory is empty</p>
          <p className="mt-1 text-[12px] text-muted-foreground">
            No files, directories, or symbolic links were returned for this path.
          </p>
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Type</TableHead>
              <TableHead className="text-right">Size</TableHead>
              <TableHead>Permissions</TableHead>
              <TableHead>Modified</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {listing.entries.map((entry) => {
              const openable = canOpenEntry(entry);
              const blockedReason = entryBlockedReason(entry);
              return (
                <TableRow key={entry.path}>
                  <TableCell className="max-w-[26rem] min-w-[14rem]">
                    <div className="flex min-w-0 items-start gap-2">
                      <span className="mt-0.5 shrink-0">
                        <EntryIcon entry={entry} />
                      </span>
                      <div className="min-w-0">
                        {openable ? (
                          <button
                            type="button"
                            onClick={() => onOpenEntry(entry)}
                            className="block max-w-full truncate rounded-sm text-left text-[12px] font-medium text-foreground underline-offset-2 hover:text-info hover:underline"
                            aria-label={`Open ${typeLabels[entry.entryType].toLowerCase()} ${entry.name}`}
                            title={entry.path}
                          >
                            {entry.name}
                          </button>
                        ) : (
                          <span
                            className="block max-w-full truncate text-[12px] font-medium text-foreground"
                            title={blockedReason ? `${entry.path} — ${blockedReason}` : entry.path}
                          >
                            {entry.name}
                          </span>
                        )}
                        <SymlinkDetail entry={entry} />
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <Badge variant={typeVariants[entry.entryType]}>
                      {typeLabels[entry.entryType]}
                    </Badge>
                  </TableCell>
                  <TableCell className="text-right font-mono text-[11px] whitespace-nowrap">
                    {formatFileSize(entry.size)}
                  </TableCell>
                  <TableCell className="font-mono text-[11px] whitespace-nowrap">
                    {entry.permissions ?? "Unavailable"}
                  </TableCell>
                  <TableCell className="text-[11px] whitespace-nowrap">
                    <time dateTime={modifiedDateTimeValue(entry.modifiedAt)}>
                      {formatModifiedAt(entry.modifiedAt)}
                    </time>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border px-3.5 py-2 text-[10px] text-muted-foreground">
        <span>
          Listed in {listing.durationMs} ms ·{" "}
          <time dateTime={dateTimeFromMillis(listing.sampledAt)}>
            {formatSampledAt(listing.sampledAt)}
          </time>
        </span>
        <span>
          Entry metadata only. ServerDesk does not read file content from this view and does not
          open a separate file-transfer port.
        </span>
      </div>
      {listing.truncated ? (
        <p className="border-t border-border px-3.5 py-2 text-[11px] text-warning">
          The listing stopped at the safety limit. Open a narrower directory to see the remaining
          entries.
        </p>
      ) : null}
    </section>
  );
}

function FileManagerPage() {
  const servers = useServerStore((state) => state.servers);
  const status = useServerStore((state) => state.status);
  const serverError = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const connections = useConnectionStore((state) => state.connections);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const lastActiveServerId = usePreferencesStore((state) => state.preferences.lastActiveServerId);
  const browse = useFileStore((state) => state.browse);
  const openRoot = useFileStore((state) => state.openRoot);
  const refresh = useFileStore((state) => state.refresh);
  const goUp = useFileStore((state) => state.goUp);
  const clearError = useFileStore((state) => state.clearError);

  const [selectedId, setSelectedId] = useState<string | null>(lastActiveServerId);
  const [pathInput, setPathInput] = useState(() => {
    const existing = lastActiveServerId
      ? useFileStore.getState().entries[lastActiveServerId]
      : undefined;
    return existing?.path ?? existing?.root ?? DEFAULT_ROOT;
  });
  const [pathError, setPathError] = useState<string | null>(null);

  const selected =
    servers.find((server) => server.id === selectedId) ??
    servers.find((server) => server.id === lastActiveServerId) ??
    servers[0] ??
    null;
  const selectedServerId = selected?.id ?? null;
  const connected = selected ? connections[selected.id]?.state === "connected" : false;
  const entry = useFileStore((state) =>
    selectedServerId ? state.entries[selectedServerId] : undefined,
  );
  const listing = entry?.listing ?? null;

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

  function selectServer(serverId: string) {
    setSelectedId(serverId);
    setPathError(null);
    const existing = useFileStore.getState().entries[serverId];
    setPathInput(existing?.path ?? existing?.root ?? DEFAULT_ROOT);
  }

  function handleBrowse() {
    if (!selected || !connected) {
      return;
    }
    const normalized = normalizeRemotePath(pathInput);
    if (!normalized) {
      setPathError(
        "Enter an absolute, normalized remote path such as /var/www. Parent segments and control characters are rejected.",
      );
      return;
    }
    setPathInput(normalized);
    setPathError(null);
    void openRoot(selected.id, normalized);
  }

  function handleOpenEntry(target: RemoteFileEntry) {
    if (!selected || !connected) {
      return;
    }
    const root = entry?.root ?? entry?.listing?.root;
    if (!root) {
      return;
    }
    if (!canOpenEntry(target)) {
      return;
    }
    void browse(selected.id, root, target.path);
  }

  function handleRefresh() {
    if (!selected || !connected) {
      return;
    }
    void refresh(selected.id);
  }

  function handleUp() {
    if (!selected || !connected) {
      return;
    }
    void goUp(selected.id);
  }

  function handleBreadcrumb(path: string) {
    if (!selected || !connected || !entry?.root) {
      return;
    }
    void browse(selected.id, entry.root, path);
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="File Manager"
        description="Browse remote files and directories over the authenticated SSH connection. Listings stay inside the selected root and report metadata without downloading file content."
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
              Remote file browsing needs a live SSH session. Open the desktop build and connect a
              server to browse directories. No remote files are fabricated in browser preview.
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
          icon={<FilesIcon className="size-4" aria-hidden="true" />}
          title="No server selected"
          description="Add a server profile and connect it to browse remote files over SFTP inside the authenticated SSH session."
          action={
            <Button asChild size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      ) : null}

      {selected ? (
        <>
          <FileBrowserToolbar
            servers={servers}
            selected={selected}
            connected={connected}
            pathInput={pathInput}
            pathError={pathError}
            entry={entry}
            onSelectServer={selectServer}
            onPathChange={(value) => {
              setPathInput(value);
              if (pathError) {
                setPathError(null);
              }
            }}
            onBrowse={handleBrowse}
            onRefresh={handleRefresh}
            onUp={handleUp}
          />

          {!connected ? (
            <Alert variant="info">
              <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
              <div>
                <AlertTitle>
                  {listing ? "Showing the last directory listing" : "File browsing is paused"}
                </AlertTitle>
                <AlertDescription>
                  {listing
                    ? "The SSH connection is not active, so this listing is not live. Its original sample time stays visible; reconnect to refresh it."
                    : "Connect this server to browse remote files over the authenticated SSH session."}
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
                <AlertTitle>Could not browse this remote path</AlertTitle>
                <AlertDescription>
                  <span className="block">{entry.error}</span>
                  {entry.errorDetails ? (
                    <span className="mt-1 block whitespace-pre-wrap font-mono text-[10px]">
                      {entry.errorDetails}
                    </span>
                  ) : null}
                  {listing
                    ? " The last successful listing stays visible with its original sample time."
                    : null}
                </AlertDescription>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  if (selectedServerId) {
                    clearError(selectedServerId);
                  }
                }}
              >
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {entry?.pending && !listing ? <LoadingState rows={5} /> : null}

          {listing ? (
            <DirectoryListing
              listing={listing}
              pending={entry?.pending ?? false}
              onNavigate={handleBreadcrumb}
              onOpenEntry={handleOpenEntry}
            />
          ) : null}

          {!entry?.pending && !listing && !entry?.error ? (
            <EmptyState
              icon={<ListMagnifyingGlassIcon className="size-4" aria-hidden="true" />}
              title={connected ? "No directory listed yet" : "File browsing is paused"}
              description={
                connected
                  ? "Enter an absolute root directory and choose Browse. Only that directory and its descendants are read."
                  : "Connect this server to browse remote files over SFTP on the authenticated SSH connection."
              }
              action={
                connected ? (
                  <Button size="sm" onClick={handleBrowse}>
                    Browse selected root
                  </Button>
                ) : undefined
              }
            />
          ) : null}

          <p className="text-[10px] text-muted-foreground">
            File metadata stays in memory only. ServerDesk does not stream file content in this
            view, does not store listings in SQLite, and does not require a separate public
            file-transfer port. Existing terminal and command flows use independent SSH channels.
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

export { FileManagerPage };
