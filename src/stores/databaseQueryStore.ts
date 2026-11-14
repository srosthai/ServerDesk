import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  analyzeDatabaseSql,
  cancelDatabaseSql,
  createDatabaseQueryRequestId,
  executeDatabaseSql,
} from "@/services/databaseQueries";
import type {
  CommandError,
  DatabaseEngine,
  DatabaseQueryExecutionResult,
  DatabaseQueryOptions,
  DatabaseSqlAnalysis,
} from "@/types/contracts";

export type DatabaseQueryStatus =
  "idle" | "analyzing" | "ready" | "running" | "succeeded" | "error" | "cancelled";

export type DatabaseQueryEntry = {
  status: DatabaseQueryStatus;
  analysis?: DatabaseSqlAnalysis;
  result?: DatabaseQueryExecutionResult;
  options: DatabaseQueryOptions;
  requestId?: string;
  analysisToken?: string;
  error?: CommandError;
  errorMessage?: string;
  updatedAt?: number;
};

type DatabaseQueryStore = {
  entries: Record<string, DatabaseQueryEntry>;
  analyze: (
    profileId: string,
    engine: DatabaseEngine,
    sql: string,
  ) => Promise<DatabaseSqlAnalysis | null>;
  execute: (
    profileId: string,
    sql: string,
    options?: DatabaseQueryOptions,
  ) => Promise<DatabaseQueryExecutionResult | null>;
  cancel: (profileId: string) => Promise<boolean>;
  clear: (profileId: string) => void;
  resetProfile: (profileId: string) => void;
  clearError: (profileId: string) => void;
};

function commandErrorFor(error: unknown): CommandError | null {
  return isCommandError(error) ? error : null;
}

function idleEntry(options: DatabaseQueryOptions = {}): DatabaseQueryEntry {
  return { status: "idle", options: { ...options } };
}

function unavailableError(): CommandError {
  return {
    code: "connection",
    message: "SQL execution is available in the desktop application.",
  };
}

export const useDatabaseQueryStore = create<DatabaseQueryStore>((set, get) => ({
  entries: {},

  analyze: async (profileId, engine, sql) => {
    if (!isTauriRuntime()) {
      const error = unavailableError();
      set((state) => ({
        entries: {
          ...state.entries,
          [profileId]: {
            ...(state.entries[profileId] ?? idleEntry()),
            status: "error",
            error,
            errorMessage: error.message,
          },
        },
      }));
      return null;
    }

    const analysisToken = createDatabaseQueryRequestId();
    set((state) => ({
      entries: {
        ...state.entries,
        [profileId]: {
          ...(state.entries[profileId] ?? idleEntry()),
          status: "analyzing",
          analysisToken,
          error: undefined,
          errorMessage: undefined,
        },
      },
    }));

    try {
      const analysis = await analyzeDatabaseSql(engine, sql);
      if (get().entries[profileId]?.analysisToken !== analysisToken) {
        return null;
      }
      set((state) => ({
        entries: {
          ...state.entries,
          [profileId]: {
            ...(state.entries[profileId] ?? idleEntry()),
            status: "ready",
            analysis,
            analysisToken: undefined,
            updatedAt: Date.now(),
          },
        },
      }));
      return analysis;
    } catch (error) {
      if (get().entries[profileId]?.analysisToken !== analysisToken) {
        return null;
      }
      const commandError = commandErrorFor(error);
      const message = commandError?.message ?? getCommandErrorMessage(error);
      set((state) => ({
        entries: {
          ...state.entries,
          [profileId]: {
            ...(state.entries[profileId] ?? idleEntry()),
            status: "error",
            analysisToken: undefined,
            error: commandError ?? undefined,
            errorMessage: message,
          },
        },
      }));
      return null;
    }
  },

  execute: async (profileId, sql, options = {}) => {
    const existing = get().entries[profileId];
    const mergedOptions: DatabaseQueryOptions = {
      ...(existing?.options ?? {}),
      ...options,
    };

    if (!isTauriRuntime()) {
      const error = unavailableError();
      set((state) => ({
        entries: {
          ...state.entries,
          [profileId]: {
            ...(state.entries[profileId] ?? idleEntry(mergedOptions)),
            status: "error",
            options: mergedOptions,
            error,
            errorMessage: error.message,
          },
        },
      }));
      return null;
    }

    const requestId = createDatabaseQueryRequestId();
    set((state) => ({
      entries: {
        ...state.entries,
        [profileId]: {
          ...(state.entries[profileId] ?? idleEntry(mergedOptions)),
          status: "running",
          options: mergedOptions,
          requestId,
          result: undefined,
          error: undefined,
          errorMessage: undefined,
        },
      },
    }));

    try {
      const result = await executeDatabaseSql(profileId, sql, mergedOptions, requestId);
      if (get().entries[profileId]?.requestId !== requestId) {
        return null;
      }
      set((state) => ({
        entries: {
          ...state.entries,
          [profileId]: {
            ...(state.entries[profileId] ?? idleEntry(mergedOptions)),
            status: "succeeded",
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
      if (get().entries[profileId]?.requestId !== requestId) {
        return null;
      }
      const commandError = commandErrorFor(error);
      const message = commandError?.message ?? getCommandErrorMessage(error);
      const cancelled = commandError?.code === "cancelled";
      set((state) => ({
        entries: {
          ...state.entries,
          [profileId]: {
            ...(state.entries[profileId] ?? idleEntry(mergedOptions)),
            status: cancelled ? "cancelled" : "error",
            options: mergedOptions,
            requestId: undefined,
            error: cancelled ? undefined : (commandError ?? undefined),
            errorMessage: message,
            updatedAt: Date.now(),
          },
        },
      }));
      return null;
    }
  },

  cancel: async (profileId) => {
    const entry = get().entries[profileId];
    if (!entry?.requestId) {
      return false;
    }

    const result = await cancelDatabaseSql(entry.requestId);
    if (result.cancelled) {
      set((state) => ({
        entries: {
          ...state.entries,
          [profileId]: {
            ...(state.entries[profileId] ?? idleEntry()),
            status: "cancelled",
            requestId: undefined,
            error: undefined,
            errorMessage: "The SQL execution was cancelled.",
          },
        },
      }));
    }
    return result.cancelled;
  },

  clear: (profileId) => {
    set((state) => {
      const entries = { ...state.entries };
      delete entries[profileId];
      return { entries };
    });
  },

  resetProfile: (profileId) => {
    set((state) => {
      const entries = { ...state.entries };
      delete entries[profileId];
      return { entries };
    });
  },

  clearError: (profileId) => {
    set((state) => {
      const entry = state.entries[profileId];
      if (!entry || (entry.status !== "error" && entry.status !== "cancelled")) {
        return {};
      }
      return {
        entries: {
          ...state.entries,
          [profileId]: {
            ...entry,
            status: entry.result ? "succeeded" : entry.analysis ? "ready" : "idle",
            error: undefined,
            errorMessage: undefined,
          },
        },
      };
    });
  },
}));
