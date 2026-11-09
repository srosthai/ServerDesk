use std::{sync::Arc, time::Instant};

use tauri::State;

use crate::{
    core::{
        AppError, ErrorCode,
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
            let _ = record_operation(
                database.inner(),
                &NewOperationRecord {
                    server_id: Some(id),
                    project_id: None,
                    category: OperationCategory::Service,
                    operation,
                    safe_summary: service_safe_summary(&result),
                    path: None,
                    output_summary: summarize_output(&result.stdout, &result.stderr),
                    result: service_operation_result(&result),
                    exit_code,
                    started_at,
                    duration_ms: i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
                },
            );
            Ok(result)
        }
        Err(error) => {
            let outcome_unknown = action != ServiceAction::Status
                && matches!(
                    error.code,
                    ErrorCode::Timeout | ErrorCode::Network | ErrorCode::Connection
                );
            let _ = record_operation(
                database.inner(),
                &NewOperationRecord {
                    server_id: Some(id),
                    project_id: None,
                    category: OperationCategory::Service,
                    operation,
                    safe_summary: if outcome_unknown {
                        format!(
                            "The service {} action for {} has an unknown outcome; verify the unit \
                             state before retrying",
                            action.as_str(),
                            unit
                        )
                    } else {
                        format!(
                            "The service {} action for {} did not complete",
                            action.as_str(),
                            unit
                        )
                    },
                    path: None,
                    output_summary: summarize_error(&error),
                    result: if outcome_unknown {
                        OperationResult::Unknown
                    } else {
                        OperationResult::Failure
                    },
                    exit_code: None,
                    started_at,
                    duration_ms: elapsed_ms,
                },
            );
            Err(error)
        }
    }
}

/// Maps a service action to a history result.
///
/// A zero exit status is not enough for actions that have an expected state.
/// Only the post-action status query may prove that the unit reached it.
fn service_operation_result(result: &ServiceActionResult) -> OperationResult {
    if !result.succeeded {
        return match result.exit_code {
            Some(0) => OperationResult::Unknown,
            _ => result_from_exit_code(result.exit_code),
        };
    }

    match result.action.expected_state() {
        None => OperationResult::Success,
        Some(_) => match result.state_verified {
            Some(true) => OperationResult::Success,
            Some(false) => OperationResult::Failure,
            None => OperationResult::Unknown,
        },
    }
}

fn service_safe_summary(result: &ServiceActionResult) -> String {
    let exit_text = result
        .exit_code
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "no exit status".to_string());

    if !result.succeeded {
        return format!(
            "The {} action for {} failed ({exit_text})",
            result.action.as_str(),
            result.unit
        );
    }

    match (result.action.expected_state(), result.state_verified) {
        (None, _) => format!("Inspected {} ({exit_text})", result.unit),
        (Some(_), Some(true)) => format!(
            "{} {} and verified the resulting state ({exit_text})",
            service_action_verb(result.action),
            result.unit
        ),
        (Some(_), Some(false)) => format!(
            "{} {} reported success, but the verified state did not match ({exit_text})",
            service_action_verb(result.action),
            result.unit
        ),
        (Some(_), None) => format!(
            "{} {} reported success, but the resulting state could not be verified ({exit_text})",
            service_action_verb(result.action),
            result.unit
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::services::{ServiceState, ServiceStatus};

    fn result(
        action: ServiceAction,
        succeeded: bool,
        exit_code: Option<u32>,
        state_verified: Option<bool>,
    ) -> ServiceActionResult {
        ServiceActionResult {
            server_id: "server-1".to_string(),
            unit: "nginx.service".to_string(),
            action,
            command: "systemctl restart -- nginx.service".to_string(),
            succeeded,
            stdout: String::new(),
            stderr: String::new(),
            exit_code,
            duration_ms: 10,
            permission_denied: false,
            sudo_guidance: None,
            verified_status: state_verified.map(|_| ServiceStatus {
                server_id: "server-1".to_string(),
                unit: "nginx.service".to_string(),
                state: ServiceState::Active,
                active_state: "active".to_string(),
                sub_state: Some("running".to_string()),
                load_state: Some("loaded".to_string()),
                unit_file_state: Some("enabled".to_string()),
                description: None,
                sampled_at: 1,
                duration_ms: 2,
            }),
            state_verified,
            verification_error: None,
            error: None,
            connection_state: crate::core::ssh::ConnectionState::Connected,
        }
    }

    #[test]
    fn history_requires_the_verified_state_for_service_actions() {
        assert_eq!(
            service_operation_result(&result(ServiceAction::Restart, true, Some(0), Some(true))),
            OperationResult::Success
        );
        assert_eq!(
            service_operation_result(&result(ServiceAction::Restart, true, Some(0), Some(false))),
            OperationResult::Failure
        );
        assert_eq!(
            service_operation_result(&result(ServiceAction::Restart, true, Some(0), None)),
            OperationResult::Unknown
        );
        assert_eq!(
            service_operation_result(&result(ServiceAction::Status, false, Some(0), None)),
            OperationResult::Unknown
        );
    }

    #[test]
    fn history_summary_never_claims_an_unverified_state() {
        let summary = service_safe_summary(&result(ServiceAction::Restart, true, Some(0), None));
        assert!(summary.contains("could not be verified"));
        assert!(!summary.contains("verified the resulting state"));

        let summary = service_safe_summary(&result(ServiceAction::Start, false, Some(1), None));
        assert!(summary.contains("failed"));
    }
}
