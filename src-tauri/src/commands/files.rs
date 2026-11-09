use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    files::{RemoteFileListing, browse_remote_files as browse},
    ssh::ConnectionManager,
};

/// Browses one directory inside the selected root over the authenticated SFTP
/// subsystem. The root and path are normalized and checked again in Rust.
#[tauri::command]
pub async fn browse_remote_files(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    root: String,
    path: String,
) -> Result<RemoteFileListing, AppError> {
    browse(manager.inner().as_ref(), &id, &root, &path).await
}
