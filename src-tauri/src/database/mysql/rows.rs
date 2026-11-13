//! MySQL row-browser queries.
//!
//! The builders in this module are pure: they receive validated metadata and a
//! validated request, then produce parameterized SQL. The adapter supplies the
//! live connection; tests inspect the generated SQL and bound values directly.
//! No caller value is ever concatenated into SQL text.

use std::{collections::HashSet, future::Future};

use sqlx::{
    MySql, Row,
    mysql::{MySqlArguments, MySqlConnection, MySqlRow},
    query::Query,
};
use tokio::time::{Instant, timeout_at};

use crate::storage::DatabaseEngine;

use super::{
    MYSQL_DIALECT,
    adapter::{decode, map_sqlx_error},
    sql,
};
use crate::database::{
    dialect::{Identifier, IdentifierQuoter, ParameterizedQuery, QualifiedName, SqlValue},
    errors::DatabaseError,
    rows::{
        MAX_ROW_CELL_CHARACTERS, MAX_ROW_COLUMNS, RowColumn, RowExactCount, RowFilterOperator,
        RowFilterValue, RowQueryControl, RowReadPlan, RowReadRequest, RowReadResult,
        RowSortDirection, RowTableMetadata, UniqueIndexCandidate, contains_pattern,
        ends_with_pattern, infer_row_identity, is_bit_type, is_spatial_type, projected_outputs,
        resolve_read_plan, row_from_values, sql_value_for_column, starts_with_pattern,
    },
    traits::ColumnIdentity,
};

const DEFAULT_ESTIMATE_INDEX_LIMIT: i64 = 1_024;

fn cancelled_error() -> DatabaseError {
    DatabaseError::cancelled("The row read was cancelled.")
}

fn timeout_error(timeout: std::time::Duration) -> DatabaseError {
    DatabaseError::timeout(format!(
        "The row read did not finish within {} seconds.",
        timeout.as_secs().max(1)
    ))
}

async fn bounded_query<F, T>(
    control: &RowQueryControl,
    deadline: Instant,
    future: F,
) -> Result<T, DatabaseError>
where
    F: Future<Output = Result<T, sqlx::Error>>,
{
    if control.is_cancelled() {
        return Err(cancelled_error());
    }
    tokio::select! {
        biased;
        _ = control.cancelled() => Err(cancelled_error()),
        result = timeout_at(deadline, future) => match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(map_sqlx_error(error)),
            Err(_) => Err(timeout_error(control.timeout)),
        },
    }
}

fn bind_values<'q>(
    mut query: Query<'q, MySql, MySqlArguments>,
    values: &[SqlValue],
) -> Query<'q, MySql, MySqlArguments> {
    for value in values {
        query = match value {
            SqlValue::Null => query.bind(Option::<String>::None),
            SqlValue::Boolean(value) => query.bind(*value),
            SqlValue::Integer(value) => query.bind(*value),
            SqlValue::Float(value) => query.bind(*value),
            SqlValue::Text(value) => query.bind(value.clone()),
            SqlValue::Bytes(value) => query.bind(value.clone()),
        };
    }
    query
}

fn requested_database(table: &crate::database::TableRef) -> Result<Option<String>, DatabaseError> {
    match (
        table.database.as_ref().map(|value| value.as_str()),
        table.schema.as_ref().map(|value| value.as_str()),
    ) {
        (Some(database), Some(schema)) if database != schema => Err(DatabaseError::validation(
            "MySQL uses one database/schema namespace; the table reference values must match.",
        )),
        (Some(database), _) => Ok(Some(database.to_string())),
        (None, Some(schema)) => Ok(Some(schema.to_string())),
        (None, None) => Ok(None),
    }
}

fn qualified_table(metadata: &RowTableMetadata) -> Result<QualifiedName, DatabaseError> {
    let database = metadata.database.as_deref().ok_or_else(|| {
        DatabaseError::validation("Select a MySQL database before browsing table rows.")
    })?;
    QualifiedName::new([Identifier::new(database)?, metadata.table.clone()])
}

fn alias(index: usize) -> Identifier {
    Identifier::new(format!("sd_{index}")).expect("generated row alias is a valid identifier")
}

/// Builds the display expression for one cell. Binary values are base64 text
/// and spatial values are WKT; everything else is cast to characters so one
/// uniform decoder can be used regardless of column type.
fn cell_expression(column: &RowColumn, truncate: bool) -> String {
    let quoted = MYSQL_DIALECT.quote_identifier(&column.name);
    let raw = if is_bit_type(&column.data_type) {
        format!("CAST({quoted} + 0 AS CHAR)")
    } else if is_spatial_type(&column.data_type) {
        format!("ST_AsText({quoted})")
    } else if column.kind == crate::database::rows::RowColumnKind::Binary {
        format!("TO_BASE64({quoted})")
    } else {
        format!("CAST({quoted} AS CHAR)")
    };
    if truncate {
        format!("LEFT({raw}, {})", MAX_ROW_CELL_CHARACTERS.saturating_add(1))
    } else {
        raw
    }
}

fn push_filter_value(
    query: &mut ParameterizedQuery,
    column: &RowColumn,
    value: &RowFilterValue,
) -> Result<(), DatabaseError> {
    query.push_value(&MYSQL_DIALECT, sql_value_for_column(column, value))?;
    Ok(())
}

fn push_where(
    query: &mut ParameterizedQuery,
    metadata: &RowTableMetadata,
    plan: &RowReadPlan,
) -> Result<(), DatabaseError> {
    let mut first = true;

    for filter in &plan.filters {
        let column = metadata.column(filter.column)?;
        query.push_sql(if first { " WHERE " } else { " AND " });
        first = false;

        match filter.operator {
            RowFilterOperator::IsNull => {
                query
                    .push_identifier(&MYSQL_DIALECT, &column.name)
                    .push_sql(" IS NULL");
            }
            RowFilterOperator::IsNotNull => {
                query
                    .push_identifier(&MYSQL_DIALECT, &column.name)
                    .push_sql(" IS NOT NULL");
            }
            RowFilterOperator::In => {
                query
                    .push_identifier(&MYSQL_DIALECT, &column.name)
                    .push_sql(" IN (");
                for (index, value) in filter.values.iter().enumerate() {
                    if index > 0 {
                        query.push_sql(", ");
                    }
                    push_filter_value(query, column, value)?;
                }
                query.push_sql(")");
            }
            RowFilterOperator::Contains
            | RowFilterOperator::StartsWith
            | RowFilterOperator::EndsWith => {
                let Some(RowFilterValue::Text(value)) = &filter.value else {
                    return Err(DatabaseError::validation(
                        "A text-match filter requires a text value.",
                    ));
                };
                let pattern = match filter.operator {
                    RowFilterOperator::Contains => contains_pattern(value),
                    RowFilterOperator::StartsWith => starts_with_pattern(value),
                    RowFilterOperator::EndsWith => ends_with_pattern(value),
                    _ => unreachable!("matched above"),
                };
                query
                    .push_sql("LOWER(")
                    .push_identifier(&MYSQL_DIALECT, &column.name)
                    .push_sql(") LIKE LOWER(")
                    .push_value(&MYSQL_DIALECT, pattern)?
                    .push_sql(") ESCAPE '='");
            }
            operator => {
                let Some(value) = &filter.value else {
                    return Err(DatabaseError::validation(
                        "This filter requires exactly one value.",
                    ));
                };
                query.push_identifier(&MYSQL_DIALECT, &column.name);
                query.push_sql(match operator {
                    RowFilterOperator::Equals => " = ",
                    RowFilterOperator::NotEquals => " <> ",
                    RowFilterOperator::GreaterThan => " > ",
                    RowFilterOperator::GreaterThanOrEqual => " >= ",
                    RowFilterOperator::LessThan => " < ",
                    RowFilterOperator::LessThanOrEqual => " <= ",
                    _ => unreachable!("other operators handled above"),
                });
                push_filter_value(query, column, value)?;
            }
        }
    }

    if let Some(search) = &plan.search {
        query.push_sql(if first { " WHERE (" } else { " AND (" });
        let pattern = contains_pattern(&search.query);
        for (index, column_index) in search.columns.iter().enumerate() {
            if index > 0 {
                query.push_sql(" OR ");
            }
            let column = metadata.column(*column_index)?;
            query
                .push_sql("LOWER(")
                .push_identifier(&MYSQL_DIALECT, &column.name)
                .push_sql(") LIKE LOWER(")
                .push_value(&MYSQL_DIALECT, pattern.clone())?
                .push_sql(") ESCAPE '='");
        }
        query.push_sql(")");
    }

    Ok(())
}

fn push_order(
    query: &mut ParameterizedQuery,
    metadata: &RowTableMetadata,
    plan: &RowReadPlan,
    identity_columns: &[usize],
) -> Result<(), DatabaseError> {
    let mut order = plan.sort.clone();
    let mut seen: HashSet<usize> = order.iter().map(|(index, _)| *index).collect();
    for index in identity_columns {
        if seen.insert(*index) {
            order.push((*index, RowSortDirection::Asc));
        }
    }
    if order.is_empty() {
        return Ok(());
    }

    query.push_sql(" ORDER BY ");
    for (position, (index, direction)) in order.iter().enumerate() {
        if position > 0 {
            query.push_sql(", ");
        }
        let column = metadata.column(*index)?;
        query
            .push_identifier(&MYSQL_DIALECT, &column.name)
            .push_sql(match direction {
                RowSortDirection::Asc => " ASC",
                RowSortDirection::Desc => " DESC",
            });
    }
    Ok(())
}

/// Builds one bounded page query for a resolved plan.
pub fn build_page_query_for_plan(
    metadata: &RowTableMetadata,
    plan: &RowReadPlan,
    identity_columns: &[usize],
    page: crate::database::rows::RowPageRequest,
) -> Result<crate::database::rows::RowSelectQuery, DatabaseError> {
    let outputs = projected_outputs(plan, metadata);
    if outputs.len() > MAX_ROW_COLUMNS {
        return Err(DatabaseError::validation(
            "The row query would select too many columns.",
        ));
    }

    let mut query = ParameterizedQuery::new();
    query.push_sql("SELECT ");
    for (index, output) in outputs.iter().enumerate() {
        if index > 0 {
            query.push_sql(", ");
        }
        let column = metadata.column(output.column)?;
        query
            .push_sql(&cell_expression(column, output.truncate))
            .push_sql(" AS ")
            .push_identifier(&MYSQL_DIALECT, &alias(index));
    }

    query
        .push_sql(" FROM ")
        .push_qualified_name(&MYSQL_DIALECT, &qualified_table(metadata)?);
    push_where(&mut query, metadata, plan)?;
    push_order(&mut query, metadata, plan, identity_columns)?;
    query
        .push_sql(" LIMIT ")
        .push_value(&MYSQL_DIALECT, i64::from(page.fetch_limit()))?
        .push_sql(" OFFSET ")
        .push_value(
            &MYSQL_DIALECT,
            i64::try_from(page.offset).unwrap_or(i64::MAX),
        )?;

    Ok(crate::database::rows::RowSelectQuery {
        query,
        outputs,
        visible_columns: plan.visible.clone(),
        identity_columns: identity_columns.to_vec(),
    })
}

/// Builds the page query for a request.
pub fn build_page_query(
    metadata: &RowTableMetadata,
    request: &RowReadRequest,
) -> Result<crate::database::rows::RowSelectQuery, DatabaseError> {
    let plan = resolve_read_plan(request, metadata)?;
    let identity = metadata.identity_indices();
    build_page_query_for_plan(metadata, &plan, &identity, request.page)
}

/// Builds the exact-count query for a resolved plan.
pub fn build_count_query_for_plan(
    metadata: &RowTableMetadata,
    plan: &RowReadPlan,
) -> Result<ParameterizedQuery, DatabaseError> {
    let mut query = ParameterizedQuery::new();
    query
        .push_sql("SELECT COUNT(*) AS total FROM ")
        .push_qualified_name(&MYSQL_DIALECT, &qualified_table(metadata)?);
    push_where(&mut query, metadata, plan)?;
    Ok(query)
}

pub fn build_count_query(
    metadata: &RowTableMetadata,
    request: &RowReadRequest,
) -> Result<ParameterizedQuery, DatabaseError> {
    let plan = resolve_read_plan(request, metadata)?;
    build_count_query_for_plan(metadata, &plan)
}

/// Runs one bounded row read on a live MySQL connection.
pub async fn read_rows(
    connection: &mut MySqlConnection,
    request: &RowReadRequest,
    control: &RowQueryControl,
    session_read_only: bool,
) -> Result<RowReadResult, DatabaseError> {
    request.validate()?;
    let deadline = Instant::now() + control.timeout.max(std::time::Duration::from_millis(1));
    let database = requested_database(&request.table)?;
    let table_name = request.table.name.as_str();

    let estimate_rows = bounded_query(
        control,
        deadline,
        sqlx::query(sql::ROW_TABLE_ESTIMATE)
            .bind(database.as_deref())
            .bind(table_name)
            .fetch_all(&mut *connection),
    )
    .await?;
    let Some(estimate_row) = estimate_rows.first() else {
        return Err(DatabaseError::query(
            "The selected table was not found in this database.",
        ));
    };
    let resolved_database: String = decode(estimate_row, "database_name")?;
    let estimated_rows: Option<i64> = decode(estimate_row, "estimated_rows")?;

    let column_rows = bounded_query(
        control,
        deadline,
        sqlx::query(sql::ROW_TABLE_COLUMNS)
            .bind(&resolved_database)
            .bind(table_name)
            .bind(i64::try_from(MAX_ROW_COLUMNS + 1).unwrap_or(i64::MAX))
            .fetch_all(&mut *connection),
    )
    .await?;
    if column_rows.len() > MAX_ROW_COLUMNS {
        return Err(DatabaseError::query(format!(
            "The selected table has more than {MAX_ROW_COLUMNS} columns."
        )));
    }

    let index_rows = bounded_query(
        control,
        deadline,
        sqlx::query(sql::ROW_UNIQUE_INDEXES)
            .bind(&resolved_database)
            .bind(table_name)
            .bind(DEFAULT_ESTIMATE_INDEX_LIMIT)
            .fetch_all(&mut *connection),
    )
    .await?;

    let metadata = metadata_from_records(
        resolved_database,
        request.table.name.clone(),
        estimated_rows,
        column_rows
            .iter()
            .map(MysqlRowColumnRecord::from_row)
            .collect::<Result<Vec<_>, _>>()?,
        index_rows
            .iter()
            .map(MysqlRowIndexRecord::from_row)
            .collect::<Result<Vec<_>, _>>()?,
    )?;

    let plan = resolve_read_plan(request, &metadata)?;
    let identity_columns = metadata.identity_indices();
    let select = build_page_query_for_plan(&metadata, &plan, &identity_columns, request.page)?;
    let mut page_query = sqlx::query(sqlx::AssertSqlSafe(select.query.sql().to_string()));
    page_query = bind_values(page_query, select.query.values());
    let page_rows =
        bounded_query(control, deadline, page_query.fetch_all(&mut *connection)).await?;

    let has_more = page_rows.len() > request.page.limit as usize;
    let mut records = Vec::with_capacity(page_rows.len().min(request.page.limit as usize));
    for row in page_rows.iter().take(request.page.limit as usize) {
        let mut values = Vec::with_capacity(select.outputs.len());
        for index in 0..select.outputs.len() {
            values.push(row.try_get::<Option<String>, _>(index).map_err(|error| {
                map_sqlx_error(error)
                    .with_details("A row-browser cell could not be decoded as text.")
            })?);
        }
        records.push(row_from_values(&metadata, &select.outputs, values)?);
    }

    let exact_rows = if request.include_exact_count {
        let count_query = build_count_query_for_plan(&metadata, &plan)?;
        let mut query = sqlx::query(sqlx::AssertSqlSafe(count_query.sql().to_string()));
        query = bind_values(query, count_query.values());
        let count_row = bounded_query(control, deadline, query.fetch_one(&mut *connection)).await?;
        let total: i64 = decode(&count_row, "total")?;
        Some(RowExactCount {
            value: u64::try_from(total)
                .map_err(|_| DatabaseError::query("The database returned a negative row count."))?,
            filtered: count_is_filtered(request),
        })
    } else {
        None
    };

    Ok(RowReadResult::new(
        metadata,
        &plan,
        records,
        request.page,
        has_more,
        exact_rows,
        session_read_only,
    ))
}

pub(crate) fn count_is_filtered(request: &RowReadRequest) -> bool {
    !request.filters.is_empty()
        || request
            .search
            .as_ref()
            .is_some_and(|search| !search.query.trim().is_empty())
}

#[derive(Debug, Clone)]
pub(crate) struct MysqlRowColumnRecord {
    name: String,
    data_type: String,
    nullable: bool,
    ordinal_position: i64,
    primary_key: bool,
    auto_increment: bool,
    generated: bool,
}

impl MysqlRowColumnRecord {
    fn from_row(row: &MySqlRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            name: decode(row, "name")?,
            data_type: decode(row, "data_type")?,
            nullable: decode::<i64>(row, "nullable")? != 0,
            ordinal_position: decode(row, "ordinal_position")?,
            primary_key: decode::<i64>(row, "primary_key")? != 0,
            auto_increment: decode::<i64>(row, "auto_increment")? != 0,
            generated: decode::<i64>(row, "generated")? != 0,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MysqlRowIndexRecord {
    index_name: String,
    column_name: Option<String>,
    ordinal_position: i64,
    sub_part: Option<i64>,
}

impl MysqlRowIndexRecord {
    fn from_row(row: &MySqlRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            index_name: decode(row, "index_name")?,
            column_name: decode(row, "column_name")?,
            ordinal_position: decode(row, "ordinal_position")?,
            sub_part: decode(row, "sub_part")?,
        })
    }
}

pub(crate) fn metadata_from_records(
    database: String,
    table: Identifier,
    estimated_rows: Option<i64>,
    column_records: Vec<MysqlRowColumnRecord>,
    index_records: Vec<MysqlRowIndexRecord>,
) -> Result<RowTableMetadata, DatabaseError> {
    let mut columns = Vec::with_capacity(column_records.len());
    let mut seen = HashSet::new();
    for record in column_records {
        if !seen.insert(record.name.clone()) {
            return Err(DatabaseError::query(
                "The table metadata contains duplicate column names.",
            ));
        }
        let identity = if record.auto_increment {
            ColumnIdentity::AutoIncrement
        } else {
            ColumnIdentity::None
        };
        columns.push(RowColumn::new(
            Identifier::new(record.name)?,
            record.data_type,
            record.nullable,
            u32::try_from(record.ordinal_position).unwrap_or(u32::MAX),
            record.primary_key,
            identity,
            record.generated,
        ));
    }
    columns.sort_by_key(|column| column.ordinal_position);
    if columns.is_empty() {
        return Err(DatabaseError::query(
            "The selected table does not expose any columns.",
        ));
    }

    let candidates = unique_index_candidates(index_records);
    let identity = infer_row_identity(&columns, candidates);

    Ok(RowTableMetadata {
        engine: DatabaseEngine::Mysql,
        database: Some(database.clone()),
        schema: Some(database),
        table,
        columns,
        identity,
        estimated_rows: estimated_rows
            .and_then(|value| u64::try_from(value).ok())
            .into(),
    })
}

fn unique_index_candidates(records: Vec<MysqlRowIndexRecord>) -> Vec<UniqueIndexCandidate> {
    let mut groups: Vec<(String, Vec<MysqlRowIndexRecord>)> = Vec::new();
    for record in records {
        match groups
            .iter_mut()
            .find(|(name, _)| name == &record.index_name)
        {
            Some((_, group)) => group.push(record),
            None => groups.push((record.index_name.clone(), vec![record])),
        }
    }

    groups
        .into_iter()
        .filter_map(|(name, mut records)| {
            records.sort_by_key(|record| record.ordinal_position);
            let name = Identifier::new(name).ok()?;
            let complete = !records.is_empty()
                && records
                    .iter()
                    .all(|record| record.column_name.is_some() && record.sub_part.is_none());
            let columns = records
                .into_iter()
                .filter_map(|record| record.column_name)
                .collect();
            Some(UniqueIndexCandidate {
                name,
                columns,
                complete,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::errors::DatabaseErrorKind;
    use crate::database::rows::{
        MAX_ROW_PAGE_LIMIT, RowColumnKind, RowFilter, RowIdentity, RowIdentityKind, RowPageRequest,
        RowSearch, RowSort, RowTableMetadata,
    };
    use crate::database::traits::{MetadataValue, TableRef};

    fn identifier(value: &str) -> Identifier {
        Identifier::new(value).expect("identifier")
    }

    fn column(name: &str, data_type: &str, nullable: bool, primary: bool) -> RowColumn {
        RowColumn::new(
            identifier(name),
            data_type.to_string(),
            nullable,
            1,
            primary,
            ColumnIdentity::None,
            false,
        )
    }

    fn metadata() -> RowTableMetadata {
        RowTableMetadata {
            engine: DatabaseEngine::Mysql,
            database: Some("app".to_string()),
            schema: Some("app".to_string()),
            table: identifier("users"),
            columns: vec![
                RowColumn {
                    ordinal_position: 1,
                    ..column("id", "bigint", false, true)
                },
                RowColumn {
                    ordinal_position: 2,
                    ..column("name", "varchar(120)", false, false)
                },
                RowColumn {
                    ordinal_position: 3,
                    ..column("payload", "json", true, false)
                },
            ],
            identity: RowIdentity::primary_key(vec![identifier("id")]),
            estimated_rows: MetadataValue::available(12),
        }
    }

    fn base_request() -> RowReadRequest {
        RowReadRequest {
            table: TableRef::new(Some("app"), Some("app"), "users").expect("table"),
            page: RowPageRequest::try_new(10, 0).expect("page"),
            visible_columns: Vec::new(),
            sort: Vec::new(),
            filters: Vec::new(),
            search: None,
            include_exact_count: false,
        }
    }

    #[test]
    fn page_query_quotes_identifiers_and_binds_sort_and_filter_values() {
        let mut request = base_request();
        request.visible_columns = vec![identifier("name")];
        request.sort.push(RowSort {
            column: identifier("name"),
            direction: RowSortDirection::Desc,
        });
        request.filters.push(RowFilter {
            column: identifier("name"),
            operator: RowFilterOperator::Contains,
            value: Some(RowFilterValue::Text(
                "Robert'; DROP TABLE users; --".to_string(),
            )),
            values: Vec::new(),
        });

        let select = build_page_query(&metadata(), &request).expect("page query");
        let sql = select.query.sql();

        assert!(sql.starts_with("SELECT LEFT(CAST(`name` AS CHAR)"));
        assert!(sql.contains("FROM `app`.`users`"));
        assert!(sql.contains("LOWER(`name`) LIKE LOWER(?) ESCAPE '='"));
        assert!(sql.contains("ORDER BY `name` DESC, `id` ASC"));
        assert!(sql.ends_with("LIMIT ? OFFSET ?"));
        assert!(!sql.contains("DROP TABLE"));
        assert_eq!(select.query.values().len(), 3);
        assert_eq!(
            select.query.values()[0],
            SqlValue::Text("%Robert'; DROP TABLE users; --%".to_string())
        );
        assert_eq!(select.query.values()[1], SqlValue::Integer(11));
        assert_eq!(select.query.values()[2], SqlValue::Integer(0));
        assert_eq!(select.visible_columns, vec![1]);
        assert_eq!(select.identity_columns, vec![0]);
        assert_eq!(select.outputs[1].column, 0);
        assert!(!select.outputs[1].truncate);
    }

    #[test]
    fn sorting_a_json_column_is_rejected_before_sql_is_built() {
        let mut request = base_request();
        request.sort.push(RowSort {
            column: identifier("payload"),
            direction: RowSortDirection::Asc,
        });
        let error = build_page_query(&metadata(), &request).expect_err("json sort");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);
    }

    #[test]
    fn count_query_uses_the_same_parameterized_filters_without_a_limit() {
        let mut request = base_request();
        request.filters.push(RowFilter {
            column: identifier("id"),
            operator: RowFilterOperator::GreaterThanOrEqual,
            value: Some(RowFilterValue::Integer(5)),
            values: Vec::new(),
        });
        request.search = Some(RowSearch {
            query: "ann".to_string(),
            columns: vec![identifier("name")],
        });

        let query = build_count_query(&metadata(), &request).expect("count query");
        assert!(
            query
                .sql()
                .starts_with("SELECT COUNT(*) AS total FROM `app`.`users`")
        );
        assert!(query.sql().contains("`id` >= ?"));
        assert!(query.sql().contains("LOWER(`name`) LIKE LOWER(?)"));
        assert!(!query.sql().contains("LIMIT"));
        assert_eq!(query.values().len(), 2);
    }

    #[test]
    fn exact_page_size_is_hard_bounded_and_fetches_one_extra_row() {
        let mut request = base_request();
        request.page = RowPageRequest::try_new(MAX_ROW_PAGE_LIMIT, 7).expect("page");
        let select = build_page_query(&metadata(), &request).expect("page query");
        assert_eq!(
            select.query.values()[select.query.values().len() - 2],
            SqlValue::Integer(i64::from(MAX_ROW_PAGE_LIMIT) + 1)
        );
        assert_eq!(
            select.query.values()[select.query.values().len() - 1],
            SqlValue::Integer(7)
        );
        assert!(RowPageRequest::try_new(MAX_ROW_PAGE_LIMIT + 1, 0).is_err());
    }

    #[test]
    fn injection_text_in_a_value_never_reaches_the_sql_string() {
        let mut request = base_request();
        request.filters.push(RowFilter {
            column: identifier("name"),
            operator: RowFilterOperator::Equals,
            value: Some(RowFilterValue::Text(
                "x' OR 1=1; DROP TABLE audit; --".to_string(),
            )),
            values: Vec::new(),
        });
        request.search = Some(RowSearch {
            query: "'); DELETE FROM users; --".to_string(),
            columns: vec![identifier("name")],
        });
        let select = build_page_query(&metadata(), &request).expect("page query");
        assert!(!select.query.sql().contains("DROP TABLE"));
        assert!(!select.query.sql().contains("DELETE FROM"));
        assert!(
            select
                .query
                .values()
                .iter()
                .any(|value| matches!(value, SqlValue::Text(text) if text.contains("DROP TABLE")))
        );
    }

    #[test]
    fn injection_text_in_an_identifier_is_quoted_as_one_identifier() {
        let mut metadata = metadata();
        metadata.columns[1] = RowColumn {
            ordinal_position: 2,
            ..column("name` DESC, `id", "varchar(120)", false, false)
        };
        let mut request = base_request();
        request.sort.push(RowSort {
            column: identifier("name` DESC, `id"),
            direction: RowSortDirection::Asc,
        });
        let select = build_page_query(&metadata, &request).expect("page query");
        assert!(select.query.sql().contains("`name`` DESC, ``id` ASC"));
    }

    #[test]
    fn unique_index_candidates_skip_prefix_indexes_and_use_non_null_columns() {
        let columns = vec![
            column("email", "varchar(320)", false, false),
            column("tenant", "varchar(64)", true, false),
        ];
        let records = vec![
            MysqlRowIndexRecord {
                index_name: "users_tenant_email_key".to_string(),
                column_name: Some("tenant".to_string()),
                ordinal_position: 1,
                sub_part: None,
            },
            MysqlRowIndexRecord {
                index_name: "users_tenant_email_key".to_string(),
                column_name: Some("email".to_string()),
                ordinal_position: 2,
                sub_part: None,
            },
            MysqlRowIndexRecord {
                index_name: "users_email_prefix_key".to_string(),
                column_name: Some("email".to_string()),
                ordinal_position: 1,
                sub_part: Some(10),
            },
        ];
        let identity = infer_row_identity(&columns, unique_index_candidates(records));
        assert_eq!(identity.kind, RowIdentityKind::None);
        assert!(!identity.stable);
    }

    #[test]
    fn metadata_builds_columns_and_reports_estimate_without_exact_label() {
        let metadata = metadata_from_records(
            "app".to_string(),
            identifier("users"),
            Some(42),
            vec![MysqlRowColumnRecord {
                name: "id".to_string(),
                data_type: "bigint unsigned".to_string(),
                nullable: false,
                ordinal_position: 1,
                primary_key: true,
                auto_increment: true,
                generated: false,
            }],
            Vec::new(),
        )
        .expect("metadata");
        assert_eq!(metadata.columns[0].identity, ColumnIdentity::AutoIncrement);
        assert_eq!(metadata.columns[0].kind, RowColumnKind::Integer);
        assert_eq!(metadata.estimated_rows.value().copied(), Some(42));
    }
}
