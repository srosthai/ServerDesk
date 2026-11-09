import {
  useEffect,
  useRef,
  useState,
  type ComponentProps,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { Link } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  ArrowLeftIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  CaretRightIcon,
  CircleNotchIcon,
  DownloadSimpleIcon,
  FileIcon,
  FilesIcon,
  FolderOpenIcon,
  FolderPlusIcon,
  FolderSimpleIcon,
  InfoIcon,
  LinkIcon,
  ListMagnifyingGlassIcon,
  PencilSimpleIcon,
  TrashIcon,
  UploadSimpleIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
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
  CreateFolderDialog,
  DownloadSelectionDialog,
  RenameEntryDialog,
} from "@/features/files/FileMutationDialogs";
import {
  addFileSelectionRange,
  retainFileSelection,
  toggleFileSelection,
} from "@/features/files/fileSelection";
import {
  FileTransferPanel,
  TransferDialog,
  type TransferPrompt,
} from "@/features/files/FileTransferPanel";
import { useFileTransferEvents } from "@/hooks/useFileTransferEvents";
import {
  formatFileSize,
  normalizeRemotePath,
  remoteBreadcrumbs,
  remoteParentPath,
} from "@/lib/remotePaths";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import {
  canNavigateBack,
  canNavigateForward,
  useFileStore,
  type FileBrowserEntry,
  type FileTransferState,
} from "@/stores/fileStore";
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

function transferDestinationName(transfer: FileTransferState): string {
  const path =
    transfer.direction === "upload" ? transfer.request.remotePath : transfer.request.localPath;
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] ?? path;
}

function joinLocalPath(directory: string, name: string): string {
  const base = directory.replace(/[\\/]+$/, "");
  return `${base}/${name}`;
}

function SelectionCheckbox({
  indeterminate = false,
  ...props
}: { indeterminate?: boolean } & ComponentProps<"input">) {
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (ref.current) {
      ref.current.indeterminate = indeterminate;
    }
  }, [indeterminate]);

  return (
    <input
      ref={ref}
      type="checkbox"
      className="size-3.5 shrink-0 accent-primary focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      {...props}
    />
  );
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
  onBack,
  onForward,
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
  onBack: () => void;
  onForward: () => void;
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
            onClick={onBack}
            disabled={!connected || pending || !canNavigateBack(entry)}
            title="Go back to the previous directory"
          >
            <ArrowLeftIcon className="size-3.5" aria-hidden="true" />
            Back
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={onForward}
            disabled={!connected || pending || !canNavigateForward(entry)}
            title="Go forward to the next directory"
          >
            <ArrowRightIcon className="size-3.5" aria-hidden="true" />
            Forward
          </Button>
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
  onDownload,
  onRename,
  onDelete,
  onDownloadSelected,
  onDeleteSelected,
}: {
  listing: RemoteFileListing;
  pending: boolean;
  onNavigate: (path: string) => void;
  onOpenEntry: (entry: RemoteFileEntry) => void;
  onDownload: (entry: RemoteFileEntry) => void;
  onRename: (entry: RemoteFileEntry) => void;
  onDelete: (entry: RemoteFileEntry) => void;
  onDownloadSelected: (entries: RemoteFileEntry[]) => void;
  onDeleteSelected: (entries: RemoteFileEntry[]) => void;
}) {
  const directoryCount = listing.entries.filter((entry) => entry.entryType === "directory").length;
  const orderedPaths = listing.entries.map((entry) => entry.path);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(() => new Set());
  const [selectionAnchor, setSelectionAnchor] = useState<string | null>(null);
  const selectedEntries = listing.entries.filter((entry) => selectedPaths.has(entry.path));
  const downloadableSelected = selectedEntries.filter(
    (entry) => entry.entryType === "file" || entry.entryType === "directory",
  );
  const allSelected =
    listing.entries.length > 0 && selectedEntries.length === listing.entries.length;
  const someSelected = selectedEntries.length > 0 && !allSelected;
  const [activePath, setActivePath] = useState<string | null>(orderedPaths[0] ?? null);
  const effectiveActivePath =
    activePath && orderedPaths.includes(activePath) ? activePath : (orderedPaths[0] ?? null);

  function clearSelection() {
    setSelectedPaths(new Set());
    setSelectionAnchor(null);
  }

  function selectAll() {
    setSelectedPaths(new Set(orderedPaths));
    setSelectionAnchor(orderedPaths[0] ?? null);
  }

  function toggleSelection(path: string) {
    setSelectionAnchor(path);
    setSelectedPaths(toggleFileSelection(retainFileSelection(selectedPaths, orderedPaths), path));
  }

  function selectRange(path: string) {
    const anchor = selectionAnchor ?? path;
    setSelectionAnchor(anchor);
    setSelectedPaths(
      addFileSelectionRange(
        retainFileSelection(selectedPaths, orderedPaths),
        orderedPaths,
        anchor,
        path,
      ),
    );
  }

  function handleTableKeyDown(event: ReactKeyboardEvent<HTMLTableElement>) {
    const target = event.target as HTMLElement;
    const row = target.closest<HTMLTableRowElement>("tr[data-entry-path]");
    const index = row?.dataset.entryPath ? orderedPaths.indexOf(row.dataset.entryPath) : -1;

    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
      event.preventDefault();
      selectAll();
      return;
    }

    if (event.key === "Escape" && selectedEntries.length > 0) {
      event.preventDefault();
      clearSelection();
      return;
    }

    if (index < 0) {
      return;
    }

    if (
      event.key === "ArrowDown" ||
      event.key === "ArrowUp" ||
      event.key === "Home" ||
      event.key === "End"
    ) {
      event.preventDefault();
      const rows = Array.from(
        event.currentTarget.querySelectorAll<HTMLTableRowElement>("tbody tr[data-entry-path]"),
      );
      let nextIndex = index;
      if (event.key === "ArrowDown") {
        nextIndex = Math.min(orderedPaths.length - 1, index + 1);
      } else if (event.key === "ArrowUp") {
        nextIndex = Math.max(0, index - 1);
      } else if (event.key === "Home") {
        nextIndex = 0;
      } else {
        nextIndex = orderedPaths.length - 1;
      }
      const nextPath = orderedPaths[nextIndex];
      rows[nextIndex]?.focus();
      setActivePath(nextPath);
      if (event.shiftKey) {
        const anchor = selectionAnchor ?? orderedPaths[index];
        setSelectionAnchor(anchor);
        setSelectedPaths(addFileSelectionRange(selectedPaths, orderedPaths, anchor, nextPath));
      }
      return;
    }

    const interactive = target.closest("button, a, input, select, textarea");
    if (interactive) {
      return;
    }

    if (event.key === " " || event.key === "Enter") {
      event.preventDefault();
      const entry = listing.entries[index];
      if (event.key === "Enter" && canOpenEntry(entry)) {
        onOpenEntry(entry);
      } else {
        toggleSelection(entry.path);
      }
    }
  }

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

      {selectedEntries.length > 0 ? (
        <div
          role="status"
          aria-label="Selected remote entries"
          className="flex flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/40 px-3.5 py-2"
        >
          <p className="text-[12px] text-foreground">
            <span className="font-medium">
              {selectedEntries.length} {selectedEntries.length === 1 ? "entry" : "entries"}
            </span>{" "}
            selected
            <span className="ml-1 text-muted-foreground">
              · {downloadableSelected.length} downloadable
            </span>
          </p>
          <div className="flex flex-wrap items-center gap-1.5">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onDownloadSelected(downloadableSelected)}
              disabled={downloadableSelected.length === 0}
            >
              <DownloadSimpleIcon className="size-3.5" aria-hidden="true" />
              Download selected
            </Button>
            <Button
              variant="secondary"
              size="sm"
              className="text-destructive hover:text-destructive"
              onClick={() => onDeleteSelected(selectedEntries)}
            >
              <TrashIcon className="size-3.5" aria-hidden="true" />
              Delete selected
            </Button>
            <Button variant="ghost" size="sm" onClick={clearSelection}>
              Clear selection
            </Button>
          </div>
        </div>
      ) : null}

      {listing.entries.length === 0 ? (
        <div className="px-3.5 py-8 text-center">
          <p className="text-[13px] font-medium text-foreground">This directory is empty</p>
          <p className="mt-1 text-[12px] text-muted-foreground">
            No files, directories, or symbolic links were returned for this path.
          </p>
        </div>
      ) : (
        <Table
          aria-label="Remote directory entries"
          aria-describedby="file-table-keyboard-help"
          onKeyDown={handleTableKeyDown}
        >
          <TableHeader>
            <TableRow>
              <TableHead className="w-10 pr-0">
                <SelectionCheckbox
                  checked={allSelected}
                  indeterminate={someSelected}
                  aria-label="Select all entries"
                  onChange={() => {
                    if (allSelected) {
                      clearSelection();
                    } else {
                      selectAll();
                    }
                  }}
                />
              </TableHead>
              <TableHead>Name</TableHead>
              <TableHead>Type</TableHead>
              <TableHead className="text-right">Size</TableHead>
              <TableHead>Permissions</TableHead>
              <TableHead>Modified</TableHead>
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {listing.entries.map((entry) => {
              const openable = canOpenEntry(entry);
              const blockedReason = entryBlockedReason(entry);
              const isSelected = selectedPaths.has(entry.path);
              return (
                <TableRow
                  key={entry.path}
                  data-entry-path={entry.path}
                  data-state={isSelected ? "selected" : undefined}
                  tabIndex={effectiveActivePath === entry.path ? 0 : -1}
                  onFocus={() => setActivePath(entry.path)}
                  className="outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50"
                >
                  <TableCell className="w-10 pr-0">
                    <SelectionCheckbox
                      checked={isSelected}
                      aria-label={`Select ${entry.name}`}
                      onChange={() => toggleSelection(entry.path)}
                      onClick={(event) => {
                        if (event.shiftKey) {
                          event.preventDefault();
                          selectRange(entry.path);
                        }
                      }}
                    />
                  </TableCell>
                  <TableCell className="max-w-[24rem] min-w-[12rem]">
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
                  <TableCell>
                    <div className="flex justify-end gap-0.5">
                      {entry.entryType === "file" || entry.entryType === "directory" ? (
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => onDownload(entry)}
                          aria-label={`Download ${entry.name}`}
                          title={`Download ${entry.name}`}
                        >
                          <DownloadSimpleIcon className="size-3.5" aria-hidden="true" />
                        </Button>
                      ) : null}
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => onRename(entry)}
                        aria-label={`Rename ${entry.name}`}
                        title={`Rename ${entry.name}`}
                      >
                        <PencilSimpleIcon className="size-3.5" aria-hidden="true" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => onDelete(entry)}
                        aria-label={`Delete ${entry.name}`}
                        title={`Delete ${entry.name}`}
                        className="text-destructive hover:text-destructive"
                      >
                        <TrashIcon className="size-3.5" aria-hidden="true" />
                      </Button>
                    </div>
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
        <span id="file-table-keyboard-help">
          Arrow keys move between rows, Space selects, Enter opens, and Ctrl/Cmd+A selects all.
        </span>
      </div>
      <p className="flex flex-wrap items-center justify-between gap-2 border-t border-border px-3.5 py-2 text-[10px] text-muted-foreground">
        <span>
          Transfers use this same authenticated SSH session. No separate file-transfer port is
          opened.
        </span>
        <span>Shift+click a checkbox to extend the selection through that range.</span>
      </p>
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
  useFileTransferEvents();

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
  const goBack = useFileStore((state) => state.goBack);
  const goForward = useFileStore((state) => state.goForward);
  const clearError = useFileStore((state) => state.clearError);
  const startUpload = useFileStore((state) => state.startUpload);
  const startDownload = useFileStore((state) => state.startDownload);
  const cancelTransfer = useFileStore((state) => state.cancelTransfer);
  const retryTransfer = useFileStore((state) => state.retryTransfer);
  const dismissTransfer = useFileStore((state) => state.dismissTransfer);
  const createFolder = useFileStore((state) => state.createFolder);
  const renameEntry = useFileStore((state) => state.renameEntry);
  const deleteEntry = useFileStore((state) => state.deleteEntry);
  const deleteEntries = useFileStore((state) => state.deleteEntries);
  const clearMutation = useFileStore((state) => state.clearMutation);
  const transfers = useFileStore((state) => state.transfers);
  const mutations = useFileStore((state) => state.mutations);

  const [createFolderOpen, setCreateFolderOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState<RemoteFileEntry | null>(null);
  const [deleteTargets, setDeleteTargets] = useState<RemoteFileEntry[] | null>(null);
  const [downloadSelection, setDownloadSelection] = useState<RemoteFileEntry[] | null>(null);
  const [transferPrompt, setTransferPrompt] = useState<TransferPrompt | null>(null);
  const [overwriteTransfer, setOverwriteTransfer] = useState<FileTransferState | null>(null);

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
  const mutation = selectedServerId ? mutations[selectedServerId] : undefined;
  const root = entry?.root ?? listing?.root ?? null;
  const serverTransfers = selectedServerId
    ? Object.values(transfers)
        .filter((transfer) => transfer.serverId === selectedServerId)
        .sort((left, right) => {
          if (left.status === "running" && right.status !== "running") {
            return -1;
          }
          if (left.status !== "running" && right.status === "running") {
            return 1;
          }
          return right.startedAt - left.startedAt;
        })
    : [];

  const overwriteServer = overwriteTransfer
    ? (servers.find((server) => server.id === overwriteTransfer.serverId) ?? selected)
    : null;
  const serverLabel = selected
    ? `${selected.name} (${selected.username}@${selected.host}:${selected.port})`
    : "";
  const singleDeleteTarget = deleteTargets?.length === 1 ? deleteTargets[0] : null;
  const deleteTargetCount = deleteTargets?.length ?? 0;
  const deletePathPreview = (() => {
    const paths = deleteTargets?.map((target) => target.path) ?? [];
    if (paths.length <= 8) {
      return paths.join("\n");
    }
    return `${paths.slice(0, 8).join("\n")}\nand ${paths.length - 8} more`;
  })();

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

  function handleBack() {
    if (!selected || !connected) {
      return;
    }
    void goBack(selected.id);
  }

  function handleForward() {
    if (!selected || !connected) {
      return;
    }
    void goForward(selected.id);
  }

  function handleRetryBrowse() {
    if (!selected || !connected || !entry?.root) {
      return;
    }
    const target = entry.errorPath ?? entry.path ?? entry.root;
    void browse(selected.id, entry.root, target);
  }

  function handleDeleteSelected(targets: RemoteFileEntry[]) {
    if (targets.length > 0) {
      setDeleteTargets(targets);
    }
  }

  function startSelectedDownloads(localDirectory: string) {
    if (!selected || !root || !downloadSelection) {
      return;
    }
    for (const target of downloadSelection) {
      if (target.entryType !== "file" && target.entryType !== "directory") {
        continue;
      }
      void startDownload({
        serverId: selected.id,
        root,
        remotePath: target.path,
        localPath: joinLocalPath(localDirectory, target.name),
      });
    }
  }

  async function openOverwritePromptIfNeeded(transferId: string) {
    const transfer = useFileStore.getState().transfers[transferId];
    if (transfer?.errorCode === "conflict") {
      setOverwriteTransfer(transfer);
    }
  }

  function beginTransfer(input: {
    direction: "upload" | "download";
    remotePath: string;
    localPath: string;
  }): Promise<void> {
    if (!selected || !root) {
      return Promise.resolve();
    }
    const started =
      input.direction === "upload"
        ? startUpload({
            serverId: selected.id,
            root,
            localPath: input.localPath,
            remotePath: input.remotePath,
          })
        : startDownload({
            serverId: selected.id,
            root,
            remotePath: input.remotePath,
            localPath: input.localPath,
          });
    // Close the dialog immediately; the transfer panel owns progress and
    // cancellation while the backend keeps running.
    void started.then((transferId) => openOverwritePromptIfNeeded(transferId));
    return Promise.resolve();
  }

  async function handleRetry(transferId: string) {
    const nextId = await retryTransfer(transferId);
    if (nextId) {
      await openOverwritePromptIfNeeded(nextId);
    }
  }

  async function handleConfirmOverwrite() {
    const transfer = overwriteTransfer;
    if (!transfer) {
      return;
    }
    setOverwriteTransfer(null);
    const request = transfer.request;
    if (request.direction === "upload") {
      await startUpload({ ...request, overwrite: true });
    } else {
      await startDownload({ ...request, overwrite: true });
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="File Manager"
        description="Browse, upload, download, create, rename, and delete remote files over the authenticated SSH connection. Every transfer reports progress, cancellation, and partial-file state."
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
              Remote file operations need a live SSH session. Open the desktop build and connect a
              server to browse or transfer files. No remote files are fabricated in browser preview.
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
            onBack={handleBack}
            onForward={handleForward}
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
                <AlertTitle>
                  {entry.errorCode === "permission"
                    ? "Permission denied for this remote path"
                    : "Could not browse this remote path"}
                </AlertTitle>
                <AlertDescription>
                  {entry.errorCode === "permission" ? (
                    <span className="block">
                      The SSH account {selected.username} cannot read{" "}
                      {entry.errorPath ?? entry.path ?? "the requested path"} on {selected.name}.
                    </span>
                  ) : null}
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
              <div className="flex shrink-0 flex-wrap items-center gap-1">
                {connected && entry.root ? (
                  <Button variant="secondary" size="sm" onClick={handleRetryBrowse}>
                    Retry
                  </Button>
                ) : null}
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
              </div>
            </Alert>
          ) : null}

          {entry?.pending && !listing ? <LoadingState rows={5} /> : null}

          {listing ? (
            <section
              aria-label="File operations"
              className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border bg-card px-3.5 py-3"
            >
              <div>
                <h2 className="text-[13px] font-semibold text-foreground">File operations</h2>
                <p className="mt-0.5 text-[11px] text-muted-foreground">
                  Upload files or directories into {listing.path}. Rename and delete actions require
                  confirmation.
                </p>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => setCreateFolderOpen(true)}
                  disabled={!connected || (entry?.pending ?? false)}
                >
                  <FolderPlusIcon className="size-3.5" aria-hidden="true" />
                  New folder
                </Button>
                <Button
                  size="sm"
                  onClick={() => setTransferPrompt({ direction: "upload" })}
                  disabled={!connected || (entry?.pending ?? false)}
                >
                  <UploadSimpleIcon className="size-3.5" aria-hidden="true" />
                  Upload
                </Button>
              </div>
            </section>
          ) : null}

          <FileTransferPanel
            transfers={serverTransfers}
            onCancel={(transferId) => void cancelTransfer(transferId)}
            onRetry={(transferId) => void handleRetry(transferId)}
            onRequestOverwrite={(transfer) => setOverwriteTransfer(transfer)}
            onDismiss={dismissTransfer}
          />

          {mutation?.pending ? (
            <p className="flex items-center gap-2 text-[12px] text-muted-foreground" role="status">
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              Applying the remote change and verifying the result...
            </p>
          ) : null}

          {mutation?.result ? (
            <Alert variant={mutation.result.verified ? "success" : "warning"}>
              <InfoIcon className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
              <div className="flex-1">
                <AlertTitle>
                  {mutation.result.action === "create_folder"
                    ? "Folder created"
                    : mutation.result.action === "rename"
                      ? "Entry renamed"
                      : mutation.result.deletedEntries === 1
                        ? "Entry deleted"
                        : `${mutation.result.deletedEntries} entries deleted`}
                </AlertTitle>
                <AlertDescription>
                  <span className="block">
                    {mutation.result.targetPath ?? mutation.result.path}
                  </span>
                  <span className="mt-0.5 block">
                    {mutation.result.verified
                      ? "Verified by re-reading the remote path."
                      : (mutation.result.verificationDetail ??
                        "The operation completed but could not be fully verified.")}
                  </span>
                </AlertDescription>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  if (selectedServerId) {
                    clearMutation(selectedServerId);
                  }
                }}
              >
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {mutation?.error ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Could not change the remote filesystem</AlertTitle>
                <AlertDescription>
                  <span className="block">{mutation.error}</span>
                  {mutation.errorDetails ? (
                    <span className="mt-1 block whitespace-pre-wrap font-mono text-[10px]">
                      {mutation.errorDetails}
                    </span>
                  ) : null}
                </AlertDescription>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  if (selectedServerId) {
                    clearMutation(selectedServerId);
                  }
                }}
              >
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {listing ? (
            <DirectoryListing
              key={`${selectedServerId}:${listing.path}`}
              listing={listing}
              pending={entry?.pending ?? false}
              onNavigate={handleBreadcrumb}
              onOpenEntry={handleOpenEntry}
              onDownload={(target) => setTransferPrompt({ direction: "download", entry: target })}
              onRename={setRenameTarget}
              onDelete={(target) => setDeleteTargets([target])}
              onDownloadSelected={setDownloadSelection}
              onDeleteSelected={handleDeleteSelected}
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
            File listings and transfer results stay in memory only. Transfers use the authenticated
            SSH connection on the same port; ServerDesk never opens a separate public file-transfer
            listener. Existing terminal and command flows use independent SSH channels.
          </p>

          {createFolderOpen ? (
            <CreateFolderDialog
              open
              parent={listing?.path ?? root ?? DEFAULT_ROOT}
              serverLabel={serverLabel}
              onClose={() => setCreateFolderOpen(false)}
              onCreate={(name) =>
                selected && root && listing
                  ? createFolder(selected.id, root, listing.path, name)
                  : Promise.resolve(false)
              }
            />
          ) : null}

          {renameTarget ? (
            <RenameEntryDialog
              entry={renameTarget}
              serverLabel={serverLabel}
              onClose={() => setRenameTarget(null)}
              onRename={(newName) =>
                selected && root && renameTarget
                  ? renameEntry(selected.id, root, renameTarget.path, newName)
                  : Promise.resolve(false)
              }
            />
          ) : null}

          {downloadSelection ? (
            <DownloadSelectionDialog
              open
              entries={downloadSelection}
              serverLabel={serverLabel}
              remoteDirectory={listing?.path ?? root ?? DEFAULT_ROOT}
              onClose={() => setDownloadSelection(null)}
              onStart={(localDirectory) => {
                startSelectedDownloads(localDirectory);
                setDownloadSelection(null);
              }}
            />
          ) : null}

          <ConfirmDialog
            open={deleteTargets !== null}
            onOpenChange={(open) => {
              if (!open) {
                setDeleteTargets(null);
              }
            }}
            title={
              deleteTargetCount === 1
                ? `Delete ${singleDeleteTarget?.name}?`
                : `Delete ${deleteTargetCount} selected entries?`
            }
            description={
              deleteTargetCount > 1
                ? deleteTargets?.some((target) => target.entryType === "directory")
                  ? "This deletes the selected entries, including every entry inside selected directories. ServerDesk re-reads the directory afterward to verify the result."
                  : "This deletes the selected entries from the remote server. ServerDesk re-reads the directory afterward to verify the result."
                : singleDeleteTarget?.entryType === "directory"
                  ? "This deletes the directory and every entry inside it. ServerDesk re-reads the path afterward to verify the result."
                  : "This deletes the entry from the remote server. ServerDesk re-reads the path afterward to verify the result."
            }
            confirmLabel={
              deleteTargetCount > 1
                ? `Delete ${deleteTargetCount} entries`
                : singleDeleteTarget?.entryType === "directory"
                  ? "Delete directory"
                  : "Delete file"
            }
            details={
              deleteTargets
                ? [
                    { label: "Server", value: serverLabel },
                    deleteTargetCount === 1
                      ? { label: "Path", value: singleDeleteTarget?.path ?? "" }
                      : { label: "Targets", value: deletePathPreview },
                    deleteTargetCount === 1
                      ? {
                          label: "Type",
                          value: singleDeleteTarget ? typeLabels[singleDeleteTarget.entryType] : "",
                        }
                      : { label: "Count", value: String(deleteTargetCount) },
                  ]
                : undefined
            }
            pending={mutation?.pending ?? false}
            onConfirm={() => {
              const targets = deleteTargets;
              if (!selected || !root || !targets || targets.length === 0) {
                return;
              }
              const operation =
                targets.length === 1
                  ? deleteEntry(
                      selected.id,
                      root,
                      targets[0].path,
                      targets[0].entryType === "directory",
                    )
                  : deleteEntries(
                      selected.id,
                      root,
                      targets.map((target) => ({
                        path: target.path,
                        recursive: target.entryType === "directory",
                      })),
                    );
              void operation.finally(() => setDeleteTargets(null));
            }}
          />

          {transferPrompt ? (
            <TransferDialog
              prompt={transferPrompt}
              server={selected}
              listing={listing}
              onClose={() => setTransferPrompt(null)}
              onStart={beginTransfer}
            />
          ) : null}

          <ConfirmDialog
            open={overwriteTransfer !== null}
            onOpenChange={(open) => {
              if (!open) {
                setOverwriteTransfer(null);
              }
            }}
            title={
              overwriteTransfer
                ? `Overwrite ${transferDestinationName(overwriteTransfer)}?`
                : "Overwrite the existing destination?"
            }
            description={
              overwriteTransfer
                ? `The destination already exists on ${overwriteServer?.name ?? "this server"}. ServerDesk writes to a separate partial file and replaces the destination only after the complete transfer is verified.`
                : ""
            }
            confirmLabel="Overwrite and transfer"
            details={
              overwriteTransfer
                ? [
                    {
                      label: "Server",
                      value: overwriteServer
                        ? `${overwriteServer.name} (${overwriteServer.username}@${overwriteServer.host}:${overwriteServer.port})`
                        : serverLabel,
                    },
                    {
                      label: "Direction",
                      value: overwriteTransfer.direction === "upload" ? "Upload" : "Download",
                    },
                    {
                      label: "Source",
                      value:
                        overwriteTransfer.direction === "upload"
                          ? overwriteTransfer.request.localPath
                          : overwriteTransfer.request.remotePath,
                    },
                    {
                      label: "Destination",
                      value:
                        overwriteTransfer.direction === "upload"
                          ? overwriteTransfer.request.remotePath
                          : overwriteTransfer.request.localPath,
                    },
                  ]
                : undefined
            }
            onConfirm={() => void handleConfirmOverwrite()}
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

export { FileManagerPage };
