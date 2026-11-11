import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseManagerPage } from "@/features/databases/DatabaseManagerPage";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { getConnectionState } from "@/services/connections";
import {
  connectDatabaseProfile,
  deleteDatabaseProfile,
  disconnectDatabaseProfile,
  getDatabaseConnectionState,
  reconnectDatabaseProfile,
  testDatabaseProfile,
} from "@/services/databaseProfiles";
import { useConnectionStore } from "@/stores/connectionStore";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ConnectionSnapshot,
  DatabaseCapabilities,
  DatabaseConnectionSnapshot,
  DatabaseProfile,
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
