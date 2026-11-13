import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseProfileDialog } from "@/features/databases/DatabaseProfileDialog";
import {
  createDatabaseProfile,
  testDatabaseProfileDraft,
  updateDatabaseProfile,
} from "@/services/databaseProfiles";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import type {
  DatabaseCapabilities,
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

const server: ServerProfile = {
  id: "server-1",
  name: "Production",
  host: "198.51.100.24",
  port: 22,
  username: "deploy",
  authType: "password",
  credentialRef: "serverdesk:ssh:server-1",
  groupName: null,
  tags: [],
  notes: null,
  createdAt: 1,
  updatedAt: 1,
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
  actions: ["browse_databases", "run_query"],
};

const testResult: DatabaseTestResult = {
  profileId: profile.id,
  serverId: server.id,
  engine: "postgresql",
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  databaseName: "app",
  readOnly: true,
  capabilities,
  latencyMs: 11,
  testedAt: 2,
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function renderDialog(editing: DatabaseProfile | null = null) {
  const onOpenChange = vi.fn();
  const onSaved = vi.fn();
  render(
    <DatabaseProfileDialog
      servers={[server]}
      activeServerId={server.id}
      profile={editing}
      onOpenChange={onOpenChange}
      onSaved={onSaved}
    />,
  );
  return { onOpenChange, onSaved };
}

describe("DatabaseProfileDialog", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
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
  });

  it("tests unsaved values and reports version, latency, and active database", async () => {
    const user = userEvent.setup();
    vi.mocked(testDatabaseProfileDraft).mockResolvedValue(testResult);
    renderDialog();

    await user.type(screen.getByLabelText("Profile name"), "Primary");
    await user.type(screen.getByLabelText("Username"), "app");
    await user.type(screen.getByLabelText("Database (optional)"), "app");
    await user.type(screen.getByLabelText("Database password"), "fixture-password");
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("Connection test passed")).toBeInTheDocument();
    expect(screen.getByText(/PostgreSQL 16.2 responded in 11 ms/)).toBeInTheDocument();
    expect(screen.getByText(/Active database: app/)).toBeInTheDocument();
    expect(testDatabaseProfileDraft).toHaveBeenCalledWith(
      expect.objectContaining({
        serverId: server.id,
        name: "Primary",
        engine: "postgresql",
        host: "127.0.0.1",
        port: 5432,
        databaseName: "app",
        username: "app",
        readOnly: false,
        sslMode: null,
        password: "fixture-password",
      }),
      null,
    );
    expect(createDatabaseProfile).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain("fixture-password");
  });

  it("preserves non-secret fields and explains a database authentication failure", async () => {
    const user = userEvent.setup();
    const secret = "fixture-secret-password";
    vi.mocked(testDatabaseProfileDraft).mockRejectedValue({
      code: "database_auth",
      message: "The database rejected the username or password.",
    });
    renderDialog();

    await user.type(screen.getByLabelText("Profile name"), "Primary");
    await user.clear(screen.getByLabelText("Host"));
    await user.type(screen.getByLabelText("Host"), "10.0.0.5");
    await user.type(screen.getByLabelText("Username"), "app");
    await user.type(screen.getByLabelText("Database password"), secret);
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("Database authentication failed")).toBeInTheDocument();
    expect(screen.getByText("The database rejected the username or password.")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Check the database username and password, and confirm the account can connect from the server.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Profile name")).toHaveValue("Primary");
    expect(screen.getByLabelText("Host")).toHaveValue("10.0.0.5");
    expect(screen.getByLabelText("Username")).toHaveValue("app");
    expect(screen.getByLabelText("Database password")).toHaveValue(secret);
    expect(document.body.textContent).not.toContain(secret);
  });

  it("distinguishes SSH tunnel, timeout, and permission causes", async () => {
    const user = userEvent.setup();
    renderDialog();

    await user.type(screen.getByLabelText("Profile name"), "Primary");
    await user.type(screen.getByLabelText("Username"), "app");
    await user.type(screen.getByLabelText("Database password"), "fixture-password");

    for (const [code, title] of [
      ["authentication", "Authentication failed"],
      ["tunnel", "Database tunnel failed"],
      ["timeout", "Connection timed out"],
      ["permission", "Permission denied"],
    ] as const) {
      vi.mocked(testDatabaseProfileDraft).mockRejectedValue({
        code,
        message: `${title} fixture detail.`,
      });
      await user.click(screen.getByRole("button", { name: "Test connection" }));
      expect(await screen.findByText(title)).toBeInTheDocument();
      expect(screen.getByText(`${title} fixture detail.`)).toBeInTheDocument();
    }
  });

  it("never echoes the saved password into an edit form", async () => {
    const user = userEvent.setup();
    const fixtureSecret = "fixture-saved-password";
    vi.mocked(updateDatabaseProfile).mockResolvedValue(profile);
    const { onOpenChange, onSaved } = renderDialog(profile);

    const password = screen.getByLabelText("Database password");
    expect(password).toHaveValue("");
    expect(password).toHaveAttribute("placeholder", "Leave blank to keep the saved password");
    expect(screen.getByText("Password saved")).toBeInTheDocument();
    expect(document.body.textContent).not.toContain(fixtureSecret);
    expect(document.body.innerHTML).not.toContain(fixtureSecret);

    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => {
      expect(updateDatabaseProfile).toHaveBeenCalledWith(
        profile.id,
        expect.objectContaining({
          name: "Primary",
          host: "127.0.0.1",
          port: 5432,
          username: "app",
          readOnly: true,
          sslMode: "required",
          password: null,
        }),
      );
    });
    expect(onSaved).toHaveBeenCalledWith(profile);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("validates required fields before saving or testing", async () => {
    const user = userEvent.setup();
    renderDialog();

    await user.click(screen.getByRole("button", { name: "Save connection" }));

    expect(screen.getByText("Database profile name is required.")).toBeInTheDocument();
    expect(screen.getByText("Database username is required.")).toBeInTheDocument();
    expect(createDatabaseProfile).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Test connection" }));
    expect(screen.getByText("Database profile name is required.")).toBeInTheDocument();
    expect(testDatabaseProfileDraft).not.toHaveBeenCalled();
  });

  it("requires a password before the first test", async () => {
    const user = userEvent.setup();
    renderDialog();

    await user.type(screen.getByLabelText("Profile name"), "Primary");
    await user.type(screen.getByLabelText("Username"), "app");
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(
      await screen.findByText("Enter the database password before testing this profile."),
    ).toBeInTheDocument();
    expect(testDatabaseProfileDraft).not.toHaveBeenCalled();
  });

  it("saves a MySQL profile with all non-secret fields", async () => {
    const user = userEvent.setup();
    vi.mocked(createDatabaseProfile).mockImplementation(async (input) => ({
      id: "profile-mysql",
      serverId: input.serverId,
      name: input.name,
      engine: input.engine,
      host: input.host,
      port: input.port,
      socket: input.socket ?? null,
      databaseName: input.databaseName ?? null,
      username: input.username,
      hasCredential: true,
      readOnly: input.readOnly,
      sslMode: input.sslMode ?? null,
      createdAt: 1,
      updatedAt: 1,
      lastUsedAt: null,
    }));
    const { onOpenChange, onSaved } = renderDialog();

    await user.type(screen.getByLabelText("Profile name"), "MySQL app");
    await user.click(screen.getByLabelText("Engine"));
    await user.click(await screen.findByRole("option", { name: "MySQL" }));
    await user.clear(screen.getByLabelText("Port"));
    await user.type(screen.getByLabelText("Port"), "3307");
    await user.type(screen.getByLabelText("Socket (optional)"), "/var/run/mysqld/mysqld.sock");
    await user.type(screen.getByLabelText("Database (optional)"), "app");
    await user.type(screen.getByLabelText("Username"), "app");
    await user.click(screen.getByLabelText("Read-only connection"));
    await user.type(screen.getByLabelText("Database password"), "fixture-password");
    await user.click(screen.getByRole("button", { name: "Save connection" }));

    await waitFor(() => {
      expect(createDatabaseProfile).toHaveBeenCalledWith({
        serverId: server.id,
        name: "MySQL app",
        engine: "mysql",
        host: "127.0.0.1",
        port: 3307,
        socket: "/var/run/mysqld/mysqld.sock",
        databaseName: "app",
        username: "app",
        readOnly: true,
        sslMode: null,
        password: "fixture-password",
      });
    });
    expect(onSaved).toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
