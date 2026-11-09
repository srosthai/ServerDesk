use std::{sync::Arc, time::Instant};

use tauri::State;

use crate::{
    core::{
        AppError,
        history::{
            now_unix_seconds, record_operation, result_from_exit_code, summarize_error,
            summarize_output,
        },
        remote::{RemoteCommand, RemoteCommandResult, run_remote_command as execute},
        ssh::ConnectionManager,
    },
    storage::{Database, NewOperationRecord, OperationCategory},
};

#[tauri::command]
pub async fn run_remote_command(
    database: State<'_, Database>,
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    command: RemoteCommand,
) -> Result<RemoteCommandResult, AppError> {
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let operation = format!("remote-{}", command.template());
    let outcome = execute(manager.inner().as_ref(), &id, command).await;
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);

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
                    category: OperationCategory::Command,
                    operation,
                    safe_summary: format!(
                        "Ran the fixed {} command ({exit_text})",
                        command.template()
                    ),
                    path: None,
                    output_summary: summarize_output(&result.stdout, &result.stderr),
                    result: result_from_exit_code(result.exit_code),
                    exit_code,
                    started_at,
                    duration_ms,
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
                    category: OperationCategory::Command,
                    operation,
                    safe_summary: format!(
                        "The fixed {} command did not complete",
                        command.template()
                    ),
                    path: None,
                    output_summary: summarize_error(&error),
                    result: crate::storage::OperationResult::Failure,
                    exit_code: None,
                    started_at,
                    duration_ms,
                },
            );
            Err(error)
        }
    }
}
