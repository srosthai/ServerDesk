import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { DEFAULT_DATABASE_HISTORY_POLICY } from "@/features/databases/databaseHistoryModel";
import {
  clearDatabaseQueryHistory,
  createDatabaseSavedQuery,
  deleteDatabaseSavedQuery,
  getDatabaseQueryHistoryPolicy,
  listDatabaseQueryHistory,
  listDatabaseSavedQueries,
  updateDatabaseSavedQuery,
} from "@/services/databaseHistory";
import type {
  DatabaseQueryHistoryEntry,
  DatabaseQueryHistoryFilter,
  DatabaseQueryHistoryPolicy,
  DatabaseSavedQuery,
  DatabaseSavedQueryInput,
} from "@/types/contracts";

type DatabaseHistoryStatus = "idle" | "loading" | "ready" | "error";

type DatabaseHistoryStore = {
  history: DatabaseQueryHistoryEntry[];
  snippets: DatabaseSavedQuery[];
  policy: DatabaseQueryHistoryPolicy;
  historyStatus: DatabaseHistoryStatus;
  snippetStatus: DatabaseHistoryStatus;
  historyError: string | null;
  snippetError: string | null;
  loadHistory: (filter: DatabaseQueryHistoryFilter) => Promise<void>;
  loadSnippets: () => Promise<void>;
  loadPolicy: () => Promise<void>;
  clearHistory: (confirmed: boolean) => Promise<boolean>;
  createSnippet: (input: DatabaseSavedQueryInput) => Promise<DatabaseSavedQuery | null>;
  updateSnippet: (id: string, input: DatabaseSavedQueryInput) => Promise<DatabaseSavedQuery | null>;
  deleteSnippet: (id: string, confirmed: boolean) => Promise<boolean>;
  reset: () => void;
};

let historyRequestToken = 0;
let snippetRequestToken = 0;

function sortSnippets(snippets: DatabaseSavedQuery[]): DatabaseSavedQuery[] {
  return [...snippets].sort(
    (left, right) =>
      left.engine.localeCompare(right.engine) ||
      left.name.localeCompare(right.name, undefined, { sensitivity: "base" }),
  );
}

function upsertSnippet(
  snippets: DatabaseSavedQuery[],
  snippet: DatabaseSavedQuery,
): DatabaseSavedQuery[] {
  return sortSnippets([...snippets.filter((entry) => entry.id !== snippet.id), snippet]);
}

const initialState = {
  history: [] as DatabaseQueryHistoryEntry[],
  snippets: [] as DatabaseSavedQuery[],
  policy: DEFAULT_DATABASE_HISTORY_POLICY,
  historyStatus: "idle" as DatabaseHistoryStatus,
  snippetStatus: "idle" as DatabaseHistoryStatus,
  historyError: null,
  snippetError: null,
};

export const useDatabaseHistoryStore = create<DatabaseHistoryStore>((set) => ({
  ...initialState,

  loadHistory: async (filter) => {
    const token = ++historyRequestToken;
    if (!isTauriRuntime()) {
      set({ history: [], historyStatus: "ready", historyError: null });
      return;
    }

    set({ historyStatus: "loading", historyError: null });
    try {
      const history = await listDatabaseQueryHistory(filter);
      if (token !== historyRequestToken) {
        return;
      }
      set({ history, historyStatus: "ready", historyError: null });
    } catch (error) {
      if (token !== historyRequestToken) {
        return;
      }
      set({
        historyStatus: "error",
        historyError: getCommandErrorMessage(error),
      });
    }
  },

  loadSnippets: async () => {
    const token = ++snippetRequestToken;
    if (!isTauriRuntime()) {
      set({ snippets: [], snippetStatus: "ready", snippetError: null });
      return;
    }

    set({ snippetStatus: "loading", snippetError: null });
    try {
      const snippets = await listDatabaseSavedQueries();
      if (token !== snippetRequestToken) {
        return;
      }
      set({
        snippets: sortSnippets(snippets),
        snippetStatus: "ready",
        snippetError: null,
      });
    } catch (error) {
      if (token !== snippetRequestToken) {
        return;
      }
      set({
        snippetStatus: "error",
        snippetError: getCommandErrorMessage(error),
      });
    }
  },

  loadPolicy: async () => {
    if (!isTauriRuntime()) {
      set({ policy: DEFAULT_DATABASE_HISTORY_POLICY });
      return;
    }
    try {
      set({ policy: await getDatabaseQueryHistoryPolicy() });
    } catch {
      // The fallback matches the backend bounds and keeps the panel usable.
      set({ policy: DEFAULT_DATABASE_HISTORY_POLICY });
    }
  },

  clearHistory: async (confirmed) => {
    if (!confirmed) {
      return false;
    }
    if (!isTauriRuntime()) {
      set({ history: [], historyStatus: "ready", historyError: null });
      return true;
    }

    try {
      await clearDatabaseQueryHistory(true);
      set({ history: [], historyStatus: "ready", historyError: null });
      return true;
    } catch (error) {
      set({ historyError: getCommandErrorMessage(error) });
      return false;
    }
  },

  createSnippet: async (input) => {
    if (!isTauriRuntime()) {
      set({ snippetError: "Saved snippets are available in the desktop application." });
      return null;
    }

    try {
      const snippet = await createDatabaseSavedQuery(input);
      set((state) => ({
        snippets: upsertSnippet(state.snippets, snippet),
        snippetStatus: "ready",
        snippetError: null,
      }));
      return snippet;
    } catch (error) {
      set({ snippetError: getCommandErrorMessage(error) });
      return null;
    }
  },

  updateSnippet: async (id, input) => {
    if (!isTauriRuntime()) {
      set({ snippetError: "Saved snippets are available in the desktop application." });
      return null;
    }

    try {
      const snippet = await updateDatabaseSavedQuery(id, input);
      set((state) => ({
        snippets: upsertSnippet(state.snippets, snippet),
        snippetStatus: "ready",
        snippetError: null,
      }));
      return snippet;
    } catch (error) {
      set({ snippetError: getCommandErrorMessage(error) });
      return null;
    }
  },

  deleteSnippet: async (id, confirmed) => {
    if (!confirmed) {
      return false;
    }
    if (!isTauriRuntime()) {
      set((state) => ({
        snippets: state.snippets.filter((snippet) => snippet.id !== id),
        snippetError: null,
      }));
      return true;
    }

    try {
      await deleteDatabaseSavedQuery(id, true);
      set((state) => ({
        snippets: state.snippets.filter((snippet) => snippet.id !== id),
        snippetStatus: "ready",
        snippetError: null,
      }));
      return true;
    } catch (error) {
      set({ snippetError: getCommandErrorMessage(error) });
      return false;
    }
  },

  reset: () => {
    historyRequestToken += 1;
    snippetRequestToken += 1;
    set({ ...initialState, history: [], snippets: [] });
  },
}));

/** Exposed for tests that need to assert no stale snippets remain loaded. */
export function resetDatabaseHistoryStoreForTests() {
  useDatabaseHistoryStore.getState().reset();
}
