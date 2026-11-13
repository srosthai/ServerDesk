import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseRowCancellationResult,
  DatabaseRowOptions,
  DatabaseRowReadResult,
  DatabaseRowTable,
} from "@/types/contracts";

export const databaseRowCommands = {
  read: "read_database_table_rows",
  cancel: "cancel_database_row_read",
} as const;

/**
 * Creates the id used to cancel one in-flight row read. The backend accepts
 * only bounded ASCII letters, digits, hyphens, underscores, and dots.
 */
export function createDatabaseRowRequestId(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return cryptoApi.randomUUID();
  }
  const random = Math.random().toString(36).slice(2, 12);
  return `row-${Date.now().toString(36)}-${random}`;
}

/**
 * Reads one hard-bounded page of table rows. Visibility, sorting, filters,
 * search, exact-count opt-in, and timeout all travel as typed metadata; the
 * backend quotes every identifier and binds every value.
 */
export function readTableRows(
  profileId: string,
  table: DatabaseRowTable,
  options: DatabaseRowOptions = {},
  requestId: string = createDatabaseRowRequestId(),
): Promise<DatabaseRowReadResult> {
  return invoke<DatabaseRowReadResult>(databaseRowCommands.read, {
    profileId,
    requestId,
    database: table.database ?? null,
    schema: table.schema ?? null,
    table: table.table,
    visibleColumns: options.visibleColumns ?? null,
    sort: options.sort ?? null,
    filters: options.filters ?? null,
    search: options.search ?? null,
    limit: options.limit ?? null,
    offset: options.offset ?? null,
    includeExactCount: options.includeExactCount ?? null,
    timeoutMs: options.timeoutMs ?? null,
  });
}

/** Cancels one in-flight row read. Cancelling a finished id is idempotent. */
export function cancelTableRows(requestId: string): Promise<DatabaseRowCancellationResult> {
  return invoke<DatabaseRowCancellationResult>(databaseRowCommands.cancel, { requestId });
}
