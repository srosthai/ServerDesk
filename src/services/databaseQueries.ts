import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseEngine,
  DatabaseQueryCancellationResult,
  DatabaseQueryExecutionResult,
  DatabaseQueryOptions,
  DatabaseSqlAnalysis,
} from "@/types/contracts";

export const databaseQueryCommands = {
  analyze: "analyze_database_sql",
  execute: "execute_database_sql",
  cancel: "cancel_database_sql",
} as const;

/**
 * Creates the id used to cancel one in-flight SQL execution. The backend
 * accepts only bounded ASCII letters, digits, hyphens, underscores, and dots.
 */
export function createDatabaseQueryRequestId(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return cryptoApi.randomUUID();
  }
  const random = Math.random().toString(36).slice(2, 12);
  return `query-${Date.now().toString(36)}-${random}`;
}

/** Detects destructive and writing statements without connecting. */
export function analyzeDatabaseSql(
  engine: DatabaseEngine,
  sql: string,
): Promise<DatabaseSqlAnalysis> {
  return invoke<DatabaseSqlAnalysis>(databaseQueryCommands.analyze, {
    engine,
    sql,
  });
}

/**
 * Executes the current statement or selected SQL with bounded rows, a timeout,
 * and a cancellation id. Destructive SQL requires explicit confirmation.
 */
export function executeDatabaseSql(
  profileId: string,
  sql: string,
  options: DatabaseQueryOptions = {},
  requestId: string = createDatabaseQueryRequestId(),
): Promise<DatabaseQueryExecutionResult> {
  return invoke<DatabaseQueryExecutionResult>(databaseQueryCommands.execute, {
    profileId,
    requestId,
    sql,
    maxRows: options.maxRows ?? null,
    timeoutMs: options.timeoutMs ?? null,
    confirmed: options.confirmed ?? false,
  });
}

/** Cancels one in-flight SQL execution. Cancelling a finished id is safe. */
export function cancelDatabaseSql(requestId: string): Promise<DatabaseQueryCancellationResult> {
  return invoke<DatabaseQueryCancellationResult>(databaseQueryCommands.cancel, {
    requestId,
  });
}
