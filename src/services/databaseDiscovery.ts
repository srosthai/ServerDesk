import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseDiscoveryPage,
  DatabaseDiscoveryScope,
  DatabaseExplorerNode,
  DatabaseExplorerNodePayload,
  DatabaseObjectCounts,
  DatabasePage,
  DatabaseSchemaSummary,
  DatabaseSequenceSummary,
  DatabaseSummary,
  DatabaseTableSummary,
  DatabaseViewSummary,
} from "@/types/contracts";

export const databaseDiscoveryCommands = {
  databases: "list_database_databases",
  schemas: "list_database_schemas",
  tables: "list_database_tables",
  views: "list_database_views",
  sequences: "list_database_sequences",
  objectCounts: "get_database_object_counts",
  loadNode: "load_database_explorer_node",
  refreshNode: "refresh_database_explorer_node",
} as const;

function pageArgs(page: DatabaseDiscoveryPage = {}) {
  return {
    limit: page.limit ?? null,
    offset: page.offset ?? null,
  };
}

function scopeArgs(scope: DatabaseDiscoveryScope = {}) {
  return {
    database: scope.database ?? null,
    schema: scope.schema ?? null,
  };
}

export function listDatabases(
  profileId: string,
  page: DatabaseDiscoveryPage = {},
): Promise<DatabasePage<DatabaseSummary>> {
  return invoke<DatabasePage<DatabaseSummary>>(databaseDiscoveryCommands.databases, {
    profileId,
    ...pageArgs(page),
  });
}

export function listSchemas(
  profileId: string,
  page: DatabaseDiscoveryPage = {},
): Promise<DatabasePage<DatabaseSchemaSummary>> {
  return invoke<DatabasePage<DatabaseSchemaSummary>>(databaseDiscoveryCommands.schemas, {
    profileId,
    ...pageArgs(page),
  });
}

export function listTables(
  profileId: string,
  scope: DatabaseDiscoveryScope = {},
  page: DatabaseDiscoveryPage = {},
): Promise<DatabasePage<DatabaseTableSummary>> {
  return invoke<DatabasePage<DatabaseTableSummary>>(databaseDiscoveryCommands.tables, {
    profileId,
    ...scopeArgs(scope),
    ...pageArgs(page),
  });
}

export function listViews(
  profileId: string,
  scope: DatabaseDiscoveryScope = {},
  page: DatabaseDiscoveryPage = {},
): Promise<DatabasePage<DatabaseViewSummary>> {
  return invoke<DatabasePage<DatabaseViewSummary>>(databaseDiscoveryCommands.views, {
    profileId,
    ...scopeArgs(scope),
    ...pageArgs(page),
  });
}

export function listSequences(
  profileId: string,
  scope: DatabaseDiscoveryScope = {},
  page: DatabaseDiscoveryPage = {},
): Promise<DatabasePage<DatabaseSequenceSummary>> {
  return invoke<DatabasePage<DatabaseSequenceSummary>>(databaseDiscoveryCommands.sequences, {
    profileId,
    ...scopeArgs(scope),
    ...pageArgs(page),
  });
}

export function getObjectCounts(
  profileId: string,
  scope: DatabaseDiscoveryScope = {},
): Promise<DatabaseObjectCounts> {
  return invoke<DatabaseObjectCounts>(databaseDiscoveryCommands.objectCounts, {
    profileId,
    ...scopeArgs(scope),
  });
}

/** Loads one explorer node; siblings are not requested or cached by the backend. */
export function loadExplorerNode(
  profileId: string,
  node: DatabaseExplorerNode,
  page: DatabaseDiscoveryPage = {},
): Promise<DatabaseExplorerNodePayload> {
  return invoke<DatabaseExplorerNodePayload>(databaseDiscoveryCommands.loadNode, {
    profileId,
    node,
    ...pageArgs(page),
  });
}

/** Refreshes exactly one explorer node, leaving unrelated nodes untouched. */
export function refreshExplorerNode(
  profileId: string,
  node: DatabaseExplorerNode,
  page: DatabaseDiscoveryPage = {},
): Promise<DatabaseExplorerNodePayload> {
  return invoke<DatabaseExplorerNodePayload>(databaseDiscoveryCommands.refreshNode, {
    profileId,
    node,
    ...pageArgs(page),
  });
}
