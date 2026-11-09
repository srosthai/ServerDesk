use std::{sync::Arc, time::Instant};

use tauri::State;

use crate::{
    core::{
        AppError,
        history::{
            now_unix_seconds, record_operation, result_from_exit_code, summarize_error,
            summarize_output,
        },
        services::{
            ServiceAction, ServiceActionResult, ServiceDiscovery, ServiceStatus,
            discover_systemd_services as discover, inspect_service_status as inspect,
            run_service_action as run_action,
        },
        ssh::ConnectionManager,
    },
    storage::{Database, NewOperationRecord, OperationCategory, OperationResult},
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
    database: State<'_, Database>,
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    unit: String,
    action: ServiceAction,
    confirmed: bool,
) -> Result<ServiceActionResult, AppError> {
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let operation = format!("service-{}", action.as_str());
    let outcome = run_action(manager.inner().as_ref(), &id, &unit, action, confirmed).await;
    let elapsed_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);

    match outcome {
        Ok(result) => {
            let exit_code = result.exit_code.and_then(|code| i32::try_from(code).ok());
            let exit_text = result
                .exit_code
                .map(|code| format!("exit code {code}"))
                .unwrap_or_else(|| "no exit status".to_string());
            let _ = record_operation(
                database.inner(),
                &NewOperationRecord {
                    server_id: Some(id),
                    project_id: None,
                    category: OperationCategory::Service,
                    operation,
                    safe_summary: format!(
                        "{} {} ({exit_text})",
                        service_action_verb(result.action),
                        result.unit
                    ),
                    path: None,
                    output_summary: summarize_output(&result.stdout, &result.stderr),
                    result: result_from_exit_code(result.exit_code),
                    exit_code,
                    started_at,
                    duration_ms: i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
                },
            );
            Ok(result)
        }
        Err(error) => {
            let _ = record_operation(
                database.inner(),
                &NewOperationRecord {
                    server_id: Some(id),
                    project_id: None,
                    category: OperationCategory::Service,
                    operation,
                    safe_summary: format!(
                        "The service {} action for {} did not complete",
                        action.as_str(),
                        unit
                    ),
                    path: None,
                    output_summary: summarize_error(&error),
                    result: OperationResult::Failure,
                    exit_code: None,
                    started_at,
                    duration_ms: elapsed_ms,
                },
            );
            Err(error)
        }
    }
}

fn service_action_verb(action: ServiceAction) -> &'static str {
    match action {
        ServiceAction::Status => "Inspected",
        ServiceAction::Start => "Started",
        ServiceAction::Stop => "Stopped",
        ServiceAction::Restart => "Restarted",
    }
}
