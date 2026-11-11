import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  connectDatabaseProfile,
  createDatabaseProfile,
  deleteDatabaseProfile,
  disconnectDatabaseProfile,
  getDatabaseConnectionState,
  listDatabaseProfiles,
  reconnectDatabaseProfile,
  testDatabaseProfile,
  updateDatabaseProfile,
} from "@/services/databaseProfiles";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import type {
  DatabaseCapabilities,
  DatabaseConnectionSnapshot,
  DatabaseProfile,
  DatabaseProfileInput,
  DatabaseTestResult,
} from "@/types/contracts";

vi.mock("@/services/databaseProfiles", () => ({
  listDatabaseProfiles: vi.fn(),
  getDatabaseProfile: vi.fn(),
  createDatabaseProfile: vi.fn(),
  updateDatabaseProfile: vi.fn(),
  deleteDatabaseProfile: vi.fn(),
  testDatabaseProfile: vi.fn(),
  connectDatabaseProfile: vi.fn(),
  reconnectDatabaseProfile: vi.fn(),
  disconnectDatabaseProfile: vi.fn(),
  getDatabaseConnectionState: vi.fn(),
}));

const profile: DatabaseProfile = {
  id: "profile-1",
  serverId: "server-1",
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

const input: DatabaseProfileInput = {
  serverId: "server-1",
  name: "Primary",
  engine: "postgresql",
  host: "127.0.0.1",
  port: 5432,
  socket: null,
  databaseName: "app",
  username: "app",
  readOnly: true,
  sslMode: "required",
  password: "fixture-password",
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

function snapshot(
  state: DatabaseConnectionSnapshot["state"],
  changedAt = 1,
): DatabaseConnectionSnapshot {
  return {
    profileId: profile.id,
    serverId: profile.serverId,
    name: profile.name,
    engine: profile.engine,
    state,
    readOnly: true,
    databaseName: "app",
    changedAt,
  };
}

const testResult: DatabaseTestResult = {
  profileId: profile.id,
  serverId: profile.serverId,
  engine: profile.engine,
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  databaseName: "app",
  readOnly: true,
  capabilities,
  latencyMs: 3,
  testedAt: 2,
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("database profile store", () => {
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

  it("covers the loading and ready states for profile lists", async () => {
    const pending = deferred<DatabaseProfile[]>();
    vi.mocked(listDatabaseProfiles).mockReturnValue(pending.promise);

    const request = useDatabaseProfileStore.getState().fetchProfiles("server-1");
    expect(useDatabaseProfileStore.getState().status["server-1"]).toBe("loading");

    pending.resolve([profile]);
    await request;

    const state = useDatabaseProfileStore.getState();
    expect(listDatabaseProfiles).toHaveBeenCalledWith("server-1");
    expect(state.status["server-1"]).toBe("ready");
    expect(state.profiles["server-1"]).toEqual([profile]);
  });

  it("stores typed list errors without discarding the message", async () => {
    vi.mocked(listDatabaseProfiles).mockRejectedValue({
      code: "storage",
      message: "Local storage error",
    });

    await useDatabaseProfileStore.getState().fetchProfiles("server-1");

    const state = useDatabaseProfileStore.getState();
    expect(state.status["server-1"]).toBe("error");
    expect(state.error).toBe("Local storage error");
    expect(state.commandError?.code).toBe("storage");
  });

  it("creates, updates, and deletes profiles through the service", async () => {
    const updated = { ...profile, name: "Replica", serverId: "server-2" };
    vi.mocked(createDatabaseProfile).mockResolvedValue(profile);
    vi.mocked(updateDatabaseProfile).mockResolvedValue(updated);
    vi.mocked(deleteDatabaseProfile).mockResolvedValue(undefined);

    await expect(useDatabaseProfileStore.getState().createProfile(input)).resolves.toEqual(profile);
    expect(createDatabaseProfile).toHaveBeenCalledWith(input);
    expect(useDatabaseProfileStore.getState().profiles["server-1"]).toEqual([profile]);

    await expect(
      useDatabaseProfileStore.getState().updateProfile(profile.id, input),
    ).resolves.toEqual(updated);
    expect(updateDatabaseProfile).toHaveBeenCalledWith(profile.id, input);
    expect(useDatabaseProfileStore.getState().profiles["server-1"]).toBeUndefined();
    expect(useDatabaseProfileStore.getState().profiles["server-2"]).toEqual([updated]);

    await expect(useDatabaseProfileStore.getState().deleteProfile(profile.id)).resolves.toBe(true);
    expect(deleteDatabaseProfile).toHaveBeenCalledWith(profile.id);
    expect(useDatabaseProfileStore.getState().profiles["server-2"]).toBeUndefined();
  });

  it("drops a connected snapshot when update changes connection details", async () => {
    useDatabaseProfileStore.setState({
      profiles: { "server-1": [profile] },
      status: { "server-1": "ready" },
      snapshots: { [profile.id]: snapshot("connected", 5) },
      connectionStates: { [profile.id]: "connected" },
    });
    vi.mocked(updateDatabaseProfile).mockResolvedValue({ ...profile, host: "10.0.0.5" });

    await useDatabaseProfileStore.getState().updateProfile(profile.id, {
      ...input,
      host: "10.0.0.5",
    });

    const state = useDatabaseProfileStore.getState();
    expect(state.profiles["server-1"]?.[0]?.host).toBe("10.0.0.5");
    expect(state.connectionStates[profile.id]).toBe("disconnected");
    expect(state.snapshots[profile.id]?.state).toBe("disconnected");
  });

  it("tests a profile and keeps the typed result", async () => {
    vi.mocked(testDatabaseProfile).mockResolvedValue(testResult);

    await expect(useDatabaseProfileStore.getState().testProfile(profile.id)).resolves.toEqual(
      testResult,
    );

    const state = useDatabaseProfileStore.getState();
    expect(testDatabaseProfile).toHaveBeenCalledWith(profile.id);
    expect(state.testResults[profile.id]).toEqual(testResult);
    expect(state.busy[profile.id]).toBe(false);
  });

  it("moves through connecting, connected, and disconnected states", async () => {
    useDatabaseProfileStore.setState({
      profiles: { "server-1": [profile] },
      status: { "server-1": "ready" },
    });
    const connecting = deferred<DatabaseConnectionSnapshot>();
    vi.mocked(connectDatabaseProfile).mockReturnValue(connecting.promise);
    vi.mocked(disconnectDatabaseProfile).mockResolvedValue(snapshot("disconnected", 3));

    const request = useDatabaseProfileStore.getState().connectProfile(profile.id);
    expect(useDatabaseProfileStore.getState().connectionStates[profile.id]).toBe("connecting");
    expect(useDatabaseProfileStore.getState().busy[profile.id]).toBe(true);

    connecting.resolve(snapshot("connected", 2));
    await expect(request).resolves.toBe(true);

    let state = useDatabaseProfileStore.getState();
    expect(connectDatabaseProfile).toHaveBeenCalledWith(profile.id);
    expect(state.connectionStates[profile.id]).toBe("connected");
    expect(state.snapshots[profile.id]?.state).toBe("connected");
    expect(state.busy[profile.id]).toBe(false);

    await expect(useDatabaseProfileStore.getState().disconnectProfile(profile.id)).resolves.toBe(
      true,
    );
    expect(disconnectDatabaseProfile).toHaveBeenCalledWith(profile.id);

    state = useDatabaseProfileStore.getState();
    expect(state.connectionStates[profile.id]).toBe("disconnected");
    expect(state.snapshots[profile.id]?.state).toBe("disconnected");
  });

  it("marks reconnect as reconnecting and then connected", async () => {
    useDatabaseProfileStore.setState({
      profiles: { "server-1": [profile] },
      status: { "server-1": "ready" },
    });
    const reconnecting = deferred<DatabaseConnectionSnapshot>();
    vi.mocked(reconnectDatabaseProfile).mockReturnValue(reconnecting.promise);

    const request = useDatabaseProfileStore.getState().reconnectProfile(profile.id);
    expect(useDatabaseProfileStore.getState().connectionStates[profile.id]).toBe("reconnecting");

    reconnecting.resolve(snapshot("connected", 4));
    await expect(request).resolves.toBe(true);
    expect(useDatabaseProfileStore.getState().connectionStates[profile.id]).toBe("connected");
  });

  it("stores typed connection errors and keeps an error snapshot", async () => {
    useDatabaseProfileStore.setState({
      profiles: { "server-1": [profile] },
      status: { "server-1": "ready" },
    });
    vi.mocked(connectDatabaseProfile).mockRejectedValue({
      code: "database_auth",
      message: "The database rejected the username or password.",
    });

    await expect(useDatabaseProfileStore.getState().connectProfile(profile.id)).resolves.toBe(
      false,
    );

    const state = useDatabaseProfileStore.getState();
    expect(state.connectionStates[profile.id]).toBe("error");
    expect(state.snapshots[profile.id]?.state).toBe("error");
    expect(state.snapshots[profile.id]?.error?.code).toBe("database_auth");
    expect(state.error).toBe("The database rejected the username or password.");
    expect(state.commandError?.code).toBe("database_auth");
    expect(state.busy[profile.id]).toBe(false);
  });

  it("refreshes connection state through the service and keeps the newest snapshot", async () => {
    vi.mocked(getDatabaseConnectionState).mockResolvedValue(snapshot("connected", 10));

    await useDatabaseProfileStore.getState().refreshConnectionState(profile.id);
    expect(getDatabaseConnectionState).toHaveBeenCalledWith(profile.id);
    expect(useDatabaseProfileStore.getState().connectionStates[profile.id]).toBe("connected");

    useDatabaseProfileStore.getState().applySnapshot(snapshot("connecting", 9));
    expect(useDatabaseProfileStore.getState().connectionStates[profile.id]).toBe("connected");
  });

  it("reports a blocked preview while keeping errors clearable", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");

    await expect(useDatabaseProfileStore.getState().connectProfile(profile.id)).resolves.toBe(
      false,
    );
    expect(useDatabaseProfileStore.getState().error).toBe(
      "Database profiles are available in the desktop application.",
    );

    useDatabaseProfileStore.getState().clearError();
    expect(useDatabaseProfileStore.getState().error).toBeNull();
  });
});
