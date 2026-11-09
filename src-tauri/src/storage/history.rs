use std::str::FromStr;

use rusqlite::{Row, params, types::Type};
use uuid::Uuid;

use crate::storage::{
    Database, NewOperationRecord, OperationCategory, OperationHistoryFilter, OperationRecord,
    OperationResult, StorageError,
};

/// Maximum number of history rows kept on disk.
///
/// Inserting a new row prunes the oldest rows beyond this bound, so the local
/// audit trail cannot grow without limit. The Command History page documents
/// this policy and allows the user to clear the remaining rows explicitly.
pub const MAX_OPERATION_HISTORY_RECORDS: u32 = 1_000;

const OPERATION_COLUMNS: &str = "id, server_id, project_id, category, operation, safe_summary, \
     path, output_summary, result, exit_code, started_at, duration_ms";

impl Database {
    pub fn record_operation(
        &self,
        input: &NewOperationRecord,
    ) -> Result<OperationRecord, StorageError> {
        let id = Uuid::new_v4().to_string();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO operation_history (
                    id, server_id, project_id, category, operation, safe_summary,
                    path, output_summary, result, exit_code, started_at, duration_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    input.server_id,
                    input.project_id,
                    input.category.as_str(),
                    input.operation,
                    input.safe_summary,
                    input.path,
                    input.output_summary,
                    input.result.as_str(),
                    input.exit_code,
                    input.started_at,
                    input.duration_ms,
                ],
            )?;

            connection.execute(
                "DELETE FROM operation_history WHERE id NOT IN (
                    SELECT id FROM operation_history
                    ORDER BY started_at DESC, id DESC
                    LIMIT ?1
                )",
                params![i64::from(MAX_OPERATION_HISTORY_RECORDS)],
            )?;

            connection
                .query_row(
                    &format!("SELECT {OPERATION_COLUMNS} FROM operation_history WHERE id = ?1"),
                    params![id],
                    row_to_operation,
                )
                .map_err(StorageError::from)
        })
    }

    pub fn list_operations(&self, limit: u32) -> Result<Vec<OperationRecord>, StorageError> {
        self.list_operations_filtered(&OperationHistoryFilter {
            limit: Some(limit),
            ..OperationHistoryFilter::default()
        })
    }

    /// Returns history rows matching every supplied filter, newest first.
    pub fn list_operations_filtered(
        &self,
        filter: &OperationHistoryFilter,
    ) -> Result<Vec<OperationRecord>, StorageError> {
        let limit = filter
            .limit
            .unwrap_or(MAX_OPERATION_HISTORY_RECORDS)
            .clamp(1, MAX_OPERATION_HISTORY_RECORDS);
        let category = filter.category.map(OperationCategory::as_str);
        let result = filter.result.map(OperationResult::as_str);

        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {OPERATION_COLUMNS} FROM operation_history \
                 WHERE (?1 IS NULL OR server_id = ?1) \
                 AND (?2 IS NULL OR category = ?2) \
                 AND (?3 IS NULL OR result = ?3) \
                 AND (?4 IS NULL OR started_at >= ?4) \
                 AND (?5 IS NULL OR started_at <= ?5) \
                 ORDER BY started_at DESC, id DESC LIMIT ?6"
            ))?;
            let operations = statement
                .query_map(
                    params![
                        filter.server_id,
                        category,
                        result,
                        filter.started_after,
                        filter.started_before,
                        i64::from(limit),
                    ],
                    row_to_operation,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(operations)
        })
    }

    pub fn list_project_operations(
        &self,
        project_id: &str,
        limit: u32,
    ) -> Result<Vec<OperationRecord>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {OPERATION_COLUMNS} FROM operation_history \
                 WHERE project_id = ?1 ORDER BY started_at DESC, id DESC LIMIT ?2"
            ))?;
            let operations = statement
                .query_map(params![project_id, i64::from(limit)], row_to_operation)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(operations)
        })
    }

    pub fn clear_operation_history(&self) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute("DELETE FROM operation_history", [])?;
            Ok(())
        })
    }
}

fn row_to_operation(row: &Row<'_>) -> rusqlite::Result<OperationRecord> {
    let category: String = row.get(3)?;
    let category = OperationCategory::from_str(&category).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
    })?;
    let result: String = row.get(8)?;
    let result = OperationResult::from_str(&result).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
    })?;

    Ok(OperationRecord {
        id: row.get(0)?,
        server_id: row.get(1)?,
        project_id: row.get(2)?,
        category,
        operation: row.get(4)?,
        safe_summary: row.get(5)?,
        path: row.get(6)?,
        output_summary: row.get(7)?,
        result,
        exit_code: row.get(9)?,
        started_at: row.get(10)?,
        duration_ms: row.get(11)?,
    })
}
