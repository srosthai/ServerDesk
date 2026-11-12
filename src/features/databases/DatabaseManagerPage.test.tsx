import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseManagerPage } from "@/features/databases/DatabaseManagerPage";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { getConnectionState } from "@/services/connections";
import { previewDatabaseAction, dropDatabase, setActiveDatabase } from "@/services/databaseActions";
import { refreshExplorerNode } from "@/services/databaseDiscovery";
import {
  connectDatabaseProfile,
  deleteDatabaseProfile,
  disconnectDatabaseProfile,
  getDatabaseConnectionState,
  reconnectDatabaseProfile,
  testDatabaseProfile,
} from "@/services/databaseProfiles";
import { useConnectionStore } from "@/stores/connectionStore";
import { databaseExplorerNodeKey, useDatabaseExplorerStore } from "@/stores/databaseExplorerStore";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ConnectionSnapshot,
  DatabaseCapabilities,
  DatabaseConnectionSnapshot,
  DatabaseExplorerNodePayload,
  DatabaseProfile,
  DatabaseSummary,
  DatabaseTestResult,
  ServerProfile,
} from "@/types/contracts";

vi.mock("@/services/databaseProfiles", () => ({
  listDatabaseProfiles: vi.fn(),
  getDatabaseProfile: vi.fn(),
  createDatabaseProfile: vi.fn(),
  updateDatabaseProfile: vi.fn(),
  deleteDatabaseProfile: vi.fn(),
  testDatabaseProfile: vi.fn(),
  testDatabaseProfileDraft: vi.fn(),
  connectDatabaseProfile: vi.fn(),
  reconnectDatabaseProfile: vi.fn(),
  disconnectDatabaseProfile: vi.fn(),
  getDatabaseConnectionState: vi.fn(),
}));

vi.mock("@/services/connections", () => ({
  connectServer: vi.fn(),
  disconnectServer: vi.fn(),
  getConnectionState: vi.fn(),
  testServerConnection: vi.fn(),
}));

vi.mock("@/services/databaseActions", () => ({
  previewDatabaseAction: vi.fn(),
  createDatabase: vi.fn(),
  renameDatabase: vi.fn(),
  dropDatabase: vi.fn(),
  setActiveDatabase: vi.fn(),
  runDatabaseAction: vi.fn(),
}));

vi.mock("@/services/databaseDiscovery", () => ({
  listDatabases: vi.fn(),
  listSchemas: vi.fn(),
  listTables: vi.fn(),
  listViews: vi.fn(),
  listSequences: vi.fn(),
  getObjectCounts: vi.fn(),
  loadExplorerNode: vi.fn(),
  refreshExplorerNode: vi.fn(),
}));

const server: ServerProfile = {
  id: "server-1",
  name: "Production",
  host: "198.51.100.24",
  port: 22,
  username: "deploy",
  authType: "password",
  credentialRef: null,
  groupName: null,
  tags: [],
  notes: null,
  createdAt: 1,
  updatedAt: 1,
};

const connected: ConnectionSnapshot = {
  serverId: server.id,
  state: "connected",
  changedAt: 1,
};

const profile: DatabaseProfile = {
  id: "profile-1",
  serverId: server.id,
  name: "Primary",
  engine: "postgresql",
  host: "127.0.0.1",
  port: 5432,
  socket: null,
  databaseName: "app",
  username: "app",
  hasCredential: true,
  readOnly: true,
  sslMode: "required",
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: 1_700_000_000,
};

const capabilities: DatabaseCapabilities = {
  engine: "postgresql",
  version: "16.2.0",
  flags: {
    schemas: true,
    sequences: true,
    materializedViews: true,
    routines: true,
    triggers: true,
    roles: true,
    sessions: true,
    transactions: true,
    savepoints: true,
    cancelQuery: true,
    explain: true,
    checkConstraints: true,
    generatedColumns: true,
    identityColumns: true,
    columnReorder: false,
    databaseRename: true,
    indexRename: true,
    queryTimeout: true,
  },
  actions: ["browse_databases", "run_query"],
};

const connectedSnapshot: DatabaseConnectionSnapshot = {
  profileId: profile.id,
  serverId: server.id,
  name: profile.name,
  engine: profile.engine,
  state: "connected",
  readOnly: true,
  databaseName: "app",
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  capabilities,
  tunnel: {
    profileId: profile.id,
    serverId: server.id,
    state: "active",
    localHost: "127.0.0.1",
    localPort: 52341,
    remoteHost: "127.0.0.1",
    remotePort: 5432,
    openedAt: 1,
  },
  changedAt: 10,
};

const disconnectedSnapshot: DatabaseConnectionSnapshot = {
  profileId: profile.id,
  serverId: server.id,
  name: profile.name,
  engine: profile.engine,
  state: "disconnected",
  readOnly: true,
  databaseName: "app",
  changedAt: 12,
};

function databaseSummary(name: string): DatabaseSummary {
  return {
    name,
    system: false,
    owner: { status: "available", value: "app" },
    encoding: { status: "available", value: "UTF8" },
    collation: { status: "available", value: "C.UTF-8" },
    sizeBytes: { status: "available", value: 8_192 },
    engine: { status: "unavailable" },
  };
}

const databasesPayload: DatabaseExplorerNodePayload = {
  nodeKind: "databases",
  payload: {
    items: [databaseSummary("app"), databaseSummary("analytics")],
    limit: 100,
    offset: 0,
    hasMore: false,
    total: 2,
  },
};

const actionResult = (
  action: "create" | "rename" | "drop" | "set_active",
  database: string,
): import("@/types/contracts").DatabaseActionResult => ({
  action,
  engine: "postgresql",
  database,
  previousName: action === "rename" ? "app" : null,
  statements: [],
  verified: true,
  verification: `Metadata re-read confirms database ${database}.`,
  verificationError: null,
  metadata: databaseSummary(database),
  activeDatabase: action === "set_active" ? database : null,
  durationMs: 4,
});

const testResult: DatabaseTestResult = {
  profileId: profile.id,
  serverId: server.id,
  engine: profile.engine,
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  databaseName: "app",
  readOnly: true,
  capabilities,
  latencyMs: 14,
  testedAt: 2,
};

function renderPage() {
  return render(
    <MemoryRouter>
      <DatabaseManagerPage />
    </MemoryRouter>,
  );
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function useConnectedServer() {
  useServerStore.setState({ servers: [server], status: "ready", error: null });
  useConnectionStore.setState({
    connections: { [server.id]: connected },
    busy: {},
    error: null,
  });
}

function useProfileStore(
  overrides: Partial<ReturnType<typeof useDatabaseProfileStore.getState>> = {},
) {
  useDatabaseProfileStore.setState({
    profiles: { [server.id]: [profile] },
    status: { [server.id]: "ready" },
    snapshots: { [profile.id]: connectedSnapshot },
    connectionStates: { [profile.id]: "connected" },
    testResults: { [profile.id]: testResult },
    busy: {},
    error: null,
    commandError: null,
    ...overrides,
  });
}

function useLoadedDatabases() {
  useDatabaseExplorerStore.setState({
    nodes: {
      [databaseExplorerNodeKey(profile.id, { kind: "databases" })]: {
        status: "ready",
        payload: databasesPayload,
        updatedAt: 1,
      },
    },
    error: null,
    commandError: null,
  });
}

describe("DatabaseManagerPage", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useServerStore.setState({
      servers: [],
      status: "ready",
      error: null,
      fetchServers: vi.fn(),
    });
    useConnectionStore.setState({ connections: {}, busy: {}, error: null });
    useDatabaseProfileStore.setState({
      profiles: {},
      status: {},
      snapshots: {},
      connectionStates: {},
      testResults: {},
      busy: {},
      error: null,
      commandError: null,
    });
    useDatabaseExplorerStore.setState({ nodes: {}, error: null, commandError: null });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES },
      status: "unavailable",
      error: null,
    });
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(getDatabaseConnectionState).mockResolvedValue(connectedSnapshot);
    vi.mocked(connectDatabaseProfile).mockResolvedValue(connectedSnapshot);
    vi.mocked(reconnectDatabaseProfile).mockResolvedValue({
      ...connectedSnapshot,
      changedAt: 20,
    });
    vi.mocked(disconnectDatabaseProfile).mockResolvedValue(disconnectedSnapshot);
    vi.mocked(testDatabaseProfile).mockResolvedValue(testResult);
    vi.mocked(deleteDatabaseProfile).mockResolvedValue(undefined);
    vi.mocked(previewDatabaseAction).mockReset();
    vi.mocked(previewDatabaseAction).mockResolvedValue({
      action: "drop",
      engine: "postgresql",
      database: "analytics",
      targetDatabase: null,
      statements: ['DROP DATABASE "analytics"'],
      ddl: 'DROP DATABASE "analytics"',
      warnings: ["Dropping a database permanently removes every table, view, and row inside it."],
      destructive: true,
      requiresConfirmation: true,
    });
    vi.mocked(dropDatabase).mockReset();
    vi.mocked(dropDatabase).mockResolvedValue(actionResult("drop", "analytics"));
    vi.mocked(setActiveDatabase).mockReset();
    vi.mocked(setActiveDatabase).mockResolvedValue(actionResult("set_active", "analytics"));
    vi.mocked(refreshExplorerNode).mockReset();
    vi.mocked(refreshExplorerNode).mockResolvedValue(databasesPayload);
  });

  it("renders the connection list with engine, server, endpoint, status, and runtime details", async () => {
    useConnectedServer();
    useProfileStore();
    renderPage();

    expect(screen.getByRole("heading", { name: "Databases" })).toBeInTheDocument();
    expect(screen.getAllByText("Primary").length).toBeGreaterThan(0);
    expect(screen.getAllByText("PostgreSQL").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Production").length).toBeGreaterThan(0);
    expect(screen.getAllByText("127.0.0.1:5432").length).toBeGreaterThan(0);
    expect(screen.getAllByText("DB connected").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Tunnel active").length).toBeGreaterThan(0);
    expect(screen.getAllByText("16.2.0").length).toBeGreaterThan(0);
    expect(screen.getAllByText("14 ms").length).toBeGreaterThan(0);
    expect(screen.getAllByText("app").length).toBeGreaterThan(0);
    expect(await screen.findByText("1 saved connection on Production.")).toBeInTheDocument();
  });

  it("shows an empty workspace with the add-connection action", () => {
    useConnectedServer();
    useProfileStore({
      profiles: { [server.id]: [] },
      snapshots: {},
      connectionStates: {},
      testResults: {},
    });
    renderPage();

    expect(
      screen.getByRole("heading", { name: "No database connections yet" }),
    ).toBeInTheDocument();
    expect(screen.getByText("No saved connections")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add your first connection" })).toBeInTheDocument();
  });

  it("keeps saved profiles manageable while SSH is disconnected", () => {
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useProfileStore({ snapshots: { [profile.id]: disconnectedSnapshot } });
    renderPage();

    expect(screen.getByText("SSH session is not connected")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect Primary" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit Primary" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete Primary" })).toBeInTheDocument();
  });

  it("connects and disconnects a profile through the store", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useProfileStore({ snapshots: { [profile.id]: disconnectedSnapshot } });
    renderPage();

    await user.click(screen.getByRole("button", { name: "Connect Primary" }));

    await waitFor(() => {
      expect(connectDatabaseProfile).toHaveBeenCalledWith(profile.id);
    });
    expect(
      await screen.findByText("Connected to Primary through the SSH tunnel."),
    ).toBeInTheDocument();

    await user.click(await screen.findByRole("button", { name: "Disconnect Primary" }));

    await waitFor(() => {
      expect(disconnectDatabaseProfile).toHaveBeenCalledWith(profile.id);
    });
    expect(
      await screen.findByText("Disconnected Primary. The loopback tunnel was closed."),
    ).toBeInTheDocument();
  });

  it("reconnects an active profile", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useProfileStore();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Reconnect Primary" }));

    await waitFor(() => {
      expect(reconnectDatabaseProfile).toHaveBeenCalledWith(profile.id);
    });
    expect(
      await screen.findByText("Reconnected Primary through the SSH tunnel."),
    ).toBeInTheDocument();
  });

  it("runs a saved test and reports the measured latency", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useProfileStore();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Test Primary" }));

    await waitFor(() => {
      expect(testDatabaseProfile).toHaveBeenCalledWith(profile.id);
    });
    expect(await screen.findByText("Primary responded in 14 ms (16.2.0).")).toBeInTheDocument();
  });

  it("surfaces an actionable test failure", async () => {
    const user = userEvent.setup();
    vi.mocked(testDatabaseProfile).mockRejectedValue({
      code: "database_auth",
      message: "The database rejected the username or password.",
    });
    useConnectedServer();
    useProfileStore();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Test Primary" }));

    expect(await screen.findByText("Database profile action failed")).toBeInTheDocument();
    expect(screen.getByText("The database rejected the username or password.")).toBeInTheDocument();
  });

  it("names the profile, server, and engine in the delete confirmation", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useProfileStore({ snapshots: { [profile.id]: disconnectedSnapshot } });
    renderPage();

    await user.click(screen.getByRole("button", { name: "Delete Primary" }));

    const dialog = await screen.findByRole("dialog", { name: "Delete Primary?" });
    expect(within(dialog).getByText("Production (198.51.100.24)")).toBeInTheDocument();
    expect(within(dialog).getByText("PostgreSQL")).toBeInTheDocument();
    expect(within(dialog).getByText("127.0.0.1:5432")).toBeInTheDocument();
    expect(
      within(dialog).getByText(
        "The database password is deleted from the OS credential store. Existing server data is untouched.",
      ),
    ).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Delete connection" }));

    await waitFor(() => {
      expect(deleteDatabaseProfile).toHaveBeenCalledWith(profile.id);
    });
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Delete Primary?" })).not.toBeInTheDocument();
    });
  });

  it("opens an edit dialog without echoing a saved password", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useProfileStore();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Edit Primary" }));

    const dialog = await screen.findByRole("dialog", { name: "Edit database connection" });
    const password = within(dialog).getByLabelText("Database password");
    expect(password).toHaveValue("");
    expect(password).toHaveAttribute("placeholder", "Leave blank to keep the saved password");
    expect(within(dialog).getByText("Password saved")).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("fixture-saved-password");
  });

  it("opens the create dialog from the workspace toolbar", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Add connection" }));

    const dialog = await screen.findByRole("dialog", { name: "Add database connection" });
    expect(within(dialog).getByLabelText("Profile name")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("Server")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("Engine")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("SSL mode")).toBeInTheDocument();
  });

  it("supports keyboard navigation between workspace tabs", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useProfileStore();
    renderPage();

    await user.click(screen.getByRole("tab", { name: "Overview" }));
    await user.keyboard("{ArrowRight}");

    expect(screen.getByRole("tab", { name: "Structure" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { name: "No table selected" })).toBeInTheDocument();
  });

  it("shows the selected database and schema in the workspace header", async () => {
    const user = userEvent.setup();
    const unavailable = { status: "unavailable" } as const;
    const metadata = {
      owner: unavailable,
      encoding: unavailable,
      collation: unavailable,
      sizeBytes: unavailable,
      engine: unavailable,
    };
    const databasesPayload: DatabaseExplorerNodePayload = {
      nodeKind: "databases",
      payload: {
        items: [{ name: "app", system: false, ...metadata }],
        limit: 100,
        offset: 0,
        hasMore: false,
        total: 1,
      },
    };
    const schemasPayload: DatabaseExplorerNodePayload = {
      nodeKind: "schemas",
      payload: {
        items: [{ name: "public", system: false, ...metadata }],
        limit: 100,
        offset: 0,
        hasMore: false,
        total: 1,
      },
    };
    useDatabaseExplorerStore.setState({
      nodes: {
        [databaseExplorerNodeKey(profile.id, { kind: "databases" })]: {
          status: "ready",
          payload: databasesPayload,
        },
        [databaseExplorerNodeKey(profile.id, { kind: "schemas" })]: {
          status: "ready",
          payload: schemasPayload,
        },
      },
      error: null,
      commandError: null,
    });
    useConnectedServer();
    useProfileStore();
    renderPage();

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));
    await user.click(await screen.findByRole("treeitem", { name: /^app/ }));
    await user.click(await screen.findByRole("treeitem", { name: /^Schemas/ }));
    await user.click(await screen.findByRole("treeitem", { name: /^public/ }));

    const context = screen.getByLabelText("Selected database and schema");
    expect(within(context).getByText("app")).toBeInTheDocument();
    expect(within(context).getByText("public")).toBeInTheDocument();
  });

  it("sets the active database and refreshes the explorer context", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useProfileStore();
    useLoadedDatabases();
    renderPage();

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));
    await user.click(await screen.findByRole("button", { name: "Set analytics active" }));

    await waitFor(() => {
      expect(setActiveDatabase).toHaveBeenCalledWith(profile.id, "analytics");
    });
    expect(
      await screen.findByText("Metadata re-read confirms database analytics."),
    ).toBeInTheDocument();
    expect(refreshExplorerNode).toHaveBeenCalledWith(profile.id, { kind: "databases" }, undefined);
    const context = screen.getByLabelText("Selected database and schema");
    expect(within(context).getByText("analytics")).toBeInTheDocument();
  });

  it("previews DDL and requires confirmation before dropping a database", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    vi.mocked(getDatabaseConnectionState).mockResolvedValue({
      ...connectedSnapshot,
      readOnly: false,
    });
    useProfileStore({
      profiles: { [server.id]: [{ ...profile, readOnly: false }] },
      snapshots: { [profile.id]: { ...connectedSnapshot, readOnly: false } },
    });
    useLoadedDatabases();
    renderPage();

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));
    await user.click(await screen.findByRole("button", { name: "Drop analytics" }));

    const dialog = await screen.findByRole("dialog", { name: "Drop database analytics?" });
    expect(within(dialog).getByText("Production (198.51.100.24)")).toBeInTheDocument();
    expect(within(dialog).getByText("PostgreSQL")).toBeInTheDocument();
    expect(
      within(dialog).getByText("DROP DATABASE analytics (permanent deletion)"),
    ).toBeInTheDocument();
    expect(within(dialog).getByText('DROP DATABASE "analytics"')).toBeInTheDocument();
    expect(dropDatabase).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(dropDatabase).not.toHaveBeenCalled();
    expect(
      screen.queryByRole("dialog", { name: "Drop database analytics?" }),
    ).not.toBeInTheDocument();

    await user.click(await screen.findByRole("button", { name: "Drop analytics" }));
    const confirmDialog = await screen.findByRole("dialog", {
      name: "Drop database analytics?",
    });
    await user.click(within(confirmDialog).getByRole("button", { name: "Drop database" }));

    await waitFor(() => {
      expect(dropDatabase).toHaveBeenCalledWith(profile.id, "analytics");
    });
    expect(
      await screen.findByText("Metadata re-read confirms database analytics."),
    ).toBeInTheDocument();
  });

  it("explains that MySQL rename is unavailable", async () => {
    const user = userEvent.setup();
    const mysqlProfile: DatabaseProfile = {
      ...profile,
      engine: "mysql",
      port: 3306,
      databaseName: "app",
      readOnly: false,
    };
    const mysqlCapabilities: DatabaseCapabilities = {
      ...capabilities,
      engine: "mysql",
      flags: { ...capabilities.flags, databaseRename: false, schemas: false, sequences: false },
    };
    useConnectedServer();
    vi.mocked(getDatabaseConnectionState).mockResolvedValue({
      ...connectedSnapshot,
      engine: "mysql",
      readOnly: false,
      capabilities: mysqlCapabilities,
    });
    useProfileStore({
      profiles: { [server.id]: [mysqlProfile] },
      snapshots: {
        [mysqlProfile.id]: {
          ...connectedSnapshot,
          engine: "mysql",
          readOnly: false,
          capabilities: mysqlCapabilities,
          databaseName: "app",
        },
      },
    });
    useDatabaseExplorerStore.setState({
      nodes: {
        [databaseExplorerNodeKey(mysqlProfile.id, { kind: "databases" })]: {
          status: "ready",
          payload: databasesPayload,
          updatedAt: 1,
        },
      },
      error: null,
      commandError: null,
    });
    renderPage();

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));
    await user.click(await screen.findByRole("treeitem", { name: /^app/ }));

    const rename = await screen.findByRole("button", { name: "Rename app" });
    expect(rename).toBeDisabled();
    expect(rename).toHaveAttribute(
      "title",
      expect.stringContaining("MySQL does not support renaming a database safely"),
    );
    expect(
      screen.getByText(
        "MySQL does not support renaming a database safely. Create a new database and migrate its data instead.",
      ),
    ).toBeInTheDocument();
  });

  it("shows a loading placeholder while server profiles load", () => {
    useServerStore.setState({ servers: [], status: "loading", error: null });
    renderPage();

    expect(screen.getByRole("heading", { name: "Loading database workspace" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Loading saved server profiles...");
  });

  it("shows an error placeholder and retries loading server profiles", async () => {
    const user = userEvent.setup();
    const fetchServers = vi.fn().mockResolvedValue(undefined);
    useServerStore.setState({
      servers: [],
      status: "error",
      error: "The profile store did not respond.",
      fetchServers,
    });
    renderPage();

    expect(screen.getByText("Could not load server profiles")).toBeInTheDocument();
    expect(screen.getByText("The profile store did not respond.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(fetchServers).toHaveBeenCalledTimes(1);
  });
});
