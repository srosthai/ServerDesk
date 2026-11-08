use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    logs::{LogTail, SYSTEM_LOG_SOURCES, SystemLogSource, read_log_tail as read_tail},
    ssh::ConnectionManager,
};

/// Returns the fixed allowlist of common system logs for the log viewer.
#[tauri::command]
pub fn list_log_sources() -> Vec<SystemLogSource> {
    SYSTEM_LOG_SOURCES.to_vec()
}

/// Reads one bounded log tail from an approved remote log location.
#[tauri::command]
pub async fn read_log_tail(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    path: String,
    lines: Option<usize>,
) -> Result<LogTail, AppError> {
    read_tail(manager.inner().as_ref(), &id, &path, lines).await
}
