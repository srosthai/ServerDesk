import { create } from "zustand";
import { getCommandErrorDetails, getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { getQueueSupervisorStatus } from "@/services/queue";
import type { QueueSupervisorStatus } from "@/types/contracts";

export type QueueSupervisorEntry = {
  status: QueueSupervisorStatus | null;
  pending: boolean;
  error: string | null;
  errorDetails: string | null;
  lastAttemptAt: number | null;
};

type QueueStore = {
  entries: Record<string, QueueSupervisorEntry>;
  refresh: (serverId: string) => Promise<boolean>;
  clearServer: (serverId: string) => void;
  clearError: (serverId: string) => void;
};

export function emptyQueueSupervisorEntry(): QueueSupervisorEntry {
  return {
    status: null,
    pending: false,
    error: null,
    errorDetails: null,
    lastAttemptAt: null,
  };
}

export const useQueueStore = create<QueueStore>((set) => {
  let requestCounter = 0;
  const requestIds: Record<string, number> = {};

  function updateEntry(
    serverId: string,
    update: (entry: QueueSupervisorEntry) => QueueSupervisorEntry,
  ) {
    set((state) => ({
      entries: {
        ...state.entries,
        [serverId]: update(state.entries[serverId] ?? emptyQueueSupervisorEntry()),
      },
    }));
  }

  return {
    entries: {},

    refresh: async (serverId) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++requestCounter;
      requestIds[serverId] = requestId;
      updateEntry(serverId, (entry) => ({
        ...entry,
        pending: true,
        error: null,
        errorDetails: null,
      }));

      try {
        const status = await getQueueSupervisorStatus(serverId);
        if (requestIds[serverId] !== requestId) {
          return false;
        }
        updateEntry(serverId, (entry) => ({
          ...entry,
          status,
          pending: false,
          error: null,
          errorDetails: null,
          lastAttemptAt: Date.now(),
        }));
        return true;
      } catch (error) {
        if (requestIds[serverId] === requestId) {
          updateEntry(serverId, (entry) => ({
            ...entry,
            pending: false,
            error: getCommandErrorMessage(error),
            errorDetails: getCommandErrorDetails(error),
            lastAttemptAt: Date.now(),
          }));
        }
        return false;
      }
    },

    clearServer: (serverId) => {
      requestIds[serverId] = ++requestCounter;
      set((state) => {
        const entries = { ...state.entries };
        delete entries[serverId];
        return { entries };
      });
    },

    clearError: (serverId) =>
      updateEntry(serverId, (entry) => ({
        ...entry,
        error: null,
        errorDetails: null,
      })),
  };
});
