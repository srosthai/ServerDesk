import { invoke } from "@tauri-apps/api/core";
import type { UserPreferences } from "@/types/contracts";

export const preferenceCommands = {
  get: "get_preferences",
  save: "save_preferences",
} as const;

export function getPreferences(): Promise<UserPreferences> {
  return invoke<UserPreferences>(preferenceCommands.get);
}

export function savePreferences(preferences: UserPreferences): Promise<UserPreferences> {
  return invoke<UserPreferences>(preferenceCommands.save, { preferences });
}
