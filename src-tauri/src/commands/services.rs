use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    services::{ServiceDiscovery, discover_systemd_services as discover},
    ssh::ConnectionManager,
};

#[tauri::command]
pub async fn get_systemd_services(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
) -> Result<ServiceDiscovery, AppError> {
    discover(manager.inner().as_ref(), &id).await
}
