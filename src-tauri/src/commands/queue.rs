use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    queue::{QueueSupervisorStatus, discover_queue_supervisor as discover},
    ssh::ConnectionManager,
};

#[tauri::command]
pub async fn get_queue_supervisor_status(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
) -> Result<QueueSupervisorStatus, AppError> {
    discover(manager.inner().as_ref(), &id).await
}
