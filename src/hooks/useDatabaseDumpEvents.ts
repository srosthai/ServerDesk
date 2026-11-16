import { useTauriEvent } from "@/hooks/useTauriEvent";
import { DATABASE_DUMP_EVENT } from "@/services/databaseDump";
import { useDatabaseDumpStore } from "@/stores/databaseDumpStore";
import type { DatabaseDumpProgress } from "@/types/contracts";

export function useDatabaseDumpEvents() {
  const applyProgress = useDatabaseDumpStore((state) => state.applyTransferProgress);

  useTauriEvent<DatabaseDumpProgress>(DATABASE_DUMP_EVENT, applyProgress);
}
