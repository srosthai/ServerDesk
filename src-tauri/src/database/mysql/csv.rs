//! MySQL streaming CSV export and transactional import.
//!
//! Table exports select every column without the row-browser display limit, so
//! a long value is written in full. Query exports execute exactly one
//! read-only statement. Imports run in one transaction: a driver failure,
//! timeout, or cancellation rolls the whole import back, while a row that fails
//! local type conversion is reported and skipped.

use std::time::Duration;

use futures_util::StreamExt;
use sqlx::{
    AssertSqlSafe, Column, Row,
    mysql::{MySqlConnection, MySqlRow},
};
use tokio::time::{Instant, timeout_at};

use crate::{
    database::{
        csv::{
            CsvExportSink, CsvExportSource, CsvExportSummary, CsvImportObserver, CsvImportRequest,
            CsvImportSource, CsvImportSummary, CsvWarning, MAX_CSV_COLUMNS,
            MAX_CSV_REPORTED_ISSUES,
        },
        dialect::ParameterizedQuery,
        errors::DatabaseError,
        queries::{QueryControl, QueryRequest},
        row_writes::{ResolvedValue, ResolvedWriteValue, resolve_import_values},
        rows::{RowColumnKind, RowQueryControl, RowTableMetadata},
    },
    storage::DatabaseEngine,
};

use super::{
    MYSQL_DIALECT, adapter::map_sqlx_error, queries as engine_queries, rows as engine_rows,
};

const ROLLBACK_TIMEOUT: Duration = Duration::from_secs(5);

fn cancelled_error() -> DatabaseError {
    DatabaseError::cancelled("The CSV transfer was cancelled.")
}

fn timeout_error(timeout: Duration) -> DatabaseError {
    DatabaseError::timeout(format!(
        "The CSV transfer did not finish within {} seconds.",
        timeout.as_secs().max(1)
    ))
}

fn row_error(row: u64, column: Option<u32>, message: impl Into<String>) -> CsvWarning {
    CsvWarning {
        row_number: row,
        column_number: column,
        column_name: None,
        message: message.into(),
    }
}

fn push_warning(warnings: &mut Vec<CsvWarning>, truncated: &mut bool, warning: CsvWarning) {
    if warnings.len() < MAX_CSV_REPORTED_ISSUES {
        warnings.push(warning);
    } else {
        *truncated = true;
    }
}

fn mapping(
    metadata: &RowTableMetadata,
    request: &CsvImportRequest,
    headers: &[String],
) -> Result<Vec<(usize, usize)>, DatabaseError> {
    if request.mappings.is_empty() {
        return Err(DatabaseError::validation(
            "Map at least one source column to a destination column.",
        ));
    }
    let mut sources = std::collections::HashSet::new();
    let mut targets = std::collections::HashSet::new();
    let mut resolved = Vec::with_capacity(request.mappings.len());
    for entry in &request.mappings {
        let source = usize::try_from(entry.source_index).unwrap_or(usize::MAX);
        if source >= headers.len() {
            return Err(DatabaseError::validation(format!(
                "Source column {} does not exist in the CSV header.",
                entry.source_index
            )));
        }
        if !sources.insert(source) {
            return Err(DatabaseError::validation(format!(
                "Source column {} is mapped more than once.",
                entry.source_index
            )));
        }
        if !targets.insert(entry.target_column.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Destination column {} is mapped more than once.",
                entry.target_column
            )));
        }
        let index = metadata
            .columns
            .iter()
            .position(|column| column.name == entry.target_column)
            .ok_or_else(|| {
                DatabaseError::validation(format!(
                    "Destination column {} does not exist in table {}.",
                    entry.target_column, metadata.table
                ))
            })?;
        let column = metadata.column(index)?;
        if column.generated || column.identity == crate::database::ColumnIdentity::Always {
            return Err(DatabaseError::validation(format!(
                "Column {} is generated and cannot receive imported values.",
                column.name
            )));
        }
        if column.kind == RowColumnKind::Other {
            return Err(DatabaseError::validation(format!(
                "Column {} has a type that cannot be imported safely.",
                column.name
            )));
        }
        resolved.push((source, index));
    }
    Ok(resolved)
}

fn build_export_query(metadata: &RowTableMetadata) -> Result<ParameterizedQuery, DatabaseError> {
    if metadata.columns.is_empty() {
        return Err(DatabaseError::validation(
            "The selected table does not expose any columns.",
        ));
    }
    if metadata.columns.len() > MAX_CSV_COLUMNS {
        return Err(DatabaseError::validation(format!(
            "A CSV export cannot contain more than {MAX_CSV_COLUMNS} columns."
        )));
    }
    let mut query = ParameterizedQuery::new();
    query.push_sql("SELECT ");
    for (index, column) in metadata.columns.iter().enumerate() {
        if index > 0 {
            query.push_sql(", ");
        }
        query
            .push_sql(&engine_rows::cell_expression(column, false))
            .push_sql(" AS ")
            .push_identifier(&MYSQL_DIALECT, &engine_rows::alias(index));
    }
    query
        .push_sql(" FROM ")
        .push_qualified_name(&MYSQL_DIALECT, &engine_rows::qualified_table(metadata)?);
    let identity = metadata.identity_indices();
    if !identity.is_empty() {
        query.push_sql(" ORDER BY ");
        for (position, index) in identity.iter().enumerate() {
            if position > 0 {
                query.push_sql(", ");
            }
            query
                .push_identifier(&MYSQL_DIALECT, &metadata.column(*index)?.name)
                .push_sql(" ASC");
        }
    }
    Ok(query)
}

fn build_import_query(
    metadata: &RowTableMetadata,
    values: &[ResolvedValue],
) -> Result<ParameterizedQuery, DatabaseError> {
    let mut query = ParameterizedQuery::new();
    query
        .push_sql("INSERT INTO ")
        .push_qualified_name(&MYSQL_DIALECT, &engine_rows::qualified_table(metadata)?)
        .push_sql(" (");
    for (position, value) in values.iter().enumerate() {
        if position > 0 {
            query.push_sql(", ");
        }
        query.push_identifier(&MYSQL_DIALECT, &metadata.column(value.column)?.name);
    }
    query.push_sql(") VALUES (");
    for (position, value) in values.iter().enumerate() {
        if position > 0 {
            query.push_sql(", ");
        }
        let column = metadata.column(value.column)?;
        match &value.value {
            ResolvedWriteValue::Null => {
                query.push_sql("NULL");
            }
            ResolvedWriteValue::Default => {
                query.push_sql("DEFAULT");
            }
            ResolvedWriteValue::Value(bound) => {
                if column.kind == RowColumnKind::Json {
                    query.push_sql("CAST(");
                    query.push_value(&MYSQL_DIALECT, bound.clone())?;
                    query.push_sql(" AS JSON)");
                } else {
                    query.push_value(&MYSQL_DIALECT, bound.clone())?;
                }
            }
        }
    }
    query.push_sql(")");
    Ok(query)
}

fn mapped_values(
    request: &CsvImportRequest,
    mapping: &[(usize, usize)],
    fields: &[crate::database::csv::CsvField],
) -> Vec<Option<String>> {
    mapping
        .iter()
        .map(|(source, _)| {
            let field = &fields[*source];
            if request.options.null_mode.is_null(field) {
                None
            } else {
                Some(field.value.clone())
            }
        })
        .collect()
}

pub(crate) async fn export(
    connection: &mut MySqlConnection,
    source: &CsvExportSource,
    sink: &mut dyn CsvExportSink,
    control: &RowQueryControl,
) -> Result<CsvExportSummary, DatabaseError> {
    match source {
        CsvExportSource::Table { table } => {
            let deadline = Instant::now() + control.timeout.max(Duration::from_millis(1));
            let metadata =
                engine_rows::load_table_metadata(connection, table, control, deadline).await?;
            let query = build_export_query(&metadata)?;
            let columns = metadata
                .columns
                .iter()
                .map(|column| column.name.as_str().to_string())
                .collect::<Vec<_>>();
            sink.write_header(&columns)?;

            let mut statement = sqlx::query(AssertSqlSafe(query.sql().to_string()));
            statement = engine_rows::bind_values(statement, query.values());
            let mut stream = statement.fetch(&mut *connection);
            let mut rows = 0u64;
            loop {
                let item = tokio::select! {
                    biased;
                    _ = control.cancelled() => return Err(cancelled_error()),
                    result = timeout_at(deadline, stream.next()) => match result {
                        Err(_) => return Err(timeout_error(control.timeout)),
                        Ok(None) => break,
                        Ok(Some(Ok(row))) => row,
                        Ok(Some(Err(error))) => return Err(map_sqlx_error(error)),
                    },
                };
                let values = row_values(&item, metadata.columns.len())?;
                sink.write_row(&values)?;
                rows = rows.saturating_add(1);
            }
            Ok(CsvExportSummary { columns, rows })
        }
        CsvExportSource::Query { sql } => export_query(connection, sql, sink, control).await,
    }
}

fn row_values(row: &MySqlRow, count: usize) -> Result<Vec<Option<String>>, DatabaseError> {
    if row.columns().len() > MAX_CSV_COLUMNS {
        return Err(DatabaseError::validation(format!(
            "A CSV export cannot contain more than {MAX_CSV_COLUMNS} columns."
        )));
    }
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        values.push(row.try_get::<Option<String>, _>(index).map_err(|error| {
            map_sqlx_error(error).with_details("An exported value could not be decoded as text.")
        })?);
    }
    Ok(values)
}

async fn export_query(
    connection: &mut MySqlConnection,
    sql: &str,
    sink: &mut dyn CsvExportSink,
    control: &RowQueryControl,
) -> Result<CsvExportSummary, DatabaseError> {
    let request = QueryRequest::try_new(sql.to_string(), None)?;
    let analysis = request.analyze(DatabaseEngine::Mysql)?;
    if analysis.statement_count != 1 {
        return Err(DatabaseError::validation(
            "CSV export supports exactly one read-only SQL statement.",
        ));
    }
    if analysis.has_writes {
        return Err(DatabaseError::validation(
            "CSV export only runs read-only SQL. The statement would write data or schema.",
        ));
    }
    if control.is_cancelled() {
        return Err(cancelled_error());
    }

    let deadline = Instant::now() + control.timeout.max(Duration::from_millis(1));
    let describe_control = QueryControl::new(control.timeout, None, 1);
    let described =
        engine_queries::describe_columns(connection, sql, &describe_control, deadline).await;
    if described.len() > MAX_CSV_COLUMNS {
        return Err(DatabaseError::validation(format!(
            "A CSV export cannot contain more than {MAX_CSV_COLUMNS} columns."
        )));
    }
    let mut header_written = false;
    let mut columns = described
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    if !columns.is_empty() {
        sink.write_header(&columns)?;
        header_written = true;
    }

    let mut stream = sqlx::raw_sql(AssertSqlSafe(sql.to_string())).fetch_many(&mut *connection);
    let mut rows = 0u64;
    loop {
        let item = tokio::select! {
            biased;
            _ = control.cancelled() => return Err(cancelled_error()),
            result = timeout_at(deadline, stream.next()) => match result {
                Err(_) => return Err(timeout_error(control.timeout)),
                Ok(None) => break,
                Ok(Some(Ok(item))) => item,
                Ok(Some(Err(error))) => return Err(map_sqlx_error(error)),
            },
        };
        if let sqlx::Either::Right(row) = item {
            if !header_written {
                columns = row
                    .columns()
                    .iter()
                    .map(|column| column.name().to_string())
                    .collect();
                if columns.len() > MAX_CSV_COLUMNS {
                    return Err(DatabaseError::validation(format!(
                        "A CSV export cannot contain more than {MAX_CSV_COLUMNS} columns."
                    )));
                }
                sink.write_header(&columns)?;
                header_written = true;
            }
            let values = row_values(&row, columns.len())?;
            sink.write_row(&values)?;
            rows = rows.saturating_add(1);
        }
    }
    Ok(CsvExportSummary { columns, rows })
}

pub(crate) async fn import(
    connection: &mut MySqlConnection,
    request: &CsvImportRequest,
    source: &mut dyn CsvImportSource,
    observer: &mut dyn CsvImportObserver,
    control: &RowQueryControl,
    read_only: bool,
) -> Result<CsvImportSummary, DatabaseError> {
    if read_only {
        return Err(DatabaseError::permission(
            "The database session is read-only; CSV imports are disabled.",
        ));
    }
    let deadline = Instant::now() + control.timeout.max(Duration::from_millis(1));
    let metadata =
        engine_rows::load_table_metadata(connection, &request.table, control, deadline).await?;
    let mapping = mapping(&metadata, request, source.headers())?;
    let mut tx = engine_rows::begin_transaction(connection, deadline).await?;

    let mut warnings = Vec::new();
    let mut warnings_truncated = false;
    let mut conversion_skipped = 0u64;
    let mut imported = 0u64;
    loop {
        if control.is_cancelled() {
            return Err(engine_rows::fail_and_rollback(
                tx,
                cancelled_error(),
                Instant::now() + ROLLBACK_TIMEOUT,
            )
            .await);
        }
        let row = match source.next_row() {
            Ok(Some(row)) => row,
            Ok(None) => break,
            Err(error) => {
                return Err(engine_rows::fail_and_rollback(
                    tx,
                    error,
                    Instant::now() + ROLLBACK_TIMEOUT,
                )
                .await);
            }
        };
        let values = mapped_values(request, &mapping, &row.fields);
        let targets = mapping
            .iter()
            .map(|(_, target)| *target)
            .collect::<Vec<_>>();
        let resolved = match resolve_import_values(&metadata, &targets, &values) {
            Ok(resolved) => resolved,
            Err(error) => {
                conversion_skipped = conversion_skipped.saturating_add(1);
                let warning = row_error(row.record_number, None, error.message);
                push_warning(&mut warnings, &mut warnings_truncated, warning.clone());
                observer.row_skipped(&warning);
                continue;
            }
        };
        let query = build_import_query(&metadata, &resolved)?;
        let mut statement = sqlx::query(AssertSqlSafe(query.sql().to_string()));
        statement = engine_rows::bind_values(statement, query.values());
        let outcome =
            engine_rows::bounded_query(control, deadline, statement.execute(&mut *tx)).await;
        match outcome {
            Ok(result) if result.rows_affected() == 1 => {
                imported = imported.saturating_add(1);
                observer.row_imported(row.record_number);
            }
            Ok(_) => {
                return Err(engine_rows::fail_and_rollback(
                    tx,
                    DatabaseError::query(format!(
                        "Row {} did not insert exactly one row; the import was rolled back.",
                        row.record_number
                    )),
                    Instant::now() + ROLLBACK_TIMEOUT,
                )
                .await);
            }
            Err(error) => {
                return Err(engine_rows::fail_and_rollback(
                    tx,
                    error.with_details(format!(
                        "Row {} failed; the import was rolled back.",
                        row.record_number
                    )),
                    Instant::now() + ROLLBACK_TIMEOUT,
                )
                .await);
            }
        }
    }

    if control.is_cancelled() {
        return Err(engine_rows::fail_and_rollback(
            tx,
            cancelled_error(),
            Instant::now() + ROLLBACK_TIMEOUT,
        )
        .await);
    }
    for issue in source.take_issues() {
        push_warning(&mut warnings, &mut warnings_truncated, issue);
    }
    warnings_truncated |= source.issues_truncated();
    let rows_skipped = source.skipped_rows().saturating_add(conversion_skipped);
    if let Err(error) = engine_rows::commit_transaction(tx, deadline).await {
        return Err(error.with_details(
            "The CSV import transaction commit could not be confirmed; re-read the table before retrying.",
        ));
    }
    Ok(CsvImportSummary {
        table: request.table.clone(),
        engine: DatabaseEngine::Mysql,
        rows_read: source.rows_read(),
        rows_imported: imported,
        rows_skipped,
        warnings,
        warnings_truncated,
    })
}
