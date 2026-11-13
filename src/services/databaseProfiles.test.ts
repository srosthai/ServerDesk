import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  closeDatabaseTunnel,
  connectDatabaseProfile,
  createDatabaseProfile,
  databaseCapabilityCommand,
  databaseProfileCommands,
  databaseTunnelCommands,
  deleteDatabaseProfile,
  disconnectDatabaseProfile,
  getDatabaseCapabilities,
  getDatabaseConnectionState,
  getDatabaseProfile,
  getDatabaseTunnelState,
  listDatabaseProfiles,
  listDatabaseTunnels,
  openDatabaseTunnel,
  reconnectDatabaseProfile,
  testDatabaseProfile,
  testDatabaseProfileDraft,
  updateDatabaseProfile,
} from "@/services/databaseProfiles";
import type {
  DatabaseCapabilities,
  DatabaseConnectionSnapshot,
  DatabaseProfile,
  DatabaseProfileInput,
  DatabaseTestResult,
  DatabaseTunnelSnapshot,
} from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
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
  sslMode: "verify-ca",
  createdAt: 1,
  updatedAt: 2,
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
  sslMode: "verify-ca",
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
  actions: ["browse_databases", "browse_tables", "run_query"],
};

const snapshot: DatabaseConnectionSnapshot = {
  profileId: profile.id,
  serverId: profile.serverId,
  name: profile.name,
  engine: profile.engine,
  state: "connected",
  readOnly: true,
  databaseName: "app",
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  capabilities,
  changedAt: 10,
};

const testResult: DatabaseTestResult = {
  profileId: profile.id,
  serverId: profile.serverId,
  engine: profile.engine,
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  databaseName: "app",
  readOnly: true,
  capabilities,
  latencyMs: 3,
  testedAt: 11,
};

const tunnel: DatabaseTunnelSnapshot = {
  profileId: profile.id,
  serverId: profile.serverId,
  state: "active",
  localHost: "127.0.0.1",
  localPort: 49152,
  remoteHost: "127.0.0.1",
  remotePort: 5432,
  openedAt: 12,
};

describe("database profile service contract", () => {
  it("uses the registered profile command names and payloads", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce([profile])
      .mockResolvedValueOnce(profile)
      .mockResolvedValueOnce(profile)
      .mockResolvedValueOnce(profile)
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(testResult)
      .mockResolvedValueOnce(snapshot)
      .mockResolvedValueOnce(snapshot)
      .mockResolvedValueOnce(snapshot)
      .mockResolvedValueOnce(snapshot);

    await expect(listDatabaseProfiles("server-1")).resolves.toEqual([profile]);
    expect(invoke).toHaveBeenNthCalledWith(1, databaseProfileCommands.list, {
      serverId: "server-1",
    });

    await expect(getDatabaseProfile("profile-1")).resolves.toEqual(profile);
    expect(invoke).toHaveBeenNthCalledWith(2, databaseProfileCommands.get, {
      id: "profile-1",
    });

    await expect(createDatabaseProfile(input)).resolves.toEqual(profile);
    expect(invoke).toHaveBeenNthCalledWith(3, databaseProfileCommands.create, { input });

    await expect(updateDatabaseProfile("profile-1", input)).resolves.toEqual(profile);
    expect(invoke).toHaveBeenNthCalledWith(4, databaseProfileCommands.update, {
      id: "profile-1",
      input,
    });

    await expect(deleteDatabaseProfile("profile-1")).resolves.toBeUndefined();
    expect(invoke).toHaveBeenNthCalledWith(5, databaseProfileCommands.delete, {
      id: "profile-1",
    });

    await expect(testDatabaseProfile("profile-1")).resolves.toEqual(testResult);
    expect(invoke).toHaveBeenNthCalledWith(6, databaseProfileCommands.test, {
      id: "profile-1",
    });

    await expect(connectDatabaseProfile("profile-1")).resolves.toEqual(snapshot);
    expect(invoke).toHaveBeenNthCalledWith(7, databaseProfileCommands.connect, {
      id: "profile-1",
    });

    await expect(reconnectDatabaseProfile("profile-1")).resolves.toEqual(snapshot);
    expect(invoke).toHaveBeenNthCalledWith(8, databaseProfileCommands.reconnect, {
      id: "profile-1",
    });

    await expect(disconnectDatabaseProfile("profile-1")).resolves.toEqual(snapshot);
    expect(invoke).toHaveBeenNthCalledWith(9, databaseProfileCommands.disconnect, {
      id: "profile-1",
    });

    await expect(getDatabaseConnectionState("profile-1")).resolves.toEqual(snapshot);
    expect(invoke).toHaveBeenNthCalledWith(10, databaseProfileCommands.state, {
      id: "profile-1",
    });
  });

  it("uses the registered capability and tunnel command names and payloads", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(capabilities)
      .mockResolvedValueOnce(tunnel)
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(tunnel)
      .mockResolvedValueOnce([tunnel]);

    await expect(getDatabaseCapabilities("postgresql", "16.2")).resolves.toEqual(capabilities);
    expect(invoke).toHaveBeenNthCalledWith(1, databaseCapabilityCommand, {
      engine: "postgresql",
      version: "16.2",
    });

    await expect(openDatabaseTunnel("profile-1")).resolves.toEqual(tunnel);
    expect(invoke).toHaveBeenNthCalledWith(2, databaseTunnelCommands.open, {
      profileId: "profile-1",
    });

    await expect(closeDatabaseTunnel("profile-1")).resolves.toBeUndefined();
    expect(invoke).toHaveBeenNthCalledWith(3, databaseTunnelCommands.close, {
      profileId: "profile-1",
    });

    await expect(getDatabaseTunnelState("profile-1")).resolves.toEqual(tunnel);
    expect(invoke).toHaveBeenNthCalledWith(4, databaseTunnelCommands.get, {
      profileId: "profile-1",
    });

    await expect(listDatabaseTunnels()).resolves.toEqual([tunnel]);
    expect(invoke).toHaveBeenNthCalledWith(5, databaseTunnelCommands.list);
  });

  it("tests draft form values without saving them", async () => {
    vi.mocked(invoke).mockResolvedValue(testResult);

    await expect(testDatabaseProfileDraft(input, "profile-1")).resolves.toEqual(testResult);
    expect(invoke).toHaveBeenCalledWith(databaseProfileCommands.testDraft, {
      input,
      profileId: "profile-1",
    });

    await expect(testDatabaseProfileDraft(input)).resolves.toEqual(testResult);
    expect(invoke).toHaveBeenLastCalledWith(databaseProfileCommands.testDraft, {
      input,
      profileId: null,
    });
  });

  it("preserves typed command errors for display", async () => {
    const error = {
      code: "conflict" as const,
      message: "A database profile with this name already exists for the server.",
    };
    vi.mocked(invoke).mockRejectedValueOnce(error);

    await expect(createDatabaseProfile(input)).rejects.toEqual(error);
  });

  it("keeps every registered command name in the public contract", () => {
    expect(Object.values(databaseProfileCommands)).toEqual([
      "list_database_profiles",
      "get_database_profile",
      "create_database_profile",
      "update_database_profile",
      "delete_database_profile",
      "test_database_profile",
      "test_database_profile_draft",
      "connect_database_profile",
      "reconnect_database_profile",
      "disconnect_database_profile",
      "get_database_connection_state",
    ]);
    expect(Object.values(databaseTunnelCommands)).toEqual([
      "open_database_tunnel",
      "close_database_tunnel",
      "get_database_tunnel_state",
      "list_database_tunnels",
    ]);
    expect(databaseCapabilityCommand).toBe("get_database_capabilities");
  });
});
