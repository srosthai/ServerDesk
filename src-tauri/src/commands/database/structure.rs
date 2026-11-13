//! Structure-change preview commands.
//!
//! The command surface is preview-only for now. It validates every identifier
//! and value, asks the adapter for a capability check, and returns generated
//! DDL without executing it. The structure editor UI remains issue #174.

use tauri::State;

use crate::{
    core::AppError,
    database::{
        DatabaseConnectionRegistry, DatabaseStructureService, StructureActionRequest,
        StructureDdlPreview,
    },
};

/// Generates engine-specific DDL for a table, column, index, or constraint
/// change without executing it.
#[tauri::command]
pub async fn preview_database_structure_action(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: StructureActionRequest,
) -> Result<StructureDdlPreview, AppError> {
    DatabaseStructureService::new(connections.inner())
        .preview(&profile_id, request)
        .await
}
