import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { getMonitoringSnapshot as getMonitoringSnapshotService } from "@/services/monitoring";
import type { MonitoringSnapshot } from "@/types/contracts";

export type MonitoringEntry = {
  snapshot: MonitoringSnapshot | null;
  stale: boolean;
  staleReason: string | null;
  error: string | null;
  pending: boolean;
  lastAttemptAt: number | null;
};

type MonitoringStore = {
  entries: Record<string, MonitoringEntry>;
  refresh: (serverId: string) => Promise<boolean>;
  markStale: (serverId: string, reason: string) => void;
  clearServer: (serverId: string) => void;
  clearError: (serverId: string) => void;
};

function emptyEntry(): MonitoringEntry {
  return {
    snapshot: null,
    stale: false,
    staleReason: null,
    error: null,
    pending: false,
    lastAttemptAt: null,
  };
}

export const useMonitoringStore = create<MonitoringStore>((set) => {
  let requestCounter = 0;
  const requestIds: Record<string, number> = {};

  return {
    entries: {},

    refresh: async (serverId) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++requestCounter;
      requestIds[serverId] = requestId;
      set((state) => {
        const current = state.entries[serverId] ?? emptyEntry();
        return {
          entries: {
            ...state.entries,
            [serverId]: { ...current, pending: true, error: null },
          },
        };
      });

      try {
        const snapshot = await getMonitoringSnapshotService(serverId);
        if (requestIds[serverId] !== requestId) {
          return false;
        }

        set((state) => {
          const current = state.entries[serverId] ?? emptyEntry();
          return {
            entries: {
              ...state.entries,
              [serverId]: {
                ...current,
                snapshot,
                stale: false,
                staleReason: null,
                error: null,
                pending: true,
                lastAttemptAt: Date.now(),
              },
            },
          };
        });
        return true;
      } catch (error) {
        const message = getCommandErrorMessage(error);
        if (requestIds[serverId] === requestId) {
          set((state) => {
            const current = state.entries[serverId] ?? emptyEntry();
            return {
              entries: {
                ...state.entries,
                [serverId]: {
                  ...current,
                  stale: true,
                  staleReason: message,
                  error: message,
                  pending: true,
                  lastAttemptAt: Date.now(),
                },
              },
            };
          });
        }
        return false;
      } finally {
        if (requestIds[serverId] === requestId) {
          set((state) => {
            const current = state.entries[serverId] ?? emptyEntry();
            return {
              entries: {
                ...state.entries,
                [serverId]: { ...current, pending: false },
              },
            };
          });
        }
      }
    },

    markStale: (serverId, reason) =>
      set((state) => {
        const current = state.entries[serverId];
        if (!current) {
          return state;
        }

        const stale = current.snapshot ? true : current.stale;
        if (current.stale === stale && current.staleReason === reason) {
          return state;
        }

        return {
          entries: {
            ...state.entries,
            [serverId]: { ...current, stale, staleReason: reason },
          },
        };
      }),

    clearServer: (serverId) => {
      requestIds[serverId] = ++requestCounter;
      set((state) => {
        const entries = { ...state.entries };
        delete entries[serverId];
        return { entries };
      });
    },

    clearError: (serverId) =>
      set((state) => {
        const current = state.entries[serverId];
        if (!current) {
          return state;
        }

        return {
          entries: {
            ...state.entries,
            [serverId]: { ...current, error: null },
          },
        };
      }),
  };
});
