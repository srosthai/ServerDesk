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
    core::AppError,
    database::{
        DatabaseConnectionRegistry, DatabaseQueryService, QueryControl, QueryExecutionResult,
        QueryRegistry, QueryRequest, SqlAnalysis, analyze_sql, query_timeout,
    },
    storage::DatabaseEngine,
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
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn execute_database_sql(
    connections: State<'_, DatabaseConnectionRegistry>,
    requests: State<'_, QueryRegistry>,
    profile_id: String,
    request_id: String,
    sql: String,
    max_rows: Option<u32>,
    timeout_ms: Option<u64>,
    confirmed: bool,
) -> Result<QueryExecutionResult, AppError> {
    let request = QueryRequest::try_new(sql, max_rows).map_err(AppError::from)?;
    let guard = requests.begin(&request_id).map_err(AppError::from)?;
    let control = QueryControl::new(
        query_timeout(timeout_ms),
        Some(guard.cancellation()),
        request.max_rows,
    );

    let started = Instant::now();
    let outcome = DatabaseQueryService::new(connections.inner())
        .execute(&profile_id, &request, confirmed, &control)
        .await;

    outcome.map_err(|error| {
        error.with_duration_ms(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
    })
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
    use crate::database::{MAX_QUERY_RESULT_LIMIT, MAX_QUERY_TIMEOUT, SqlWarningCode};

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
}
