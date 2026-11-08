use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    services::{
        ServiceAction, ServiceActionResult, ServiceDiscovery, ServiceStatus,
        discover_systemd_services as discover, inspect_service_status as inspect,
        run_service_action as run_action,
    },
    ssh::ConnectionManager,
};

#[tauri::command]
pub async fn get_systemd_services(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
) -> Result<ServiceDiscovery, AppError> {
    discover(manager.inner().as_ref(), &id).await
}

#[tauri::command]
pub async fn get_service_status(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    unit: String,
) -> Result<ServiceStatus, AppError> {
    inspect(manager.inner().as_ref(), &id, &unit).await
}

#[tauri::command]
pub async fn run_service_action(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    unit: String,
    action: ServiceAction,
) -> Result<ServiceActionResult, AppError> {
    run_action(manager.inner().as_ref(), &id, &unit, action).await
}
