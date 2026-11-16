import { useEffect } from "react";
import {
  csvTaskDetail,
  dumpExportTaskDetail,
  dumpRestoreTaskDetail,
  taskStatusForTransfer,
} from "@/features/databases/databaseTransferModel";
import { updateOperationTask } from "@/features/databases/databaseOperationTasks";
import { useDatabaseBackupStore } from "@/stores/databaseBackupStore";
import type { DatabaseBackupTransferState } from "@/stores/databaseBackupStore";
import { useDatabaseCsvStore } from "@/stores/databaseCsvStore";
import type { DatabaseCsvTransferState } from "@/stores/databaseCsvStore";
import { useDatabaseDumpStore } from "@/stores/databaseDumpStore";
import type { DatabaseDumpTransferState } from "@/stores/databaseDumpStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";

/**
 * Keeps a connection tab's operation record in sync even when the wizard that
 * started the transfer has been closed. The transfer stores remain the source
 * of truth; this hook only updates an existing task for the owning profile.
 */
export function useDatabaseOperationTaskSync() {
  const csvTransfer = useDatabaseCsvStore((state) => state.transfer);
  const dumpTransfer = useDatabaseDumpStore((state) => state.transfer);
  const backupTransfer = useDatabaseBackupStore((state) => state.transfer);

  useEffect(() => {
    function syncCsv(transfer: DatabaseCsvTransferState) {
      if (!transfer.profileId || !transfer.transferId || transfer.status === "idle") {
        return;
      }
      const task = useDatabaseWorkspaceStore
        .getState()
        .tabs[transfer.profileId]?.tasks.find((entry) => entry.id === transfer.transferId);
      if (!task) {
        return;
      }
      const result =
        transfer.direction === "export" ? transfer.exportResult : transfer.importResult;
      updateOperationTask({
        profileId: transfer.profileId,
        id: transfer.transferId,
        status: taskStatusForTransfer(transfer.status),
        detail: csvTaskDetail(
          transfer.direction,
          transfer.status,
          transfer.progress,
          transfer.exportResult,
          transfer.importResult,
        ),
        partial: result?.mayBeIncomplete ?? false,
      });
    }

    function syncDump(transfer: DatabaseDumpTransferState) {
      if (!transfer.profileId || !transfer.transferId || transfer.status === "idle") {
        return;
      }
      const task = useDatabaseWorkspaceStore
        .getState()
        .tabs[transfer.profileId]?.tasks.find((entry) => entry.id === transfer.transferId);
      if (!task) {
        return;
      }
      const isExport = transfer.direction === "export";
      const result = isExport ? transfer.exportResult : transfer.restoreResult;
      updateOperationTask({
        profileId: transfer.profileId,
        id: transfer.transferId,
        status: taskStatusForTransfer(transfer.status),
        detail: isExport
          ? dumpExportTaskDetail(transfer.status, transfer.progress, transfer.exportResult)
          : dumpRestoreTaskDetail(transfer.status, transfer.progress, transfer.restoreResult),
        partial: result?.mayBeIncomplete ?? false,
      });
    }

    function syncBackup(transfer: DatabaseBackupTransferState) {
      if (!transfer.profileId || !transfer.transferId || transfer.status === "idle") {
        return;
      }
      const task = useDatabaseWorkspaceStore
        .getState()
        .tabs[transfer.profileId]?.tasks.find((entry) => entry.id === transfer.transferId);
      if (!task) {
        return;
      }
      const isExport = transfer.direction === "backup";
      const result = isExport ? transfer.backupResult : transfer.restoreResult;
      updateOperationTask({
        profileId: transfer.profileId,
        id: transfer.transferId,
        status: taskStatusForTransfer(transfer.status),
        detail: isExport
          ? dumpExportTaskDetail(transfer.status, transfer.progress, transfer.backupResult)
          : dumpRestoreTaskDetail(transfer.status, transfer.progress, transfer.restoreResult),
        partial: result?.mayBeIncomplete ?? false,
      });
    }

    syncCsv(csvTransfer);
    syncDump(dumpTransfer);
    syncBackup(backupTransfer);
  }, [backupTransfer, csvTransfer, dumpTransfer]);
}
