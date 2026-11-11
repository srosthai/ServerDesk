import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import type { Icon } from "@phosphor-icons/react";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  CodeIcon,
  DatabaseIcon,
  HardDrivesIcon,
  InfoIcon,
  KeyIcon,
  MagnifyingGlassIcon,
  PlugsIcon,
  PlusIcon,
  RowsIcon,
  TableIcon,
  TreeStructureIcon,
  WrenchIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { ErrorState, LoadingState } from "@/components/ui/states";
import { StatusBadge } from "@/components/ui/status-badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { DatabaseConnectionList, EngineBadge } from "@/features/databases/DatabaseConnectionList";
import { DatabaseProfileDialog } from "@/features/databases/DatabaseProfileDialog";
import {
  DATABASE_ENGINE_LABELS,
  databaseConnectionStateLabel,
  formatDatabaseEndpoint,
  formatLatency,
} from "@/features/databases/databaseProfileForm";
import { useActiveServer } from "@/hooks/useActiveServer";
import { isTauriRuntime } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { useConnectionStore } from "@/stores/connectionStore";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ConnectionState,
  DatabaseConnectionSnapshot,
  DatabaseConnectionState,
  DatabaseProfile,
  DatabaseTestResult,
  DatabaseTunnelSnapshot,
  ServerProfile,
} from "@/types/contracts";

type WorkspaceState = "loading" | "error" | "empty" | "no-server" | "ready";

type DatabaseTab = {
  value: string;
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
  refreshing,
  onRefresh,
  onAddConnection,
}: {
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
  selectedProfile: DatabaseProfile | null;
  selectedState: DatabaseConnectionState;
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
          <p className="text-[10px] text-muted-foreground">
            MySQL and PostgreSQL over loopback SSH tunnels
          </p>
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
  onSelect,
  onAddConnection,
  onRetry,
}: {
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
  profiles: DatabaseProfile[];
  profileStatus: "idle" | "loading" | "ready" | "error";
  snapshots: Record<string, DatabaseConnectionSnapshot>;
  selectedProfileId: string | null;
  onSelect: (profileId: string) => void;
  onAddConnection: () => void;
  onRetry: () => void;
}) {
  const [query, setQuery] = useState("");
  const normalizedQuery = query.trim().toLowerCase();
  const filtered = profiles.filter((profile) => {
    if (!normalizedQuery) {
      return true;
    }
    return [
      profile.name,
      DATABASE_ENGINE_LABELS[profile.engine],
      profile.host,
      String(profile.port),
      profile.databaseName ?? "",
    ]
      .join(" ")
      .toLowerCase()
      .includes(normalizedQuery);
  });

  return (
    <aside
      aria-label="Database explorer"
      className="flex min-w-0 flex-col border-b border-border bg-muted/20 lg:border-r lg:border-b-0"
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
        <label htmlFor="database-connection-search" className="sr-only">
          Search database connections
        </label>
        <div className="relative mt-2">
          <MagnifyingGlassIcon
            className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            id="database-connection-search"
            type="search"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="Search connections"
            className="h-8 pl-8 text-[12px]"
          />
        </div>
      </div>

      <div className="border-b border-border px-3 py-3">
        <p className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
          Active server
        </p>
        <div className="mt-1.5 flex min-w-0 items-center gap-2">
          <span className="grid size-7 shrink-0 place-items-center rounded-sm border border-border bg-card text-muted-foreground">
            <HardDrivesIcon className="size-3.5" aria-hidden="true" />
          </span>
          <div className="min-w-0">
            <p className="truncate text-[12px] font-medium text-foreground">
              {activeServer?.name ?? "No server selected"}
            </p>
            <p className="truncate font-mono text-[10px] text-muted-foreground">
              {activeServer
                ? `${activeServer.username}@${activeServer.host}:${activeServer.port}`
                : "Choose a profile to begin"}
            </p>
          </div>
        </div>
        <StatusBadge
          className="mt-2"
          state={activeServer ? connectionState : "disconnected"}
          label={activeServer ? sshStateLabel(connectionState) : "No server"}
        />
      </div>

      <div className="min-h-40 flex-1 overflow-y-auto px-3 py-3">
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Connections
          </h3>
          {profiles.length > 0 ? (
            <span className="text-[10px] text-muted-foreground">{filtered.length} shown</span>
          ) : null}
        </div>

        {profileStatus === "loading" || (profileStatus === "idle" && activeServer) ? (
          <div className="mt-2.5">
            <LoadingState rows={3} />
          </div>
        ) : null}

        {profileStatus === "error" ? (
          <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/8 px-2.5 py-2">
            <p className="text-[11px] font-medium text-destructive">Could not load connections</p>
            <Button variant="ghost" size="sm" className="mt-1 h-7 px-1.5" onClick={onRetry}>
              Try again
            </Button>
          </div>
        ) : null}

        {profileStatus === "ready" && profiles.length === 0 ? (
          <div className="mt-2 flex items-start gap-2 rounded-md border border-dashed border-border px-2.5 py-3">
            <PlugsIcon
              className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <div className="min-w-0">
              <p className="text-[11px] font-medium text-foreground">No saved connections</p>
              <p className="mt-0.5 text-[10px] leading-4 text-muted-foreground">
                Add a profile from the workspace toolbar.
              </p>
            </div>
          </div>
        ) : null}

        {profileStatus === "ready" && profiles.length > 0 && filtered.length === 0 ? (
          <p className="mt-2 rounded-md border border-dashed border-border px-2.5 py-3 text-[10px] leading-4 text-muted-foreground">
            No connections match this search.
          </p>
        ) : null}

        {filtered.length > 0 ? (
          <ul className="mt-2 space-y-1">
            {filtered.map((profile) => {
              const snapshot = snapshots[profile.id];
              const state = snapshot?.state ?? "disconnected";
              const selected = profile.id === selectedProfileId;

              return (
                <li key={profile.id}>
                  <button
                    type="button"
                    onClick={() => onSelect(profile.id)}
                    aria-pressed={selected}
                    className={cn(
                      "w-full rounded-sm border px-2 py-1.5 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/40",
                      selected
                        ? "border-primary/40 bg-card text-foreground"
                        : "border-transparent text-muted-foreground hover:border-border hover:bg-card/60 hover:text-foreground",
                    )}
                  >
                    <span className="flex min-w-0 items-center gap-1.5">
                      <span
                        aria-hidden="true"
                        className={cn("size-1.5 shrink-0 rounded-full", profileStateColor(state))}
                      />
                      <span className="truncate text-[11px] font-medium">{profile.name}</span>
                    </span>
                    <span className="mt-0.5 flex min-w-0 items-center gap-1 pl-3 text-[10px] text-muted-foreground">
                      <span className="shrink-0">{DATABASE_ENGINE_LABELS[profile.engine]}</span>
                      <span aria-hidden="true">·</span>
                      <span className="truncate font-mono">
                        {formatDatabaseEndpoint(profile.host, profile.port, profile.socket)}
                      </span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        ) : null}
      </div>

      <div className="border-t border-border px-3 py-2">
        <p className="text-[10px] text-muted-foreground">
          Object tree arrives with the explorer. Tunnels use 127.0.0.1 only.
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
  const [requestedProfileId, setRequestedProfileId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<
    { mode: "create" } | { mode: "edit"; profile: DatabaseProfile } | null
  >(null);
  const [deleteTarget, setDeleteTarget] = useState<DatabaseProfile | null>(null);
  const [deletePending, setDeletePending] = useState(false);

  const workspaceState = workspaceStateFor({
    serverStatus,
    serverCount: servers.length,
    activeServer,
  });
  const selectedProfile =
    profiles.find((profile) => profile.id === requestedProfileId) ?? profiles[0] ?? null;
  const selectedProfileId = selectedProfile?.id ?? null;
  const selectedSnapshot = selectedProfile ? snapshots[selectedProfile.id] : undefined;
  const selectedTestResult = selectedProfile ? testResults[selectedProfile.id] : undefined;
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

  function openCreateDialog() {
    setDialog({ mode: "create" });
  }

  function openEditDialog(profile: DatabaseProfile) {
    setRequestedProfileId(profile.id);
    setDialog({ mode: "edit", profile });
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
    const connected = await connectProfile(profile.id);
    if (connected) {
      setRequestedProfileId(profile.id);
      setNotice(`Connected to ${profile.name} through the SSH tunnel.`);
    }
  }

  async function handleReconnect(profile: DatabaseProfile) {
    setNotice(null);
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
      setNotice(`Deleted ${deleteTarget.name}. The remote database was not changed.`);
      setDeleteTarget(null);
    }
  }

  const deleteServer = deleteTarget
    ? (servers.find((server) => server.id === deleteTarget.serverId) ?? null)
    : null;

  return (
    <div className="space-y-5">
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
          refreshing={refreshing}
          onRefresh={() => void handleRefresh()}
          onAddConnection={openCreateDialog}
        />

        <div className="grid min-h-[30rem] lg:grid-cols-[13.5rem_minmax(0,1fr)]">
          <DatabaseExplorerRail
            activeServer={activeServer}
            connectionState={connectionState}
            profiles={profiles}
            profileStatus={profileStatus}
            snapshots={profilesRecord}
            selectedProfileId={selectedProfileId}
            onSelect={setRequestedProfileId}
            onAddConnection={openCreateDialog}
            onRetry={() => {
              if (activeServer) {
                void fetchProfiles(activeServer.id);
              }
            }}
          />

          <div className="min-w-0">
            <Tabs defaultValue="overview" className="min-h-full gap-0">
              <div className="overflow-x-auto border-b border-border px-3 py-2">
                <TabsList aria-label="Database workspace views">
                  {databaseTabs.map((tab) => {
                    const TabIcon = tab.icon;
                    return (
                      <TabsTrigger key={tab.value} value={tab.value} className="gap-1.5">
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
                        onSelect={setRequestedProfileId}
                        onConnect={(profile) => void handleConnect(profile)}
                        onReconnect={(profile) => void handleReconnect(profile)}
                        onDisconnect={(profile) => void handleDisconnect(profile)}
                        onTest={(profile) => void handleTest(profile)}
                        onEdit={openEditDialog}
                        onDelete={setDeleteTarget}
                        onAdd={openCreateDialog}
                      />
                    ) : (
                      <EmptyState
                        icon={<TabIcon className="size-4" aria-hidden="true" />}
                        title={tab.emptyTitle}
                        description={
                          selectedProfile
                            ? `${selectedProfile.name} is selected. ${tab.emptyDescription}`
                            : `${tab.emptyDescription} Add a connection profile to begin.`
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
            setRequestedProfileId(saved.id);
            const savedServer = servers.find((server) => server.id === saved.serverId);
            setNotice(
              savedServer && savedServer.id !== activeServerId
                ? `Saved ${saved.name} for ${savedServer.name}. Select that server to open it.`
                : `Saved ${saved.name}.`,
            );
          }}
        />
      ) : null}

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
