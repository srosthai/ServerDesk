import { useTauriEvent } from "@/hooks/useTauriEvent";
import { DATABASE_BACKUP_EVENT } from "@/services/databaseBackup";
import { useDatabaseBackupStore } from "@/stores/databaseBackupStore";
import type { DatabaseDumpProgress } from "@/types/contracts";

export function useDatabaseBackupEvents() {
  const applyProgress = useDatabaseBackupStore((state) => state.applyTransferProgress);

  useTauriEvent<DatabaseDumpProgress>(DATABASE_BACKUP_EVENT, applyProgress);
}
