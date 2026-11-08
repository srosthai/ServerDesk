import { invoke } from "@tauri-apps/api/core";
import type { LogTail, SystemLogSource } from "@/types/contracts";

export const logCommands = {
  sources: "list_log_sources",
  read: "read_log_tail",
} as const;

export function listLogSources(): Promise<SystemLogSource[]> {
  return invoke<SystemLogSource[]>(logCommands.sources);
}

export function readLogTail(serverId: string, path: string, lines: number): Promise<LogTail> {
  return invoke<LogTail>(logCommands.read, { id: serverId, path, lines });
}
