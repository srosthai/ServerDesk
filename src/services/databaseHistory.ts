import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseQueryHistoryEntry,
  DatabaseQueryHistoryFilter,
  DatabaseQueryHistoryPolicy,
  DatabaseSavedQuery,
  DatabaseSavedQueryInput,
} from "@/types/contracts";

export const databaseHistoryCommands = {
  listHistory: "list_database_query_history",
  historyPolicy: "get_database_query_history_policy",
  clearHistory: "clear_database_query_history",
  listSnippets: "list_database_saved_queries",
  createSnippet: "create_database_saved_query",
  updateSnippet: "update_database_saved_query",
  deleteSnippet: "delete_database_saved_query",
} as const;

/** Lists sanitized history rows matching every supplied filter. */
export function listDatabaseQueryHistory(
  filter: DatabaseQueryHistoryFilter = {},
): Promise<DatabaseQueryHistoryEntry[]> {
  return invoke<DatabaseQueryHistoryEntry[]>(databaseHistoryCommands.listHistory, { filter });
}

export function getDatabaseQueryHistoryPolicy(): Promise<DatabaseQueryHistoryPolicy> {
  return invoke<DatabaseQueryHistoryPolicy>(databaseHistoryCommands.historyPolicy);
}

/** Clears history after the UI confirms the destructive action. */
export function clearDatabaseQueryHistory(confirmed: boolean): Promise<void> {
  return invoke<void>(databaseHistoryCommands.clearHistory, { confirmed });
}

export function listDatabaseSavedQueries(): Promise<DatabaseSavedQuery[]> {
  return invoke<DatabaseSavedQuery[]>(databaseHistoryCommands.listSnippets);
}

export function createDatabaseSavedQuery(
  input: DatabaseSavedQueryInput,
): Promise<DatabaseSavedQuery> {
  return invoke<DatabaseSavedQuery>(databaseHistoryCommands.createSnippet, { input });
}

export function updateDatabaseSavedQuery(
  id: string,
  input: DatabaseSavedQueryInput,
): Promise<DatabaseSavedQuery> {
  return invoke<DatabaseSavedQuery>(databaseHistoryCommands.updateSnippet, { id, input });
}

/** Deletes one snippet after the UI confirms the destructive action. */
export function deleteDatabaseSavedQuery(id: string, confirmed: boolean): Promise<void> {
  return invoke<void>(databaseHistoryCommands.deleteSnippet, { id, confirmed });
}
