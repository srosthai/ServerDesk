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
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { ErrorState, LoadingState } from "@/components/ui/states";
import { StatusBadge } from "@/components/ui/status-badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useActiveServer } from "@/hooks/useActiveServer";
import { cn } from "@/lib/utils";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { useServerStore } from "@/stores/serverStore";
import type { ConnectionState, ServerProfile } from "@/types/contracts";

type WorkspaceState = "loading" | "error" | "empty" | "disconnected" | "ready";

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
      "Query tabs, execution plans, and results will appear here once a database connection is available.",
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

const plannedConnectionSteps = [
  {
    title: "Choose a saved server profile",
    description: "Reuse the existing SSH session and credential handling from ServerDesk.",
  },
  {
    title: "Select an engine and endpoint",
    description: "Pick MySQL or PostgreSQL and the database host and port on the server.",
  },
  {
    title: "Provide database credentials",
    description: "Credentials stay in the operating system keyring, never in SQLite or logs.",
  },
  {
    title: "Open an SSH local-forward tunnel",
    description: "The database port stays bound to the loopback interface through the SSH session.",
  },
];

function workspaceStateFor({
  serverStatus,
  serverCount,
  activeServer,
  connectionState,
}: {
  serverStatus: string;
  serverCount: number;
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
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
  if (!activeServer || connectionState !== "connected") {
    return "disconnected";
  }
  return "ready";
}

function disconnectedCopy(
  activeServer: ServerProfile | null,
  connectionState: ConnectionState,
): { title: string; description: string } {
  if (!activeServer) {
    return {
      title: "No active server selected",
      description:
        "Choose an active server profile before opening its database workspace. Server selection is shared with the rest of ServerDesk.",
    };
  }

  switch (connectionState) {
    case "connecting":
      return {
        title: "Connecting to the server",
        description: `The database workspace opens after the SSH session to ${activeServer.name} reaches Connected.`,
      };
    case "reconnecting":
      return {
        title: "Reconnecting to the server",
        description: `ServerDesk is restoring the SSH session to ${activeServer.name}. The workspace stays closed until the session is healthy again.`,
      };
    case "host-key-verification":
      return {
        title: "Host key verification required",
        description: `Verify the host key for ${activeServer.name} before the database workspace can use its SSH session.`,
      };
    case "error":
      return {
        title: "SSH connection failed",
        description: `The SSH session to ${activeServer.name} reported an error. Reconnect the server before opening its database workspace.`,
      };
    default:
      return {
        title: "Database workspace is disconnected",
        description: `Connect ${activeServer.name} over SSH to explore database objects. The workspace never opens a database port outside the SSH session.`,
      };
  }
}

function statusMessage(
  state: WorkspaceState,
  activeServer: ServerProfile | null,
  connectionState: ConnectionState,
): string {
  switch (state) {
    case "loading":
      return "Loading saved server profiles...";
    case "error":
      return "Server profiles could not be loaded.";
    case "empty":
      return "No server selected. Add or select a server profile.";
    case "ready":
      return "Workspace ready. No database connections configured.";
    default:
      if (!activeServer) {
        return "No active server selected.";
      }
      switch (connectionState) {
        case "connecting":
          return `Connecting to ${activeServer.name} over SSH...`;
        case "reconnecting":
          return `Reconnecting to ${activeServer.name}...`;
        case "host-key-verification":
          return `Host key verification required for ${activeServer.name}.`;
        case "error":
          return `SSH connection error for ${activeServer.name}.`;
        default:
          return `SSH disconnected from ${activeServer.name}.`;
      }
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

function WorkspaceStatePanel({
  state,
  activeServer,
  connectionState,
  error,
  tabLabel,
  onRetry,
}: {
  state: Exclude<WorkspaceState, "ready">;
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
  error: string | null;
  tabLabel: string;
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

  const copy = disconnectedCopy(activeServer, connectionState);

  return (
    <EmptyState
      icon={<PlugsIcon className="size-4" aria-hidden="true" />}
      title={copy.title}
      description={`${tabLabel} is unavailable. ${copy.description}`}
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
  refreshing,
  onRefresh,
  onAddConnection,
}: {
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
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
          <p className="text-[10px] text-muted-foreground">MySQL and PostgreSQL over SSH</p>
        </div>
        <StatusBadge
          state={activeServer ? connectionState : "disconnected"}
          label={activeServer ? undefined : "No server"}
        />
        <Badge variant="outline">Engine not selected</Badge>
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <div className="hidden min-w-0 text-right sm:block">
          <p className="max-w-48 truncate text-[11px] font-medium text-foreground">
            {activeServer?.name ?? "No server selected"}
          </p>
          <p className="max-w-56 truncate font-mono text-[10px] text-muted-foreground">
            {activeServer
              ? `${activeServer.username}@${activeServer.host}:${activeServer.port}`
              : "Choose or add a server profile"}
          </p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={onRefresh}
          disabled={refreshing}
          aria-label="Refresh connection status"
        >
          {refreshing ? (
            <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
          ) : (
            <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
          )}
          {refreshing ? "Refreshing..." : "Refresh status"}
        </Button>
        <Button size="sm" onClick={onAddConnection}>
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
  workspaceState,
}: {
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
  workspaceState: WorkspaceState;
}) {
  const connected = workspaceState === "ready";

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
          <Badge variant="outline">0 objects</Badge>
        </div>
        <label htmlFor="database-object-search" className="sr-only">
          Search database objects
        </label>
        <div className="relative mt-2">
          <MagnifyingGlassIcon
            className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            id="database-object-search"
            type="search"
            disabled
            placeholder="Search objects"
            aria-describedby="database-object-search-help"
            className="h-8 pl-8 text-[12px]"
          />
        </div>
        <p id="database-object-search-help" className="mt-1.5 text-[10px] text-muted-foreground">
          {connected
            ? "Search becomes available after a connection profile exists."
            : "Connect the active server to search database objects."}
        </p>
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
          label={activeServer ? undefined : "No server"}
        />
      </div>

      <div className="min-h-40 flex-1 px-3 py-3">
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Connections
          </h3>
          <Badge variant="outline">0</Badge>
        </div>
        <div className="mt-2 flex items-start gap-2 rounded-md border border-dashed border-border px-2.5 py-3">
          <PlugsIcon
            className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
            aria-hidden="true"
          />
          <div className="min-w-0">
            <p className="text-[11px] font-medium text-foreground">
              {connected ? "No saved connections" : "Explorer unavailable"}
            </p>
            <p className="mt-0.5 text-[10px] leading-4 text-muted-foreground">
              {connected
                ? "Connection profiles will appear here. Add one from the workspace toolbar."
                : "Connect the active server to load its database connections."}
            </p>
          </div>
        </div>
      </div>

      <p className="border-t border-border px-3 py-2 text-[10px] text-muted-foreground">
        SSH tunnel · No database listener
      </p>
    </aside>
  );
}

function DatabaseWorkspaceStatusBar({
  state,
  activeServer,
  connectionState,
  notice,
}: {
  state: WorkspaceState;
  activeServer: ServerProfile | null;
  connectionState: ConnectionState;
  notice: string | null;
}) {
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
          {notice ?? statusMessage(state, activeServer, connectionState)}
        </span>
      </p>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-muted-foreground">
        <span>0 open connections</span>
        <span aria-hidden="true">·</span>
        <span>0 running queries</span>
        <span aria-hidden="true">·</span>
        <span>SSH tunnel idle</span>
      </div>
    </div>
  );
}

function AddConnectionDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Add database connection</DialogTitle>
          <DialogDescription>
            Connection profiles arrive with the Database Manager backend. This placeholder shows the
            planned flow only; nothing is saved and no connection is opened.
          </DialogDescription>
        </DialogHeader>

        <div>
          <h3 className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
            Planned connection flow
          </h3>
          <ol className="mt-2.5 space-y-2.5">
            {plannedConnectionSteps.map((step, index) => (
              <li key={step.title} className="flex gap-2.5">
                <span
                  aria-hidden="true"
                  className="mt-0.5 grid size-5 shrink-0 place-items-center rounded-full border border-border bg-muted/50 font-mono text-[10px] text-muted-foreground"
                >
                  {index + 1}
                </span>
                <div className="min-w-0">
                  <p className="text-[12px] font-medium text-foreground">{step.title}</p>
                  <p className="mt-0.5 text-[11px] leading-5 text-muted-foreground">
                    {step.description}
                  </p>
                </div>
              </li>
            ))}
          </ol>
        </div>

        <Alert variant="info">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Placeholder only</AlertTitle>
            <AlertDescription>
              No database driver is loaded and no tunnel or connection will be opened. ServerDesk
              continues to use only the existing SSH session.
            </AlertDescription>
          </div>
        </Alert>

        <DialogFooter>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            Back to workspace
          </Button>
          <Button asChild>
            <Link to="/servers" onClick={() => onOpenChange(false)}>
              Open servers
            </Link>
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
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
  const [addConnectionOpen, setAddConnectionOpen] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const workspaceState = workspaceStateFor({
    serverStatus,
    serverCount: servers.length,
    activeServer,
    connectionState,
  });

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

  async function handleRefreshStatus() {
    if (!isTauriRuntime()) {
      setNotice("Live SSH status requires the desktop build. No connection was opened.");
      return;
    }
    if (!activeServer) {
      setNotice("Select an active server before refreshing connection status.");
      return;
    }

    setRefreshing(true);
    setNotice(null);
    try {
      await refreshConnectionStates(servers.map((server) => server.id));
      const checkedAt = new Date().toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      });
      setNotice(`Connection status refreshed at ${checkedAt}.`);
    } finally {
      setRefreshing(false);
    }
  }

  return (
    <div className="space-y-5">
      <PageHeader
        title="Databases"
        description="A database administration workspace for MySQL and PostgreSQL on your servers. This release adds the workspace shell only: no database drivers are loaded and no connections are opened."
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
              The database workspace uses saved server profiles and live SSH state in the desktop
              build. This preview shows the layout and placeholders only.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      <section
        aria-label="Database manager workspace"
        className="overflow-hidden rounded-lg border border-border bg-card"
      >
        <DatabaseWorkspaceTopBar
          activeServer={activeServer}
          connectionState={connectionState}
          refreshing={refreshing}
          onRefresh={() => void handleRefreshStatus()}
          onAddConnection={() => setAddConnectionOpen(true)}
        />

        <div className="grid min-h-[30rem] lg:grid-cols-[13.5rem_minmax(0,1fr)]">
          <DatabaseExplorerRail
            activeServer={activeServer}
            connectionState={connectionState}
            workspaceState={workspaceState}
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
                    {workspaceState === "ready" ? (
                      <EmptyState
                        icon={<TabIcon className="size-4" aria-hidden="true" />}
                        title={tab.emptyTitle}
                        description={tab.emptyDescription}
                        action={
                          tab.value === "overview" ? (
                            <Button size="sm" onClick={() => setAddConnectionOpen(true)}>
                              <PlusIcon className="size-3.5" aria-hidden="true" />
                              Add your first connection
                            </Button>
                          ) : undefined
                        }
                      />
                    ) : (
                      <WorkspaceStatePanel
                        state={workspaceState}
                        activeServer={activeServer}
                        connectionState={connectionState}
                        error={serverError}
                        tabLabel={tab.label}
                        onRetry={() => void fetchServers()}
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
          connectionState={connectionState}
          notice={notice}
        />
      </section>

      <p className="text-[10px] text-muted-foreground">
        This workspace is a shell. ServerDesk does not load database drivers, open database ports,
        or create tunnels yet. The Add connection flow is a placeholder and saves nothing.
      </p>

      <AddConnectionDialog open={addConnectionOpen} onOpenChange={setAddConnectionOpen} />
    </div>
  );
}

export { DatabaseManagerPage };
