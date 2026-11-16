import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseDumpCancellationResult,
  DatabaseDumpExportRequest,
  DatabaseDumpExportResult,
  DatabaseDumpRestoreRequest,
  DatabaseDumpRestoreResult,
  DatabaseDumpToolReport,
} from "@/types/contracts";

export const DATABASE_DUMP_EVENT = "database-dump-event";

export const databaseDumpCommands = {
  tools: "get_database_dump_tools",
  export: "export_database_dump",
  restore: "import_database_dump",
  cancel: "cancel_database_dump",
} as const;

/** Creates a bounded ASCII id used to cancel one dump or restore. */
export function createDatabaseDumpRequestId(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return `dump-${cryptoApi.randomUUID()}`;
  }
  const random = Math.random().toString(36).slice(2, 12);
  return `dump-${Date.now().toString(36)}-${random}`;
}

/** Detects remote dump tools before a transfer starts. */
export function getDatabaseDumpTools(profileId: string): Promise<DatabaseDumpToolReport> {
  return invoke<DatabaseDumpToolReport>(databaseDumpCommands.tools, { profileId });
}

/** Exports one database, schema, table set, or PostgreSQL cluster. */
export function exportDatabaseDump(
  profileId: string,
  request: DatabaseDumpExportRequest,
  requestId: string = createDatabaseDumpRequestId(),
  timeoutMs?: number,
): Promise<DatabaseDumpExportResult> {
  return invoke<DatabaseDumpExportResult>(databaseDumpCommands.export, {
    profileId,
    requestId,
    request,
    timeoutMs: timeoutMs ?? null,
  });
}

/** Restores one SQL dump into a target database. */
export function importDatabaseDump(
  profileId: string,
  request: DatabaseDumpRestoreRequest,
  requestId: string = createDatabaseDumpRequestId(),
  timeoutMs?: number,
): Promise<DatabaseDumpRestoreResult> {
  return invoke<DatabaseDumpRestoreResult>(databaseDumpCommands.restore, {
    profileId,
    requestId,
    request,
    timeoutMs: timeoutMs ?? null,
  });
}

/** Cancels one in-flight dump or restore. Cancelling a finished id is safe. */
export function cancelDatabaseDump(requestId: string): Promise<DatabaseDumpCancellationResult> {
  return invoke<DatabaseDumpCancellationResult>(databaseDumpCommands.cancel, { requestId });
}
