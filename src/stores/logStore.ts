import { create } from "zustand";
import { getCommandErrorDetails, getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  listLogSources as listLogSourcesService,
  readLogTail as readLogTailService,
} from "@/services/logs";
import type { LogTail, SystemLogSource } from "@/types/contracts";

export type LogEntry = {
  tail: LogTail | null;
  pending: boolean;
  error: string | null;
  errorDetails: string | null;
  lastAttemptAt: number | null;
};

type LogStore = {
  sources: SystemLogSource[];
  sourcesPending: boolean;
  sourcesError: string | null;
  sourcesLoaded: boolean;
  entries: Record<string, LogEntry>;
  fetchSources: () => Promise<boolean>;
  read: (serverId: string, path: string, lines: number) => Promise<boolean>;
  clearEntry: (serverId: string, path: string) => void;
  clearError: (serverId: string, path: string) => void;
  clearServer: (serverId: string) => void;
};

export function logEntryKey(serverId: string, path: string): string {
  return `${serverId}:${path}`;
}

export function emptyLogEntry(): LogEntry {
  return {
    tail: null,
    pending: false,
    error: null,
    errorDetails: null,
    lastAttemptAt: null,
  };
}

export const useLogStore = create<LogStore>((set, get) => {
  let requestCounter = 0;
  const requestIds: Record<string, number> = {};
  let sourcesPromise: Promise<boolean> | null = null;

  function updateEntry(key: string, update: (entry: LogEntry) => LogEntry) {
    set((state) => ({
      entries: {
        ...state.entries,
        [key]: update(state.entries[key] ?? emptyLogEntry()),
      },
    }));
  }

  return {
    sources: [],
    sourcesPending: false,
    sourcesError: null,
    sourcesLoaded: false,
    entries: {},

    fetchSources: async () => {
      if (!isTauriRuntime()) {
        return false;
      }
      if (sourcesPromise) {
        return sourcesPromise;
      }
      if (get().sourcesLoaded && get().sourcesError === null) {
        return true;
      }

      set({ sourcesPending: true, sourcesError: null });
      sourcesPromise = (async () => {
        try {
          const sources = await listLogSourcesService();
          set({
            sources,
            sourcesPending: false,
            sourcesError: null,
            sourcesLoaded: true,
          });
          return true;
        } catch (error) {
          set({
            sourcesPending: false,
            sourcesError: getCommandErrorMessage(error),
            sourcesLoaded: true,
          });
          return false;
        } finally {
          sourcesPromise = null;
        }
      })();

      return sourcesPromise;
    },

    read: async (serverId, path, lines) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const key = logEntryKey(serverId, path);
      const requestId = ++requestCounter;
      requestIds[key] = requestId;
      updateEntry(key, (entry) => ({
        ...entry,
        pending: true,
        error: null,
        errorDetails: null,
      }));

      try {
        const tail = await readLogTailService(serverId, path, lines);
        if (requestIds[key] !== requestId) {
          return false;
        }
        updateEntry(key, (entry) => ({
          ...entry,
          tail,
          pending: false,
          error: null,
          errorDetails: null,
          lastAttemptAt: Date.now(),
        }));
        return true;
      } catch (error) {
        if (requestIds[key] === requestId) {
          updateEntry(key, (entry) => ({
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

    clearEntry: (serverId, path) =>
      set((state) => {
        const entries = { ...state.entries };
        delete entries[logEntryKey(serverId, path)];
        return { entries };
      }),

    clearError: (serverId, path) =>
      updateEntry(logEntryKey(serverId, path), (entry) => ({
        ...entry,
        error: null,
        errorDetails: null,
      })),

    clearServer: (serverId) => {
      const prefix = `${serverId}:`;
      for (const key of Object.keys(requestIds)) {
        if (key.startsWith(prefix)) {
          requestIds[key] = ++requestCounter;
        }
      }
      set((state) => ({
        entries: Object.fromEntries(
          Object.entries(state.entries).filter(([key]) => !key.startsWith(prefix)),
        ),
      }));
    },
  };
});
