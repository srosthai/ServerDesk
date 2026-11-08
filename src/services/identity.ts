import { invoke } from "@tauri-apps/api/core";
import type { SystemIdentity } from "@/types/contracts";

export const systemIdentityCommands = {
  get: "get_system_identity",
} as const;

export function getSystemIdentity(id: string): Promise<SystemIdentity> {
  return invoke<SystemIdentity>(systemIdentityCommands.get, { id });
}
