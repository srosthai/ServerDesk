import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseManagerPage } from "@/features/databases/DatabaseManagerPage";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { getConnectionState } from "@/services/connections";
import { previewDatabaseAction, dropDatabase, setActiveDatabase } from "@/services/databaseActions";
import {
  getTableDependencies,
  listColumns,
  listConstraints,
  listIndexes,
  refreshExplorerNode,
} from "@/services/databaseDiscovery";
import {
  previewDatabaseStructureAction,
  runDatabaseStructureAction,
} from "@/services/databaseStructure";
import {
  connectDatabaseProfile,
  deleteDatabaseProfile,
  disconnectDatabaseProfile,
  getDatabaseConnectionState,
  reconnectDatabaseProfile,
  testDatabaseProfile,
} from "@/services/databaseProfiles";
import { objectRowId } from "@/features/databases/databaseExplorerTreeModel";
import type { DatabaseWorkspaceView } from "@/stores/databaseWorkspaceStore";
import { useConnectionStore } from "@/stores/connectionStore";
import { databaseExplorerNodeKey, useDatabaseExplorerStore } from "@/stores/databaseExplorerStore";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ConnectionSnapshot,
  DatabaseCapabilities,
  DatabaseColumnSummary,
  DatabaseConnectionSnapshot,
  DatabaseConstraintSummary,
  DatabaseExplorerNodePayload,
  DatabaseIndexSummary,
  DatabaseProfile,
  DatabaseStructureActionRequest,
  DatabaseStructureActionResult,
  DatabaseSummary,
  DatabaseBounded,
  DatabaseDependencySummary,
  DatabaseMetadataValue,
  DatabaseTableDependencies,
  DatabaseTableSummary,
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
  listColumns: vi.fn(),
  listIndexes: vi.fn(),
  listConstraints: vi.fn(),
  getTableDependencies: vi.fn(),
  getObjectCounts: vi.fn(),
  loadExplorerNode: vi.fn(),
  refreshExplorerNode: vi.fn(),
}));

vi.mock("@/services/databaseStructure", () => ({
  previewDatabaseStructureAction: vi.fn(),
  runDatabaseStructureAction: vi.fn(),
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
  readOnly: false,
  sslMode: "required",
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: 1_700_000_000,
};

const analyticsProfile: DatabaseProfile = {
  id: "profile-2",
  serverId: server.id,
  name: "Analytics",
  engine: "mysql",
  host: "127.0.0.1",
  port: 3306,
  socket: null,
  databaseName: "metrics",
  username: "reporting",
  hasCredential: true,
  readOnly: false,
  sslMode: null,
  createdAt: 2,
  updatedAt: 2,
  lastUsedAt: null,
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
    autoIncrementColumns: false,
    columnTypeChange: true,
    columnRename: true,
    tableRename: true,
    tableDuplicate: true,
    columnReorder: false,
    databaseRename: true,
    indexRename: true,
    partialIndexes: true,
    queryTimeout: true,
  },
  actions: [
    "browse_databases",
    "browse_schemas",
    "browse_tables",
    "browse_columns",
    "browse_indexes",
    "browse_constraints",
    "browse_views",
    "browse_sequences",
    "show_object_size",
    "create_table",
    "rename_table",
    "duplicate_table",
    "truncate_table",
    "drop_table",
    "add_column",
    "edit_column",
    "rename_column",
    "drop_column",
    "create_index",
    "rename_index",
    "drop_index",
    "create_check_constraint",
    "create_generated_column",
    "create_identity_column",
    "run_query",
  ],
};

const connectedSnapshot: DatabaseConnectionSnapshot = {
  profileId: profile.id,
  serverId: server.id,
  name: profile.name,
  engine: profile.engine,
  state: "connected",
  readOnly: false,
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

const analyticsCapabilities: DatabaseCapabilities = {
  ...capabilities,
  engine: "mysql",
  flags: {
    ...capabilities.flags,
    schemas: false,
    sequences: false,
    databaseRename: false,
  },
};

const analyticsConnectedSnapshot: DatabaseConnectionSnapshot = {
  profileId: analyticsProfile.id,
  serverId: server.id,
  name: analyticsProfile.name,
  engine: analyticsProfile.engine,
  state: "connected",
  readOnly: false,
  databaseName: "metrics",
  version: { version: "8.4.2", raw: "MySQL 8.4.2" },
  capabilities: analyticsCapabilities,
  tunnel: {
    profileId: analyticsProfile.id,
    serverId: server.id,
    state: "active",
    localHost: "127.0.0.1",
    localPort: 52342,
    remoteHost: "127.0.0.1",
    remotePort: 3306,
    openedAt: 1,
  },
  changedAt: 11,
};

const analyticsDisconnectedSnapshot: DatabaseConnectionSnapshot = {
  ...analyticsConnectedSnapshot,
  state: "disconnected",
  tunnel: undefined,
  changedAt: 13,
};

const interruptedSnapshot: DatabaseConnectionSnapshot = {
  ...connectedSnapshot,
  state: "error",
  tunnel: undefined,
  error: { code: "connection", message: "The tunnel was interrupted." },
  changedAt: 14,
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

function available<T>(value: T): DatabaseMetadataValue<T> {
  return { status: "available", value };
}

const unavailable = { status: "unavailable" } as const;

function bounded<T>(items: T[]): DatabaseBounded<T> {
  return { items, limit: 100, truncated: false };
}

const usersTable: DatabaseTableSummary = {
  database: "app",
  schema: "public",
  name: "users",
  kind: "table",
  estimatedRows: available(42),
  metadataRefreshedAt: available("2026-02-01 12:00:00"),
  owner: available("app"),
  encoding: available("UTF8"),
  collation: available("C.UTF-8"),
  sizeBytes: available(16_384),
  engine: unavailable,
};

const tablesPayload: DatabaseExplorerNodePayload = {
  nodeKind: "tables",
  payload: {
    items: [usersTable],
    limit: 100,
    offset: 0,
    hasMore: false,
    total: 1,
  },
};

function columnSummary(
  overrides: Partial<DatabaseColumnSummary> &
    Pick<DatabaseColumnSummary, "name" | "ordinalPosition">,
): DatabaseColumnSummary {
  return {
    dataType: "bigint",
    length: available(null),
    precision: available(20),
    scale: available(0),
    nullable: false,
    defaultValue: available(null),
    primaryKey: false,
    identity: available("none"),
    generated: available(false),
    generationExpression: available(null),
    comment: available(null),
    collation: available(null),
    ...overrides,
  };
}

const columns: DatabaseColumnSummary[] = [
  columnSummary({
    name: "id",
    ordinalPosition: 1,
    primaryKey: true,
    identity: available("by_default"),
  }),
  columnSummary({
    name: "email",
    ordinalPosition: 2,
    dataType: "character varying",
    length: available(255),
    precision: unavailable,
    scale: unavailable,
    nullable: true,
    defaultValue: available("'pending'"),
    comment: available("Login email"),
  }),
];

const indexes: DatabaseIndexSummary[] = [
  {
    name: "users_pkey",
    columns: ["id"],
    unique: true,
    primary: true,
    method: available("btree"),
    predicate: available(null),
  },
  {
    name: "users_email_idx",
    columns: ["email"],
    unique: true,
    primary: false,
    method: available("btree"),
    predicate: available(null),
  },
];

const constraints: DatabaseConstraintSummary[] = [
  {
    name: "users_pkey",
    kind: "primary_key",
    columns: ["id"],
    referencedSchema: unavailable,
    referencedTable: unavailable,
    referencedColumns: [],
    onUpdate: unavailable,
    onDelete: unavailable,
    checkExpression: unavailable,
    deferrable: available(false),
    initiallyDeferred: available(false),
  },
  {
    name: "users_team_id_fkey",
    kind: "foreign_key",
    columns: ["team_id"],
    referencedSchema: available("public"),
    referencedTable: available("teams"),
    referencedColumns: ["id"],
    onUpdate: available("CASCADE"),
    onDelete: available("SET NULL"),
    checkExpression: unavailable,
    deferrable: available(false),
    initiallyDeferred: available(false),
  },
];

const dependency: DatabaseDependencySummary = {
  kind: "incoming_foreign_key",
  name: "orders_user_id_fkey",
  database: "app",
  schema: "public",
  table: "orders",
  columns: ["user_id"],
  referencedColumns: ["id"],
  onUpdate: available("CASCADE"),
  onDelete: available("CASCADE"),
  details: available("orders.user_id references users.id"),
};

const dependencies: DatabaseTableDependencies = {
  incomingForeignKeys: available(bounded([dependency])),
  dependentViews: available(bounded([])),
  ownedSequences: unavailable,
  triggers: available(bounded([])),
};

const tableSelection = {
  id: objectRowId(profile.id, "app", "public", "tables", "users"),
  profileId: profile.id,
  kind: "table" as const,
  label: "users",
  engine: profile.engine,
  database: "app",
  schema: "public",
  objectName: "users",
};

const tableStructurePreview = (
  request: DatabaseStructureActionRequest,
): import("@/types/contracts").DatabaseStructureDdlPreview => {
  const object = {
    kind:
      request.kind === "drop_column" ||
      request.kind === "add_column" ||
      request.kind === "alter_column" ||
      request.kind === "rename_column"
        ? ("column" as const)
        : request.kind === "create_index" ||
            request.kind === "rename_index" ||
            request.kind === "drop_index"
          ? ("index" as const)
          : request.kind === "add_constraint" || request.kind === "drop_constraint"
            ? ("constraint" as const)
            : ("table" as const),
    database: "app",
    schema: "public",
    table:
      request.kind === "create_table"
        ? request.name
        : request.kind === "rename_table" ||
            request.kind === "duplicate_table" ||
            request.kind === "truncate_table" ||
            request.kind === "drop_table"
          ? request.name
          : request.table,
    name:
      request.kind === "drop_column" || request.kind === "rename_column"
        ? request.name
        : request.kind === "drop_index" || request.kind === "rename_index"
          ? request.name
          : request.kind === "drop_constraint"
            ? request.name
            : null,
  };
  const destructive = request.kind === "drop_column" || request.kind === "drop_table";
  const ddl = `ALTER TABLE "public"."users" /* ${request.kind} */;`;
  return {
    action: request.kind,
    engine: "postgresql",
    object,
    statements: [ddl],
    ddl,
    warnings: destructive ? ["Dependent objects may be affected."] : [],
    destructive,
    requiresConfirmation: destructive,
  };
};

const structureResult = (
  request: DatabaseStructureActionRequest,
): DatabaseStructureActionResult => ({
  action: request.kind,
  engine: "postgresql",
  object:
    request.kind === "drop_table" || request.kind === "truncate_table"
      ? {
          kind: "table",
          database: "app",
          schema: "public",
          table: request.name,
          name: null,
        }
      : {
          kind: "table",
          database: "app",
          schema: "public",
          table: "table" in request ? request.table : request.name,
          name: null,
        },
  targetTable: null,
  statements: [],
  warnings: [],
  verified: true,
  verification: `Metadata re-read confirms ${request.kind}.`,
  verificationError: null,
  metadata: usersTable,
  durationMs: 3,
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

function useTwoProfiles() {
  useProfileStore({
    profiles: { [server.id]: [profile, analyticsProfile] },
    snapshots: {
      [profile.id]: connectedSnapshot,
      [analyticsProfile.id]: analyticsConnectedSnapshot,
    },
    connectionStates: {
      [profile.id]: "connected",
      [analyticsProfile.id]: "connected",
    },
    testResults: {},
  });
}

function useTwoDatabaseNodes() {
  const analyticsDatabasesPayload: DatabaseExplorerNodePayload = {
    nodeKind: "databases",
    payload: {
      items: [databaseSummary("metrics")],
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
        updatedAt: 1,
      },
      [databaseExplorerNodeKey(analyticsProfile.id, { kind: "databases" })]: {
        status: "ready",
        payload: analyticsDatabasesPayload,
        updatedAt: 1,
      },
    },
    error: null,
    commandError: null,
  });
}

function useSelectedTable(
  view: DatabaseWorkspaceView,
  snapshot: DatabaseConnectionSnapshot = connectedSnapshot,
) {
  useConnectedServer();
  useProfileStore({ snapshots: { [profile.id]: snapshot } });
  useDatabaseWorkspaceStore.getState().openTab(profile);
  useDatabaseWorkspaceStore.getState().setSelection(profile.id, tableSelection);
  useDatabaseWorkspaceStore.getState().setActiveView(profile.id, view);
  useDatabaseExplorerStore.setState({
    nodes: {
      [databaseExplorerNodeKey(profile.id, {
        kind: "tables",
        database: "app",
        schema: "public",
      })]: {
        status: "ready",
        payload: tablesPayload,
        updatedAt: 1,
      },
    },
    error: null,
    commandError: null,
  });
}

function useWorkspaceWithoutTable(view: DatabaseWorkspaceView) {
  useConnectedServer();
  useProfileStore();
  useDatabaseWorkspaceStore.getState().openTab(profile);
  useDatabaseWorkspaceStore.getState().setActiveView(profile.id, view);
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
    useDatabaseWorkspaceStore.setState({ tabs: {}, order: [], activeTabId: null });
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
    vi.mocked(disconnectDatabaseProfile).mockImplementation((id) =>
      Promise.resolve(
        id === analyticsProfile.id ? analyticsDisconnectedSnapshot : disconnectedSnapshot,
      ),
    );
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
    vi.mocked(listColumns).mockReset();
    vi.mocked(listColumns).mockResolvedValue({
      items: columns,
      limit: 100,
      offset: 0,
      hasMore: false,
      total: columns.length,
    });
    vi.mocked(listIndexes).mockReset();
    vi.mocked(listIndexes).mockResolvedValue({
      items: indexes,
      limit: 100,
      offset: 0,
      hasMore: false,
      total: indexes.length,
    });
    vi.mocked(listConstraints).mockReset();
    vi.mocked(listConstraints).mockResolvedValue({
      items: constraints,
      limit: 100,
      offset: 0,
      hasMore: false,
      total: constraints.length,
    });
    vi.mocked(getTableDependencies).mockReset();
    vi.mocked(getTableDependencies).mockResolvedValue(dependencies);
    vi.mocked(previewDatabaseStructureAction).mockReset();
    vi.mocked(previewDatabaseStructureAction).mockImplementation((_profileId, request) =>
      Promise.resolve(tableStructurePreview(request)),
    );
    vi.mocked(runDatabaseStructureAction).mockReset();
    vi.mocked(runDatabaseStructureAction).mockImplementation((_profileId, request) =>
      Promise.resolve(structureResult(request)),
    );
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
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

  it("opens multiple database connections concurrently and shows identity and engine in each tab", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useTwoProfiles();
    renderPage();

    const primaryTab = screen.getByRole("tab", {
      name: "Primary (PostgreSQL) workspace tab",
    });
    expect(primaryTab).toHaveAttribute("aria-selected", "true");
    expect(
      within(screen.getByLabelText("Workspace tab header")).getByText("Primary"),
    ).toBeInTheDocument();
    expect(
      within(screen.getByLabelText("Workspace tab header")).getByText("PostgreSQL"),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Workspace tab header")).toHaveTextContent(
      /Production · 127\.0\.0\.1:5432/,
    );

    await user.click(screen.getByRole("button", { name: "Open Analytics workspace" }));

    expect(screen.getByRole("tab", { name: "Analytics (MySQL) workspace tab" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(primaryTab).toBeInTheDocument();
    expect(useDatabaseWorkspaceStore.getState().order).toEqual([profile.id, analyticsProfile.id]);
    const header = screen.getByLabelText("Workspace tab header");
    expect(header).toHaveTextContent("Analytics");
    expect(header).toHaveTextContent("MySQL");
    expect(header).toHaveTextContent("127.0.0.1:3306");

    const sqlView = screen.getByRole("tab", { name: "SQL" });
    expect(sqlView).toHaveAttribute("title", "SQL · Analytics · MySQL");

    await user.click(primaryTab);
    expect(screen.getByRole("tab", { name: "SQL" })).toHaveAttribute(
      "title",
      "SQL · Primary · PostgreSQL",
    );
  });

  it("keeps SQL drafts and active workspace views scoped to each connection", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useTwoProfiles();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Open Analytics workspace" }));
    await user.click(screen.getByRole("tab", { name: "SQL" }));

    const analyticsEditor = screen.getByLabelText("SQL editor for Analytics");
    await user.type(analyticsEditor, "select * from metrics");

    await user.click(screen.getByRole("tab", { name: "Primary (PostgreSQL) workspace tab" }));
    // The primary tab was still on Overview, so its own view state is restored.
    expect(screen.queryByLabelText("SQL editor for Primary")).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "SQL" }));

    const primaryEditor = screen.getByLabelText("SQL editor for Primary");
    expect(primaryEditor).toHaveValue("");
    await user.type(primaryEditor, "select * from users");

    await user.click(screen.getByRole("tab", { name: "Analytics (MySQL) workspace tab" }));
    expect(screen.getByLabelText("SQL editor for Analytics")).toHaveValue("select * from metrics");

    useDatabaseWorkspaceStore.getState().setSqlResult(profile.id, {
      statement: "select * from users",
      columns: ["id"],
      rows: [["1"]],
      affectedRows: null,
      durationMs: 4,
      error: null,
      executedAt: 10,
    });
    useDatabaseWorkspaceStore.getState().setSqlResult(analyticsProfile.id, {
      statement: "select * from metrics",
      columns: ["visits"],
      rows: [["42"]],
      affectedRows: null,
      durationMs: 6,
      error: null,
      executedAt: 11,
    });

    expect(
      await screen.findByRole("region", { name: "SQL results for Analytics" }),
    ).toHaveTextContent("visits");
    await user.click(screen.getByRole("tab", { name: "Primary (PostgreSQL) workspace tab" }));
    expect(
      await screen.findByRole("region", { name: "SQL results for Primary" }),
    ).toHaveTextContent("id");
    expect(
      screen.queryByRole("region", { name: "SQL results for Analytics" }),
    ).not.toBeInTheDocument();

    const state = useDatabaseWorkspaceStore.getState();
    expect(state.tabs[profile.id].sqlDraft).toBe("select * from users");
    expect(state.tabs[profile.id].activeView).toBe("sql");
    expect(state.tabs[profile.id].sqlResult?.columns).toEqual(["id"]);
    expect(state.tabs[analyticsProfile.id].sqlDraft).toBe("select * from metrics");
    expect(state.tabs[analyticsProfile.id].activeView).toBe("sql");
    expect(state.tabs[analyticsProfile.id].sqlResult?.columns).toEqual(["visits"]);
  });

  it("keeps selected database context scoped to each connection", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useTwoProfiles();
    useTwoDatabaseNodes();
    renderPage();

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));
    await user.click(await screen.findByRole("treeitem", { name: /^app/ }));

    let header = screen.getByLabelText("Workspace tab header");
    expect(within(header).getByText("app")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Open Analytics workspace" }));
    await user.click(screen.getByRole("treeitem", { name: /^Analytics/ }));
    await user.click(await screen.findByRole("treeitem", { name: /^metrics/ }));

    header = screen.getByLabelText("Workspace tab header");
    expect(within(header).getByText("Analytics")).toBeInTheDocument();
    // MySQL uses the database as its schema, so the context shows it twice.
    expect(within(header).getAllByText("metrics")).toHaveLength(2);

    await user.click(screen.getByRole("tab", { name: "Primary (PostgreSQL) workspace tab" }));
    header = screen.getByLabelText("Workspace tab header");
    expect(within(header).getByText("Primary")).toBeInTheDocument();
    expect(within(header).getByText("app")).toBeInTheDocument();
    expect(within(header).queryByText("metrics")).not.toBeInTheDocument();
  });

  it("closes one connection tab, disconnects only it, and clears only its state", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useTwoProfiles();
    useTwoDatabaseNodes();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Open Analytics workspace" }));
    useDatabaseWorkspaceStore.getState().addTask(profile.id, {
      id: "primary-task",
      label: "Export app",
      status: "running",
      startedAt: 1,
    });
    useDatabaseWorkspaceStore.getState().addTask(analyticsProfile.id, {
      id: "analytics-task",
      label: "Export metrics",
      status: "running",
      startedAt: 1,
    });

    await user.click(screen.getByRole("button", { name: "Close Primary workspace tab" }));

    await waitFor(() => {
      expect(disconnectDatabaseProfile).toHaveBeenCalledWith(profile.id);
    });
    expect(disconnectDatabaseProfile).not.toHaveBeenCalledWith(analyticsProfile.id);

    const state = useDatabaseWorkspaceStore.getState();
    expect(state.tabs[profile.id]).toBeUndefined();
    expect(state.tabs[analyticsProfile.id]).toMatchObject({ name: "Analytics" });
    expect(state.tabs[analyticsProfile.id].tasks).toHaveLength(1);
    expect(state.order).toEqual([analyticsProfile.id]);
    expect(state.activeTabId).toBe(analyticsProfile.id);

    const nodes = useDatabaseExplorerStore.getState().nodes;
    expect(nodes[databaseExplorerNodeKey(profile.id, { kind: "databases" })]).toBeUndefined();
    expect(
      nodes[databaseExplorerNodeKey(analyticsProfile.id, { kind: "databases" })],
    ).toBeDefined();
    expect(await screen.findByText(/Primary workspace closed/)).toBeInTheDocument();
  });

  it("disconnects only the affected connection and preserves its tab context", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    useTwoProfiles();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Open Analytics workspace" }));
    useDatabaseWorkspaceStore.getState().setActiveView(profile.id, "sql");
    useDatabaseWorkspaceStore.getState().setSqlDraft(profile.id, "select * from users");

    await user.click(screen.getByRole("button", { name: "Disconnect Primary" }));

    await waitFor(() => {
      expect(disconnectDatabaseProfile).toHaveBeenCalledWith(profile.id);
    });
    const profileState = useDatabaseProfileStore.getState();
    expect(profileState.snapshots[profile.id].state).toBe("disconnected");
    expect(profileState.snapshots[analyticsProfile.id].state).toBe("connected");
    expect(useDatabaseWorkspaceStore.getState().tabs[profile.id]).toMatchObject({
      activeView: "sql",
      sqlDraft: "select * from users",
    });
    expect(useDatabaseWorkspaceStore.getState().tabs[analyticsProfile.id]).toMatchObject({
      activeView: "overview",
    });
    expect(useDatabaseWorkspaceStore.getState().activeTabId).toBe(analyticsProfile.id);

    await user.click(screen.getByRole("tab", { name: "Primary (PostgreSQL) workspace tab" }));
    expect(screen.getByLabelText("SQL editor for Primary")).toHaveValue("select * from users");
    expect(screen.getByLabelText("Workspace tab header")).toHaveTextContent("DB disconnected");
  });

  it("reconnects one connection without disturbing another tab or its context", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    vi.mocked(getDatabaseConnectionState).mockImplementation((id) =>
      Promise.resolve(id === profile.id ? interruptedSnapshot : analyticsConnectedSnapshot),
    );
    useProfileStore({
      profiles: { [server.id]: [profile, analyticsProfile] },
      snapshots: {
        [profile.id]: interruptedSnapshot,
        [analyticsProfile.id]: analyticsConnectedSnapshot,
      },
      connectionStates: { [profile.id]: "error", [analyticsProfile.id]: "connected" },
      testResults: {},
    });
    renderPage();

    await user.click(screen.getByRole("button", { name: "Open Analytics workspace" }));
    useDatabaseWorkspaceStore.getState().setActiveView(profile.id, "sql");
    useDatabaseWorkspaceStore.getState().setSqlDraft(profile.id, "select current_user");

    await user.click(screen.getByRole("button", { name: "Reconnect Primary" }));

    await waitFor(() => {
      expect(reconnectDatabaseProfile).toHaveBeenCalledWith(profile.id);
    });
    const state = useDatabaseProfileStore.getState();
    expect(state.snapshots[profile.id].state).toBe("connected");
    expect(state.snapshots[analyticsProfile.id].state).toBe("connected");
    expect(useDatabaseWorkspaceStore.getState().activeTabId).toBe(profile.id);
    expect(useDatabaseWorkspaceStore.getState().tabs[profile.id]).toMatchObject({
      activeView: "sql",
      sqlDraft: "select current_user",
    });
    expect(useDatabaseWorkspaceStore.getState().tabs[analyticsProfile.id].activeView).toBe(
      "overview",
    );
    expect(screen.getByLabelText("SQL editor for Primary")).toHaveValue("select current_user");
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

  it("shows table overview, structure, and indexes for the selected table", async () => {
    const user = userEvent.setup();
    useSelectedTable("overview");
    renderPage();

    expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute("aria-selected", "true");
    const summary = screen.getByRole("region", { name: "Table summary" });
    expect(within(summary).getByText("users")).toBeInTheDocument();
    expect(within(summary).getByText("42")).toBeInTheDocument();
    expect(within(summary).getByText("16 KB")).toBeInTheDocument();

    const relationships = screen.getByRole("region", { name: "Table relationships" });
    expect(await within(relationships).findByText("users_team_id_fkey")).toBeInTheDocument();
    expect(within(relationships).getByText("public.teams (id)")).toBeInTheDocument();
    expect(await within(relationships).findByText("orders_user_id_fkey")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Structure" }));
    const columnsRegion = await screen.findByRole("region", { name: "Table columns" });
    expect(within(columnsRegion).getByText("email")).toBeInTheDocument();
    expect(within(columnsRegion).getByText("character varying(255)")).toBeInTheDocument();
    expect(within(columnsRegion).getByText("Login email")).toBeInTheDocument();
    const constraintsRegion = screen.getByRole("region", { name: "Table constraints" });
    expect(within(constraintsRegion).getByText("users_team_id_fkey")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Indexes" }));
    const indexesRegion = await screen.findByRole("region", { name: "Table indexes" });
    expect(within(indexesRegion).getByText("users_email_idx")).toBeInTheDocument();
    expect(within(indexesRegion).getByText("users_pkey")).toBeInTheDocument();
  });

  it("previews and copies DDL before adding a column", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    useSelectedTable("structure");
    renderPage();

    expect(await screen.findByRole("region", { name: "Table columns" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add column" }));

    const dialog = await screen.findByRole("dialog", { name: "Add column" });
    const apply = within(dialog).getByRole("button", { name: "Add column" });
    expect(apply).toBeDisabled();

    await user.type(within(dialog).getByLabelText("Column name"), "nickname");
    await user.type(within(dialog).getByLabelText("Data type"), "character varying(120)");
    await user.click(within(dialog).getByRole("button", { name: "Preview DDL" }));

    expect(await within(dialog).findByText(/add_column/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Copy DDL" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      'ALTER TABLE "public"."users" /* add_column */;',
    );
    expect(apply).toBeEnabled();

    await user.click(apply);

    await waitFor(() => {
      expect(runDatabaseStructureAction).toHaveBeenCalledWith(
        profile.id,
        expect.objectContaining({
          kind: "add_column",
          table: "users",
          column: expect.objectContaining({ name: "nickname" }),
        }),
        false,
      );
    });
  });

  it("sends only changed column properties on PostgreSQL edits", async () => {
    const user = userEvent.setup();
    useSelectedTable("structure");
    renderPage();

    expect(await screen.findByRole("region", { name: "Table columns" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Edit column email" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit column email" });
    const comment = within(dialog).getByLabelText("Comment");
    await user.clear(comment);
    await user.type(comment, "Updated sign-in address");
    await user.click(within(dialog).getByRole("button", { name: "Preview DDL" }));
    expect(await within(dialog).findByText(/alter_column/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Apply column change" }));

    await waitFor(() => {
      expect(runDatabaseStructureAction).toHaveBeenCalledTimes(1);
    });
    const request = vi.mocked(runDatabaseStructureAction).mock.calls[0][1];
    expect(request).toMatchObject({
      kind: "alter_column",
      name: "email",
      comment: "Updated sign-in address",
    });
    expect(request).not.toHaveProperty("dataType");
    expect(request).not.toHaveProperty("nullable");
  });

  it("requires confirmation after showing DDL before dropping a column", async () => {
    const user = userEvent.setup();
    useSelectedTable("structure");
    renderPage();

    expect(await screen.findByRole("region", { name: "Table columns" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Drop column email" }));

    const dialog = await screen.findByRole("dialog", { name: "Drop column email?" });
    expect(await within(dialog).findByText(/drop_column/)).toBeInTheDocument();
    expect(
      within(dialog).getByText("Column data will be permanently deleted."),
    ).toBeInTheDocument();
    expect(runDatabaseStructureAction).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Drop column email?" })).not.toBeInTheDocument();
    expect(runDatabaseStructureAction).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Drop column email" }));
    const confirmDialog = await screen.findByRole("dialog", { name: "Drop column email?" });
    expect(await within(confirmDialog).findByText(/drop_column/)).toBeInTheDocument();
    await user.click(within(confirmDialog).getByRole("button", { name: "Drop column" }));

    await waitFor(() => {
      expect(runDatabaseStructureAction).toHaveBeenCalledWith(
        profile.id,
        expect.objectContaining({ kind: "drop_column", name: "email" }),
        true,
      );
    });
  });

  it("disables unsupported structure actions and explains why", async () => {
    const limitedCapabilities: DatabaseCapabilities = {
      ...capabilities,
      actions: capabilities.actions.filter(
        (action) => action !== "rename_column" && action !== "create_check_constraint",
      ),
    };
    useSelectedTable("structure", {
      ...connectedSnapshot,
      capabilities: limitedCapabilities,
    });
    vi.mocked(getDatabaseConnectionState).mockResolvedValue({
      ...connectedSnapshot,
      capabilities: limitedCapabilities,
    });
    renderPage();

    const rename = await screen.findByRole("button", { name: "Rename column email" });
    expect(rename).toBeDisabled();
    expect(rename).toHaveAttribute(
      "title",
      "Column renaming is not supported by PostgreSQL 16.2.0.",
    );
    const notice = screen.getByRole("note", { name: "Unavailable structure actions" });
    expect(notice).toHaveTextContent("Column renaming is not supported by PostgreSQL 16.2.0.");
    expect(notice).toHaveTextContent(
      "Check constraint creation is not supported by PostgreSQL 16.2.0.",
    );
    expect(screen.getByRole("button", { name: "Add constraint" })).toBeEnabled();
  });

  it("keeps dependency errors inside the destructive confirmation", async () => {
    const user = userEvent.setup();
    vi.mocked(runDatabaseStructureAction).mockRejectedValue({
      code: "conflict",
      message: "orders_user_id_fkey still references users.id.",
      details: "Drop or redefine the incoming foreign key before dropping this column.",
    });
    useSelectedTable("structure");
    renderPage();

    expect(await screen.findByRole("region", { name: "Table columns" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Drop column id" }));
    const dialog = await screen.findByRole("dialog", { name: "Drop column id?" });
    expect(await within(dialog).findByText(/drop_column/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Drop column" }));

    expect(await within(dialog).findByText("Blocked by dependencies")).toBeInTheDocument();
    expect(
      within(dialog).getByText("orders_user_id_fkey still references users.id."),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText(
        "Drop or redefine the incoming foreign key before dropping this column.",
      ),
    ).toBeInTheDocument();
  });

  it("shows a permission-denied state scoped to the table", async () => {
    vi.mocked(listColumns).mockRejectedValue({
      code: "permission",
      message: "The database account cannot read this table's columns.",
      scope: { object: "table", name: "users" },
    });
    useSelectedTable("structure");
    renderPage();

    expect(await screen.findByText("Permission denied")).toBeInTheDocument();
    expect(
      screen.getByText(/The database account cannot read this table's columns/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Scoped to table users/)).toBeInTheDocument();
  });

  it("keeps typed permission errors inside the action dialog", async () => {
    const user = userEvent.setup();
    vi.mocked(runDatabaseStructureAction).mockRejectedValue({
      code: "permission",
      message: "The database account cannot add columns to this table.",
    });
    useSelectedTable("structure");
    renderPage();

    expect(await screen.findByRole("region", { name: "Table columns" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add column" }));
    const dialog = await screen.findByRole("dialog", { name: "Add column" });
    await user.type(within(dialog).getByLabelText("Column name"), "handle");
    await user.type(within(dialog).getByLabelText("Data type"), "text");
    await user.click(within(dialog).getByRole("button", { name: "Preview DDL" }));
    expect(await within(dialog).findByText(/add_column/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Add column" }));

    expect(await within(dialog).findByText("Permission denied")).toBeInTheDocument();
    expect(
      within(dialog).getByText("The database account cannot add columns to this table."),
    ).toBeInTheDocument();
  });

  it("shows an empty index state when the table has no indexes", async () => {
    vi.mocked(listIndexes).mockResolvedValue({
      items: [],
      limit: 100,
      offset: 0,
      hasMore: false,
      total: 0,
    });
    useSelectedTable("indexes");
    renderPage();

    expect(await screen.findByText("No indexes reported")).toBeInTheDocument();
  });

  it("shows a disconnected state for a selected table", () => {
    useSelectedTable("structure", disconnectedSnapshot);
    renderPage();

    expect(screen.getByText("Connection required")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect Primary" })).toBeInTheDocument();
  });

  it("creates a table from the structure workspace after a DDL preview", async () => {
    const user = userEvent.setup();
    useWorkspaceWithoutTable("structure");
    renderPage();

    expect(screen.getByRole("heading", { name: "No table selected" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Create table" }));

    const dialog = await screen.findByRole("dialog", { name: "Create table" });
    const apply = within(dialog).getByRole("button", { name: "Create table" });
    expect(apply).toBeDisabled();

    await user.type(within(dialog).getByLabelText("Table name"), "audit_log");
    await user.type(within(dialog).getByLabelText("Column 1 name"), "id");
    await user.type(within(dialog).getByLabelText("Column 1 type"), "bigint");
    await user.click(within(dialog).getByRole("button", { name: "Preview DDL" }));
    expect(await within(dialog).findByText(/create_table/)).toBeInTheDocument();
    await user.click(apply);

    await waitFor(() => {
      expect(runDatabaseStructureAction).toHaveBeenCalledWith(
        profile.id,
        expect.objectContaining({
          kind: "create_table",
          name: "audit_log",
          columns: [expect.objectContaining({ name: "id", dataType: "bigint" })],
        }),
        false,
      );
    });
  });
});
