//! Typed database discovery commands.
//!
//! Every command maps one explorer node or metadata list to the shared adapter
//! contract. Requests carry only a profile id and validated database/schema
//! names; responses are bounded pages or typed object counts. No SQL,
//! connection string, host, or credential crosses this boundary.

use tauri::State;

use crate::{
    core::AppError,
    database::{
        ColumnSummary, ConstraintSummary, DEFAULT_PAGE_LIMIT, DatabaseConnectionRegistry,
        DatabaseDiscoveryService, DatabaseSummary, ExplorerNode, ExplorerNodePayload, IndexSummary,
        MAX_PAGE_LIMIT, MetadataScope, ObjectCounts, Page, PageRequest, SchemaSummary,
        SequenceSummary, TableDependencies, TableRef, TableSummary, ViewSummary,
    },
};

fn page(limit: Option<u32>, offset: Option<u64>) -> PageRequest {
    PageRequest::clamped(limit, offset)
}

fn scope(database: Option<String>, schema: Option<String>) -> Result<MetadataScope, AppError> {
    MetadataScope::new(database.as_deref(), schema.as_deref()).map_err(AppError::from)
}

fn table_ref(
    database: Option<String>,
    schema: Option<String>,
    table: String,
) -> Result<TableRef, AppError> {
    TableRef::new(database.as_deref(), schema.as_deref(), &table).map_err(AppError::from)
}

fn dependency_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT)
}

/// Lists databases (MySQL schemas are databases too).
#[tauri::command]
pub async fn list_database_databases(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<DatabaseSummary>, AppError> {
    DatabaseDiscoveryService::new(connections.inner())
        .databases(&profile_id, page(limit, offset))
        .await
}

/// Lists PostgreSQL schemas. MySQL returns its one database namespace per row.
#[tauri::command]
pub async fn list_database_schemas(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<SchemaSummary>, AppError> {
    DatabaseDiscoveryService::new(connections.inner())
        .schemas(&profile_id, page(limit, offset))
        .await
}

/// Lists base tables for one database/schema scope.
#[tauri::command]
pub async fn list_database_tables(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    database: Option<String>,
    schema: Option<String>,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<TableSummary>, AppError> {
    let scope = scope(database, schema)?;
    DatabaseDiscoveryService::new(connections.inner())
        .tables(&profile_id, &scope, page(limit, offset))
        .await
}

/// Lists views, plus materialized views where the server supports them.
#[tauri::command]
pub async fn list_database_views(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    database: Option<String>,
    schema: Option<String>,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<ViewSummary>, AppError> {
    let scope = scope(database, schema)?;
    DatabaseDiscoveryService::new(connections.inner())
        .views(&profile_id, &scope, page(limit, offset))
        .await
}

/// Lists sequences. MySQL returns a typed capability error.
#[tauri::command]
pub async fn list_database_sequences(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    database: Option<String>,
    schema: Option<String>,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<SequenceSummary>, AppError> {
    let scope = scope(database, schema)?;
    DatabaseDiscoveryService::new(connections.inner())
        .sequences(&profile_id, &scope, page(limit, offset))
        .await
}

/// Returns table/view/sequence counts for one explorer node.
#[tauri::command]
pub async fn get_database_object_counts(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    database: Option<String>,
    schema: Option<String>,
) -> Result<ObjectCounts, AppError> {
    let scope = scope(database, schema)?;
    DatabaseDiscoveryService::new(connections.inner())
        .object_counts(&profile_id, &scope)
        .await
}

/// Lists one bounded page of columns for a table.
#[tauri::command]
pub async fn list_database_columns(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    database: Option<String>,
    schema: Option<String>,
    table: String,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<ColumnSummary>, AppError> {
    let table = table_ref(database, schema, table)?;
    DatabaseDiscoveryService::new(connections.inner())
        .columns(&profile_id, &table, page(limit, offset))
        .await
}

/// Lists one bounded page of indexes for a table.
#[tauri::command]
pub async fn list_database_indexes(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    database: Option<String>,
    schema: Option<String>,
    table: String,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<IndexSummary>, AppError> {
    let table = table_ref(database, schema, table)?;
    DatabaseDiscoveryService::new(connections.inner())
        .indexes(&profile_id, &table, page(limit, offset))
        .await
}

/// Lists one bounded page of primary, unique, foreign-key, and check
/// constraints for a table.
#[tauri::command]
pub async fn list_database_constraints(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    database: Option<String>,
    schema: Option<String>,
    table: String,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<ConstraintSummary>, AppError> {
    let table = table_ref(database, schema, table)?;
    DatabaseDiscoveryService::new(connections.inner())
        .constraints(&profile_id, &table, page(limit, offset))
        .await
}

/// Returns the bounded dependency inventory used before destructive changes.
#[tauri::command]
pub async fn get_database_table_dependencies(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    database: Option<String>,
    schema: Option<String>,
    table: String,
    limit: Option<u32>,
) -> Result<TableDependencies, AppError> {
    let table = table_ref(database, schema, table)?;
    DatabaseDiscoveryService::new(connections.inner())
        .table_dependencies(&profile_id, &table, dependency_limit(limit))
        .await
}

/// Lazily loads exactly one explorer node.
#[tauri::command]
pub async fn load_database_explorer_node(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    node: ExplorerNode,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<ExplorerNodePayload, AppError> {
    DatabaseDiscoveryService::new(connections.inner())
        .load_node(&profile_id, &node, page(limit, offset))
        .await
}

/// Refreshes exactly one explorer node without reading any sibling node.
#[tauri::command]
pub async fn refresh_database_explorer_node(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    node: ExplorerNode,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<ExplorerNodePayload, AppError> {
    DatabaseDiscoveryService::new(connections.inner())
        .load_node(&profile_id, &node, page(limit, offset))
        .await
}
