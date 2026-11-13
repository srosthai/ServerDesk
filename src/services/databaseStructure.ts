import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseStructureActionRequest,
  DatabaseStructureDdlPreview,
} from "@/types/contracts";

export const databaseStructureCommands = {
  preview: "preview_database_structure_action",
} as const;

/**
 * Requests generated DDL for a table, column, index, or constraint change.
 * This never executes the DDL; execution will be added with the structure
 * editor in #174 and must go through the same preview/capability path.
 */
export function previewDatabaseStructureAction(
  profileId: string,
  request: DatabaseStructureActionRequest,
): Promise<DatabaseStructureDdlPreview> {
  return invoke<DatabaseStructureDdlPreview>(databaseStructureCommands.preview, {
    profileId,
    request,
  });
}
