import { invoke } from "@tauri-apps/api/core";
import type { RemoteCommandName, RemoteCommandResult } from "@/types/contracts";

export const remoteCommandCommands = {
  run: "run_remote_command",
} as const;

export const remoteCommands: RemoteCommandName[] = ["hostname", "uptime"];

export function runRemoteCommand(
  id: string,
  command: RemoteCommandName,
): Promise<RemoteCommandResult> {
  return invoke<RemoteCommandResult>(remoteCommandCommands.run, { id, command });
}
