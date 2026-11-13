//! Bounded table-row browsing commands.
//!
//! The data grid is #176; this module provides the typed backend contract it
//! consumes. A request names one table, selected visible columns, filters,
//! search terms, sort columns, and one hard-bounded page. Values are always
//! parameterized, identifiers are resolved against live column metadata, and
//! one request id can be cancelled while it runs.

use serde::Serialize;
use tauri::State;

use crate::{
    core::AppError,
    database::{
        DEFAULT_ROW_PAGE_LIMIT, DatabaseConnectionRegistry, DatabaseRowService, Identifier,
        RowFilter, RowPageRequest, RowQueryControl, RowQueryRegistry, RowReadRequest,
        RowReadResult, RowSearch, RowSort, TableRef, row_query_timeout, validate_row_query_id,
    },
};

/// Result of asking the command layer to cancel one row read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowQueryCancellationResult {
    pub request_id: String,
    pub cancelled: bool,
}

fn page(limit: Option<u32>, offset: Option<u64>) -> Result<RowPageRequest, AppError> {
    RowPageRequest::try_new(limit.unwrap_or(DEFAULT_ROW_PAGE_LIMIT), offset.unwrap_or(0))
        .map_err(AppError::from)
}

fn table_ref(
    database: Option<String>,
    schema: Option<String>,
    table: String,
) -> Result<TableRef, AppError> {
    TableRef::new(database.as_deref(), schema.as_deref(), &table).map_err(AppError::from)
}

/// Reads one bounded page of rows with optional filters, search, sort, exact
/// count, and timeout. `request_id` must be unique among in-flight reads so
/// [`cancel_database_row_read`] can target exactly this call.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn read_database_table_rows(
    connections: State<'_, DatabaseConnectionRegistry>,
    requests: State<'_, RowQueryRegistry>,
    profile_id: String,
    request_id: String,
    database: Option<String>,
    schema: Option<String>,
    table: String,
    visible_columns: Option<Vec<Identifier>>,
    sort: Option<Vec<RowSort>>,
    filters: Option<Vec<RowFilter>>,
    search: Option<RowSearch>,
    limit: Option<u32>,
    offset: Option<u64>,
    include_exact_count: Option<bool>,
    timeout_ms: Option<u64>,
) -> Result<RowReadResult, AppError> {
    validate_row_query_id(&request_id).map_err(AppError::from)?;
    let request = RowReadRequest {
        table: table_ref(database, schema, table)?,
        page: page(limit, offset)?,
        visible_columns: visible_columns.unwrap_or_default(),
        sort: sort.unwrap_or_default(),
        filters: filters.unwrap_or_default(),
        search,
        include_exact_count: include_exact_count.unwrap_or(false),
    };
    request.validate().map_err(AppError::from)?;

    let guard = requests.begin(&request_id).map_err(AppError::from)?;
    let control = RowQueryControl::new(row_query_timeout(timeout_ms), Some(guard.cancellation()));
    DatabaseRowService::new(connections.inner())
        .read(&profile_id, &request, &control)
        .await
}

/// Cancels one in-flight row read by request id. Cancelling an id that is not
/// running is idempotent and returns `cancelled: false`.
#[tauri::command]
pub fn cancel_database_row_read(
    requests: State<'_, RowQueryRegistry>,
    request_id: String,
) -> Result<RowQueryCancellationResult, AppError> {
    validate_row_query_id(&request_id).map_err(AppError::from)?;
    let cancelled = requests.cancel(&request_id);
    Ok(RowQueryCancellationResult {
        request_id,
        cancelled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{MAX_ROW_PAGE_LIMIT, MAX_ROW_QUERY_TIMEOUT};

    #[test]
    fn command_page_shape_rejects_large_pages() {
        assert!(page(Some(10), Some(0)).is_ok());
        assert!(page(Some(MAX_ROW_PAGE_LIMIT + 1), Some(0)).is_err());
        assert!(page(Some(0), Some(0)).is_err());
    }

    #[test]
    fn command_timeout_shape_is_bounded() {
        assert_eq!(row_query_timeout(Some(u64::MAX)), MAX_ROW_QUERY_TIMEOUT);
    }
}
