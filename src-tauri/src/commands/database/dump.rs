//! SQL dump, backup, and restore commands.
//!
//! The command layer owns profile and server lookup, credential loading,
//! confirmation validation, cancellation ids, progress events, and audit
//! history. The database layer owns tool detection, command construction,
//! execution, verification, and cleanup. History records only the engine,
//! server, scope, format, counts, and outcome. Paths, tool stderr, and
//! credentials are never stored.

use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use tauri::{AppHandle, Emitter, State};

use crate::{
    core::{
        AppError, ErrorCode,
        credential_index::CredentialIndex,
        credentials::CredentialStore,
        database_credentials::DatabaseCredentialService,
        history::{record_operation, summarize_error},
        ssh::ConnectionManager,
    },
    database::{
        DATABASE_DUMP_EVENT, DatabaseConnectionRegistry, DatabaseDumpService,
        DumpCancellationResult, DumpConnection, DumpControl, DumpEventSink, DumpExportRequest,
        DumpExportResult, DumpLocationKind, DumpProgress, DumpRestorePreview,
        DumpRestorePreviewRequest, DumpRestoreRequest, DumpRestoreResult, DumpServerContext,
        DumpToolReport, DumpTransferRegistry, SshDumpRemote, dump_timeout, restore_replaced_data,
        validate_dump_request_id,
    },
    storage::{
        Database, DatabaseConnection, NewOperationRecord, OperationCategory, OperationResult,
        Server,
    },
};

pub struct TauriDumpEventSink {
    app: AppHandle,
}

impl TauriDumpEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl DumpEventSink for TauriDumpEventSink {
    fn emit(&self, progress: &DumpProgress) {
        let _ = self
            .app
            .emit(crate::database::DATABASE_DUMP_EVENT, progress.clone());
    }
}

/// Detects the remote dump and restore tools before a transfer starts.
#[tauri::command]
pub async fn get_database_dump_tools(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    manager: State<'_, Arc<ConnectionManager>>,
    profile_id: String,
) -> Result<DumpToolReport, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let info = connections.info(&profile.id).await?;
    let context = DumpServerContext::from_info(&info);
    let remote = SshDumpRemote::new(Arc::clone(manager.inner()));
    DatabaseDumpService::new(&remote)
        .detect(&profile.server_id, &context)
        .await
}

/// Exports one database, schema, table set, or PostgreSQL cluster.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn export_database_dump(
    app: AppHandle,
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    manager: State<'_, Arc<ConnectionManager>>,
    transfers: State<'_, DumpTransferRegistry>,
    profile_id: String,
    request_id: String,
    request: DumpExportRequest,
    timeout_ms: Option<u64>,
) -> Result<DumpExportResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let server = load_server(database.inner(), &profile.server_id)?;
    let info = connections.info(&profile.id).await?;
    let password = saved_password(
        credentials.inner().as_ref(),
        credential_index.inner(),
        &profile,
    )?;
    let context = DumpServerContext::from_info(&info);
    let connection = DumpConnection::from(&profile);
    let guard = transfers.begin(&request_id).map_err(AppError::from)?;
    let control = DumpControl::new(dump_timeout(timeout_ms), Some(guard.cancellation()));
    let started = Instant::now();
    let started_at = now_unix_seconds();
    let remote = SshDumpRemote::new(Arc::clone(manager.inner()));
    let outcome = DatabaseDumpService::new(&remote)
        .export(
            &profile.server_id,
            &context,
            &connection,
            &password,
            &request_id,
            &request,
            &control,
            Arc::new(TauriDumpEventSink::new(app)),
        )
        .await;
    let duration_ms = outcome
        .as_ref()
        .map(|result| result.duration_ms)
        .unwrap_or_else(|_| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_export_history(
        database.inner(),
        &profile,
        &server,
        &request,
        &outcome,
        started_at,
        i64::try_from(duration_ms).unwrap_or(i64::MAX),
    );
    outcome
}

/// Restores one SQL dump into a target database. The request must carry an
/// explicit confirmation naming the server, target database, and replaced
/// data; without it no connection or tool is touched.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn import_database_dump(
    app: AppHandle,
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    manager: State<'_, Arc<ConnectionManager>>,
    transfers: State<'_, DumpTransferRegistry>,
    profile_id: String,
    request_id: String,
    request: DumpRestoreRequest,
    timeout_ms: Option<u64>,
) -> Result<DumpRestoreResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    if profile.read_only {
        return Err(AppError::permission(
            "The database connection profile is read-only; restoring a dump requires a writable connection profile.",
        )
        .with_details(
            "No dump tool was started and no data was changed. Use a writable database profile to restore data.",
        ));
    }
    let server = load_server(database.inner(), &profile.server_id)?;
    validate_restore_command_confirmation(&server, &request)?;
    let info = connections.info(&profile.id).await?;
    let password = saved_password(
        credentials.inner().as_ref(),
        credential_index.inner(),
        &profile,
    )?;
    let context = DumpServerContext::from_info(&info);
    let connection = DumpConnection::from(&profile);
    let guard = transfers.begin(&request_id).map_err(AppError::from)?;
    let control = DumpControl::new(dump_timeout(timeout_ms), Some(guard.cancellation()));
    let started = Instant::now();
    let started_at = now_unix_seconds();
    let remote = SshDumpRemote::new(Arc::clone(manager.inner()));
    let outcome = DatabaseDumpService::new(&remote)
        .restore(
            &profile.server_id,
            &context,
            &connection,
            &password,
            &request_id,
            &request,
            &control,
            Arc::new(TauriDumpEventSink::new(app)),
        )
        .await;
    let duration_ms = outcome
        .as_ref()
        .map(|result| result.duration_ms)
        .unwrap_or_else(|_| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_restore_history(
        database.inner(),
        &profile,
        &server,
        &request,
        &outcome,
        started_at,
        i64::try_from(duration_ms).unwrap_or(i64::MAX),
    );
    outcome
}

/// Builds the read-only restore confirmation document. It names the server,
/// target database, tool, and exact data a restore may replace. No data is
/// changed and no credential is loaded.
#[tauri::command]
pub async fn preview_database_restore(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    manager: State<'_, Arc<ConnectionManager>>,
    profile_id: String,
    request: DumpRestorePreviewRequest,
) -> Result<DumpRestorePreview, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let server = load_server(database.inner(), &profile.server_id)?;
    let info = connections.info(&profile.id).await?;
    let context = DumpServerContext::from_info(&info);
    let remote = SshDumpRemote::new(Arc::clone(manager.inner()));
    DatabaseDumpService::new(&remote)
        .preview_restore(
            &profile.server_id,
            &server.name,
            &profile.id,
            &profile.name,
            &context,
            &request,
        )
        .await
}

/// Backup workflow alias for [`export_database_dump`].
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_database_backup(
    app: AppHandle,
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    manager: State<'_, Arc<ConnectionManager>>,
    transfers: State<'_, DumpTransferRegistry>,
    profile_id: String,
    request_id: String,
    request: DumpExportRequest,
    timeout_ms: Option<u64>,
) -> Result<DumpExportResult, AppError> {
    export_database_dump(
        app,
        database,
        credentials,
        credential_index,
        connections,
        manager,
        transfers,
        profile_id,
        request_id,
        request,
        timeout_ms,
    )
    .await
}

/// Backup workflow alias for [`import_database_dump`].
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn restore_database_backup(
    app: AppHandle,
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    manager: State<'_, Arc<ConnectionManager>>,
    transfers: State<'_, DumpTransferRegistry>,
    profile_id: String,
    request_id: String,
    request: DumpRestoreRequest,
    timeout_ms: Option<u64>,
) -> Result<DumpRestoreResult, AppError> {
    import_database_dump(
        app,
        database,
        credentials,
        credential_index,
        connections,
        manager,
        transfers,
        profile_id,
        request_id,
        request,
        timeout_ms,
    )
    .await
}

/// Detects backup tools. This is the backup workflow name for the same
/// read-only tool inventory.
#[tauri::command]
pub async fn get_database_backup_tools(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    manager: State<'_, Arc<ConnectionManager>>,
    profile_id: String,
) -> Result<DumpToolReport, AppError> {
    get_database_dump_tools(database, connections, manager, profile_id).await
}

/// Cancels one in-flight backup or restore. Cancelling a finished id is safe.
#[tauri::command]
pub fn cancel_database_backup(
    transfers: State<'_, DumpTransferRegistry>,
    request_id: String,
) -> Result<DumpCancellationResult, AppError> {
    cancel_database_dump(transfers, request_id)
}

/// Cancels one in-flight dump or restore. Cancelling a finished id is safe.
#[tauri::command]
pub fn cancel_database_dump(
    transfers: State<'_, DumpTransferRegistry>,
    request_id: String,
) -> Result<DumpCancellationResult, AppError> {
    if validate_dump_request_id(&request_id).is_err() {
        return Err(AppError::validation("The dump transfer id is not valid."));
    }
    Ok(DumpCancellationResult {
        cancelled: transfers.cancel(&request_id),
        request_id,
    })
}

fn load_profile(database: &Database, profile_id: &str) -> Result<DatabaseConnection, AppError> {
    database
        .get_database_connection(profile_id)?
        .ok_or_else(|| AppError::not_found("Database profile not found."))
}

fn load_server(database: &Database, server_id: &str) -> Result<Server, AppError> {
    database
        .get_server(server_id)?
        .ok_or_else(|| AppError::not_found("Server not found."))
}

/// Verifies that the confirmation echoed by the caller names the stored
/// server, the exact target database, and the exact replaced-data text.
fn validate_restore_command_confirmation(
    server: &Server,
    request: &DumpRestoreRequest,
) -> Result<(), AppError> {
    let confirmation = request.confirmation.as_ref().ok_or_else(|| {
        AppError::validation(
            "The restore confirmation must name the server, target database, and replaced data.",
        )
    })?;
    if !request.confirmed || !confirmation.acknowledged {
        return Err(AppError::validation(
            "Restoring a SQL dump requires explicit confirmation.",
        ));
    }
    if confirmation.server_id != server.id || confirmation.server_name != server.name {
        return Err(AppError::validation(
            "The restore confirmation names a different server.",
        ));
    }
    if confirmation.target_database != request.target_database.as_str() {
        return Err(AppError::validation(
            "The restore confirmation does not name the target database.",
        ));
    }
    let replaced_data = restore_replaced_data(request.target_database.as_str(), request.clean);
    if confirmation.replaced_data != replaced_data {
        return Err(AppError::validation(
            "The restore confirmation does not describe the data that will be replaced.",
        ));
    }
    Ok(())
}

fn saved_password(
    credentials: &dyn CredentialStore,
    credential_index: &CredentialIndex,
    profile: &DatabaseConnection,
) -> Result<crate::database::DatabasePassword, AppError> {
    let reference = profile.credential_ref.as_deref().ok_or_else(|| {
        AppError::credential_missing(
            "No database password is saved for this profile. Save the password again.",
        )
    })?;
    let secret = DatabaseCredentialService::new(credentials, credential_index).read(reference)?;
    Ok(crate::database::DatabasePassword::new(secret))
}

fn engine_label(engine: crate::storage::DatabaseEngine) -> &'static str {
    match engine {
        crate::storage::DatabaseEngine::Mysql => "MySQL",
        crate::storage::DatabaseEngine::Postgresql => "PostgreSQL",
    }
}

fn location_label(kind: DumpLocationKind) -> &'static str {
    match kind {
        DumpLocationKind::Remote => "remote",
        DumpLocationKind::Local => "local",
    }
}

fn record_export_history(
    database: &Database,
    profile: &DatabaseConnection,
    server: &Server,
    request: &DumpExportRequest,
    outcome: &Result<DumpExportResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let engine = engine_label(profile.engine);
    let scope = request.scope.label();
    let format = request.format.label();
    let location = match &request.destination {
        crate::database::DumpDestination::Remote { .. } => DumpLocationKind::Remote,
        crate::database::DumpDestination::Local { .. } => DumpLocationKind::Local,
    };
    let location = location_label(location);

    let (result, safe_summary, output_summary, exit_code) = match outcome {
        Ok(export) if export.status == crate::database::DumpStatus::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the {format} dump of {scope} on {engine} connection {} for server {}; temporary files were removed and the destination was not changed",
                profile.name, server.name
            ),
            export.failure_summary.clone(),
            export.exit_code.and_then(|code| i32::try_from(code).ok()),
        ),
        Ok(export)
            if matches!(
                export.status,
                crate::database::DumpStatus::Failed | crate::database::DumpStatus::Running
            ) =>
        {
            (
                OperationResult::Failure,
                format!(
                    "The {format} dump of {scope} on {engine} connection {} for server {} did not complete; the destination was not changed",
                    profile.name, server.name
                ),
                export.failure_summary.clone(),
                export.exit_code.and_then(|code| i32::try_from(code).ok()),
            )
        }
        Ok(export) => (
            OperationResult::Success,
            format!(
                "Exported a {format} dump of {scope} on {engine} connection {} for server {} to a {location} file",
                profile.name, server.name
            ),
            Some(format!(
                "{} bytes, verified: {}, warnings: {}",
                export.bytes_written,
                export.verified,
                export.warnings.len()
            )),
            export.exit_code.and_then(|code| i32::try_from(code).ok()),
        ),
        Err(error) if error.code == ErrorCode::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the {format} dump of {scope} on {engine} connection {} for server {}; the destination was not changed",
                profile.name, server.name
            ),
            summarize_error(error),
            None,
        ),
        Err(error) => {
            let unknown = matches!(
                error.code,
                ErrorCode::Timeout
                    | ErrorCode::Connection
                    | ErrorCode::Network
                    | ErrorCode::Tunnel
                    | ErrorCode::Internal
            );
            (
                if unknown {
                    OperationResult::Unknown
                } else {
                    OperationResult::Failure
                },
                format!(
                    "The {format} dump of {scope} on {engine} connection {} for server {} did not complete; the destination was not changed",
                    profile.name, server.name
                ),
                summarize_error(error),
                None,
            )
        }
    };

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(profile.server_id.clone()),
            project_id: None,
            category: OperationCategory::Database,
            operation: "database-sql-dump-export".to_string(),
            safe_summary,
            path: None,
            output_summary,
            result,
            exit_code,
            started_at,
            duration_ms,
        },
    );
}

fn record_restore_history(
    database: &Database,
    profile: &DatabaseConnection,
    server: &Server,
    request: &DumpRestoreRequest,
    outcome: &Result<DumpRestoreResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let engine = engine_label(profile.engine);
    let format = request.format.label();
    let target = request.target_database.as_str();

    let (result, safe_summary, output_summary, exit_code) = match outcome {
        Ok(restore) if restore.status == crate::database::DumpStatus::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the {format} restore into {target} on {engine} connection {} for server {}; the target database may contain partially restored objects",
                profile.name, server.name
            ),
            restore.failure_summary.clone(),
            restore.exit_code.and_then(|code| i32::try_from(code).ok()),
        ),
        Ok(restore)
            if matches!(
                restore.status,
                crate::database::DumpStatus::Failed | crate::database::DumpStatus::Running
            ) =>
        {
            (
                OperationResult::Failure,
                format!(
                    "The {format} restore into {target} on {engine} connection {} for server {} did not complete; the target database may contain partially restored objects",
                    profile.name, server.name
                ),
                restore.failure_summary.clone(),
                restore.exit_code.and_then(|code| i32::try_from(code).ok()),
            )
        }
        Ok(restore) => (
            OperationResult::Success,
            format!(
                "Restored a {format} dump into {target} on {engine} connection {} for server {}; the target database may have been replaced",
                profile.name, server.name
            ),
            Some(format!(
                "{} bytes, warnings: {}",
                restore.bytes_read,
                restore.warnings.len()
            )),
            restore.exit_code.and_then(|code| i32::try_from(code).ok()),
        ),
        Err(error) if error.code == ErrorCode::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the {format} restore into {target} on {engine} connection {} for server {}; the target database may contain partially restored objects",
                profile.name, server.name
            ),
            summarize_error(error),
            None,
        ),
        Err(error) => {
            let unknown = matches!(
                error.code,
                ErrorCode::Timeout
                    | ErrorCode::Connection
                    | ErrorCode::Network
                    | ErrorCode::Tunnel
                    | ErrorCode::Internal
            );
            (
                if unknown {
                    OperationResult::Unknown
                } else {
                    OperationResult::Failure
                },
                format!(
                    "The {format} restore into {target} on {engine} connection {} for server {} did not complete; the target database may contain partially restored objects",
                    profile.name, server.name
                ),
                summarize_error(error),
                None,
            )
        }
    };

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(profile.server_id.clone()),
            project_id: None,
            category: OperationCategory::Database,
            operation: "database-sql-dump-restore".to_string(),
            safe_summary,
            path: None,
            output_summary,
            result,
            exit_code,
            started_at,
            duration_ms,
        },
    );
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

// Keep the event constant referenced here so an accidental rename is caught by
// the compiler even when the sink is constructed elsewhere.
const _: &str = DATABASE_DUMP_EVENT;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database::{DumpFormat, DumpScope, DumpStatus, DumpTool},
        storage::{AuthType, DatabaseEngine, NewServer},
    };

    fn profile(engine: DatabaseEngine) -> DatabaseConnection {
        DatabaseConnection {
            id: "profile-1".to_string(),
            server_id: "server-1".to_string(),
            name: "Primary".to_string(),
            engine,
            host: "127.0.0.1".to_string(),
            port: 5432,
            socket: None,
            database_name: Some("app".to_string()),
            username: "app".to_string(),
            credential_ref: None,
            read_only: false,
            ssl_mode: None,
            created_at: 1,
            updated_at: 1,
            last_used_at: None,
        }
    }

    fn export_request() -> DumpExportRequest {
        DumpExportRequest {
            scope: DumpScope::Database { database: None },
            format: DumpFormat::PlainSql,
            tool: None,
            destination: crate::database::DumpDestination::Remote {
                path: "/home/deploy/backup.sql".to_string(),
                overwrite: false,
            },
            no_owner: false,
        }
    }

    fn restore_request() -> DumpRestoreRequest {
        DumpRestoreRequest {
            source: crate::database::DumpSource::Remote {
                path: "/home/deploy/backup.sql".to_string(),
            },
            format: DumpFormat::PlainSql,
            tool: None,
            target_database: crate::database::Identifier::new("app").expect("database"),
            clean: false,
            no_owner: false,
            confirmed: true,
            confirmation: Some(crate::database::DumpRestoreConfirmation {
                server_id: "server-1".to_string(),
                server_name: "Fixture".to_string(),
                target_database: "app".to_string(),
                replaced_data: crate::database::restore_replaced_data("app", false),
                acknowledged: true,
            }),
        }
    }

    fn restore_request_for(server: &Server) -> DumpRestoreRequest {
        let mut request = restore_request();
        if let Some(confirmation) = request.confirmation.as_mut() {
            confirmation.server_id = server.id.clone();
            confirmation.server_name = server.name.clone();
        }
        request
    }

    fn persisted(engine: DatabaseEngine) -> (Database, DatabaseConnection, Server) {
        let database = Database::in_memory().expect("in-memory storage");
        let server = database
            .create_server(&NewServer {
                name: "Fixture".to_string(),
                host: "127.0.0.1".to_string(),
                port: 22,
                username: "deploy".to_string(),
                auth_type: AuthType::Password,
                credential_ref: None,
                group_name: None,
                tags: Vec::new(),
                notes: None,
            })
            .expect("server");
        let mut profile = profile(engine);
        profile.server_id = server.id.clone();
        (database, profile, server)
    }

    fn export_result(engine: DatabaseEngine) -> DumpExportResult {
        DumpExportResult {
            status: DumpStatus::Completed,
            engine,
            scope: DumpScope::Database { database: None },
            format: DumpFormat::PlainSql,
            destination_kind: DumpLocationKind::Remote,
            destination_path: "/home/deploy/secret.sql".to_string(),
            bytes_written: 2048,
            verified: true,
            tool: DumpTool::Mysqldump,
            tool_version: Some("8.0.36".to_string()),
            exit_code: Some(0),
            warnings: Vec::new(),
            failure_summary: None,
            partial_removed: false,
            temp_cleaned: true,
            may_be_incomplete: false,
            duration_ms: 5,
        }
    }

    fn restore_result(engine: DatabaseEngine) -> DumpRestoreResult {
        DumpRestoreResult {
            status: DumpStatus::Completed,
            engine,
            format: DumpFormat::PlainSql,
            source_kind: DumpLocationKind::Remote,
            source_path: "/home/deploy/secret.sql".to_string(),
            target_database: "app".to_string(),
            bytes_read: 2048,
            tool: DumpTool::Psql,
            tool_version: Some("16.2".to_string()),
            exit_code: Some(0),
            warnings: Vec::new(),
            failure_summary: None,
            partial_removed: false,
            temp_cleaned: true,
            may_be_incomplete: false,
            duration_ms: 5,
        }
    }

    #[test]
    fn restore_confirmation_must_match_the_stored_server_database_and_replaced_data() {
        let (_database, _profile, server) = persisted(DatabaseEngine::Postgresql);
        let request = restore_request_for(&server);
        assert!(validate_restore_command_confirmation(&server, &request).is_ok());

        let mut wrong_name = request.clone();
        if let Some(confirmation) = wrong_name.confirmation.as_mut() {
            confirmation.server_name = "Another server".to_string();
        }
        assert!(validate_restore_command_confirmation(&server, &wrong_name).is_err());

        let mut wrong_database = request.clone();
        if let Some(confirmation) = wrong_database.confirmation.as_mut() {
            confirmation.target_database = "other".to_string();
        }
        assert!(validate_restore_command_confirmation(&server, &wrong_database).is_err());

        let mut wrong_replaced = request.clone();
        if let Some(confirmation) = wrong_replaced.confirmation.as_mut() {
            confirmation.replaced_data = "something else".to_string();
        }
        assert!(validate_restore_command_confirmation(&server, &wrong_replaced).is_err());

        let mut unacknowledged = request;
        if let Some(confirmation) = unacknowledged.confirmation.as_mut() {
            confirmation.acknowledged = false;
        }
        assert!(validate_restore_command_confirmation(&server, &unacknowledged).is_err());
    }

    #[test]
    fn export_history_names_scope_and_size_without_paths_or_secrets() {
        let (database, profile, server) = persisted(DatabaseEngine::Mysql);
        record_export_history(
            &database,
            &profile,
            &server,
            &export_request(),
            &Ok(export_result(DatabaseEngine::Mysql)),
            1,
            5,
        );
        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("history entry");
        assert_eq!(row.operation, "database-sql-dump-export");
        assert!(row.safe_summary.contains("Exported a plain SQL dump"));
        assert!(row.safe_summary.contains("Primary"));
        assert!(!row.safe_summary.contains("/home/deploy"));
        assert!(!row.safe_summary.contains("secret"));
        assert!(
            row.output_summary
                .as_deref()
                .unwrap_or_default()
                .contains("2048 bytes")
        );
        assert_eq!(row.exit_code, Some(0));
    }

    #[test]
    fn cancelled_export_history_is_not_success() {
        let (database, profile, server) = persisted(DatabaseEngine::Postgresql);
        let mut result = export_result(DatabaseEngine::Postgresql);
        result.status = DumpStatus::Cancelled;
        result.verified = false;
        record_export_history(
            &database,
            &profile,
            &server,
            &export_request(),
            &Ok(result),
            1,
            5,
        );
        let rows = database.list_operations(10).expect("history");
        assert_eq!(rows[0].result, OperationResult::Cancelled);
        assert!(rows[0].safe_summary.contains("destination was not changed"));
    }

    #[test]
    fn restore_history_warns_about_partial_database_state() {
        let (database, profile, server) = persisted(DatabaseEngine::Postgresql);
        let mut result = restore_result(DatabaseEngine::Postgresql);
        result.status = DumpStatus::Failed;
        result.may_be_incomplete = true;
        result.failure_summary = Some("pg_restore exited with status 1.".to_string());
        record_restore_history(
            &database,
            &profile,
            &server,
            &restore_request(),
            &Ok(result),
            1,
            5,
        );
        let rows = database.list_operations(10).expect("history");
        assert_eq!(rows[0].operation, "database-sql-dump-restore");
        assert_eq!(rows[0].result, OperationResult::Failure);
        assert!(rows[0].safe_summary.contains("partially restored objects"));
        assert!(!rows[0].safe_summary.contains("/home/deploy"));
    }

    #[test]
    fn restore_history_does_not_store_source_path_or_credentials() {
        let (database, profile, server) = persisted(DatabaseEngine::Mysql);
        record_restore_history(
            &database,
            &profile,
            &server,
            &restore_request(),
            &Ok(restore_result(DatabaseEngine::Mysql)),
            1,
            5,
        );
        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("history entry");
        assert!(!row.safe_summary.contains("/home/deploy"));
        assert!(!row.safe_summary.contains("secret"));
        assert!(
            !row.output_summary
                .as_deref()
                .unwrap_or_default()
                .contains("hunter2")
        );
    }
}
