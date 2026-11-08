import { invoke } from "@tauri-apps/api/core";
import type { ServerProfile, ServerProfileInput } from "@/types/contracts";

export const serverCommands = {
  list: "list_servers",
  get: "get_server",
  create: "create_server",
  update: "update_server",
  delete: "delete_server",
} as const;

export function listServers(): Promise<ServerProfile[]> {
  return invoke<ServerProfile[]>(serverCommands.list);
}

export function getServer(id: string): Promise<ServerProfile | null> {
  return invoke<ServerProfile | null>(serverCommands.get, { id });
}

export function createServer(input: ServerProfileInput): Promise<ServerProfile> {
  return invoke<ServerProfile>(serverCommands.create, { input });
}

export function updateServer(id: string, input: ServerProfileInput): Promise<ServerProfile> {
  return invoke<ServerProfile>(serverCommands.update, { id, input });
}

export function deleteServer(id: string): Promise<void> {
  return invoke<void>(serverCommands.delete, { id });
}
