use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    identity::{SystemIdentity, sample_system_identity as sample},
    ssh::ConnectionManager,
};

#[tauri::command]
pub async fn get_system_identity(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
) -> Result<SystemIdentity, AppError> {
    sample(manager.inner().as_ref(), &id).await
}
