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
  "idle" | "analyzing" | "ready" | "running" | "succeeded" | "error" | "cancelled" | "blocked";

/**
 * One analysis/execution document. Entries are keyed by the SQL query tab id,
 * not by connection profile id, so two query tabs on the same connection keep
 * independent drafts, results, errors, and cancellation handles.
 */
export type DatabaseQueryEntry = {
  status: DatabaseQueryStatus;
  analysis?: DatabaseSqlAnalysis;
  result?: DatabaseQueryExecutionResult;
  options: DatabaseQueryOptions;
  requestId?: string;
  analysisToken?: string;
  executedSql?: string;
  startedAt?: number;
  error?: CommandError;
  errorMessage?: string;
  updatedAt?: number;
};

type DatabaseQueryStore = {
  entries: Record<string, DatabaseQueryEntry>;
  analyze: (
    queryId: string,
    engine: DatabaseEngine,
    sql: string,
  ) => Promise<DatabaseSqlAnalysis | null>;
  execute: (
    queryId: string,
    profileId: string,
    sql: string,
    options?: DatabaseQueryOptions,
  ) => Promise<DatabaseQueryExecutionResult | null>;
  cancel: (queryId: string) => Promise<boolean>;
  block: (queryId: string, analysis: DatabaseSqlAnalysis, message: string) => void;
  fail: (queryId: string, error: CommandError) => void;
  clear: (queryId: string) => void;
  clearMany: (queryIds: readonly string[]) => void;
  clearError: (queryId: string) => void;
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

  analyze: async (queryId, engine, sql) => {
    if (!isTauriRuntime()) {
      const error = unavailableError();
      set((state) => ({
        entries: {
          ...state.entries,
          [queryId]: {
            ...(state.entries[queryId] ?? idleEntry()),
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
        [queryId]: {
          ...(state.entries[queryId] ?? idleEntry()),
          status: "analyzing",
          analysisToken,
          error: undefined,
          errorMessage: undefined,
        },
      },
    }));

    try {
      const analysis = await analyzeDatabaseSql(engine, sql);
      if (get().entries[queryId]?.analysisToken !== analysisToken) {
        return null;
      }
      set((state) => ({
        entries: {
          ...state.entries,
          [queryId]: {
            ...(state.entries[queryId] ?? idleEntry()),
            status: "ready",
            analysis,
            analysisToken: undefined,
            updatedAt: Date.now(),
          },
        },
      }));
      return analysis;
    } catch (error) {
      if (get().entries[queryId]?.analysisToken !== analysisToken) {
        return null;
      }
      const commandError = commandErrorFor(error) ?? {
        code: "internal",
        message: getCommandErrorMessage(error),
      };
      set((state) => ({
        entries: {
          ...state.entries,
          [queryId]: {
            ...(state.entries[queryId] ?? idleEntry()),
            status: "error",
            analysisToken: undefined,
            error: commandError,
            errorMessage: commandError.message,
          },
        },
      }));
      return null;
    }
  },

  execute: async (queryId, profileId, sql, options = {}) => {
    const existing = get().entries[queryId];
    const mergedOptions: DatabaseQueryOptions = {
      ...(existing?.options ?? {}),
      ...options,
    };

    if (!isTauriRuntime()) {
      const error = unavailableError();
      set((state) => ({
        entries: {
          ...state.entries,
          [queryId]: {
            ...(state.entries[queryId] ?? idleEntry(mergedOptions)),
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
        [queryId]: {
          ...(state.entries[queryId] ?? idleEntry(mergedOptions)),
          status: "running",
          options: mergedOptions,
          requestId,
          executedSql: sql,
          startedAt: Date.now(),
          result: undefined,
          error: undefined,
          errorMessage: undefined,
        },
      },
    }));

    try {
      const result = await executeDatabaseSql(profileId, sql, mergedOptions, requestId);
      if (get().entries[queryId]?.requestId !== requestId) {
        return null;
      }
      set((state) => ({
        entries: {
          ...state.entries,
          [queryId]: {
            ...(state.entries[queryId] ?? idleEntry(mergedOptions)),
            status: "succeeded",
            result,
            options: mergedOptions,
            requestId: undefined,
            startedAt: undefined,
            executedSql: sql,
            error: undefined,
            errorMessage: undefined,
            updatedAt: Date.now(),
          },
        },
      }));
      return result;
    } catch (error) {
      if (get().entries[queryId]?.requestId !== requestId) {
        return null;
      }
      const commandError = commandErrorFor(error) ?? {
        code: "internal",
        message: getCommandErrorMessage(error),
      };
      const cancelled = commandError.code === "cancelled";
      set((state) => ({
        entries: {
          ...state.entries,
          [queryId]: {
            ...(state.entries[queryId] ?? idleEntry(mergedOptions)),
            status: cancelled ? "cancelled" : "error",
            options: mergedOptions,
            requestId: undefined,
            startedAt: undefined,
            executedSql: sql,
            error: cancelled ? undefined : commandError,
            errorMessage: commandError.message,
            updatedAt: Date.now(),
          },
        },
      }));
      return null;
    }
  },

  cancel: async (queryId) => {
    const entry = get().entries[queryId];
    if (!entry?.requestId) {
      return false;
    }

    try {
      const result = await cancelDatabaseSql(entry.requestId);
      if (!result.cancelled) {
        return false;
      }
      if (get().entries[queryId]?.requestId !== entry.requestId) {
        return false;
      }
      set((state) => ({
        entries: {
          ...state.entries,
          [queryId]: {
            ...(state.entries[queryId] ?? idleEntry()),
            status: "cancelled",
            requestId: undefined,
            startedAt: undefined,
            error: undefined,
            errorMessage: "The SQL execution was cancelled.",
            updatedAt: Date.now(),
          },
        },
      }));
      return true;
    } catch {
      return false;
    }
  },

  block: (queryId, analysis, message) => {
    set((state) => ({
      entries: {
        ...state.entries,
        [queryId]: {
          ...(state.entries[queryId] ?? idleEntry()),
          status: "blocked",
          analysis,
          requestId: undefined,
          startedAt: undefined,
          error: { code: "permission", message },
          errorMessage: message,
          updatedAt: Date.now(),
        },
      },
    }));
  },

  fail: (queryId, error) => {
    set((state) => ({
      entries: {
        ...state.entries,
        [queryId]: {
          ...(state.entries[queryId] ?? idleEntry()),
          status: "error",
          requestId: undefined,
          analysisToken: undefined,
          startedAt: undefined,
          error,
          errorMessage: error.message,
        },
      },
    }));
  },

  clear: (queryId) => {
    set((state) => {
      const entries = { ...state.entries };
      delete entries[queryId];
      return { entries };
    });
  },

  clearMany: (queryIds) => {
    if (queryIds.length === 0) {
      return;
    }
    set((state) => {
      const entries = { ...state.entries };
      queryIds.forEach((queryId) => {
        delete entries[queryId];
      });
      return { entries };
    });
  },

  clearError: (queryId) => {
    set((state) => {
      const entry = state.entries[queryId];
      if (
        !entry ||
        (entry.status !== "error" && entry.status !== "cancelled" && entry.status !== "blocked")
      ) {
        return {};
      }
      return {
        entries: {
          ...state.entries,
          [queryId]: {
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
