//! CSV import and export commands.
//!
//! The command layer owns profile lookup, cancellation ids, progress events,
//! and audit history. The database layer owns parsing, type validation, and
//! transactional execution. History records only the direction, table or query
//! source, row counts, engine, and connection profile name. CSV paths, SQL
//! text, row values, and credentials are never stored.

use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::{
    core::{
        AppError, ErrorCode,
        history::{record_operation, summarize_error},
    },
    database::{
        CsvExportRequest, CsvExportResult, CsvImportOptions, CsvImportPreview, CsvImportRequest,
        CsvImportResult, CsvTransferEventSink, CsvTransferProgress, CsvTransferRegistry,
        CsvTransferStatus, DatabaseConnectionRegistry, Identifier, RowQueryControl, TableRef,
        csv_transfer_timeout,
    },
    storage::{
        Database, DatabaseConnection, NewOperationRecord, OperationCategory, OperationResult,
    },
};

pub struct TauriCsvTransferEventSink {
    app: AppHandle,
}

impl TauriCsvTransferEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl CsvTransferEventSink for TauriCsvTransferEventSink {
    fn emit(&self, progress: &CsvTransferProgress) {
        let _ = self.app.emit(
            crate::database::DATABASE_CSV_TRANSFER_EVENT,
            progress.clone(),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvTransferCancellationResult {
    pub request_id: String,
    pub cancelled: bool,
}

/// Parses a bounded preview of the source file before any import starts.
#[tauri::command]
pub fn preview_database_csv_import(
    source_path: String,
    options: CsvImportOptions,
    limit: Option<u32>,
) -> Result<CsvImportPreview, AppError> {
    crate::database::preview_csv_import(&source_path, &options, limit).map_err(AppError::from)
}

/// Exports one table or one read-only query to a local CSV file. The
/// destination is replaced only after every row is written and flushed.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn export_database_csv(
    app: AppHandle,
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    transfers: State<'_, CsvTransferRegistry>,
    profile_id: String,
    request_id: String,
    request: CsvExportRequest,
    timeout_ms: Option<u64>,
) -> Result<CsvExportResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let guard = transfers.begin(&request_id).map_err(AppError::from)?;
    let control = RowQueryControl {
        timeout: csv_transfer_timeout(timeout_ms),
        cancellation: Some(guard.cancellation()),
    };
    let started = Instant::now();
    let started_at = now_unix_seconds();
    let outcome = crate::database::csv::DatabaseCsvService::new(connections.inner())
        .export(
            &profile_id,
            &request_id,
            &request,
            &control,
            Arc::new(TauriCsvTransferEventSink::new(app)),
        )
        .await;
    let duration_ms = outcome
        .as_ref()
        .map(|result| result.duration_ms)
        .unwrap_or_else(|_| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_export_history(
        database.inner(),
        &profile,
        &request,
        &outcome,
        started_at,
        i64::try_from(duration_ms).unwrap_or(i64::MAX),
    );
    outcome
}

/// Imports one CSV file into a table in a single transaction. Rows that fail
/// local type conversion are reported and skipped; a driver failure rolls the
/// transaction back. Cancellation returns a result with `transaction:
/// rolled_back`, not a partial success.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn import_database_csv(
    app: AppHandle,
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    transfers: State<'_, CsvTransferRegistry>,
    profile_id: String,
    request_id: String,
    request: CsvImportRequest,
    timeout_ms: Option<u64>,
) -> Result<CsvImportResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let guard = transfers.begin(&request_id).map_err(AppError::from)?;
    let control = RowQueryControl {
        timeout: csv_transfer_timeout(timeout_ms),
        cancellation: Some(guard.cancellation()),
    };
    let started = Instant::now();
    let started_at = now_unix_seconds();
    let outcome = crate::database::csv::DatabaseCsvService::new(connections.inner())
        .import(
            &profile_id,
            &request_id,
            &request,
            &control,
            Arc::new(TauriCsvTransferEventSink::new(app)),
        )
        .await;
    let duration_ms = outcome
        .as_ref()
        .map(|result| result.duration_ms)
        .unwrap_or_else(|_| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_import_history(
        database.inner(),
        &profile,
        &request,
        &outcome,
        started_at,
        i64::try_from(duration_ms).unwrap_or(i64::MAX),
    );
    outcome
}

/// Cancels one in-flight CSV transfer. Cancelling a finished id is idempotent.
#[tauri::command]
pub fn cancel_database_csv_transfer(
    transfers: State<'_, CsvTransferRegistry>,
    request_id: String,
) -> Result<CsvTransferCancellationResult, AppError> {
    if crate::database::validate_csv_request_id(&request_id).is_err() {
        return Err(AppError::validation("The CSV transfer id is not valid."));
    }
    Ok(CsvTransferCancellationResult {
        cancelled: transfers.cancel(&request_id),
        request_id,
    })
}

fn load_profile(database: &Database, profile_id: &str) -> Result<DatabaseConnection, AppError> {
    database
        .get_database_connection(profile_id)?
        .ok_or_else(|| AppError::not_found("Database profile not found."))
}

fn engine_label(engine: crate::storage::DatabaseEngine) -> &'static str {
    match engine {
        crate::storage::DatabaseEngine::Mysql => "MySQL",
        crate::storage::DatabaseEngine::Postgresql => "PostgreSQL",
    }
}

fn table_label(table: &TableRef) -> String {
    let mut label = String::new();
    if let Some(database) = table.database.as_ref().map(Identifier::as_str) {
        label.push_str(database);
        label.push('.');
    }
    if let Some(schema) = table.schema.as_ref().map(Identifier::as_str)
        && table.database.as_ref().map(Identifier::as_str) != Some(schema)
    {
        label.push_str(schema);
        label.push('.');
    }
    label.push_str(table.name.as_str());
    label
}

fn export_source_label(request: &CsvExportRequest) -> String {
    match &request.source {
        crate::database::CsvExportSource::Table { table } => table_label(table),
        crate::database::CsvExportSource::Query { .. } => "a read-only query".to_string(),
    }
}

fn record_export_history(
    database: &Database,
    profile: &DatabaseConnection,
    request: &CsvExportRequest,
    outcome: &Result<CsvExportResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let engine = engine_label(profile.engine);
    let source = export_source_label(request);
    let (result, safe_summary, output_summary) = match outcome {
        Ok(export) if export.status == CsvTransferStatus::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the CSV export from {source} on {engine} connection {}; no destination \
                 file was created",
                profile.name
            ),
            Some(
                "The partial CSV file was removed and the destination was not changed.".to_string(),
            ),
        ),
        Ok(export) => (
            OperationResult::Success,
            format!(
                "Exported {} rows from {source} on {engine} connection {} to a local CSV file",
                export.rows_exported, profile.name
            ),
            Some(format!(
                "{} rows, {} bytes, values truncated: {}",
                export.rows_exported, export.bytes_written, export.values_truncated
            )),
        ),
        Err(error) if error.code == ErrorCode::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the CSV export from {source} on {engine} connection {}; no destination \
                 file was created",
                profile.name
            ),
            summarize_error(error),
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
                    "The CSV export from {source} on {engine} connection {} did not complete; no \
                     destination file was created",
                    profile.name
                ),
                summarize_error(error),
            )
        }
    };

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(profile.server_id.clone()),
            project_id: None,
            category: OperationCategory::Database,
            operation: "database-csv-export".to_string(),
            safe_summary,
            path: None,
            output_summary,
            result,
            exit_code: None,
            started_at,
            duration_ms,
        },
    );
}

fn record_import_history(
    database: &Database,
    profile: &DatabaseConnection,
    request: &CsvImportRequest,
    outcome: &Result<CsvImportResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let engine = engine_label(profile.engine);
    let table = table_label(&request.table);
    let (result, safe_summary, output_summary) = match outcome {
        Ok(import) if import.status == CsvTransferStatus::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the CSV import into {table} on {engine} connection {}; the transaction \
                 was rolled back and no rows were imported",
                profile.name
            ),
            Some(format!(
                "{} rows read, 0 rows imported, {} rows skipped",
                import.rows_read, import.rows_skipped
            )),
        ),
        Ok(import) => (
            OperationResult::Success,
            format!(
                "Imported {} rows into {table} on {engine} connection {}; {} rows skipped; \
                 transaction committed",
                import.rows_imported, profile.name, import.rows_skipped
            ),
            Some(format!(
                "{} rows read, {} imported, {} skipped, warnings truncated: {}",
                import.rows_read,
                import.rows_imported,
                import.rows_skipped,
                import.warnings_truncated
            )),
        ),
        Err(error) if error.code == ErrorCode::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the CSV import into {table} on {engine} connection {}; the transaction \
                 was rolled back and no rows were imported",
                profile.name
            ),
            summarize_error(error),
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
                    "The CSV import into {table} on {engine} connection {} did not complete; no \
                     rows were imported",
                    profile.name
                ),
                summarize_error(error),
            )
        }
    };

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(profile.server_id.clone()),
            project_id: None,
            category: OperationCategory::Database,
            operation: "database-csv-import".to_string(),
            safe_summary,
            path: None,
            output_summary,
            result,
            exit_code: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database::{CsvColumnMapping, CsvDelimiter, CsvEncoding, CsvImportOptions, CsvNullMode},
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

    fn table() -> TableRef {
        TableRef::new(Some("app"), Some("public"), "users").expect("table")
    }

    fn export_request() -> CsvExportRequest {
        CsvExportRequest {
            source: crate::database::CsvExportSource::Table { table: table() },
            destination_path: "/tmp/users.csv".to_string(),
            delimiter: CsvDelimiter::Comma,
            encoding: CsvEncoding::Utf8,
            include_header: true,
            overwrite: false,
        }
    }

    fn import_request() -> CsvImportRequest {
        CsvImportRequest {
            table: table(),
            source_path: "/tmp/users.csv".to_string(),
            options: CsvImportOptions {
                delimiter: CsvDelimiter::Comma,
                encoding: CsvEncoding::Utf8,
                has_header: true,
                null_mode: CsvNullMode::UnquotedEmpty,
            },
            mappings: vec![CsvColumnMapping {
                source_index: 0,
                target_column: Identifier::new("name").expect("column"),
            }],
        }
    }

    fn persisted(engine: DatabaseEngine) -> (Database, DatabaseConnection) {
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
        profile.server_id = server.id;
        (database, profile)
    }

    #[test]
    fn export_history_names_source_and_count_without_path_or_sql() {
        let (database, profile) = persisted(DatabaseEngine::Postgresql);
        let result = CsvExportResult {
            status: CsvTransferStatus::Completed,
            engine: DatabaseEngine::Postgresql,
            table: Some(table()),
            destination_path: "/home/user/secret.csv".to_string(),
            rows_exported: 42,
            columns: vec!["name".to_string()],
            bytes_written: 100,
            encoding: CsvEncoding::Utf8,
            delimiter: CsvDelimiter::Comma,
            include_header: true,
            warnings: Vec::new(),
            may_be_incomplete: false,
            values_truncated: false,
            duration_ms: 5,
        };
        record_export_history(&database, &profile, &export_request(), &Ok(result), 1, 5);
        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("history entry");
        assert_eq!(row.operation, "database-csv-export");
        assert!(row.safe_summary.contains("Exported 42 rows"));
        assert!(row.safe_summary.contains("app.public.users"));
        assert!(!row.safe_summary.contains("/home/user"));
        assert!(!row.safe_summary.contains("SELECT"));
    }

    #[test]
    fn import_history_names_table_and_counts_without_values_or_paths() {
        let (database, profile) = persisted(DatabaseEngine::Mysql);
        let result = CsvImportResult {
            status: CsvTransferStatus::Completed,
            engine: DatabaseEngine::Mysql,
            table: table(),
            rows_read: 10,
            rows_imported: 8,
            rows_skipped: 2,
            warnings: Vec::new(),
            warnings_truncated: false,
            transaction: crate::database::RowTransactionState::Committed,
            may_be_incomplete: false,
            duration_ms: 3,
        };
        record_import_history(&database, &profile, &import_request(), &Ok(result), 1, 3);
        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("history entry");
        assert_eq!(row.operation, "database-csv-import");
        assert!(row.safe_summary.contains("Imported 8 rows"));
        assert!(row.safe_summary.contains("2 rows skipped"));
        assert!(!row.safe_summary.contains("/tmp"));
        assert!(
            !row.output_summary
                .as_deref()
                .unwrap_or_default()
                .contains("secret")
        );
    }

    #[test]
    fn cancelled_import_history_is_not_success() {
        let (database, profile) = persisted(DatabaseEngine::Postgresql);
        let result = CsvImportResult {
            status: CsvTransferStatus::Cancelled,
            engine: DatabaseEngine::Postgresql,
            table: table(),
            rows_read: 4,
            rows_imported: 0,
            rows_skipped: 0,
            warnings: Vec::new(),
            warnings_truncated: false,
            transaction: crate::database::RowTransactionState::RolledBack,
            may_be_incomplete: false,
            duration_ms: 1,
        };
        record_import_history(&database, &profile, &import_request(), &Ok(result), 1, 1);
        let rows = database.list_operations(10).expect("history");
        assert_eq!(rows[0].result, OperationResult::Cancelled);
        assert!(rows[0].safe_summary.contains("rolled back"));
    }
}
