import { invoke } from "@tauri-apps/api/core";
import type {
  ConnectionSnapshot,
  ConnectionTestInput,
  ConnectionTestOutcome,
} from "@/types/contracts";

export const CONNECTION_STATE_EVENT = "connection-state-changed";

export const connectionCommands = {
  connect: "connect_server",
  disconnect: "disconnect_server",
  state: "get_connection_state",
  test: "test_server_connection",
} as const;

export function connectServer(id: string): Promise<ConnectionSnapshot> {
  return invoke<ConnectionSnapshot>(connectionCommands.connect, { id });
}

export function disconnectServer(id: string): Promise<ConnectionSnapshot> {
  return invoke<ConnectionSnapshot>(connectionCommands.disconnect, { id });
}

export function getConnectionState(id: string): Promise<ConnectionSnapshot> {
  return invoke<ConnectionSnapshot>(connectionCommands.state, { id });
}

export function testServerConnection(input: ConnectionTestInput): Promise<ConnectionTestOutcome> {
  return invoke<ConnectionTestOutcome>(connectionCommands.test, { input });
}
