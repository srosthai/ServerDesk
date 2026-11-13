import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  cancelTableRows,
  createDatabaseRowRequestId,
  readTableRows,
} from "@/services/databaseRows";
import type {
  CommandError,
  DatabaseRowOptions,
  DatabaseRowReadResult,
  DatabaseRowTable,
} from "@/types/contracts";

export type DatabaseRowStatus = "idle" | "loading" | "ready" | "error" | "cancelled";

export type DatabaseRowEntry = {
  status: DatabaseRowStatus;
  result?: DatabaseRowReadResult;
  options: DatabaseRowOptions;
  requestId?: string;
  error?: CommandError;
  errorMessage?: string;
  updatedAt?: number;
};

type DatabaseRowStore = {
  entries: Record<string, DatabaseRowEntry>;
  loadRows: (
    profileId: string,
    table: DatabaseRowTable,
    options?: DatabaseRowOptions,
  ) => Promise<DatabaseRowReadResult | null>;
  cancelRows: (profileId: string, table: DatabaseRowTable) => Promise<boolean>;
  clearRows: (profileId: string, table: DatabaseRowTable) => void;
  resetProfile: (profileId: string) => void;
  clearError: (profileId: string, table: DatabaseRowTable) => void;
};

/** Stable key for one table's current row page inside one connection. */
export function databaseRowKey(profileId: string, table: DatabaseRowTable): string {
  return `${profileId}::${table.database ?? ""}::${table.schema ?? ""}::${table.table}`;
}

function commandErrorFor(error: unknown): CommandError | null {
  return isCommandError(error) ? error : null;
}

function idleEntry(options: DatabaseRowOptions = {}): DatabaseRowEntry {
  return { status: "idle", options: { ...options } };
}

export const useDatabaseRowStore = create<DatabaseRowStore>((set, get) => ({
  entries: {},

  loadRows: async (profileId, table, options = {}) => {
    const key = databaseRowKey(profileId, table);
    const existing = get().entries[key];
    const mergedOptions: DatabaseRowOptions = {
      ...(existing?.options ?? {}),
      ...options,
    };

    if (!isTauriRuntime()) {
      const commandError: CommandError = {
        code: "connection",
        message: "Table row browsing is available in the desktop application.",
      };
      set((state) => ({
        entries: {
          ...state.entries,
          [key]: {
            ...idleEntry(mergedOptions),
            status: "error",
            error: commandError,
            errorMessage: commandError.message,
          },
        },
      }));
      return null;
    }

    const requestId = createDatabaseRowRequestId();
    set((state) => ({
      entries: {
        ...state.entries,
        [key]: {
          ...(state.entries[key] ?? idleEntry()),
          status: "loading",
          options: mergedOptions,
          requestId,
          error: undefined,
          errorMessage: undefined,
        },
      },
    }));

    try {
      const result = await readTableRows(profileId, table, mergedOptions, requestId);
      if (get().entries[key]?.requestId !== requestId) {
        // A newer request superseded this one. Never replace the newer page.
        return null;
      }
      set((state) => ({
        entries: {
          ...state.entries,
          [key]: {
            ...(state.entries[key] ?? idleEntry(mergedOptions)),
            status: "ready",
            result,
            options: mergedOptions,
            requestId: undefined,
            error: undefined,
            errorMessage: undefined,
            updatedAt: Date.now(),
          },
        },
      }));
      return result;
    } catch (error) {
      if (get().entries[key]?.requestId !== requestId) {
        return null;
      }
      const commandError = commandErrorFor(error);
      const message = commandError?.message ?? getCommandErrorMessage(error);
      const cancelled = commandError?.code === "cancelled";
      set((state) => ({
        entries: {
          ...state.entries,
          [key]: {
            ...(state.entries[key] ?? idleEntry(mergedOptions)),
            status: cancelled ? "cancelled" : "error",
            options: mergedOptions,
            requestId: undefined,
            error: cancelled ? undefined : (commandError ?? undefined),
            errorMessage: message,
          },
        },
      }));
      return null;
    }
  },

  cancelRows: async (profileId, table) => {
    const key = databaseRowKey(profileId, table);
    const entry = get().entries[key];
    if (!entry?.requestId) {
      return false;
    }

    const result = await cancelTableRows(entry.requestId);
    if (result.cancelled) {
      set((state) => ({
        entries: {
          ...state.entries,
          [key]: {
            ...(state.entries[key] ?? idleEntry()),
            status: "cancelled",
            requestId: undefined,
            error: undefined,
            errorMessage: "The row read was cancelled.",
          },
        },
      }));
    }
    return result.cancelled;
  },

  clearRows: (profileId, table) => {
    const key = databaseRowKey(profileId, table);
    set((state) => {
      const entries = { ...state.entries };
      delete entries[key];
      return { entries };
    });
  },

  resetProfile: (profileId) => {
    const prefix = `${profileId}::`;
    set((state) => ({
      entries: Object.fromEntries(
        Object.entries(state.entries).filter(([key]) => !key.startsWith(prefix)),
      ),
    }));
  },

  clearError: (profileId, table) => {
    const key = databaseRowKey(profileId, table);
    set((state) => {
      const entry = state.entries[key];
      if (!entry || (entry.status !== "error" && entry.status !== "cancelled")) {
        return {};
      }
      return {
        entries: {
          ...state.entries,
          [key]: {
            ...entry,
            status: entry.result ? "ready" : "idle",
            error: undefined,
            errorMessage: undefined,
          },
        },
      };
    });
  },
}));
