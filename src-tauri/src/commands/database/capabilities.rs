use crate::{
    core::AppError,
    database::{CapabilitySnapshot, DatabaseCapabilities},
    storage::DatabaseEngine,
};

/// Returns the actions a selected engine/version supports.
///
/// This is the only database capability information exposed to React, and it
/// contains no SQL text, driver errors, identifiers, or credentials.
#[tauri::command]
pub fn get_database_capabilities(
    engine: DatabaseEngine,
    version: Option<String>,
) -> Result<CapabilitySnapshot, AppError> {
    Ok(DatabaseCapabilities::for_engine(engine, version.as_deref()).snapshot())
}
