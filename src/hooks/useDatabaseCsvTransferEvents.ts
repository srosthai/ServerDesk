import { useTauriEvent } from "@/hooks/useTauriEvent";
import { DATABASE_CSV_TRANSFER_EVENT } from "@/services/databaseCsv";
import { useDatabaseCsvStore } from "@/stores/databaseCsvStore";
import type { DatabaseCsvTransferProgress } from "@/types/contracts";

export function useDatabaseCsvTransferEvents() {
  const applyProgress = useDatabaseCsvStore((state) => state.applyTransferProgress);

  useTauriEvent<DatabaseCsvTransferProgress>(DATABASE_CSV_TRANSFER_EVENT, applyProgress);
}
