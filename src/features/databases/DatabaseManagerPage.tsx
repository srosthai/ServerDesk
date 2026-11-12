import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import type { Icon } from "@phosphor-icons/react";
import {
  ArrowClockwiseIcon,
  BroomIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  CodeIcon,
  DatabaseIcon,
  HardDrivesIcon,
  InfoIcon,
  KeyIcon,
  ListChecksIcon,
  PlugsIcon,
  PlayIcon,
  PlusIcon,
  RowsIcon,
  TableIcon,
  TreeStructureIcon,
  WarningCircleIcon,
  WrenchIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { ErrorState, LoadingState } from "@/components/ui/states";
import { StatusBadge } from "@/components/ui/status-badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { DatabaseConnectionList, EngineBadge } from "@/features/databases/DatabaseConnectionList";
import { DatabaseActionDialog } from "@/features/databases/DatabaseActionDialog";
import { DatabaseExplorerTree } from "@/features/databases/DatabaseExplorerTree";
import {
  databaseRowId,
  type DatabaseExplorerSelection,
} from "@/features/databases/databaseExplorerTreeModel";
import { DatabaseProfileDialog } from "@/features/databases/DatabaseProfileDialog";
import {
  DATABASE_ENGINE_LABELS,
  databaseConnectionStateLabel,
  formatDatabaseEndpoint,
  formatLatency,
} from "@/features/databases/databaseProfileForm";
import { useActiveServer } from "@/hooks/useActiveServer";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { dropDatabase, previewDatabaseAction, setActiveDatabase } from "@/services/databaseActions";
import { useConnectionStore } from "@/stores/connectionStore";
import { useDatabaseExplorerStore } from "@/stores/databaseExplorerStore";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import {
  useDatabaseWorkspaceStore,
  type DatabaseWorkspaceTab,
  type DatabaseWorkspaceTaskStatus,
  type DatabaseWorkspaceView,
} from "@/stores/databaseWorkspaceStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ConnectionState,
  DatabaseActionResult,
  DatabaseConnectionSnapshot,
  DatabaseConnectionState,
  DatabaseDdlPreview,
  DatabaseProfile,
  DatabaseTestResult,
  DatabaseTunnelSnapshot,
  ServerProfile,
} from "@/types/contracts";

type WorkspaceState = "loading" | "error" | "empty" | "no-server" | "ready";

type DatabaseTab = {
  value: DatabaseWorkspaceView;
  label: string;
  icon: Icon;
  emptyTitle: string;
  emptyDescription: string;
};

const databaseTabs: DatabaseTab[] = [
  {
    value: "overview",
    label: "Overview",
    icon: DatabaseIcon,
    emptyTitle: "No database connections yet",
    emptyDescription:
      "Add a MySQL or PostgreSQL connection profile to explore schemas, inspect data, and run queries over a secure SSH tunnel.",
  },
  {
    value: "structure",
    label: "Structure",
    icon: TableIcon,
    emptyTitle: "No table selected",
    emptyDescription:
      "Select a table from the explorer to inspect its columns, types, defaults, and constraints.",
  },
  {
    value: "data",
    label: "Data",
    icon: RowsIcon,
    emptyTitle: "No rows to show",
    emptyDescription:
      "Choose a table and a page of records. Data stays paginated so large tables are never loaded at once.",
  },
  {
    value: "sql",
    label: "SQL",
    icon: CodeIcon,
    emptyTitle: "The SQL workspace is empty",
    emptyDescription:
      "Query tabs, execution plans, and results will appear here once a database connection is active.",
  },
  {
    value: "indexes",
    label: "Indexes",
    icon: KeyIcon,
    emptyTitle: "No indexes to inspect",
    emptyDescription:
      "Select a table to review its primary key, unique constraints, and secondary indexes.",
  },
  {
    value: "operations",
    label: "Operations",
    icon: WrenchIcon,
    emptyTitle: "No operations queued",
    emptyDescription:
      "Imports, exports, backups, and restores will report progress and cancellation from this workspace.",
  },
];

const EMPTY_PROFILES: DatabaseProfile[] = [];
const EMPTY_SNAPSHOTS: Record<string, DatabaseConnectionSnapshot> = {};
const EMPTY_TEST_RESULTS: Record<string, DatabaseTestResult> = {};
const EMPTY_BUSY: Record<string, boolean> = {};

function workspaceStateFor({
  serverStatus,
  serverCount,
  activeServer,
}: {
  serverStatus: string;
  serverCount: number;
  activeServer: ServerProfile | null;
}): WorkspaceState {
  if (serverStatus === "loading" && serverCount === 0) {
    return "loading";
  }
  if (serverStatus === "error" && serverCount === 0) {
    return "error";
  }
  if (serverCount === 0) {
    return "empty";
  }
  if (!activeServer) {
    return "no-server";
  }
  return "ready";
}

function sshStateLabel(state: ConnectionState): string {
  switch (state) {
    case "connected":
      return "SSH connected";
    case "connecting":
      return "SSH connecting";
    case "reconnecting":
      return "SSH reconnecting";
    case "host-key-verification":
      return "SSH host key";
    case "error":
      return "SSH error";
    default:
      return "SSH disconnected";
  }
}

function tunnelSummary(tunnel: DatabaseTunnelSnapshot | undefined): string {
  if (!tunnel) {
    return "No tunnel";
  }
  switch (tunnel.state) {
    case "active":
      return tunnel.localPort ? `Tunnel ${tunnel.localHost}:${tunnel.localPort}` : "Tunnel active";
    case "opening":
      return "Tunnel opening";
    default:
      return "Tunnel closed";
  }
}

function statusMessage({
  state,
  activeServer,
  sshConnected,
  profiles,
  notice,
}: {
  state: WorkspaceState;
  activeServer: ServerProfile | null;
  sshConnected: boolean;
  profiles: DatabaseProfile[];
  notice: string | null;
}): string {
  if (notice) {
    return notice;
  }
  switch (state) {
    case "loading":
      return "Loading saved server profiles...";
    case "error":
      return "Server profiles could not be loaded.";
    case "empty":
      return "No server selected. Add or select a server profile.";
    case "no-server":
      return "No active server selected.";
    default:
      if (!activeServer) {
        return "No active server selected.";
      }
      if (!sshConnected) {
        return `SSH disconnected from ${activeServer.name}. Profiles can be edited; connect the server to open tunnels.`;
      }
      if (profiles.length === 0) {
        return "Workspace ready. No database connections configured.";
      }
      return `${profiles.length} saved connection${profiles.length === 1 ? "" : "s"} on ${activeServer.name}.`;
  }
}

function StatusDot({ state }: { state: WorkspaceState }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "size-1.5 shrink-0 rounded-full",
        state === "ready"
          ? "bg-success"
          : state === "error"
            ? "bg-destructive"
            : state === "loading"
              ? "animate-pulse bg-warning"
              : "bg-muted-foreground/60",
      )}
    />
  );
}

function profileStateColor(state: DatabaseConnectionState): string {
  switch (state) {
    case "connected":
      return "bg-success";
    case "error":
      return "bg-destructive";
    case "connecting":
    case "reconnecting":
      return "animate-pulse bg-warning";
    default:
      return "bg-muted-foreground/50";
  }
}

function WorkspaceStatePanel({
  state,
  error,
  onRetry,
}: {
  state: Exclude<WorkspaceState, "ready">;
  error: string | null;
  onRetry: () => void;
}) {
  if (state === "loading") {
    return (
      <div className="space-y-4">
        <div>
          <h2 className="text-[15px] font-semibold tracking-[-0.01em] text-foreground">
            Loading database workspace
          </h2>
          <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">
            Reading saved server profiles and connection state. No database driver is loaded.
          </p>
        </div>
        <LoadingState rows={4} />
      </div>
    );
  }

  if (state === "error") {
    return (
      <ErrorState
        title="Could not load server profiles"
        message={error ?? "The local server profile store did not respond."}
        onRetry={onRetry}
      />
    );
  }

  if (state === "empty") {
    return (
      <EmptyState
        icon={<HardDrivesIcon className="size-4" aria-hidden="true" />}
        title="No server profiles yet"
        description="Add a server profile and connect it over SSH before opening the database workspace."
        action={
          <Button asChild size="sm">
            <Link to="/servers">Open servers</Link>
          </Button>
        }
      />
    );
  }

  return (
    <EmptyState
      icon={<PlugsIcon className="size-4" aria-hidden="true" />}
      title="No active server selected"
      description="Choose an active server profile before creating database connections. Server selection is shared with the rest of ServerDesk."
      action={
        <Button asChild variant="secondary" size="sm">
          <Link to="/servers">Open servers</Link>
        </Button>
      }
    />
  );
}

function DatabaseWorkspaceTopBar({
  activeServer,
  connectionState,
  selectedProfile,
  selectedState,
  selectedDatabase,
  selectedSchema,
  selectedObject,
  refreshing,
  onRefresh,
  onAddConnection,
}: {
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
  selectedProfile: DatabaseProfile | null;
  selectedState: DatabaseConnectionState;
  selectedDatabase: string | null;
  selectedSchema: string | null;
  selectedObject: { label: string; kind: string } | null;
  refreshing: boolean;
  onRefresh: () => void;
  onAddConnection: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-4 py-3">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="grid size-8 shrink-0 place-items-center rounded-md border border-border bg-muted/50 text-muted-foreground">
          <DatabaseIcon className="size-4" aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <h2 className="text-[13px] font-semibold tracking-[-0.01em] text-foreground">
            Database workspace
          </h2>
          <div
            aria-label="Selected database and schema"
            className="mt-0.5 flex min-w-0 flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground"
          >
            <span className="inline-flex max-w-44 items-center gap-1 rounded-sm border border-border bg-muted/40 px-1.5 py-0.5">
              <DatabaseIcon className="size-3 shrink-0" aria-hidden="true" />
              <span className="shrink-0">DB</span>
              <span className="truncate font-mono text-foreground">
                {selectedDatabase ?? "none"}
              </span>
            </span>
            <span aria-hidden="true">/</span>
            <span className="inline-flex max-w-44 items-center gap-1 rounded-sm border border-border bg-muted/40 px-1.5 py-0.5">
              <TreeStructureIcon className="size-3 shrink-0" aria-hidden="true" />
              <span className="shrink-0">Schema</span>
              <span className="truncate font-mono text-foreground">{selectedSchema ?? "none"}</span>
            </span>
            {selectedObject ? (
              <>
                <span aria-hidden="true">/</span>
                <span className="inline-flex max-w-52 items-center gap-1 rounded-sm border border-border bg-muted/40 px-1.5 py-0.5">
                  <TableIcon className="size-3 shrink-0" aria-hidden="true" />
                  <span className="shrink-0 capitalize">{selectedObject.kind}</span>
                  <span className="truncate font-mono text-foreground">{selectedObject.label}</span>
                </span>
              </>
            ) : null}
          </div>
        </div>
        <StatusBadge
          state={activeServer ? connectionState : "disconnected"}
          label={activeServer ? sshStateLabel(connectionState) : "No server"}
        />
        {selectedProfile ? (
          <>
            <EngineBadge engine={selectedProfile.engine} />
            <StatusBadge
              state={selectedState}
              label={databaseConnectionStateLabel(selectedState)}
            />
          </>
        ) : (
          <Badge variant="outline">No connection selected</Badge>
        )}
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <div className="hidden min-w-0 text-right sm:block">
          <p className="max-w-56 truncate text-[11px] font-medium text-foreground">
            {selectedProfile?.name ?? activeServer?.name ?? "No server selected"}
          </p>
          <p className="max-w-64 truncate font-mono text-[10px] text-muted-foreground">
            {selectedProfile
              ? formatDatabaseEndpoint(
                  selectedProfile.host,
                  selectedProfile.port,
                  selectedProfile.socket,
                )
              : activeServer
                ? `${activeServer.username}@${activeServer.host}:${activeServer.port}`
                : "Choose or add a server profile"}
          </p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={onRefresh}
          disabled={refreshing}
          aria-label="Refresh database connections"
        >
          {refreshing ? (
            <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
          ) : (
            <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
          )}
          {refreshing ? "Refreshing..." : "Refresh"}
        </Button>
        <Button size="sm" onClick={onAddConnection} disabled={!isTauriRuntime()}>
          <PlusIcon className="size-3.5" aria-hidden="true" />
          Add connection
        </Button>
      </div>
    </div>
  );
}

function DatabaseExplorerRail({
  activeServer,
  connectionState,
  profiles,
  profileStatus,
  snapshots,
  selectedProfileId,
  selection,
  onSelectProfile,
  onSelectionChange,
  onConnectProfile,
  onAddConnection,
  onRetry,
  onCreateDatabase,
  onSetActiveDatabase,
  onRenameDatabase,
  onDropDatabase,
}: {
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
  profiles: DatabaseProfile[];
  profileStatus: "idle" | "loading" | "ready" | "error";
  snapshots: Record<string, DatabaseConnectionSnapshot>;
  selectedProfileId: string | null;
  selection: DatabaseExplorerSelection | null;
  onSelectProfile: (profileId: string) => void;
  onSelectionChange: (selection: DatabaseExplorerSelection | null) => void;
  onConnectProfile: (profile: DatabaseProfile) => void;
  onAddConnection: () => void;
  onRetry: () => void;
  onCreateDatabase: (profile: DatabaseProfile) => void;
  onSetActiveDatabase: (profile: DatabaseProfile, database: string) => void;
  onRenameDatabase: (profile: DatabaseProfile, database: string) => void;
  onDropDatabase: (profile: DatabaseProfile, database: string) => void;
}) {
  return (
    <aside
      aria-label="Database explorer"
      className="flex min-h-0 min-w-0 flex-col border-b border-border bg-muted/15 lg:border-r lg:border-b-0"
    >
      <div className="border-b border-border px-3 py-3">
        <div className="flex items-center justify-between gap-2">
          <h2 className="flex items-center gap-1.5 text-[10px] font-semibold tracking-wide text-muted-foreground uppercase">
            <TreeStructureIcon className="size-3.5" aria-hidden="true" />
            Explorer
          </h2>
          <div className="flex items-center gap-1">
            <Badge variant="outline">{profiles.length}</Badge>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={onAddConnection}
              disabled={!isTauriRuntime()}
              aria-label="Add database connection"
              title="Add database connection"
            >
              <PlusIcon className="size-3.5" aria-hidden="true" />
            </Button>
          </div>
        </div>

        <div className="mt-2 flex min-w-0 items-center gap-2 rounded-md border border-border bg-card/60 px-2 py-1.5">
          <span className="grid size-6 shrink-0 place-items-center rounded-sm border border-border bg-card text-muted-foreground">
            <HardDrivesIcon className="size-3.5" aria-hidden="true" />
          </span>
          <div className="min-w-0 flex-1">
            <p className="truncate text-[11px] font-medium text-foreground">
              {activeServer?.name ?? "No server selected"}
            </p>
            <p className="truncate font-mono text-[9px] text-muted-foreground">
              {activeServer
                ? `${activeServer.username}@${activeServer.host}:${activeServer.port}`
                : "Choose a profile to begin"}
            </p>
          </div>
          <StatusBadge
            state={activeServer ? connectionState : "disconnected"}
            label={activeServer ? sshStateLabel(connectionState) : "No server"}
          />
        </div>
      </div>

      <DatabaseExplorerTree
        profiles={profiles}
        profileStatus={profileStatus}
        snapshots={snapshots}
        selectedProfileId={selectedProfileId}
        selection={selection}
        onSelectProfile={onSelectProfile}
        onSelectionChange={onSelectionChange}
        onConnectProfile={onConnectProfile}
        onAddConnection={onAddConnection}
        onRetryProfiles={onRetry}
        onCreateDatabase={onCreateDatabase}
        onSetActiveDatabase={onSetActiveDatabase}
        onRenameDatabase={onRenameDatabase}
        onDropDatabase={onDropDatabase}
      />

      <div className="border-t border-border px-3 py-2">
        <p className="text-[10px] leading-4 text-muted-foreground">
          {selection?.kind === "database" && selection.engine === "mysql"
            ? "MySQL does not support renaming a database safely. Create a new database and migrate its data instead."
            : "Metadata loads one node at a time. Tunnels use 127.0.0.1 only."}
        </p>
      </div>
    </aside>
  );
}

function DatabaseWorkspaceStatusBar({
  state,
  activeServer,
  sshConnected,
  profiles,
  selectedProfile,
  selectedSnapshot,
  selectedTestResult,
  notice,
}: {
  state: WorkspaceState;
  activeServer: ServerProfile | null;
  sshConnected: boolean;
  profiles: DatabaseProfile[];
  selectedProfile: DatabaseProfile | null;
  selectedSnapshot: DatabaseConnectionSnapshot | undefined;
  selectedTestResult: DatabaseTestResult | undefined;
  notice: string | null;
}) {
  const selectedState = selectedSnapshot?.state ?? "disconnected";
  const version = selectedSnapshot?.version?.version ?? selectedTestResult?.version.version;
  const activeDatabase = selectedSnapshot?.databaseName ?? selectedProfile?.databaseName ?? null;

  return (
    <div
      aria-label="Database workspace status"
      className="flex flex-wrap items-center justify-between gap-2 border-t border-border bg-muted/30 px-4 py-2"
    >
      <p
        role="status"
        aria-live="polite"
        className="flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground"
      >
        <StatusDot state={state} />
        <span className="truncate">
          {statusMessage({ state, activeServer, sshConnected, profiles, notice })}
        </span>
      </p>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-muted-foreground">
        {selectedProfile ? (
          <>
            <span className="inline-flex items-center gap-1">
              <span
                aria-hidden="true"
                className={cn("size-1.5 rounded-full", profileStateColor(selectedState))}
              />
              {databaseConnectionStateLabel(selectedState)}
            </span>
            <span aria-hidden="true">·</span>
            <span>{tunnelSummary(selectedSnapshot?.tunnel)}</span>
            <span aria-hidden="true">·</span>
            <span>
              {selectedProfile.engine === "mysql" ? "MySQL" : "PostgreSQL"}{" "}
              {version ?? "version unknown"}
            </span>
            <span aria-hidden="true">·</span>
            <span>Latency {formatLatency(selectedTestResult?.latencyMs)}</span>
            <span aria-hidden="true">·</span>
            <span>DB {activeDatabase ?? "none"}</span>
          </>
        ) : (
          <span>No connection selected</span>
        )}
      </div>
    </div>
  );
}

function DatabaseWorkspaceTabBar({
  tabs,
  activeTabId,
  snapshots,
  onActivate,
  onClose,
}: {
  tabs: DatabaseWorkspaceTab[];
  activeTabId: string | null;
  snapshots: Record<string, DatabaseConnectionSnapshot>;
  onActivate: (profileId: string) => void;
  onClose: (profileId: string) => void;
}) {
  if (tabs.length === 0) {
    return null;
  }

  return (
    <div className="flex items-center gap-2 border-b border-border bg-muted/20 px-2 pt-1.5">
      <div
        role="tablist"
        aria-label="Open database connections"
        className="flex min-w-0 flex-1 items-end gap-1 overflow-x-auto"
      >
        {tabs.map((tab) => {
          const state = snapshots[tab.profileId]?.state ?? "disconnected";
          const active = tab.profileId === activeTabId;
          return (
            <div
              key={tab.profileId}
              className={cn(
                "flex shrink-0 items-center rounded-t-md border border-b-0",
                active
                  ? "border-border bg-card text-foreground shadow-sm"
                  : "border-transparent bg-muted/30 text-muted-foreground hover:bg-muted/50",
              )}
            >
              <button
                type="button"
                role="tab"
                aria-selected={active}
                aria-label={`${tab.name} (${DATABASE_ENGINE_LABELS[tab.engine]}) workspace tab`}
                onClick={() => onActivate(tab.profileId)}
                className="flex min-w-0 items-center gap-1.5 rounded-tl-md py-1.5 pr-1.5 pl-2.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
              >
                <span
                  aria-hidden="true"
                  className={cn("size-1.5 shrink-0 rounded-full", profileStateColor(state))}
                />
                <span className="max-w-36 truncate text-[12px] font-medium">{tab.name}</span>
                <span className="text-[9px] font-medium tracking-wide text-muted-foreground uppercase">
                  {DATABASE_ENGINE_LABELS[tab.engine]}
                </span>
              </button>
              <button
                type="button"
                onClick={() => onClose(tab.profileId)}
                aria-label={`Close ${tab.name} workspace tab`}
                title={`Close ${tab.name}, disconnect its tunnel, and clear its tasks`}
                className="mr-1 grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
              >
                <XIcon className="size-3" aria-hidden="true" />
              </button>
            </div>
          );
        })}
      </div>
      <span className="hidden shrink-0 pr-1 text-[10px] text-muted-foreground sm:block">
        {tabs.length} open
      </span>
    </div>
  );
}

function DatabaseWorkspaceTabHeader({
  tab,
  snapshot,
  server,
}: {
  tab: DatabaseWorkspaceTab;
  snapshot: DatabaseConnectionSnapshot | undefined;
  server: ServerProfile | null;
}) {
  const state = snapshot?.state ?? "disconnected";
  const selection = tab.selection;
  const database = selection?.database ?? snapshot?.databaseName ?? tab.databaseName;
  const schema = selection?.schema ?? (tab.engine === "mysql" ? database : null);
  const object =
    selection &&
    (selection.kind === "table" || selection.kind === "view" || selection.kind === "sequence")
      ? (selection.objectName ?? selection.label)
      : null;

  return (
    <div
      aria-label="Workspace tab header"
      className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-border bg-muted/15 px-4 py-2"
    >
      <div className="flex min-w-0 items-center gap-2">
        <span className="grid size-7 shrink-0 place-items-center rounded-md border border-border bg-card text-muted-foreground">
          <DatabaseIcon className="size-3.5" aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
            <h2 className="max-w-52 truncate text-[13px] font-semibold tracking-[-0.01em] text-foreground">
              {tab.name}
            </h2>
            <EngineBadge engine={tab.engine} />
            <StatusBadge state={state} label={databaseConnectionStateLabel(state)} />
            {tab.readOnly ? <Badge variant="warning">Read-only</Badge> : null}
          </div>
          <p className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground">
            {server?.name ?? tab.serverId} ·{" "}
            {formatDatabaseEndpoint(tab.host, tab.port, tab.socket)}
          </p>
        </div>
      </div>

      <div
        aria-label="Workspace tab context"
        className="flex min-w-0 flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground"
      >
        <span className="inline-flex max-w-44 items-center gap-1 rounded-sm border border-border bg-card px-1.5 py-0.5">
          <DatabaseIcon className="size-3 shrink-0" aria-hidden="true" />
          <span className="shrink-0">DB</span>
          <span className="truncate font-mono text-foreground">{database ?? "none"}</span>
        </span>
        <span aria-hidden="true">/</span>
        <span className="inline-flex max-w-44 items-center gap-1 rounded-sm border border-border bg-card px-1.5 py-0.5">
          <TreeStructureIcon className="size-3 shrink-0" aria-hidden="true" />
          <span className="shrink-0">Schema</span>
          <span className="truncate font-mono text-foreground">{schema ?? "none"}</span>
        </span>
        {object ? (
          <>
            <span aria-hidden="true">/</span>
            <span className="inline-flex max-w-52 items-center gap-1 rounded-sm border border-border bg-card px-1.5 py-0.5">
              <TableIcon className="size-3 shrink-0" aria-hidden="true" />
              <span className="truncate font-mono text-foreground">{object}</span>
            </span>
          </>
        ) : null}
      </div>
    </div>
  );
}

function SqlResultPanel({ tab }: { tab: DatabaseWorkspaceTab }) {
  const result = tab.sqlResult;
  if (!result) {
    return (
      <EmptyState
        icon={<RowsIcon className="size-4" aria-hidden="true" />}
        title="No query results yet"
        description={`Results, errors, and execution time stay with the ${tab.name} tab when you switch connections.`}
      />
    );
  }

  return (
    <div
      role="region"
      aria-label={`SQL results for ${tab.name}`}
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/20 px-3 py-2">
        <p className="flex items-center gap-1.5 text-[11px] font-medium text-foreground">
          {result.error ? (
            <WarningCircleIcon className="size-3.5 text-destructive" aria-hidden="true" />
          ) : (
            <CheckCircleIcon className="size-3.5 text-success" aria-hidden="true" />
          )}
          {result.error ? "Query failed" : "Query finished"}
          <span className="font-normal text-muted-foreground">
            {result.affectedRows !== null
              ? `${result.affectedRows} row${result.affectedRows === 1 ? "" : "s"} affected`
              : `${result.rows.length} row${result.rows.length === 1 ? "" : "s"}`}
          </span>
        </p>
        <p className="font-mono text-[10px] text-muted-foreground">
          {result.durationMs} ms · {new Date(result.executedAt).toLocaleTimeString()}
        </p>
      </div>
      {result.error ? (
        <p className="px-3 py-2.5 text-[11px] leading-5 text-destructive">{result.error}</p>
      ) : result.columns.length === 0 ? (
        <p className="px-3 py-2.5 text-[11px] text-muted-foreground">No result columns.</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-left text-[11px]">
            <thead className="bg-muted/20 text-[10px] tracking-wide text-muted-foreground uppercase">
              <tr>
                {result.columns.map((column) => (
                  <th key={column} className="border-b border-border px-2.5 py-1.5 font-medium">
                    {column}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {result.rows.map((row, index) => (
                <tr key={index} className="border-b border-border last:border-0">
                  {row.map((value, columnIndex) => (
                    <td
                      key={columnIndex}
                      className="max-w-64 truncate px-2.5 py-1.5 font-mono text-foreground"
                      title={value}
                    >
                      {value === "" ? <span className="text-muted-foreground">empty</span> : value}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <p className="border-t border-border px-3 py-1.5 font-mono text-[10px] text-muted-foreground">
        {result.statement}
      </p>
    </div>
  );
}

function DatabaseSqlWorkspace({
  tab,
  snapshot,
}: {
  tab: DatabaseWorkspaceTab;
  snapshot: DatabaseConnectionSnapshot | undefined;
}) {
  const setSqlDraft = useDatabaseWorkspaceStore((state) => state.setSqlDraft);
  const setSqlResult = useDatabaseWorkspaceStore((state) => state.setSqlResult);
  const connected = snapshot?.state === "connected";

  return (
    <div className="space-y-3">
      <div className="overflow-hidden rounded-md border border-border bg-card">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-3 py-2">
          <div className="flex items-center gap-2">
            <CodeIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            <h3 className="text-[12px] font-semibold text-foreground">SQL editor</h3>
            <EngineBadge engine={tab.engine} />
            <Badge variant={connected ? "success" : "outline"}>
              {connected ? "Connected" : "Disconnected"}
            </Badge>
            {tab.readOnly ? <Badge variant="warning">Read-only</Badge> : null}
          </div>
          <span className="font-mono text-[10px] text-muted-foreground">{tab.name}</span>
        </div>
        <textarea
          id={`database-sql-editor-${tab.profileId}`}
          aria-label={`SQL editor for ${tab.name}`}
          value={tab.sqlDraft}
          onChange={(event) => setSqlDraft(tab.profileId, event.target.value)}
          spellCheck={false}
          placeholder="SELECT current_database(), current_user;"
          className="min-h-40 w-full resize-y bg-transparent p-3 font-mono text-[12px] leading-5 text-foreground outline-none placeholder:text-muted-foreground/70 focus-visible:ring-2 focus-visible:ring-ring/30"
        />
        <div className="flex flex-wrap items-center gap-2 border-t border-border bg-muted/15 px-3 py-2">
          <Button
            size="sm"
            disabled
            title="SQL execution is added with the query milestone; this editor owns the draft and result state now."
          >
            <PlayIcon className="size-3.5" aria-hidden="true" />
            Run
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setSqlDraft(tab.profileId, "");
              setSqlResult(tab.profileId, null);
            }}
            disabled={tab.sqlDraft.length === 0 && tab.sqlResult === null}
          >
            <BroomIcon className="size-3.5" aria-hidden="true" />
            Clear
          </Button>
          <span className="text-[10px] text-muted-foreground">
            {connected
              ? "Draft and results are scoped to this connection tab."
              : "Connect this profile before running queries."}
          </span>
        </div>
      </div>
      <SqlResultPanel tab={tab} />
    </div>
  );
}

function DatabaseOperationsWorkspace({ tab }: { tab: DatabaseWorkspaceTab }) {
  const taskTone: Record<DatabaseWorkspaceTaskStatus, string> = {
    running: "text-warning",
    succeeded: "text-success",
    failed: "text-destructive",
    cancelled: "text-muted-foreground",
  };
  const TaskIcon = ListChecksIcon;

  return (
    <div className="space-y-3">
      <div className="rounded-md border border-border bg-card">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-3 py-2">
          <div className="flex items-center gap-2">
            <TaskIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            <h3 className="text-[12px] font-semibold text-foreground">Background tasks</h3>
          </div>
          <span className="text-[10px] text-muted-foreground">
            Scoped to {tab.name} · cleared when this tab closes
          </span>
        </div>
        {tab.tasks.length === 0 ? (
          <p className="px-3 py-3 text-[11px] leading-5 text-muted-foreground">
            No imports, exports, backups, or restores are running for this connection. Imports and
            backups attach here and are released with the tab.
          </p>
        ) : (
          <ul className="divide-y divide-border">
            {tab.tasks.map((task) => (
              <li
                key={task.id}
                className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
              >
                <span className="min-w-0">
                  <span className="block truncate text-[11px] font-medium text-foreground">
                    {task.label}
                  </span>
                  {task.detail ? (
                    <span className="block truncate text-[10px] text-muted-foreground">
                      {task.detail}
                    </span>
                  ) : null}
                </span>
                <span className={cn("text-[10px] font-medium capitalize", taskTone[task.status])}>
                  {task.status}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <EmptyState
        icon={<WrenchIcon className="size-4" aria-hidden="true" />}
        title="No operations queued"
        description="Imports, exports, backups, and restores will report progress and cancellation from this connection tab."
      />
    </div>
  );
}

function DatabaseManagerPage() {
  const servers = useServerStore((state) => state.servers);
  const serverStatus = useServerStore((state) => state.status);
  const serverError = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const { activeServer, connection } = useActiveServer();
  const connectionState = connection?.state ?? "disconnected";
  const sshConnected = connectionState === "connected";
  const activeServerId = activeServer?.id ?? "";

  const profiles = useDatabaseProfileStore(
    (state) => state.profiles[activeServerId] ?? EMPTY_PROFILES,
  );
  const profileStatus = useDatabaseProfileStore((state) => state.status[activeServerId] ?? "idle");
  const snapshots = useDatabaseProfileStore((state) => state.snapshots);
  const testResults = useDatabaseProfileStore((state) => state.testResults);
  const busy = useDatabaseProfileStore((state) => state.busy);
  const databaseError = useDatabaseProfileStore((state) => state.error);
  const fetchProfiles = useDatabaseProfileStore((state) => state.fetchProfiles);
  const connectProfile = useDatabaseProfileStore((state) => state.connectProfile);
  const reconnectProfile = useDatabaseProfileStore((state) => state.reconnectProfile);
  const disconnectProfile = useDatabaseProfileStore((state) => state.disconnectProfile);
  const testProfile = useDatabaseProfileStore((state) => state.testProfile);
  const deleteProfile = useDatabaseProfileStore((state) => state.deleteProfile);
  const refreshConnectionState = useDatabaseProfileStore((state) => state.refreshConnectionState);
  const clearDatabaseError = useDatabaseProfileStore((state) => state.clearError);

  const [refreshing, setRefreshing] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [dialog, setDialog] = useState<
    { mode: "create" } | { mode: "edit"; profile: DatabaseProfile } | null
  >(null);
  const [deleteTarget, setDeleteTarget] = useState<DatabaseProfile | null>(null);
  const [deletePending, setDeletePending] = useState(false);
  const [databaseAction, setDatabaseAction] = useState<
    | { mode: "create"; profile: DatabaseProfile }
    | { mode: "rename"; profile: DatabaseProfile; database: string }
    | null
  >(null);
  const [dropTarget, setDropTarget] = useState<{
    profile: DatabaseProfile;
    database: string;
    preview: DatabaseDdlPreview;
  } | null>(null);
  const [dropPending, setDropPending] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const workspaceTabs = useDatabaseWorkspaceStore((state) => state.tabs);
  const workspaceOrder = useDatabaseWorkspaceStore((state) => state.order);
  const activeTabId = useDatabaseWorkspaceStore((state) => state.activeTabId);
  const openWorkspaceTab = useDatabaseWorkspaceStore((state) => state.openTab);
  const closeWorkspaceTab = useDatabaseWorkspaceStore((state) => state.closeTab);
  const activateWorkspaceTab = useDatabaseWorkspaceStore((state) => state.activateTab);
  const setWorkspaceView = useDatabaseWorkspaceStore((state) => state.setActiveView);
  const setWorkspaceSelection = useDatabaseWorkspaceStore((state) => state.setSelection);
  const autoOpenedTab = useRef(false);

  const workspaceState = workspaceStateFor({
    serverStatus,
    serverCount: servers.length,
    activeServer,
  });
  const activeTab = activeTabId ? (workspaceTabs[activeTabId] ?? null) : null;
  // Profiles are cached per server. An active tab from another server keeps
  // its own identity even when the current explorer cannot list it.
  const activeTabProfile = activeTab
    ? (profiles.find((profile) => profile.id === activeTab.profileId) ?? null)
    : null;
  const selectedProfile = activeTab ? activeTabProfile : (profiles[0] ?? null);
  const selectedProfileId = activeTab?.profileId ?? selectedProfile?.id ?? null;
  const selectedSnapshot = selectedProfileId ? snapshots[selectedProfileId] : undefined;
  const selectedTestResult = selectedProfileId ? testResults[selectedProfileId] : undefined;
  const activeExplorerSelection = activeTab?.selection ?? null;
  const selectedDatabase =
    activeExplorerSelection?.database ??
    selectedSnapshot?.databaseName ??
    selectedProfile?.databaseName ??
    activeTab?.databaseName ??
    null;
  const selectedEngine = selectedProfile?.engine ?? activeTab?.engine ?? null;
  const selectedSchema =
    activeExplorerSelection?.schema ?? (selectedEngine === "mysql" ? selectedDatabase : null);
  const selectedObject =
    activeExplorerSelection &&
    (activeExplorerSelection.kind === "table" ||
      activeExplorerSelection.kind === "view" ||
      activeExplorerSelection.kind === "sequence")
      ? {
          label: activeExplorerSelection.objectName ?? activeExplorerSelection.label,
          kind: activeExplorerSelection.kind,
        }
      : null;
  const profilesRecord = profiles.length > 0 ? snapshots : EMPTY_SNAPSHOTS;
  const testResultsRecord = profiles.length > 0 ? testResults : EMPTY_TEST_RESULTS;
  const busyRecord = profiles.length > 0 ? busy : EMPTY_BUSY;

  useEffect(() => {
    if (serverStatus === "idle") {
      void fetchServers();
    }
  }, [serverStatus, fetchServers]);

  useEffect(() => {
    if (serverStatus === "ready" && servers.length > 0) {
      void refreshConnectionStates(servers.map((server) => server.id));
    }
  }, [serverStatus, servers, refreshConnectionStates]);

  useEffect(() => {
    if (activeServer && profileStatus === "idle") {
      void fetchProfiles(activeServer.id);
    }
  }, [activeServer, profileStatus, fetchProfiles]);

  useEffect(() => {
    if (profiles.length === 0) {
      return;
    }
    if (isTauriRuntime()) {
      void Promise.all(profiles.map((profile) => refreshConnectionState(profile.id)));
    }
  }, [profiles, refreshConnectionState]);

  useEffect(() => {
    if (autoOpenedTab.current) {
      return;
    }
    if (activeTabId) {
      autoOpenedTab.current = true;
      return;
    }
    if (profileStatus === "ready" && profiles.length > 0) {
      autoOpenedTab.current = true;
      openWorkspaceTab(profiles[0]);
    }
  }, [activeTabId, openWorkspaceTab, profileStatus, profiles]);

  function openCreateDialog() {
    setDialog({ mode: "create" });
  }

  function openEditDialog(profile: DatabaseProfile) {
    openWorkspaceTab(profile);
    setDialog({ mode: "edit", profile });
  }

  function selectDatabaseContext(profile: DatabaseProfile, database: string) {
    openWorkspaceTab(profile);
    setWorkspaceSelection(profile.id, {
      id: databaseRowId(profile.id, database),
      profileId: profile.id,
      kind: "database",
      label: database,
      engine: profile.engine,
      database,
      schema: null,
      objectName: null,
    });
  }

  function handleOpenWorkspace(profile: DatabaseProfile) {
    openWorkspaceTab(profile);
  }

  function handleActivateWorkspaceTab(profileId: string) {
    const profile = profiles.find((entry) => entry.id === profileId);
    if (profile) {
      openWorkspaceTab(profile);
      return;
    }
    activateWorkspaceTab(profileId);
  }

  function handleSelectExplorerProfile(profileId: string) {
    const profile = profiles.find((entry) => entry.id === profileId);
    if (profile) {
      openWorkspaceTab(profile);
    }
  }

  function handleWorkspaceSelectionChange(selection: DatabaseExplorerSelection | null) {
    if (!selection) {
      return;
    }
    const profile = profiles.find((entry) => entry.id === selection.profileId);
    if (profile) {
      openWorkspaceTab(profile);
    }
    setWorkspaceSelection(selection.profileId, selection);
  }

  async function refreshDatabaseExplorer(profileId: string): Promise<boolean> {
    const payload = await useDatabaseExplorerStore.getState().refreshNode(profileId, {
      kind: "databases",
    });
    return payload !== null;
  }

  async function applyActionResult(result: DatabaseActionResult, profile: DatabaseProfile) {
    setActionError(null);
    const refreshed = await refreshDatabaseExplorer(profile.id);

    if (result.verified) {
      setNotice(result.verification);
    } else {
      setNotice(null);
      setActionError(
        result.verificationError ??
          result.verification ??
          "The action finished, but metadata verification did not confirm the result.",
      );
      return;
    }

    if (!refreshed) {
      setActionError(
        "The database action was verified, but the explorer could not refresh. Use Refresh to reload the tree.",
      );
    }
  }

  async function handleSetActiveDatabase(profile: DatabaseProfile, database: string) {
    setActionError(null);
    setNotice(null);
    try {
      const result = await setActiveDatabase(profile.id, database);
      if (result.verified) {
        selectDatabaseContext(profile, database);
        await refreshConnectionState(profile.id);
        await applyActionResult(result, profile);
      } else {
        await applyActionResult(result, profile);
      }
    } catch (error) {
      setActionError(getCommandErrorMessage(error));
    }
  }

  async function openDropDialog(profile: DatabaseProfile, database: string) {
    setActionError(null);
    try {
      const preview = await previewDatabaseAction(profile.id, { kind: "drop", name: database });
      setDropTarget({ profile, database, preview });
    } catch (error) {
      setActionError(getCommandErrorMessage(error));
    }
  }

  async function handleDropDatabase() {
    if (!dropTarget) {
      return;
    }

    setDropPending(true);
    setActionError(null);
    try {
      const result = await dropDatabase(dropTarget.profile.id, dropTarget.database);
      const dropped = dropTarget.database;
      const tab = useDatabaseWorkspaceStore.getState().tabs[dropTarget.profile.id];
      if (tab?.selection?.database === dropped) {
        setWorkspaceSelection(dropTarget.profile.id, null);
      }
      await applyActionResult(result, dropTarget.profile);
      setDropTarget(null);
    } catch (error) {
      setActionError(getCommandErrorMessage(error));
      setDropTarget(null);
    } finally {
      setDropPending(false);
    }
  }

  async function handleStructureCompleted(
    result: DatabaseActionResult,
    action:
      | { mode: "create"; profile: DatabaseProfile }
      | { mode: "rename"; profile: DatabaseProfile; database: string },
  ) {
    if (result.verified) {
      const tab = useDatabaseWorkspaceStore.getState().tabs[action.profile.id];
      if (action.mode === "create") {
        selectDatabaseContext(action.profile, result.database);
      } else if (tab?.selection?.database === action.database) {
        selectDatabaseContext(action.profile, result.database);
      }
    }
    await applyActionResult(result, action.profile);
  }

  async function handleRefresh() {
    if (!isTauriRuntime()) {
      setNotice("Live connections require the desktop build. No connection was opened.");
      return;
    }

    setRefreshing(true);
    setNotice(null);
    try {
      await refreshConnectionStates(servers.map((server) => server.id));
      if (activeServer) {
        await fetchProfiles(activeServer.id);
        const currentProfiles = useDatabaseProfileStore.getState().profiles[activeServer.id] ?? [];
        await Promise.all(currentProfiles.map((profile) => refreshConnectionState(profile.id)));
      }
      const checkedAt = new Date().toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      });
      setNotice(`Database connection state refreshed at ${checkedAt}.`);
    } finally {
      setRefreshing(false);
    }
  }

  async function handleConnect(profile: DatabaseProfile) {
    setNotice(null);
    openWorkspaceTab(profile);
    const connected = await connectProfile(profile.id);
    if (connected) {
      setNotice(`Connected to ${profile.name} through the SSH tunnel.`);
    }
  }

  async function handleReconnect(profile: DatabaseProfile) {
    setNotice(null);
    openWorkspaceTab(profile);
    const connected = await reconnectProfile(profile.id);
    if (connected) {
      setNotice(`Reconnected ${profile.name} through the SSH tunnel.`);
    }
  }

  async function handleDisconnect(profile: DatabaseProfile) {
    setNotice(null);
    const disconnected = await disconnectProfile(profile.id);
    if (disconnected) {
      setNotice(`Disconnected ${profile.name}. The loopback tunnel was closed.`);
    }
  }

  async function handleCloseWorkspace(profileId: string) {
    const tab = useDatabaseWorkspaceStore.getState().tabs[profileId];
    if (!tab) {
      return;
    }
    const snapshot = useDatabaseProfileStore.getState().snapshots[profileId];
    const live = snapshot !== undefined && snapshot.state !== "disconnected";

    // Removing the tab clears selection, SQL text, results, and background
    // task records for that connection only. Cached explorer nodes are scoped
    // by profile id, so resetting one cannot touch another connection's tree.
    closeWorkspaceTab(profileId);
    useDatabaseExplorerStore.getState().resetProfile(profileId);

    if (isTauriRuntime()) {
      // The command is idempotent and also closes a tunnel that exists without
      // a live adapter, so closing a tab always releases loopback ports.
      await disconnectProfile(profileId);
    }
    setNotice(
      `${tab.name} workspace closed. ${live ? "Its tunnel and background tasks were closed." : "No live tunnel was open."}`,
    );
  }

  async function handleTest(profile: DatabaseProfile) {
    setNotice(null);
    const result = await testProfile(profile.id);
    if (result) {
      setNotice(
        `${profile.name} responded in ${formatLatency(result.latencyMs)} (${result.version.version}).`,
      );
    }
  }

  async function handleDelete() {
    if (!deleteTarget) {
      return;
    }

    setDeletePending(true);
    const deleted = await deleteProfile(deleteTarget.id);
    setDeletePending(false);

    if (deleted) {
      closeWorkspaceTab(deleteTarget.id);
      useDatabaseExplorerStore.getState().resetProfile(deleteTarget.id);
      setNotice(`Deleted ${deleteTarget.name}. The remote database was not changed.`);
      setDeleteTarget(null);
    }
  }

  const deleteServer = deleteTarget
    ? (servers.find((server) => server.id === deleteTarget.serverId) ?? null)
    : null;
  const dropServer = dropTarget
    ? (servers.find((server) => server.id === dropTarget.profile.serverId) ?? null)
    : null;

  return (
    <div className="mx-auto w-full max-w-[80rem] space-y-5">
      <PageHeader
        title="Databases"
        description="Manage MySQL and PostgreSQL connection profiles on your servers. Every session uses a loopback SSH tunnel and keeps passwords in the OS credential store."
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
              Database profiles, connection tests, and tunnels are available in the desktop build.
              This preview shows the layout only.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {serverError && servers.length > 0 ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>Server profile action failed</AlertTitle>
            <AlertDescription>{serverError}</AlertDescription>
          </div>
        </Alert>
      ) : null}

      {databaseError ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div className="flex-1">
            <AlertTitle>Database profile action failed</AlertTitle>
            <AlertDescription>{databaseError}</AlertDescription>
          </div>
          <Button variant="ghost" size="sm" onClick={clearDatabaseError}>
            Dismiss
          </Button>
        </Alert>
      ) : null}

      {actionError ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div className="flex-1">
            <AlertTitle>Database action failed</AlertTitle>
            <AlertDescription>{actionError}</AlertDescription>
          </div>
          <Button variant="ghost" size="sm" onClick={() => setActionError(null)}>
            Dismiss
          </Button>
        </Alert>
      ) : null}

      {workspaceState === "ready" && activeServer && !sshConnected ? (
        <Alert variant="warning">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div className="flex-1">
            <AlertTitle>SSH session is not connected</AlertTitle>
            <AlertDescription>
              Profiles on {activeServer.name} can still be created and edited. Connect the server
              over SSH before testing or opening a database tunnel.
            </AlertDescription>
          </div>
          <Button asChild variant="secondary" size="sm">
            <Link to="/servers">Open servers</Link>
          </Button>
        </Alert>
      ) : null}

      <section
        aria-label="Database manager workspace"
        className="overflow-hidden rounded-lg border border-border bg-card"
      >
        <DatabaseWorkspaceTopBar
          activeServer={activeServer}
          connectionState={connectionState}
          selectedProfile={selectedProfile}
          selectedState={selectedSnapshot?.state ?? "disconnected"}
          selectedDatabase={selectedDatabase}
          selectedSchema={selectedSchema}
          selectedObject={selectedObject}
          refreshing={refreshing}
          onRefresh={() => void handleRefresh()}
          onAddConnection={openCreateDialog}
        />

        <DatabaseWorkspaceTabBar
          tabs={workspaceOrder
            .map((profileId) => workspaceTabs[profileId])
            .filter((tab): tab is DatabaseWorkspaceTab => tab !== undefined)}
          activeTabId={activeTabId}
          snapshots={snapshots}
          onActivate={handleActivateWorkspaceTab}
          onClose={(profileId) => void handleCloseWorkspace(profileId)}
        />

        <div className="grid min-h-[30rem] lg:grid-cols-[17rem_minmax(0,1fr)]">
          <DatabaseExplorerRail
            activeServer={activeServer}
            connectionState={connectionState}
            profiles={profiles}
            profileStatus={profileStatus}
            snapshots={profilesRecord}
            selectedProfileId={selectedProfileId}
            selection={activeExplorerSelection}
            onSelectProfile={handleSelectExplorerProfile}
            onSelectionChange={handleWorkspaceSelectionChange}
            onConnectProfile={(profile) => void handleConnect(profile)}
            onAddConnection={openCreateDialog}
            onRetry={() => {
              if (activeServer) {
                void fetchProfiles(activeServer.id);
              }
            }}
            onCreateDatabase={(profile) => setDatabaseAction({ mode: "create", profile })}
            onSetActiveDatabase={(profile, database) =>
              void handleSetActiveDatabase(profile, database)
            }
            onRenameDatabase={(profile, database) =>
              setDatabaseAction({ mode: "rename", profile, database })
            }
            onDropDatabase={(profile, database) => void openDropDialog(profile, database)}
          />

          <div className="min-w-0">
            {activeTab ? (
              <>
                <DatabaseWorkspaceTabHeader
                  tab={activeTab}
                  snapshot={selectedSnapshot}
                  server={servers.find((server) => server.id === activeTab.serverId) ?? null}
                />
                <Tabs
                  value={activeTab.activeView}
                  onValueChange={(value) =>
                    setWorkspaceView(activeTab.profileId, value as DatabaseWorkspaceView)
                  }
                  className="min-h-full gap-0"
                >
                  <div className="overflow-x-auto border-b border-border px-3 py-2">
                    <TabsList
                      aria-label={`Database workspace views for ${activeTab.name} (${DATABASE_ENGINE_LABELS[activeTab.engine]})`}
                    >
                      {databaseTabs.map((tab) => {
                        const TabIcon = tab.icon;
                        return (
                          <TabsTrigger
                            key={tab.value}
                            value={tab.value}
                            className="gap-1.5"
                            title={`${tab.label} · ${activeTab.name} · ${DATABASE_ENGINE_LABELS[activeTab.engine]}`}
                          >
                            <TabIcon className="size-3.5" aria-hidden="true" />
                            {tab.label}
                          </TabsTrigger>
                        );
                      })}
                    </TabsList>
                  </div>

                  {databaseTabs.map((tab) => {
                    const TabIcon = tab.icon;
                    return (
                      <TabsContent key={tab.value} value={tab.value} className="p-4">
                        {workspaceState !== "ready" ? (
                          <WorkspaceStatePanel
                            state={workspaceState}
                            error={serverError}
                            onRetry={() => void fetchServers()}
                          />
                        ) : tab.value === "overview" ? (
                          <DatabaseConnectionList
                            profiles={profiles}
                            servers={servers}
                            snapshots={profilesRecord}
                            testResults={testResultsRecord}
                            busy={busyRecord}
                            selectedProfileId={selectedProfileId}
                            sshConnected={sshConnected}
                            onOpen={handleOpenWorkspace}
                            onConnect={(profile) => void handleConnect(profile)}
                            onReconnect={(profile) => void handleReconnect(profile)}
                            onDisconnect={(profile) => void handleDisconnect(profile)}
                            onTest={(profile) => void handleTest(profile)}
                            onEdit={openEditDialog}
                            onDelete={setDeleteTarget}
                            onAdd={openCreateDialog}
                          />
                        ) : tab.value === "sql" ? (
                          <DatabaseSqlWorkspace tab={activeTab} snapshot={selectedSnapshot} />
                        ) : tab.value === "operations" ? (
                          <DatabaseOperationsWorkspace tab={activeTab} />
                        ) : (
                          <EmptyState
                            icon={<TabIcon className="size-4" aria-hidden="true" />}
                            title={tab.emptyTitle}
                            description={
                              activeExplorerSelection?.objectName
                                ? `${activeTab.name} · ${activeExplorerSelection.database ?? activeTab.databaseName ?? "database"}${
                                    activeExplorerSelection.schema
                                      ? ` / ${activeExplorerSelection.schema}`
                                      : ""
                                  } / ${activeExplorerSelection.objectName}. ${tab.emptyDescription}`
                                : `${activeTab.name} is selected. ${tab.emptyDescription}`
                            }
                            action={
                              profiles.length === 0 ? (
                                <Button size="sm" onClick={openCreateDialog}>
                                  <PlusIcon className="size-3.5" aria-hidden="true" />
                                  Add connection
                                </Button>
                              ) : undefined
                            }
                          />
                        )}
                      </TabsContent>
                    );
                  })}
                </Tabs>
              </>
            ) : (
              <div>
                <div className="border-b border-border px-4 py-3">
                  <h2 className="text-[13px] font-semibold tracking-[-0.01em] text-foreground">
                    No open database tabs
                  </h2>
                  <p className="mt-0.5 text-[11px] text-muted-foreground">
                    Open a saved connection to keep its explorer, SQL draft, and results in their
                    own workspace tab.
                  </p>
                </div>
                <div className="p-4">
                  {workspaceState !== "ready" ? (
                    <WorkspaceStatePanel
                      state={workspaceState}
                      error={serverError}
                      onRetry={() => void fetchServers()}
                    />
                  ) : (
                    <DatabaseConnectionList
                      profiles={profiles}
                      servers={servers}
                      snapshots={profilesRecord}
                      testResults={testResultsRecord}
                      busy={busyRecord}
                      selectedProfileId={null}
                      sshConnected={sshConnected}
                      onOpen={handleOpenWorkspace}
                      onConnect={(profile) => void handleConnect(profile)}
                      onReconnect={(profile) => void handleReconnect(profile)}
                      onDisconnect={(profile) => void handleDisconnect(profile)}
                      onTest={(profile) => void handleTest(profile)}
                      onEdit={openEditDialog}
                      onDelete={setDeleteTarget}
                      onAdd={openCreateDialog}
                    />
                  )}
                </div>
              </div>
            )}
          </div>
        </div>

        <DatabaseWorkspaceStatusBar
          state={workspaceState}
          activeServer={activeServer}
          sshConnected={sshConnected}
          profiles={profiles}
          selectedProfile={selectedProfile}
          selectedSnapshot={selectedSnapshot}
          selectedTestResult={selectedTestResult}
          notice={notice}
        />
      </section>

      <p className="text-[10px] text-muted-foreground">
        Database listeners are bound to 127.0.0.1 inside the existing SSH session. Passwords are
        written to the OS credential store and are never rendered after saving.
      </p>

      {dialog ? (
        <DatabaseProfileDialog
          key={dialog.mode === "edit" ? dialog.profile.id : `new-${activeServerId}`}
          servers={servers}
          activeServerId={activeServer?.id ?? null}
          profile={dialog.mode === "edit" ? dialog.profile : null}
          onOpenChange={(open) => {
            if (!open) {
              setDialog(null);
            }
          }}
          onSaved={(saved) => {
            if (saved.serverId === activeServerId) {
              openWorkspaceTab(saved);
            }
            const savedServer = servers.find((server) => server.id === saved.serverId);
            setNotice(
              savedServer && savedServer.id !== activeServerId
                ? `Saved ${saved.name} for ${savedServer.name}. Select that server to open it.`
                : `Saved ${saved.name}.`,
            );
          }}
        />
      ) : null}

      {databaseAction ? (
        <DatabaseActionDialog
          key={`${databaseAction.mode}-${databaseAction.profile.id}-${
            databaseAction.mode === "rename" ? databaseAction.database : "new"
          }`}
          open
          mode={databaseAction.mode}
          profile={databaseAction.profile}
          database={databaseAction.mode === "rename" ? databaseAction.database : null}
          onOpenChange={(open) => {
            if (!open) {
              setDatabaseAction(null);
            }
          }}
          onCompleted={(result) => void handleStructureCompleted(result, databaseAction)}
        />
      ) : null}

      <ConfirmDialog
        open={dropTarget !== null}
        onOpenChange={(open) => {
          if (!open && !dropPending) {
            setDropTarget(null);
          }
        }}
        title={`Drop database ${dropTarget?.database ?? ""}?`}
        description="This permanently removes the database and every object and row inside it. The action is not reversible."
        confirmLabel="Drop database"
        details={
          dropTarget
            ? [
                {
                  label: "Server",
                  value: dropServer
                    ? `${dropServer.name} (${dropServer.host})`
                    : dropTarget.profile.serverId,
                },
                {
                  label: "Engine",
                  value: DATABASE_ENGINE_LABELS[dropTarget.profile.engine],
                },
                { label: "Database", value: dropTarget.database },
                {
                  label: "Action",
                  value: `DROP DATABASE ${dropTarget.database} (permanent deletion)`,
                },
                { label: "Profile", value: dropTarget.profile.name },
              ]
            : undefined
        }
        preview={
          dropTarget
            ? {
                label: "Generated DDL",
                text: dropTarget.preview.ddl,
              }
            : null
        }
        warningTitle="This permanently deletes the database."
        warningDescription={
          dropTarget?.preview.warnings.join(" ") ??
          "Every table, view, and row in this database will be removed."
        }
        onConfirm={() => void handleDropDatabase()}
        pending={dropPending}
      />

      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deletePending) {
            setDeleteTarget(null);
          }
        }}
        title={`Delete ${deleteTarget?.name ?? "database connection"}?`}
        description="The saved profile and its database password will be removed. The remote database and server are not changed."
        confirmLabel="Delete connection"
        details={
          deleteTarget
            ? [
                { label: "Profile", value: deleteTarget.name },
                {
                  label: "Server",
                  value: deleteServer
                    ? `${deleteServer.name} (${deleteServer.host})`
                    : deleteTarget.serverId,
                },
                {
                  label: "Engine",
                  value: DATABASE_ENGINE_LABELS[deleteTarget.engine],
                },
                {
                  label: "Endpoint",
                  value: formatDatabaseEndpoint(
                    deleteTarget.host,
                    deleteTarget.port,
                    deleteTarget.socket,
                  ),
                },
              ]
            : undefined
        }
        warningTitle="This removes the saved profile."
        warningDescription="The database password is deleted from the OS credential store. Existing server data is untouched."
        onConfirm={() => void handleDelete()}
        pending={deletePending}
      />
    </div>
  );
}

export { DatabaseManagerPage };
