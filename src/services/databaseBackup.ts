import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseBackupCancellationResult,
  DatabaseBackupRequest,
  DatabaseBackupRestorePreview,
  DatabaseBackupRestorePreviewRequest,
  DatabaseBackupRestoreRequest,
  DatabaseBackupResult,
  DatabaseBackupRestoreResult,
  DatabaseDumpToolReport,
} from "@/types/contracts";

/**
 * Backup commands reuse the verified SQL dump transfer pipeline. The event
 * name is shared with that pipeline so one listener can render progress for
 * both dump and backup transfers.
 */
export const DATABASE_BACKUP_EVENT = "database-dump-event";

export const databaseBackupCommands = {
  tools: "get_database_backup_tools",
  previewRestore: "preview_database_restore",
  create: "create_database_backup",
  restore: "restore_database_backup",
  cancel: "cancel_database_backup",
} as const;

/** Creates a bounded ASCII id used to cancel one backup or restore. */
export function createDatabaseBackupRequestId(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return `backup-${cryptoApi.randomUUID()}`;
  }
  const random = Math.random().toString(36).slice(2, 12);
  return `backup-${Date.now().toString(36)}-${random}`;
}

/** Detects the vendor backup and restore tools before a transfer starts. */
export function getDatabaseBackupTools(profileId: string): Promise<DatabaseDumpToolReport> {
  return invoke<DatabaseDumpToolReport>(databaseBackupCommands.tools, { profileId });
}

/**
 * Builds the read-only restore confirmation document. It names the server,
 * target database, tool, and exact data the restore may replace.
 */
export function previewDatabaseRestore(
  profileId: string,
  request: DatabaseBackupRestorePreviewRequest,
): Promise<DatabaseBackupRestorePreview> {
  return invoke<DatabaseBackupRestorePreview>(databaseBackupCommands.previewRestore, {
    profileId,
    request,
  });
}

/** Creates one verified backup at the selected remote or local path. */
export function createDatabaseBackup(
  profileId: string,
  request: DatabaseBackupRequest,
  requestId: string = createDatabaseBackupRequestId(),
  timeoutMs?: number,
): Promise<DatabaseBackupResult> {
  return invoke<DatabaseBackupResult>(databaseBackupCommands.create, {
    profileId,
    requestId,
    request,
    timeoutMs: timeoutMs ?? null,
  });
}

/**
 * Restores one verified backup into the target database. The request must
 * carry an acknowledged confirmation naming the server, target database, and
 * replaced data.
 */
export function restoreDatabaseBackup(
  profileId: string,
  request: DatabaseBackupRestoreRequest,
  requestId: string = createDatabaseBackupRequestId(),
  timeoutMs?: number,
): Promise<DatabaseBackupRestoreResult> {
  return invoke<DatabaseBackupRestoreResult>(databaseBackupCommands.restore, {
    profileId,
    requestId,
    request,
    timeoutMs: timeoutMs ?? null,
  });
}

/** Cancels one in-flight backup or restore. Cancelling a finished id is safe. */
export function cancelDatabaseBackup(requestId: string): Promise<DatabaseBackupCancellationResult> {
  return invoke<DatabaseBackupCancellationResult>(databaseBackupCommands.cancel, { requestId });
}
