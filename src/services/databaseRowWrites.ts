import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseRowDeleteRequest,
  DatabaseRowIdentityValue,
  DatabaseRowInsertRequest,
  DatabaseRowTable,
  DatabaseRowUpdateRequest,
  DatabaseRowWriteResult,
  DatabaseRowWriteValue,
} from "@/types/contracts";

export const databaseRowWriteCommands = {
  insert: "insert_database_table_row",
  update: "update_database_table_row",
  delete: "delete_database_table_rows",
} as const;

function writeTable(table: DatabaseRowTable) {
  return {
    database: table.database ?? null,
    schema: table.schema ?? null,
    name: table.table,
  };
}

/** Inserts one row with explicit value, NULL, or DEFAULT modes. */
export function insertTableRow(
  profileId: string,
  table: DatabaseRowTable,
  values: DatabaseRowWriteValue[],
): Promise<DatabaseRowWriteResult> {
  const request: DatabaseRowInsertRequest = {
    table: writeTable(table),
    values,
  };
  return invoke<DatabaseRowWriteResult>(databaseRowWriteCommands.insert, {
    profileId,
    request,
  });
}

/**
 * Updates one row addressed by its stable identity. Every changed column must
 * carry its original value so a concurrent change is reported as a conflict
 * instead of overwriting newer data.
 */
export function updateTableRow(
  profileId: string,
  table: DatabaseRowTable,
  identity: DatabaseRowIdentityValue[],
  values: DatabaseRowWriteValue[],
  expected: DatabaseRowIdentityValue[],
): Promise<DatabaseRowWriteResult> {
  const request: DatabaseRowUpdateRequest = {
    table: writeTable(table),
    identity,
    values,
    expected,
  };
  return invoke<DatabaseRowWriteResult>(databaseRowWriteCommands.update, {
    profileId,
    request,
  });
}

/**
 * Deletes only the explicitly selected identities. `confirmed` is required by
 * the backend; a false value is rejected before any transaction starts.
 */
export function deleteTableRows(
  profileId: string,
  table: DatabaseRowTable,
  targets: DatabaseRowIdentityValue[][],
  confirmed: boolean,
): Promise<DatabaseRowWriteResult> {
  const request: DatabaseRowDeleteRequest = {
    table: writeTable(table),
    targets,
  };
  return invoke<DatabaseRowWriteResult>(databaseRowWriteCommands.delete, {
    profileId,
    request,
    confirmed,
  });
}
