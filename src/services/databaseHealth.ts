import { invoke } from "@tauri-apps/api/core";
import type { DatabaseHealthSnapshot } from "@/types/contracts";

export const databaseHealthCommands = {
  get: "get_database_health",
} as const;

/** Takes one explicit health sample. No polling or background sampling exists. */
export function getDatabaseHealth(
  profileId: string,
  limit?: number,
): Promise<DatabaseHealthSnapshot> {
  return invoke<DatabaseHealthSnapshot>(databaseHealthCommands.get, {
    profileId,
    limit: limit ?? null,
  });
}
