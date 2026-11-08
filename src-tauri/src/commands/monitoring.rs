use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    monitoring::{MonitoringSnapshot, sample_monitoring_snapshot as sample},
    ssh::ConnectionManager,
};

#[tauri::command]
pub async fn get_monitoring_snapshot(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
) -> Result<MonitoringSnapshot, AppError> {
    sample(manager.inner().as_ref(), &id).await
}
