import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseActionRequest,
  DatabaseActionResult,
  DatabaseDdlPreview,
} from "@/types/contracts";

export const databaseActionCommands = {
  preview: "preview_database_action",
  run: "run_database_action",
} as const;

export function previewDatabaseAction(
  profileId: string,
  request: DatabaseActionRequest,
): Promise<DatabaseDdlPreview> {
  return invoke<DatabaseDdlPreview>(databaseActionCommands.preview, {
    profileId,
    request,
  });
}

export function runDatabaseAction(
  profileId: string,
  request: DatabaseActionRequest,
  confirmed = false,
): Promise<DatabaseActionResult> {
  return invoke<DatabaseActionResult>(databaseActionCommands.run, {
    profileId,
    request,
    confirmed,
  });
}

export function createDatabase(
  profileId: string,
  input: {
    name: string;
    encoding?: string | null;
    collation?: string | null;
    owner?: string | null;
    template?: string | null;
  },
): Promise<DatabaseActionResult> {
  return runDatabaseAction(profileId, { kind: "create", ...input });
}

export function renameDatabase(
  profileId: string,
  name: string,
  newName: string,
): Promise<DatabaseActionResult> {
  return runDatabaseAction(profileId, { kind: "rename", name, newName });
}

export function dropDatabase(profileId: string, name: string): Promise<DatabaseActionResult> {
  return runDatabaseAction(profileId, { kind: "drop", name }, true);
}

export function setActiveDatabase(profileId: string, name: string): Promise<DatabaseActionResult> {
  return runDatabaseAction(profileId, { kind: "set_active", name });
}
