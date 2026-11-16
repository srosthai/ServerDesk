import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseObjectActionRequest,
  DatabaseObjectActionResult,
  DatabaseObjectDefinition,
  DatabaseObjectDdlPreview,
  DatabaseObjectRef,
} from "@/types/contracts";

export const databaseObjectCommands = {
  inspect: "inspect_database_object",
  preview: "preview_database_object_action",
  run: "run_database_object_action",
} as const;

/** Reads one bounded object definition. */
export function inspectDatabaseObject(
  profileId: string,
  object: DatabaseObjectRef,
): Promise<DatabaseObjectDefinition> {
  return invoke<DatabaseObjectDefinition>(databaseObjectCommands.inspect, {
    profileId,
    object,
  });
}

/** Generates DDL for one object action without executing it. */
export function previewDatabaseObjectAction(
  profileId: string,
  request: DatabaseObjectActionRequest,
): Promise<DatabaseObjectDdlPreview> {
  return invoke<DatabaseObjectDdlPreview>(databaseObjectCommands.preview, {
    profileId,
    request,
  });
}

/**
 * Executes one validated object action. The backend regenerates the same
 * preview, rejects an unconfirmed drop/replace, and re-reads metadata.
 */
export function runDatabaseObjectAction(
  profileId: string,
  request: DatabaseObjectActionRequest,
  confirmed = false,
): Promise<DatabaseObjectActionResult> {
  return invoke<DatabaseObjectActionResult>(databaseObjectCommands.run, {
    profileId,
    request,
    confirmed,
  });
}
