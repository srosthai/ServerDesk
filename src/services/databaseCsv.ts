import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseCsvCancellationResult,
  DatabaseCsvExportRequest,
  DatabaseCsvExportResult,
  DatabaseCsvImportOptions,
  DatabaseCsvImportPreview,
  DatabaseCsvImportRequest,
  DatabaseCsvImportResult,
} from "@/types/contracts";

export const DATABASE_CSV_TRANSFER_EVENT = "database-csv-transfer-event";

export const databaseCsvCommands = {
  preview: "preview_database_csv_import",
  export: "export_database_csv",
  import: "import_database_csv",
  cancel: "cancel_database_csv_transfer",
} as const;

/** Creates a bounded ASCII id used to cancel one CSV transfer. */
export function createDatabaseCsvRequestId(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return `csv-${cryptoApi.randomUUID()}`;
  }
  const random = Math.random().toString(36).slice(2, 12);
  return `csv-${Date.now().toString(36)}-${random}`;
}

/** Parses a bounded preview without importing anything. */
export function previewDatabaseCsvImport(
  sourcePath: string,
  options: DatabaseCsvImportOptions,
  limit?: number,
): Promise<DatabaseCsvImportPreview> {
  return invoke<DatabaseCsvImportPreview>(databaseCsvCommands.preview, {
    sourcePath,
    options,
    limit: limit ?? null,
  });
}

/** Exports one table or one read-only query to an absolute local path. */
export function exportDatabaseCsv(
  profileId: string,
  request: DatabaseCsvExportRequest,
  requestId: string = createDatabaseCsvRequestId(),
  timeoutMs?: number,
): Promise<DatabaseCsvExportResult> {
  return invoke<DatabaseCsvExportResult>(databaseCsvCommands.export, {
    profileId,
    requestId,
    request,
    timeoutMs: timeoutMs ?? null,
  });
}

/** Imports a CSV file into one table in a single transaction. */
export function importDatabaseCsv(
  profileId: string,
  request: DatabaseCsvImportRequest,
  requestId: string = createDatabaseCsvRequestId(),
  timeoutMs?: number,
): Promise<DatabaseCsvImportResult> {
  return invoke<DatabaseCsvImportResult>(databaseCsvCommands.import, {
    profileId,
    requestId,
    request,
    timeoutMs: timeoutMs ?? null,
  });
}

/** Cancels one in-flight CSV transfer. Cancelling a finished id is safe. */
export function cancelDatabaseCsvTransfer(
  requestId: string,
): Promise<DatabaseCsvCancellationResult> {
  return invoke<DatabaseCsvCancellationResult>(databaseCsvCommands.cancel, { requestId });
}
