import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseCapabilities,
  DatabaseConnectionSnapshot,
  DatabaseEngine,
  DatabaseProfile,
  DatabaseProfileInput,
  DatabaseTestResult,
  DatabaseTunnelSnapshot,
} from "@/types/contracts";

export const databaseProfileCommands = {
  list: "list_database_profiles",
  get: "get_database_profile",
  create: "create_database_profile",
  update: "update_database_profile",
  delete: "delete_database_profile",
  test: "test_database_profile",
  connect: "connect_database_profile",
  reconnect: "reconnect_database_profile",
  disconnect: "disconnect_database_profile",
  state: "get_database_connection_state",
} as const;

export const databaseTunnelCommands = {
  open: "open_database_tunnel",
  close: "close_database_tunnel",
  get: "get_database_tunnel_state",
  list: "list_database_tunnels",
} as const;

export const databaseCapabilityCommand = "get_database_capabilities";

export function listDatabaseProfiles(serverId: string): Promise<DatabaseProfile[]> {
  return invoke<DatabaseProfile[]>(databaseProfileCommands.list, { serverId });
}

export function getDatabaseProfile(id: string): Promise<DatabaseProfile | null> {
  return invoke<DatabaseProfile | null>(databaseProfileCommands.get, { id });
}

export function createDatabaseProfile(input: DatabaseProfileInput): Promise<DatabaseProfile> {
  return invoke<DatabaseProfile>(databaseProfileCommands.create, { input });
}

export function updateDatabaseProfile(
  id: string,
  input: DatabaseProfileInput,
): Promise<DatabaseProfile> {
  return invoke<DatabaseProfile>(databaseProfileCommands.update, { id, input });
}

export function deleteDatabaseProfile(id: string): Promise<void> {
  return invoke<void>(databaseProfileCommands.delete, { id });
}

export function testDatabaseProfile(id: string): Promise<DatabaseTestResult> {
  return invoke<DatabaseTestResult>(databaseProfileCommands.test, { id });
}

export function connectDatabaseProfile(id: string): Promise<DatabaseConnectionSnapshot> {
  return invoke<DatabaseConnectionSnapshot>(databaseProfileCommands.connect, { id });
}

export function reconnectDatabaseProfile(id: string): Promise<DatabaseConnectionSnapshot> {
  return invoke<DatabaseConnectionSnapshot>(databaseProfileCommands.reconnect, { id });
}

export function disconnectDatabaseProfile(id: string): Promise<DatabaseConnectionSnapshot> {
  return invoke<DatabaseConnectionSnapshot>(databaseProfileCommands.disconnect, { id });
}

export function getDatabaseConnectionState(id: string): Promise<DatabaseConnectionSnapshot> {
  return invoke<DatabaseConnectionSnapshot>(databaseProfileCommands.state, { id });
}

export function getDatabaseCapabilities(
  engine: DatabaseEngine,
  version?: string | null,
): Promise<DatabaseCapabilities> {
  return invoke<DatabaseCapabilities>(databaseCapabilityCommand, {
    engine,
    version: version ?? null,
  });
}

export function openDatabaseTunnel(profileId: string): Promise<DatabaseTunnelSnapshot> {
  return invoke<DatabaseTunnelSnapshot>(databaseTunnelCommands.open, { profileId });
}

export function closeDatabaseTunnel(profileId: string): Promise<void> {
  return invoke<void>(databaseTunnelCommands.close, { profileId });
}

export function getDatabaseTunnelState(profileId: string): Promise<DatabaseTunnelSnapshot | null> {
  return invoke<DatabaseTunnelSnapshot | null>(databaseTunnelCommands.get, { profileId });
}

export function listDatabaseTunnels(): Promise<DatabaseTunnelSnapshot[]> {
  return invoke<DatabaseTunnelSnapshot[]>(databaseTunnelCommands.list);
}
