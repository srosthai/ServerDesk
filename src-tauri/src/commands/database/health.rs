//! Database health commands.
//!
//! Sampling is explicit and read-only. The command returns only version,
//! latency, and catalog metadata; credentials, SQL, driver diagnostics, and
//! connection strings never cross the Tauri boundary.

use tauri::State;

use crate::{
    core::AppError,
    database::{DatabaseConnectionRegistry, DatabaseHealthService, DatabaseHealthSnapshot},
};

/// Takes one manual health sample for a connected profile.
#[tauri::command]
pub async fn get_database_health(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    limit: Option<u32>,
) -> Result<DatabaseHealthSnapshot, AppError> {
    DatabaseHealthService::new(connections.inner())
        .snapshot(&profile_id, limit)
        .await
}
