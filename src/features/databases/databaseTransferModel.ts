import type {
  DatabaseCsvExportResult,
  DatabaseCsvImportResult,
  DatabaseCsvTransferProgress,
  DatabaseCsvTransferStatus,
  DatabaseDumpExportResult,
  DatabaseDumpFormat,
  DatabaseDumpPhase,
  DatabaseDumpProgress,
  DatabaseDumpRestorePreview,
  DatabaseDumpRestoreResult,
  DatabaseDumpScope,
  DatabaseDumpStatus,
  DatabaseEngine,
} from "@/types/contracts";
import type { DatabaseWorkspaceTaskStatus } from "@/stores/databaseWorkspaceStore";

export type DatabaseTransferMode =
  "csv-export" | "csv-import" | "sql-export" | "sql-restore" | "backup" | "restore";

export type DatabaseTransferPurpose = "csv" | "dump" | "backup";

export const DATABASE_TRANSFER_LABELS: Record<DatabaseTransferMode, string> = {
  "csv-export": "Export CSV",
  "csv-import": "Import CSV",
  "sql-export": "Export SQL dump",
  "sql-restore": "Restore SQL dump",
  backup: "Create backup",
  restore: "Restore backup",
};

export const DATABASE_DUMP_FORMAT_LABELS: Record<DatabaseDumpFormat, string> = {
  plain_sql: "Plain SQL",
  custom: "PostgreSQL custom archive",
  tar: "PostgreSQL tar archive",
};

export function formatCount(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return "—";
  }
  return value.toLocaleString();
}

export function formatBytes(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return "—";
  }
  if (value < 1024) {
    return `${value.toLocaleString()} B`;
  }
  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1024;
  let unitIndex = 0;
  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex += 1;
  }
  return `${size.toFixed(size >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

export function formatDurationMs(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return "—";
  }
  if (value < 1000) {
    return `${Math.max(0, Math.round(value))} ms`;
  }
  const seconds = value / 1000;
  if (seconds < 60) {
    return `${seconds.toFixed(seconds >= 10 ? 0 : 1)} s`;
  }
  const minutes = Math.floor(seconds / 60);
  const remaining = Math.round(seconds % 60);
  return `${minutes} min ${remaining} s`;
}

export function transferStatusLabel(
  status: DatabaseCsvTransferStatus | DatabaseDumpStatus | "idle" | "error",
): string {
  switch (status) {
    case "running":
      return "Running";
    case "completed":
      return "Completed";
    case "cancelled":
      return "Cancelled";
    case "failed":
      return "Failed";
    case "error":
      return "Failed";
    default:
      return "Not started";
  }
}

export function csvTaskDetail(
  direction: DatabaseCsvTransferProgress["direction"],
  status: DatabaseCsvTransferStatus | "idle" | "error",
  progress: DatabaseCsvTransferProgress | undefined,
  exportResult: DatabaseCsvExportResult | undefined,
  importResult: DatabaseCsvImportResult | undefined,
): string {
  if (status === "running") {
    if (!progress) {
      return "Preparing the transfer";
    }
    return `${csvPhaseLabel(progress.phase)} · ${formatCount(progress.rowsProcessed)} rows · ${formatBytes(progress.bytesProcessed)}`;
  }
  if (direction === "export") {
    if (!exportResult) {
      return "Transfer ended without a result";
    }
    if (exportResult.status === "completed") {
      return `${formatCount(exportResult.rowsExported)} rows · ${formatBytes(exportResult.bytesWritten)} · ${formatDurationMs(exportResult.durationMs)}`;
    }
    return `${transferStatusLabel(exportResult.status)} · destination unchanged · ${formatDurationMs(exportResult.durationMs)}`;
  }
  if (!importResult) {
    return "Transfer ended without a result";
  }
  if (importResult.status === "completed") {
    return `${formatCount(importResult.rowsImported)} imported · ${formatCount(importResult.rowsSkipped)} skipped · ${formatDurationMs(importResult.durationMs)}`;
  }
  return `${transferStatusLabel(importResult.status)} · transaction ${importResult.transaction.replace("_", " ")} · ${formatDurationMs(importResult.durationMs)}`;
}

export function dumpExportTaskDetail(
  status: DatabaseDumpStatus | "idle" | "error",
  progress: DatabaseDumpProgress | undefined,
  result: DatabaseDumpExportResult | undefined,
): string {
  if (status === "running") {
    return progress
      ? `${dumpPhaseLabel(progress.phase)} · ${formatBytes(progress.bytesProcessed)}`
      : "Preparing the dump";
  }
  if (!result) {
    return "Transfer ended without a result";
  }
  if (result.status === "completed") {
    return `${formatBytes(result.bytesWritten)} · verified · ${formatDurationMs(result.durationMs)}`;
  }
  return `${transferStatusLabel(result.status)} · destination unchanged · ${formatDurationMs(result.durationMs)}`;
}

export function dumpRestoreTaskDetail(
  status: DatabaseDumpStatus | "idle" | "error",
  progress: DatabaseDumpProgress | undefined,
  result: DatabaseDumpRestoreResult | undefined,
): string {
  if (status === "running") {
    return progress
      ? `${dumpPhaseLabel(progress.phase)} · ${formatBytes(progress.bytesProcessed)}`
      : "Preparing the restore";
  }
  if (!result) {
    return "Transfer ended without a result";
  }
  if (result.status === "completed") {
    return `${formatBytes(result.bytesRead)} · target ${result.targetDatabase} · ${formatDurationMs(result.durationMs)}`;
  }
  return `${transferStatusLabel(result.status)} · ${result.mayBeIncomplete ? "target may be incomplete" : "no partial state reported"} · ${formatDurationMs(result.durationMs)}`;
}

export function taskStatusForTransfer(
  status: DatabaseCsvTransferStatus | DatabaseDumpStatus | "idle" | "error",
): DatabaseWorkspaceTaskStatus {
  switch (status) {
    case "completed":
      return "succeeded";
    case "cancelled":
      return "cancelled";
    case "failed":
    case "error":
      return "failed";
    default:
      return "running";
  }
}

export function csvPhaseLabel(phase: DatabaseCsvTransferProgress["phase"]): string {
  switch (phase) {
    case "preparing":
      return "Preparing the transfer";
    case "transferring":
      return "Reading and writing rows";
    case "finalizing":
      return "Finalizing and cleaning up";
    default:
      return "Working";
  }
}

export function dumpPhaseLabel(phase: DatabaseDumpPhase): string {
  switch (phase) {
    case "preparing":
      return "Preparing the transfer";
    case "dumping":
      return "Writing the dump on the server";
    case "uploading":
      return "Uploading the dump to the server";
    case "transferring":
      return "Transferring dump bytes";
    case "restoring":
      return "Restoring into the target database";
    case "verifying":
      return "Verifying the output";
    case "cleaning_up":
      return "Cleaning up temporary files";
    case "finalizing":
      return "Finalizing";
    default:
      return "Working";
  }
}

export function csvProgressPercent(
  progress: DatabaseCsvTransferProgress | undefined,
): number | null {
  if (!progress) {
    return null;
  }
  if (progress.totalBytes && progress.totalBytes > 0 && progress.bytesProcessed !== null) {
    return Math.min(100, Math.max(0, (progress.bytesProcessed / progress.totalBytes) * 100));
  }
  if (progress.totalRows && progress.totalRows > 0) {
    return Math.min(100, Math.max(0, (progress.rowsProcessed / progress.totalRows) * 100));
  }
  return null;
}

export function dumpProgressPercent(progress: DatabaseDumpProgress | undefined): number | null {
  if (!progress) {
    return null;
  }
  if (progress.percent !== null && Number.isFinite(progress.percent)) {
    return Math.min(100, Math.max(0, progress.percent));
  }
  if (progress.totalBytes && progress.totalBytes > 0) {
    return Math.min(100, Math.max(0, (progress.bytesProcessed / progress.totalBytes) * 100));
  }
  return null;
}

export function dumpScopeLabel(
  scope: DatabaseDumpScope,
  engine: DatabaseEngine,
  fallbackDatabase: string | null,
): string {
  switch (scope.kind) {
    case "cluster":
      return "Entire PostgreSQL cluster";
    case "schema":
      return `Schema ${scope.schema} in ${scope.database ?? fallbackDatabase ?? "the profile database"}`;
    case "tables": {
      const tableCount = scope.tables.length;
      const tableLabel =
        tableCount === 1 ? `Table ${scope.tables[0]}` : `${tableCount.toLocaleString()} tables`;
      const database = scope.database ?? fallbackDatabase;
      const schema = engine === "mysql" ? null : (scope.schema ?? null);
      return [tableLabel, schema, database].filter(Boolean).join(" in ");
    }
    default:
      return `Database ${scope.database ?? fallbackDatabase ?? "the profile database"}`;
  }
}

export function defaultDumpExtension(format: DatabaseDumpFormat): string {
  switch (format) {
    case "custom":
      return "dump";
    case "tar":
      return "tar";
    default:
      return "sql";
  }
}

export function isCsvRetrySafe(
  result: DatabaseCsvExportResult | DatabaseCsvImportResult | undefined,
): boolean {
  if (!result) {
    return true;
  }
  if (result.status === "completed") {
    return false;
  }
  if ("transaction" in result) {
    return result.transaction === "rolled_back";
  }
  return true;
}

/**
 * A restore that may have changed the target database is never retried from
 * the UI. A failure before execution (no result) or a clean failure that the
 * backend proves did not leave partial state is safe to try again.
 */
export function isRestoreRetrySafe(
  status: DatabaseDumpStatus | "error" | "idle" | undefined,
  mayBeIncomplete: boolean | undefined,
): boolean {
  if (status === "completed") {
    return false;
  }
  if (mayBeIncomplete) {
    return false;
  }
  return status === "failed" || status === "cancelled" || status === "error" || status === "idle";
}

export function restoreConfirmationDetails(
  preview: DatabaseDumpRestorePreview | undefined,
  fallbackServerName: string,
): Array<{ label: string; value: string }> {
  return [
    {
      label: "Server",
      value: preview ? `${preview.serverName} (${preview.serverId})` : fallbackServerName,
    },
    {
      label: "Connection",
      value: preview?.profileName ?? "this connection",
    },
    {
      label: "Target database",
      value: preview?.targetDatabase ?? "not selected",
    },
    {
      label: "Source",
      value: preview ? `${preview.sourceKind} · ${preview.sourcePath}` : "not selected",
    },
    {
      label: "Tool",
      value: preview
        ? `${preview.tool}${preview.toolVersion ? ` ${preview.toolVersion}` : ""}`
        : "—",
    },
    {
      label: "Replaced data",
      value: preview?.replacedData ?? "existing objects and rows that conflict with the dump",
    },
  ];
}

export function toolAvailabilityLabel(tool: {
  available: boolean;
  compatible: boolean;
  version?: string;
}): string {
  if (!tool.available) {
    return "Not installed";
  }
  if (!tool.compatible) {
    return tool.version ? `${tool.version} · incompatible` : "Incompatible";
  }
  return tool.version ?? "Available";
}
