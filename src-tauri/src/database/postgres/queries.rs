//! Streaming execution of editor-supplied PostgreSQL SQL.
//!
//! `raw_sql` uses the simple-query protocol, which supports several statements
//! without prepared placeholders. Driver events are folded into the shared
//! bounded collector so large results are streamed and discarded after the
//! configured limits instead of being loaded into one `Vec` first.

use futures_util::StreamExt;
use sqlx::{
    AssertSqlSafe, Column, Execute, Executor, Postgres, Row, Statement, TypeInfo,
    postgres::{PgConnection, PgRow},
};
use tokio::time::{Instant, timeout_at};

use crate::storage::DatabaseEngine;

use super::adapter::map_sqlx_error;
use crate::database::{
    errors::DatabaseError,
    queries::{
        QueryColumn, QueryControl, QueryExecutionResult, QueryRequest, QueryResultCollector,
        encode_binary,
    },
};

fn cancelled_error(started: Instant) -> DatabaseError {
    DatabaseError::cancelled("The SQL execution was cancelled.")
        .with_duration_ms(elapsed_ms(started))
}

fn timeout_error(timeout: std::time::Duration, started: Instant) -> DatabaseError {
    DatabaseError::timeout(format!(
        "The SQL execution did not finish within {} seconds.",
        timeout.as_secs().max(1)
    ))
    .with_duration_ms(elapsed_ms(started))
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(crate) async fn execute_query(
    connection: &mut PgConnection,
    request: &QueryRequest,
    control: &QueryControl,
    read_only: bool,
) -> Result<QueryExecutionResult, DatabaseError> {
    let started = Instant::now();
    let analysis = request.analyze(DatabaseEngine::Postgresql)?;

    if read_only && analysis.has_writes {
        return Err(DatabaseError::permission(
            "The PostgreSQL session is read-only; the SQL attempted to write data, schema, or \
             session state.",
        )
        .with_details("No statement was executed."));
    }
    if control.is_cancelled() {
        return Err(cancelled_error(started));
    }

    let deadline = started + control.timeout.max(std::time::Duration::from_millis(1));
    let described_columns = if analysis.statement_count == 1 && !analysis.has_writes {
        describe_columns(connection, &request.sql, control, deadline).await
    } else {
        Vec::new()
    };

    let mut stream = sqlx::raw_sql(AssertSqlSafe(request.sql.clone())).fetch_many(&mut *connection);
    let mut collector = QueryResultCollector::new(control);

    loop {
        let item = tokio::select! {
            biased;
            _ = control.cancelled() => return Err(cancelled_error(started)),
            result = timeout_at(deadline, stream.next()) => match result {
                Err(_) => return Err(timeout_error(control.timeout, started)),
                Ok(None) => break,
                Ok(Some(Ok(item))) => item,
                Ok(Some(Err(error))) => {
                    return Err(map_sqlx_error(error).with_duration_ms(elapsed_ms(started)));
                }
            },
        };

        match item {
            sqlx::Either::Right(row) => {
                let columns = row_columns(&row, control.max_columns);
                let values = row_values(&row, columns.len());
                collector.push_row(columns, values)?;
            }
            sqlx::Either::Left(result) => {
                collector.push_command(result.rows_affected(), None);
            }
        }
    }

    collector.apply_described_columns(described_columns);
    let (results, truncated) = collector.finish();

    Ok(QueryExecutionResult {
        engine: DatabaseEngine::Postgresql,
        read_only,
        results,
        warnings: analysis.warnings,
        destructive: analysis.destructive,
        requires_confirmation: analysis.requires_confirmation,
        truncated,
        max_rows: control.max_rows,
        duration_ms: elapsed_ms(started),
    })
}

async fn describe_columns(
    connection: &mut PgConnection,
    sql: &str,
    control: &QueryControl,
    deadline: Instant,
) -> Vec<QueryColumn> {
    tokio::select! {
        biased;
        _ = control.cancelled() => Vec::new(),
        result = timeout_at(
            deadline,
            connection.prepare(sqlx::query::<Postgres>(AssertSqlSafe(sql.to_string())).sql()),
        ) => match result {
            Ok(Ok(statement)) => statement
                .columns()
                .iter()
                .map(|column| QueryColumn {
                    name: column.name().to_string(),
                    data_type: column.type_info().name().to_string(),
                })
                .collect(),
            _ => Vec::new(),
        },
    }
}

fn row_columns(row: &PgRow, max_columns: usize) -> Vec<QueryColumn> {
    row.columns()
        .iter()
        .take(max_columns)
        .map(|column| QueryColumn {
            name: column.name().to_string(),
            data_type: column.type_info().name().to_string(),
        })
        .collect()
}

fn row_values(row: &PgRow, count: usize) -> Vec<Option<String>> {
    (0..count)
        .map(
            |index| match row.try_get_unchecked::<Option<String>, _>(index) {
                Ok(value) => value,
                Err(_) => match row.try_get_unchecked::<Option<Vec<u8>>, _>(index) {
                    Ok(Some(bytes)) => Some(encode_binary(&bytes)),
                    Ok(None) => None,
                    Err(_) => Some("<unreadable value>".to_string()),
                },
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_values_are_rendered_without_loss() {
        assert_eq!(encode_binary(&[0x00, 0xff]), "\\x00ff");
    }
}
