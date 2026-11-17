import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { listDatabaseSessions, runDatabaseSessionAction } from "@/services/databaseSessions";
import type {
  DatabaseSessionActionRequest,
  DatabaseSessionActionResult,
  DatabaseSessionSummary,
} from "@/types/contracts";
import { SESSION_PAGE_LIMIT } from "@/features/databases/databaseSessionsModel";

export type DatabaseSessionEntry = {
  status: "idle" | "loading" | "ready" | "error";
  sessions: DatabaseSessionSummary[];
  limit: number;
  offset: number;
  hasMore: boolean;
  total?: number;
  runningOnly: boolean;
  refreshing: boolean;
  lastRefreshedAt: number | null;
  error: string | null;
  actionStatus: "idle" | "running";
  actionError: string | null;
  lastResult: DatabaseSessionActionResult | null;
  requestSequence: number;
};

const EMPTY_ENTRY: DatabaseSessionEntry = {
  status: "idle",
  sessions: [],
  limit: SESSION_PAGE_LIMIT,
  offset: 0,
  hasMore: false,
  runningOnly: false,
  refreshing: false,
  lastRefreshedAt: null,
  error: null,
  actionStatus: "idle",
  actionError: null,
  lastResult: null,
  requestSequence: 0,
};

function entryFor(
  entries: Record<string, DatabaseSessionEntry>,
  profileId: string,
): DatabaseSessionEntry {
  return entries[profileId] ?? EMPTY_ENTRY;
}

function updateEntry(
  entries: Record<string, DatabaseSessionEntry>,
  profileId: string,
  update: Partial<DatabaseSessionEntry>,
): Record<string, DatabaseSessionEntry> {
  return {
    ...entries,
    [profileId]: { ...entryFor(entries, profileId), ...update },
  };
}

type DatabaseSessionStore = {
  entries: Record<string, DatabaseSessionEntry>;
  fetchSessions: (
    profileId: string,
    options?: { runningOnly?: boolean; reset?: boolean },
  ) => Promise<void>;
  setRunningOnly: (profileId: string, runningOnly: boolean) => Promise<void>;
  refresh: (profileId: string) => Promise<void>;
  executeAction: (
    profileId: string,
    request: DatabaseSessionActionRequest,
  ) => Promise<DatabaseSessionActionResult>;
  reset: (profileId?: string) => void;
};

export const useDatabaseSessionStore = create<DatabaseSessionStore>((set, get) => ({
  entries: {},

  async fetchSessions(profileId, options = {}) {
    const current = entryFor(get().entries, profileId);
    const runningOnly = options.runningOnly ?? current.runningOnly;
    const reset = options.reset ?? false;
    const sequence = current.requestSequence + 1;
    const hasSessions = current.sessions.length > 0;

    set((state) => ({
      entries: updateEntry(state.entries, profileId, {
        status: reset || !hasSessions ? "loading" : "ready",
        refreshing: hasSessions && !reset,
        runningOnly,
        requestSequence: sequence,
        error: null,
        actionError: null,
      }),
    }));

    try {
      const page = await listDatabaseSessions(
        profileId,
        { limit: SESSION_PAGE_LIMIT, offset: 0 },
        runningOnly,
      );
      set((state) => {
        const entry = entryFor(state.entries, profileId);
        if (entry.requestSequence !== sequence) {
          return state;
        }
        return {
          entries: updateEntry(state.entries, profileId, {
            status: "ready",
            sessions: page.items,
            limit: page.limit,
            offset: page.offset,
            hasMore: page.hasMore,
            total: page.total,
            refreshing: false,
            lastRefreshedAt: Date.now(),
            error: null,
          }),
        };
      });
    } catch (error) {
      set((state) => {
        const entry = entryFor(state.entries, profileId);
        if (entry.requestSequence !== sequence) {
          return state;
        }
        return {
          entries: updateEntry(state.entries, profileId, {
            status: "error",
            refreshing: false,
            error: getCommandErrorMessage(error),
          }),
        };
      });
    }
  },

  async setRunningOnly(profileId, runningOnly) {
    await get().fetchSessions(profileId, { runningOnly, reset: true });
  },

  async refresh(profileId) {
    const entry = entryFor(get().entries, profileId);
    await get().fetchSessions(profileId, { runningOnly: entry.runningOnly });
  },

  async executeAction(profileId, request) {
    set((state) => ({
      entries: updateEntry(state.entries, profileId, {
        actionStatus: "running",
        actionError: null,
        lastResult: null,
      }),
    }));
    try {
      const result = await runDatabaseSessionAction(profileId, request);
      set((state) => ({
        entries: updateEntry(state.entries, profileId, {
          actionStatus: "idle",
          lastResult: result,
          actionError: null,
        }),
      }));
      await get().fetchSessions(profileId);
      return result;
    } catch (error) {
      set((state) => ({
        entries: updateEntry(state.entries, profileId, {
          actionStatus: "idle",
          actionError: getCommandErrorMessage(error),
          lastResult: null,
        }),
      }));
      throw error;
    }
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
