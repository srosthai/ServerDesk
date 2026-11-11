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
        DatabaseConnectionRegistry, DatabaseDiscoveryService, DatabaseSummary, ExplorerNode,
        ExplorerNodePayload, MetadataScope, ObjectCounts, Page, PageRequest, SchemaSummary,
        SequenceSummary, TableSummary, ViewSummary,
    },
};

fn page(limit: Option<u32>, offset: Option<u64>) -> PageRequest {
    PageRequest::clamped(limit, offset)
}

fn scope(database: Option<String>, schema: Option<String>) -> Result<MetadataScope, AppError> {
    MetadataScope::new(database.as_deref(), schema.as_deref()).map_err(AppError::from)
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
