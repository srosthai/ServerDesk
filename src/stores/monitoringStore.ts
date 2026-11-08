import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { getMonitoringSnapshot as getMonitoringSnapshotService } from "@/services/monitoring";
import type { MonitoringSnapshot } from "@/types/contracts";

/**
 * One refresh attempt in the current application session.
 *
 * A `null` snapshot is an explicit gap marker: the poll failed or monitoring
 * was paused, so charts must break the line instead of interpolating across
 * the missing interval.
 */
export type MonitoringHistorySample = {
  attemptedAt: number;
  snapshot: MonitoringSnapshot | null;
};

export const MAX_MONITORING_HISTORY_SAMPLES = 180;

export type MonitoringEntry = {
  snapshot: MonitoringSnapshot | null;
  history: MonitoringHistorySample[];
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
    history: [],
    stale: false,
    staleReason: null,
    error: null,
    pending: false,
    lastAttemptAt: null,
  };
}

function appendHistory(
  history: MonitoringHistorySample[],
  sample: MonitoringHistorySample,
): MonitoringHistorySample[] {
  const last = history[history.length - 1];

  // Repeating the same backend sample must not create duplicate points.
  if (sample.snapshot && last?.snapshot && last.snapshot.sampledAt === sample.snapshot.sampledAt) {
    return [...history.slice(0, -1), sample].slice(-MAX_MONITORING_HISTORY_SAMPLES);
  }

  // Consecutive failed attempts describe one gap, so one marker is enough.
  if (sample.snapshot === null && last?.snapshot === null) {
    return history;
  }

  return [...history, sample].slice(-MAX_MONITORING_HISTORY_SAMPLES);
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

        const attemptedAt = Date.now();
        set((state) => {
          const current = state.entries[serverId] ?? emptyEntry();
          return {
            entries: {
              ...state.entries,
              [serverId]: {
                ...current,
                snapshot,
                history: appendHistory(current.history, { attemptedAt, snapshot }),
                stale: false,
                staleReason: null,
                error: null,
                pending: true,
                lastAttemptAt: attemptedAt,
              },
            },
          };
        });
        return true;
      } catch (error) {
        const message = getCommandErrorMessage(error);
        const attemptedAt = Date.now();
        if (requestIds[serverId] === requestId) {
          set((state) => {
            const current = state.entries[serverId] ?? emptyEntry();
            return {
              entries: {
                ...state.entries,
                [serverId]: {
                  ...current,
                  history: appendHistory(current.history, { attemptedAt, snapshot: null }),
                  stale: true,
                  staleReason: message,
                  error: message,
                  pending: true,
                  lastAttemptAt: attemptedAt,
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
            [serverId]: {
              ...current,
              history: current.snapshot
                ? appendHistory(current.history, {
                    attemptedAt: Date.now(),
                    snapshot: null,
                  })
                : current.history,
              stale,
              staleReason: reason,
            },
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
