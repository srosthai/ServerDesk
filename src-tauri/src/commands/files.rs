use std::{sync::Arc, time::Instant};

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::{
    core::{
        AppError, ErrorCode,
        files::{
            DownloadRequest, FILE_TRANSFER_EVENT, FileTransferDirection, FileTransferEventSink,
            FileTransferKind, FileTransferManager, FileTransferProgress, FileTransferResult,
            FileTransferStatus, RemoteFileListing, RemoteMutationAction, RemoteMutationResult,
            UploadRequest, browse_remote_files as browse, create_remote_folder as create_folder,
            delete_remote_entry as delete_entry, rename_remote_entry as rename_entry,
        },
        history::{now_unix_seconds, record_operation, summarize_error, summarize_output},
        ssh::ConnectionManager,
    },
    storage::{Database, NewOperationRecord, OperationCategory, OperationResult},
};

pub struct TauriFileTransferEventSink {
    app: AppHandle,
}

impl TauriFileTransferEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl FileTransferEventSink for TauriFileTransferEventSink {
    fn emit(&self, progress: &FileTransferProgress) {
        let _ = self.app.emit(FILE_TRANSFER_EVENT, progress.clone());
    }
}

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadFileInput {
    pub transfer_id: String,
    pub server_id: String,
    pub root: String,
    pub local_path: String,
    pub remote_path: String,
    #[serde(default)]
    pub overwrite: bool,
}

/// Uploads one local file or directory over the managed SSH session.
#[tauri::command]
pub async fn upload_remote_file(
    database: State<'_, Database>,
    transfers: State<'_, Arc<FileTransferManager>>,
    input: UploadFileInput,
) -> Result<FileTransferResult, AppError> {
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let server_id = input.server_id.clone();
    let remote_path = input.remote_path.clone();
    let outcome = transfers
        .upload(UploadRequest {
            transfer_id: input.transfer_id,
            server_id: input.server_id,
            root: input.root,
            local_path: input.local_path,
            remote_path: input.remote_path,
            overwrite: input.overwrite,
        })
        .await;

    match outcome {
        Ok(result) => {
            record_transfer(database.inner(), &result, "sftp-upload");
            Ok(result)
        }
        Err(error) => {
            record_file_failure(
                database.inner(),
                FileFailureContext {
                    server_id: &server_id,
                    operation: "sftp-upload",
                    action: "Upload",
                    path: &remote_path,
                    error: &error,
                    mutating: false,
                    started_at,
                    started,
                },
            );
            Err(error)
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFileInput {
    pub transfer_id: String,
    pub server_id: String,
    pub root: String,
    pub remote_path: String,
    pub local_path: String,
    #[serde(default)]
    pub overwrite: bool,
}

/// Downloads one remote file or directory over the managed SSH session.
#[tauri::command]
pub async fn download_remote_file(
    database: State<'_, Database>,
    transfers: State<'_, Arc<FileTransferManager>>,
    input: DownloadFileInput,
) -> Result<FileTransferResult, AppError> {
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let server_id = input.server_id.clone();
    let remote_path = input.remote_path.clone();
    let outcome = transfers
        .download(DownloadRequest {
            transfer_id: input.transfer_id,
            server_id: input.server_id,
            root: input.root,
            remote_path: input.remote_path,
            local_path: input.local_path,
            overwrite: input.overwrite,
        })
        .await;

    match outcome {
        Ok(result) => {
            record_transfer(database.inner(), &result, "sftp-download");
            Ok(result)
        }
        Err(error) => {
            record_file_failure(
                database.inner(),
                FileFailureContext {
                    server_id: &server_id,
                    operation: "sftp-download",
                    action: "Download",
                    path: &remote_path,
                    error: &error,
                    mutating: false,
                    started_at,
                    started,
                },
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub fn cancel_file_transfer(
    transfers: State<'_, Arc<FileTransferManager>>,
    transfer_id: String,
) -> Result<bool, AppError> {
    transfers.cancel(&transfer_id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderInput {
    pub server_id: String,
    pub root: String,
    pub parent: String,
    pub name: String,
}

#[tauri::command]
pub async fn create_remote_folder(
    database: State<'_, Database>,
    manager: State<'_, Arc<ConnectionManager>>,
    input: CreateFolderInput,
) -> Result<RemoteMutationResult, AppError> {
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = create_folder(
        manager.inner().as_ref(),
        &input.server_id,
        &input.root,
        &input.parent,
        &input.name,
    )
    .await;

    match outcome {
        Ok(result) => {
            record_mutation(database.inner(), &result, started_at, started);
            Ok(result)
        }
        Err(error) => {
            record_file_failure(
                database.inner(),
                FileFailureContext {
                    server_id: &input.server_id,
                    operation: "sftp-create-folder",
                    action: "Create folder",
                    path: &input.parent,
                    error: &error,
                    mutating: true,
                    started_at,
                    started,
                },
            );
            Err(error)
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameEntryInput {
    pub server_id: String,
    pub root: String,
    pub path: String,
    pub new_name: String,
    #[serde(default)]
    pub confirmed: bool,
}

#[tauri::command]
pub async fn rename_remote_entry(
    database: State<'_, Database>,
    manager: State<'_, Arc<ConnectionManager>>,
    input: RenameEntryInput,
) -> Result<RemoteMutationResult, AppError> {
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = rename_entry(
        manager.inner().as_ref(),
        &input.server_id,
        &input.root,
        &input.path,
        &input.new_name,
        input.confirmed,
    )
    .await;

    match outcome {
        Ok(result) => {
            record_mutation(database.inner(), &result, started_at, started);
            Ok(result)
        }
        Err(error) => {
            record_file_failure(
                database.inner(),
                FileFailureContext {
                    server_id: &input.server_id,
                    operation: "sftp-rename",
                    action: "Rename",
                    path: &input.path,
                    error: &error,
                    mutating: true,
                    started_at,
                    started,
                },
            );
            Err(error)
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEntryInput {
    pub server_id: String,
    pub root: String,
    pub path: String,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn delete_remote_entry(
    database: State<'_, Database>,
    manager: State<'_, Arc<ConnectionManager>>,
    input: DeleteEntryInput,
) -> Result<RemoteMutationResult, AppError> {
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = delete_entry(
        manager.inner().as_ref(),
        &input.server_id,
        &input.root,
        &input.path,
        input.confirmed,
        input.recursive,
    )
    .await;

    match outcome {
        Ok(result) => {
            record_mutation(database.inner(), &result, started_at, started);
            Ok(result)
        }
        Err(error) => {
            record_file_failure(
                database.inner(),
                FileFailureContext {
                    server_id: &input.server_id,
                    operation: "sftp-delete",
                    action: "Delete",
                    path: &input.path,
                    error: &error,
                    mutating: true,
                    started_at,
                    started,
                },
            );
            Err(error)
        }
    }
}

fn record_transfer(database: &Database, result: &FileTransferResult, operation: &str) {
    let remote_path = match result.direction {
        FileTransferDirection::Upload => &result.destination_path,
        FileTransferDirection::Download => &result.source_path,
    };
    let direction = match result.direction {
        FileTransferDirection::Upload => "Uploaded",
        FileTransferDirection::Download => "Downloaded",
    };
    let noun = match result.kind {
        FileTransferKind::File => "file",
        FileTransferKind::Directory => "directory",
    };
    let result_kind = transfer_operation_result(result);
    let safe_summary = match result.status {
        FileTransferStatus::Completed => {
            format!("{direction} a {noun} at {remote_path}")
        }
        FileTransferStatus::Cancelled => {
            format!("Cancelled the {noun} {direction} at {remote_path}")
        }
        FileTransferStatus::Failed if result_kind == OperationResult::Unknown => {
            format!(
                "The {noun} {direction} at {remote_path} has an unknown outcome; verify the path \
                 before retrying"
            )
        }
        FileTransferStatus::Failed => {
            format!("The {noun} {direction} at {remote_path} did not complete")
        }
        FileTransferStatus::Running => {
            format!("The {noun} {direction} at {remote_path} is still running")
        }
    };
    let output_summary = result.error.as_ref().and_then(summarize_error).or_else(|| {
        let transferred = format!(
            "{} file(s), {} bytes; {} directories created; status {}",
            result.files_transferred,
            result.bytes_transferred,
            result.directories_created,
            transfer_status_text(result.status)
        );
        summarize_output(&transferred, "")
    });

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(result.server_id.clone()),
            project_id: None,
            category: OperationCategory::Sftp,
            operation: operation.to_string(),
            safe_summary,
            path: Some(remote_path.clone()),
            output_summary,
            result: result_kind,
            exit_code: None,
            started_at: result.started_at / 1_000,
            duration_ms: i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
        },
    );
}

fn record_mutation(
    database: &Database,
    result: &RemoteMutationResult,
    started_at: i64,
    started: Instant,
) {
    let created_target = result.target_path.as_deref().unwrap_or(&result.path);
    let (operation, summary, history_path) = match result.action {
        RemoteMutationAction::CreateFolder => (
            "sftp-create-folder",
            format!("Created folder {created_target}"),
            created_target,
        ),
        RemoteMutationAction::Rename => (
            "sftp-rename",
            format!(
                "Renamed {} to {}",
                result.path,
                result.target_path.as_deref().unwrap_or("the new name")
            ),
            result.path.as_str(),
        ),
        RemoteMutationAction::Delete => (
            "sftp-delete",
            if result.recursive {
                format!("Recursively deleted {}", result.path)
            } else {
                format!("Deleted {}", result.path)
            },
            result.path.as_str(),
        ),
    };
    let output_summary = result
        .verification_detail
        .as_deref()
        .and_then(|detail| summarize_output(detail, ""))
        .or_else(|| {
            Some(if result.verified {
                "The expected remote state was verified after the operation.".to_string()
            } else {
                "The operation completed but the expected state could not be verified.".to_string()
            })
        });
    let result_kind = if result.verified {
        OperationResult::Success
    } else {
        OperationResult::Unknown
    };

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(result.server_id.clone()),
            project_id: None,
            category: OperationCategory::Sftp,
            operation: operation.to_string(),
            safe_summary: summary,
            path: Some(history_path.to_string()),
            output_summary,
            result: result_kind,
            exit_code: None,
            started_at,
            duration_ms: i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
        },
    );
}

struct FileFailureContext<'a> {
    server_id: &'a str,
    operation: &'a str,
    action: &'a str,
    path: &'a str,
    error: &'a AppError,
    mutating: bool,
    started_at: i64,
    started: Instant,
}

fn record_file_failure(database: &Database, failure: FileFailureContext<'_>) {
    let outcome_unknown = failure.mutating && file_mutation_failure_is_unknown(failure.error);
    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(failure.server_id.to_string()),
            project_id: None,
            category: OperationCategory::Sftp,
            operation: failure.operation.to_string(),
            safe_summary: if outcome_unknown {
                format!(
                    "{} at {} has an unknown outcome; re-read the path before retrying",
                    failure.action, failure.path
                )
            } else {
                format!("{} at {} did not complete", failure.action, failure.path)
            },
            path: Some(failure.path.to_string()),
            output_summary: summarize_error(failure.error),
            result: if outcome_unknown {
                OperationResult::Unknown
            } else {
                OperationResult::Failure
            },
            exit_code: None,
            started_at: failure.started_at,
            duration_ms: i64::try_from(failure.started.elapsed().as_millis()).unwrap_or(i64::MAX),
        },
    );
}

fn transfer_operation_result(result: &FileTransferResult) -> OperationResult {
    match result.status {
        FileTransferStatus::Completed => OperationResult::Success,
        FileTransferStatus::Cancelled => OperationResult::Cancelled,
        FileTransferStatus::Running => OperationResult::Unknown,
        FileTransferStatus::Failed
            if result.error.as_ref().is_some_and(|error| {
                matches!(
                    error.code,
                    ErrorCode::Timeout | ErrorCode::Network | ErrorCode::Connection
                )
            }) =>
        {
            OperationResult::Unknown
        }
        FileTransferStatus::Failed => OperationResult::Failure,
    }
}

fn file_mutation_failure_is_unknown(error: &AppError) -> bool {
    matches!(
        error.code,
        ErrorCode::Timeout | ErrorCode::Network | ErrorCode::Connection
    )
}

fn transfer_status_text(status: FileTransferStatus) -> &'static str {
    match status {
        FileTransferStatus::Running => "running",
        FileTransferStatus::Completed => "completed",
        FileTransferStatus::Cancelled => "cancelled",
        FileTransferStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ssh::ConnectionState;

    fn failed_transfer(code: ErrorCode) -> FileTransferResult {
        FileTransferResult {
            transfer_id: "transfer-1".to_string(),
            server_id: "server-1".to_string(),
            direction: FileTransferDirection::Upload,
            kind: FileTransferKind::File,
            status: FileTransferStatus::Failed,
            source_path: "/tmp/source.bin".to_string(),
            destination_path: "/var/www/source.bin".to_string(),
            bytes_transferred: 100,
            total_bytes: Some(200),
            files_transferred: 0,
            files_total: Some(1),
            directories_created: 0,
            skipped_entries: 0,
            warnings: Vec::new(),
            partial_paths: vec!["/var/www/source.bin.serverdesk-test.part".to_string()],
            may_be_incomplete: true,
            overwrite: false,
            started_at: 1,
            finished_at: 2,
            duration_ms: 3,
            error: Some(AppError::new(code, "transfer failed")),
            connection_state: ConnectionState::Error,
        }
    }

    #[test]
    fn interrupted_transfer_history_is_unknown_after_a_transport_loss() {
        assert_eq!(
            transfer_operation_result(&failed_transfer(ErrorCode::Network)),
            OperationResult::Unknown
        );
        assert_eq!(
            transfer_operation_result(&failed_transfer(ErrorCode::Timeout)),
            OperationResult::Unknown
        );
        assert_eq!(
            transfer_operation_result(&failed_transfer(ErrorCode::Permission)),
            OperationResult::Failure
        );
    }

    #[test]
    fn mutating_file_failures_are_only_unknown_for_transport_losses() {
        assert!(file_mutation_failure_is_unknown(&AppError::network("lost")));
        assert!(file_mutation_failure_is_unknown(&AppError::timeout("slow")));
        assert!(!file_mutation_failure_is_unknown(&AppError::permission(
            "denied"
        )));
        assert!(!file_mutation_failure_is_unknown(&AppError::validation(
            "invalid"
        )));
    }
}
