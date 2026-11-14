//! Verified row insert, update, and delete commands.
//!
//! Every write is addressed by typed metadata, executes in an adapter
//! transaction, verifies the affected rows, and re-reads the row where the
//! identity is known. A failed write rolls back and reports that no rows were
//! changed. History records only the operation, table, engine, connection
//! profile, affected count, and verification outcome: never row values,
//! generated SQL, connection details, or credentials.

use std::time::Instant;

use tauri::State;

use crate::{
    core::{
        AppError, ErrorCode,
        history::{now_unix_seconds, record_operation, summarize_error},
    },
    database::{
        DatabaseConnectionRegistry, DatabaseRowService, Identifier, RowDeleteRequest,
        RowInsertRequest, RowQueryControl, RowUpdateRequest, RowWriteOperation, RowWriteResult,
        TableRef, row_query_timeout,
    },
    storage::{
        Database, DatabaseConnection, NewOperationRecord, OperationCategory, OperationResult,
    },
};

#[tauri::command]
pub async fn insert_database_table_row(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: RowInsertRequest,
    timeout_ms: Option<u64>,
) -> Result<RowWriteResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let control = RowQueryControl::new(row_query_timeout(timeout_ms), None);
    let outcome = DatabaseRowService::new(connections.inner())
        .insert(&profile_id, &request, &control)
        .await;

    record_row_write_history(
        database.inner(),
        &profile,
        RowWriteOperation::Insert,
        &request.table,
        1,
        &outcome,
        started_at,
        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
    );

    outcome
}

#[tauri::command]
pub async fn update_database_table_row(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: RowUpdateRequest,
    timeout_ms: Option<u64>,
) -> Result<RowWriteResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let control = RowQueryControl::new(row_query_timeout(timeout_ms), None);
    let outcome = DatabaseRowService::new(connections.inner())
        .update(&profile_id, &request, &control)
        .await;

    record_row_write_history(
        database.inner(),
        &profile,
        RowWriteOperation::Update,
        &request.table,
        1,
        &outcome,
        started_at,
        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
    );

    outcome
}

#[tauri::command]
pub async fn delete_database_table_rows(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: RowDeleteRequest,
    confirmed: bool,
    timeout_ms: Option<u64>,
) -> Result<RowWriteResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let target_count = request.targets.len();
    let control = RowQueryControl::new(row_query_timeout(timeout_ms), None);
    let outcome = DatabaseRowService::new(connections.inner())
        .delete(&profile_id, &request, confirmed, &control)
        .await;

    record_row_write_history(
        database.inner(),
        &profile,
        RowWriteOperation::Delete,
        &request.table,
        target_count,
        &outcome,
        started_at,
        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
    );

    outcome
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

/// Records one sanitized row-write history entry. The request's column values
/// are deliberately absent from every argument.
#[allow(clippy::too_many_arguments)]
fn record_row_write_history(
    database: &Database,
    profile: &DatabaseConnection,
    operation: RowWriteOperation,
    table: &TableRef,
    requested_rows: usize,
    outcome: &Result<RowWriteResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let action = operation.as_str();
    let engine = engine_label(profile.engine);
    let label = table_label(table);
    let noun = if requested_rows == 1 { "row" } else { "rows" };
    let (result, safe_summary, output_summary) = match outcome {
        Ok(result) => (
            OperationResult::Success,
            match operation {
                RowWriteOperation::Insert => format!(
                    "Inserted 1 {noun} into {label} on {engine} connection {} and committed the verified transaction",
                    profile.name
                ),
                RowWriteOperation::Update => format!(
                    "Updated 1 {noun} in {label} on {engine} connection {} and committed the verified transaction",
                    profile.name
                ),
                RowWriteOperation::Delete => format!(
                    "Deleted {} {noun} from {label} on {engine} connection {} and committed the verified transaction",
                    result.affected_rows, profile.name
                ),
            },
            Some(result.verification.clone()),
        ),
        Err(error) if error.code == ErrorCode::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the {action} action for {label} on {engine} connection {} before it \
                 completed; the transaction was rolled back",
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
            let rolled_back = error
                .details
                .as_deref()
                .is_some_and(|details| details.contains("rolled back"));
            let suffix = if rolled_back {
                "; the transaction was rolled back and no rows were changed"
            } else {
                "; the transaction did not commit"
            };
            (
                if unknown {
                    OperationResult::Unknown
                } else {
                    OperationResult::Failure
                },
                format!(
                    "The {action} action for {label} on {engine} connection {} did not complete{suffix}",
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
            operation: format!("database-{action}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database::{Identifier, RowTransactionState, RowValueMode, RowWriteValue},
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

    fn success(operation: RowWriteOperation, verification: &str) -> RowWriteResult {
        RowWriteResult {
            operation,
            table: table(),
            affected_rows: 2,
            verified: true,
            verification: verification.to_string(),
            transaction: RowTransactionState::Committed,
            row: None,
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
    fn success_history_names_the_table_and_count_without_values_or_sql() {
        let (database, profile) = persisted(DatabaseEngine::Postgresql);
        let outcome = success(
            RowWriteOperation::Delete,
            "Deleted 2 rows from app.public.users and verified the targets are absent.",
        );
        record_row_write_history(
            &database,
            &profile,
            RowWriteOperation::Delete,
            &table(),
            2,
            &Ok(outcome),
            1,
            4,
        );
        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("history entry");
        assert_eq!(row.operation, "database-delete_row");
        assert_eq!(row.category, OperationCategory::Database);
        assert!(
            row.safe_summary
                .contains("Deleted 2 rows from app.public.users")
        );
        assert!(
            row.safe_summary
                .contains("committed the verified transaction")
        );
        assert!(!row.safe_summary.contains("DELETE"));
        assert!(!row.safe_summary.contains("id ="));
        assert!(
            row.output_summary
                .as_deref()
                .is_some_and(|output| output.contains("verified") && !output.contains("DELETE"))
        );
    }

    #[test]
    fn failure_history_reports_rollback_without_row_contents() {
        let (database, profile) = persisted(DatabaseEngine::Mysql);
        let error = AppError::new(ErrorCode::Conflict, "The row changed in another session.")
            .with_details(
                "The transaction was rolled back; no rows were changed. DB_PASSWORD=hunter2",
            );
        record_row_write_history(
            &database,
            &profile,
            RowWriteOperation::Update,
            &table(),
            1,
            &Err(error),
            1,
            3,
        );
        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("history entry");
        assert_eq!(row.result, OperationResult::Failure);
        assert!(row.safe_summary.contains("did not complete"));
        assert!(row.safe_summary.contains("rolled back"));
        assert!(row.output_summary.as_deref().is_some_and(
            |output| !output.contains("hunter2") && !output.contains("super-secret-row")
        ));
    }

    #[test]
    fn permission_and_confirmation_failures_are_recorded_as_failures() {
        let (database, profile) = persisted(DatabaseEngine::Postgresql);
        let error = AppError::new(
            ErrorCode::Validation,
            "Deleting rows requires explicit confirmation of the selected row identities.",
        );
        record_row_write_history(
            &database,
            &profile,
            RowWriteOperation::Delete,
            &table(),
            2,
            &Err(error),
            1,
            0,
        );
        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("history entry");
        assert_eq!(row.result, OperationResult::Failure);
        assert!(row.safe_summary.contains("did not complete"));
        assert!(!row.safe_summary.contains("app.public.users.id"));
    }

    #[test]
    fn history_never_contains_request_values_for_any_mode() {
        let values = vec![
            RowWriteValue {
                column: Identifier::new("name").expect("column"),
                mode: RowValueMode::Value,
                value: Some("super-secret-value".to_string()),
            },
            RowWriteValue {
                column: Identifier::new("note").expect("column"),
                mode: RowValueMode::Null,
                value: None,
            },
        ];
        let request = RowInsertRequest {
            table: table(),
            values,
        };
        let rendered = format!("{request:?}");
        // The request can carry values in memory, but the history recorder has
        // no argument that accepts a value. Prove the safe summary text built
        // for this request shape does not include it.
        let (database, profile) = persisted(DatabaseEngine::Postgresql);
        record_row_write_history(
            &database,
            &profile,
            RowWriteOperation::Insert,
            &request.table,
            1,
            &Ok(success(
                RowWriteOperation::Insert,
                "Inserted 1 row into app.public.users and re-read it by its stable identity.",
            )),
            1,
            2,
        );
        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("history entry");
        assert!(!row.safe_summary.contains("super-secret-value"));
        assert!(
            row.output_summary
                .as_deref()
                .is_some_and(|output| !output.contains("super-secret-value"))
        );
        assert!(rendered.contains("super-secret-value")); // request itself only, never history
    }
}
