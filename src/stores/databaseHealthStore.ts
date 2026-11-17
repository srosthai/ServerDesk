import { create } from "zustand";
import { DEFAULT_HEALTH_TABLE_LIMIT } from "@/features/databases/databaseHealthModel";
import { getCommandErrorMessage } from "@/lib/errors";
import { getDatabaseHealth } from "@/services/databaseHealth";
import type { DatabaseHealthSnapshot } from "@/types/contracts";

export type DatabaseHealthEntry = {
  status: "idle" | "loading" | "ready" | "error";
  snapshot: DatabaseHealthSnapshot | null;
  refreshing: boolean;
  error: string | null;
  /** Set by the store, not by the backend. */
  lastRequestedAt: number | null;
  requestSequence: number;
};

const EMPTY_ENTRY: DatabaseHealthEntry = {
  status: "idle",
  snapshot: null,
  refreshing: false,
  error: null,
  lastRequestedAt: null,
  requestSequence: 0,
};

function entryFor(
  entries: Record<string, DatabaseHealthEntry>,
  profileId: string,
): DatabaseHealthEntry {
  return entries[profileId] ?? EMPTY_ENTRY;
}

function updateEntry(
  entries: Record<string, DatabaseHealthEntry>,
  profileId: string,
  update: Partial<DatabaseHealthEntry>,
): Record<string, DatabaseHealthEntry> {
  return {
    ...entries,
    [profileId]: { ...entryFor(entries, profileId), ...update },
  };
}

type DatabaseHealthStore = {
  entries: Record<string, DatabaseHealthEntry>;
  /**
   * Takes one sample. A manual refresh keeps the previous snapshot visible
   * while the request runs and replaces it only after a successful response.
   */
  fetchHealth: (profileId: string) => Promise<void>;
  refresh: (profileId: string) => Promise<void>;
  reset: (profileId?: string) => void;
};

export const useDatabaseHealthStore = create<DatabaseHealthStore>((set, get) => ({
  entries: {},

  async fetchHealth(profileId) {
    const current = entryFor(get().entries, profileId);
    const sequence = current.requestSequence + 1;
    const hasSnapshot = current.snapshot !== null;

    set((state) => ({
      entries: updateEntry(state.entries, profileId, {
        status: hasSnapshot ? "ready" : "loading",
        refreshing: hasSnapshot,
        error: null,
        lastRequestedAt: Date.now(),
        requestSequence: sequence,
      }),
    }));

    try {
      const snapshot = await getDatabaseHealth(profileId, DEFAULT_HEALTH_TABLE_LIMIT);
      set((state) => {
        if (entryFor(state.entries, profileId).requestSequence !== sequence) {
          return state;
        }
        return {
          entries: updateEntry(state.entries, profileId, {
            status: "ready",
            snapshot,
            refreshing: false,
            error: null,
          }),
        };
      });
    } catch (error) {
      set((state) => {
        if (entryFor(state.entries, profileId).requestSequence !== sequence) {
          return state;
        }
        const entry = entryFor(state.entries, profileId);
        return {
          entries: updateEntry(state.entries, profileId, {
            // Keep the previous sample visible and mark it stale through its
            // sampledAt timestamp. An initial failure has no sample to show.
            status: entry.snapshot ? "ready" : "error",
            refreshing: false,
            error: getCommandErrorMessage(error),
          }),
        };
      });
    }
  },

  async refresh(profileId) {
    await get().fetchHealth(profileId);
  },

  reset(profileId) {
    if (!profileId) {
      set({ entries: {} });
      return;
    }
    set((state) => {
      const entries = { ...state.entries };
      delete entries[profileId];
      return { entries };
    });
  },
}));
