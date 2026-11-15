//! SQL execution commands for the SQL workspace.
//!
//! The UI sends one SQL string, which may be the current statement or a
//! selection containing several statements. Execution is bounded by a timeout,
//! a per-result row limit, and a cancellation id; destructive SQL requires
//! explicit confirmation and a read-only profile rejects writes before the
//! adapter runs anything. Full result sets, affected counts, duration, and
//! structured warnings are returned to the caller.

use std::time::Instant;

use serde::Serialize;
use tauri::State;

use crate::{
    core::{
        AppError, ErrorCode,
        database_history::{DatabaseQueryHistoryDraft, record_query_history},
        history::{now_unix_seconds, summarize_error},
    },
    database::{
        DatabaseConnectionRegistry, DatabaseQueryService, QueryControl, QueryExecutionResult,
        QueryRegistry, QueryRequest, SqlAnalysis, analyze_sql, query_timeout,
    },
    storage::{Database, DatabaseConnection, DatabaseEngine, DatabaseQueryResult},
};

/// Result of asking the command layer to cancel one SQL execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseQueryCancellationResult {
    pub request_id: String,
    pub cancelled: bool,
}

/// Analyzes SQL for destructive statements and write detection without
/// connecting to a database. The UI calls this before executing selected SQL.
#[tauri::command]
pub fn analyze_database_sql(engine: DatabaseEngine, sql: String) -> Result<SqlAnalysis, AppError> {
    analyze_sql(engine, &sql).map_err(AppError::from)
}

/// Executes one SQL string with bounded results, timeout, and cancellation.
///
/// A successful, failed, or cancelled execution records one sanitized history
/// row. SQL text and the short error summary are redacted before storage, and
/// history write failures never replace the execution result.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn execute_database_sql(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    requests: State<'_, QueryRegistry>,
    profile_id: String,
    request_id: String,
    sql: String,
    max_rows: Option<u32>,
    timeout_ms: Option<u64>,
    confirmed: bool,
) -> Result<QueryExecutionResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let request = QueryRequest::try_new(sql, max_rows).map_err(AppError::from)?;
    let guard = requests.begin(&request_id).map_err(AppError::from)?;
    let control = QueryControl::new(
        query_timeout(timeout_ms),
        Some(guard.cancellation()),
        request.max_rows,
    );

    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = DatabaseQueryService::new(connections.inner())
        .execute(&profile_id, &request, confirmed, &control)
        .await;

    let duration_ms = match &outcome {
        Ok(result) => i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
        Err(error) => error
            .duration_ms()
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX))
            .unwrap_or_else(|| i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)),
    };
    record_execution_history(
        database.inner(),
        &profile,
        &request.sql,
        &outcome,
        duration_ms,
        started_at,
    );

    outcome.map_err(|error| {
        error.with_duration_ms(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
    })
}

fn load_profile(database: &Database, profile_id: &str) -> Result<DatabaseConnection, AppError> {
    database
        .get_database_connection(profile_id)?
        .ok_or_else(|| AppError::not_found("Database profile not found."))
}

fn record_execution_history(
    database: &Database,
    profile: &DatabaseConnection,
    sql: &str,
    outcome: &Result<QueryExecutionResult, AppError>,
    duration_ms: i64,
    started_at: i64,
) {
    let (result, row_count, error_summary) = match outcome {
        Ok(execution) => (
            DatabaseQueryResult::Success,
            Some(result_row_count(execution)),
            None,
        ),
        Err(error) if error.code == ErrorCode::Cancelled => {
            (DatabaseQueryResult::Cancelled, None, summarize_error(error))
        }
        Err(error) => (DatabaseQueryResult::Failure, None, summarize_error(error)),
    };

    let draft = DatabaseQueryHistoryDraft {
        profile,
        sql,
        result,
        duration_ms,
        row_count,
        error_summary: error_summary.as_deref(),
        started_at,
    };
    let _ = record_query_history(database, &draft);
}

fn result_row_count(execution: &QueryExecutionResult) -> i64 {
    let total = execution
        .results
        .iter()
        .map(|set| u64::from(set.returned_rows) + set.affected_rows.unwrap_or_default())
        .sum::<u64>();
    i64::try_from(total).unwrap_or(i64::MAX)
}

/// Cancels one in-flight SQL execution by request id. Cancelling an id that is
/// not running is idempotent and returns `cancelled: false`.
#[tauri::command]
pub fn cancel_database_sql(
    requests: State<'_, QueryRegistry>,
    request_id: String,
) -> Result<DatabaseQueryCancellationResult, AppError> {
    if crate::database::validate_query_request_id(&request_id).is_err() {
        return Err(AppError::validation("The query request id is not valid."));
    }
    let cancelled = requests.cancel(&request_id);
    Ok(DatabaseQueryCancellationResult {
        request_id,
        cancelled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::database_history::list_query_history,
        database::{MAX_QUERY_RESULT_LIMIT, MAX_QUERY_TIMEOUT, QueryResultSet, SqlWarningCode},
        storage::{AuthType, DatabaseQueryHistoryFilter, NewDatabaseConnection, NewServer},
    };

    #[test]
    fn command_timeout_and_result_limits_are_bounded() {
        assert_eq!(query_timeout(None), crate::database::DEFAULT_QUERY_TIMEOUT);
        assert_eq!(query_timeout(Some(u64::MAX)), MAX_QUERY_TIMEOUT);
        assert!(QueryRequest::try_new("SELECT 1", Some(MAX_QUERY_RESULT_LIMIT + 1)).is_err());
    }

    #[test]
    fn analysis_command_returns_typed_warnings() {
        let analysis = analyze_sql(
            DatabaseEngine::Postgresql,
            "DROP TABLE users; DELETE FROM users;",
        )
        .expect("analysis");
        assert!(analysis.requires_confirmation);
        assert_eq!(analysis.warnings.len(), 2);
        assert_eq!(analysis.warnings[0].code, SqlWarningCode::Drop);
    }

    fn fixture_profile(database: &Database) -> DatabaseConnection {
        let server = database
            .create_server(&NewServer {
                name: "Production".to_string(),
                host: "198.51.100.24".to_string(),
                port: 22,
                username: "deploy".to_string(),
                auth_type: AuthType::PrivateKey,
                credential_ref: Some("credential:test-server".to_string()),
                group_name: None,
                tags: vec![],
                notes: None,
            })
            .expect("create server");
        database
            .create_database_connection(&NewDatabaseConnection {
                server_id: server.id,
                name: "Primary".to_string(),
                engine: DatabaseEngine::Postgresql,
                host: "127.0.0.1".to_string(),
                port: 5432,
                socket: None,
                database_name: Some("app".to_string()),
                username: "app".to_string(),
                credential_ref: Some("credential:database-profile".to_string()),
                read_only: false,
                ssl_mode: None,
            })
            .expect("create connection")
    }

    fn execution_result() -> QueryExecutionResult {
        QueryExecutionResult {
            engine: DatabaseEngine::Postgresql,
            read_only: false,
            results: vec![QueryResultSet {
                index: 0,
                columns: vec![],
                rows: vec![],
                returned_rows: 2,
                affected_rows: Some(3),
                last_insert_id: None,
                truncated: false,
                columns_truncated: false,
            }],
            warnings: vec![],
            destructive: false,
            requires_confirmation: false,
            truncated: false,
            max_rows: 500,
            duration_ms: 9,
        }
    }

    #[test]
    fn execution_history_records_success_failure_and_cancellation() {
        let database = Database::in_memory().expect("in-memory database");
        let profile = fixture_profile(&database);

        record_execution_history(
            &database,
            &profile,
            "SELECT 1",
            &Ok(execution_result()),
            9,
            100,
        );
        record_execution_history(
            &database,
            &profile,
            "SELECT * FORM users",
            &Err(AppError::query("syntax error DB_PASSWORD=fixture-password")),
            4,
            200,
        );
        record_execution_history(
            &database,
            &profile,
            "SELECT pg_sleep(10)",
            &Err(AppError::cancelled("The SQL execution was cancelled.")),
            1_000,
            300,
        );

        let history = list_query_history(&database, &DatabaseQueryHistoryFilter::default())
            .expect("list history");
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].result, DatabaseQueryResult::Cancelled);
        assert_eq!(history[0].started_at, 300);
        assert_eq!(history[1].result, DatabaseQueryResult::Failure);
        assert!(
            history[1]
                .error_summary
                .as_deref()
                .is_some_and(|summary| !summary.contains("fixture-password"))
        );
        assert_eq!(history[2].result, DatabaseQueryResult::Success);
        assert_eq!(history[2].duration_ms, 9);
        assert_eq!(history[2].row_count, Some(5));
        assert_eq!(history[2].sql_text, "SELECT 1");
    }
}
