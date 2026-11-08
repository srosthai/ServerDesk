use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    remote::{RemoteCommand, RemoteCommandResult, run_remote_command as execute},
    ssh::ConnectionManager,
};

#[tauri::command]
pub async fn run_remote_command(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    command: RemoteCommand,
) -> Result<RemoteCommandResult, AppError> {
    execute(manager.inner().as_ref(), &id, command).await
}
