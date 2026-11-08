use rusqlite::{Row, params};
use uuid::Uuid;

use crate::storage::{Database, NewOperationRecord, OperationRecord, StorageError};

const OPERATION_COLUMNS: &str =
    "id, server_id, project_id, operation, safe_summary, exit_code, started_at, duration_ms";

impl Database {
    pub fn record_operation(
        &self,
        input: &NewOperationRecord,
    ) -> Result<OperationRecord, StorageError> {
        let id = Uuid::new_v4().to_string();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO operation_history (
                    id, server_id, project_id, operation, safe_summary,
                    exit_code, started_at, duration_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id,
                    input.server_id,
                    input.project_id,
                    input.operation,
                    input.safe_summary,
                    input.exit_code,
                    input.started_at,
                    input.duration_ms,
                ],
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
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {OPERATION_COLUMNS} FROM operation_history ORDER BY started_at DESC LIMIT ?1"
            ))?;
            let operations = statement
                .query_map(params![i64::from(limit)], row_to_operation)?
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
                 WHERE project_id = ?1 ORDER BY started_at DESC LIMIT ?2"
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
    Ok(OperationRecord {
        id: row.get(0)?,
        server_id: row.get(1)?,
        project_id: row.get(2)?,
        operation: row.get(3)?,
        safe_summary: row.get(4)?,
        exit_code: row.get(5)?,
        started_at: row.get(6)?,
        duration_ms: row.get(7)?,
    })
}
