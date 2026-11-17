//! In-memory adapter used by tests and capability demos.
//!
//! It implements the full shared interface without a database driver, network
//! connection, SSH tunnel, or engine SQL. Tests use it to prove that commands
//! can depend on `DatabaseAdapter` alone.

use std::{
    cmp::Ordering as CmpOrdering,
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use async_trait::async_trait;

use crate::storage::DatabaseEngine;

use super::{
    actions::{DatabaseDdlPreview, ValidatedDatabaseAction},
    capabilities::{DatabaseAction, DatabaseCapabilities, DatabaseVersion},
    csv::{
        CsvExportSink, CsvExportSource, CsvExportSummary, CsvImportObserver, CsvImportRequest,
        CsvImportSource, CsvImportSummary, CsvWarning, MAX_CSV_REPORTED_ISSUES,
    },
    dialect::{Identifier, SqlDialect},
    errors::DatabaseError,
    mysql,
    objects::{
        DatabaseObjectKind, ObjectDdlPreview, ObjectDefinition, ObjectRef, RoutineKind,
        RoutineSummary, SequenceDetails, TriggerDetails, TriggerEvent, TriggerSummary,
        TriggerTiming, ValidatedObjectAction, ValidatedTriggerBody,
    },
    pagination::{Bounded, Page, PageRequest},
    postgres,
    queries::{
        QueryColumn, QueryControl, QueryExecutionResult, QueryRequest, QueryResultCollector,
        SqlToken, split_sql_statements,
    },
    roles::{
        PrivilegeGrant, PrivilegeTarget, RoleDdlPlan, RoleGrantData, RoleKind, RoleRef,
        RoleSummary, ValidatedRoleAction,
    },
    row_writes::{
        ResolvedWriteValue, RowDeleteRequest, RowInsertRequest, RowUpdateRequest,
        RowWriteOperation, RowWriteResult, resolve_delete, resolve_import_values, resolve_insert,
        resolve_update, text_matches_column,
    },
    rows::{
        MAX_ROW_CELL_CHARACTERS, RowCell, RowColumn, RowColumnKind, RowExactCount,
        RowFilterOperator, RowFilterValue, RowIdentity, RowIdentityValue, RowQueryControl,
        RowReadRequest, RowReadResult, RowRecord, RowSortDirection, RowTableMetadata,
        resolve_read_plan,
    },
    sessions::{SessionQuery, SessionSummary, ValidatedSessionAction},
    structure::{
        ColumnDefault, ColumnPosition, StructureDdlPreview, ValidatedColumnChanges,
        ValidatedColumnDefinition, ValidatedConstraint, ValidatedStructureAction,
    },
    traits::{
        ColumnIdentity, ColumnSummary, ConstraintKind, ConstraintSummary, DatabaseAdapter,
        DatabaseConnectRequest, DatabaseConnectionInfo, DatabasePing, DatabaseSummary,
        DatabaseVersionInfo, DependencyKind, DependencySummary, IndexSummary, MetadataScope,
        MetadataValue, ObjectCounts, ObjectMetadata, SchemaSummary, SequenceSummary,
        TableDependencies, TableKind, TableRef, TableSummary, ViewKind, ViewSummary,
    },
};

const FAKE_LATENCY_MS: u64 = 1;

/// One deterministic row-browser fixture for the in-memory adapter.
#[derive(Debug, Clone)]
pub struct FakeRowTable {
    pub table: TableRef,
    pub columns: Vec<RowColumn>,
    pub identity: RowIdentity,
    pub estimated_rows: MetadataValue<u64>,
    /// Row values in column order. `None` is SQL NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// Column defaults in column order. A `None` default is SQL NULL.
    pub defaults: Vec<Option<String>>,
}

impl FakeRowTable {
    pub fn new(
        table: TableRef,
        columns: Vec<RowColumn>,
        identity: RowIdentity,
        estimated_rows: MetadataValue<u64>,
        rows: Vec<Vec<Option<String>>>,
    ) -> Self {
        Self {
            table,
            columns,
            identity,
            estimated_rows,
            rows,
            defaults: Vec::new(),
        }
    }

    /// Sets one default value per column for default-mode write tests.
    pub fn with_defaults(mut self, defaults: Vec<Option<String>>) -> Self {
        self.defaults = defaults;
        self
    }

    fn matches(&self, reference: &TableRef) -> bool {
        let database = reference.database.as_ref().map(Identifier::as_str);
        let schema = reference.schema.as_ref().map(Identifier::as_str);
        self.table.name == reference.name
            && database.is_none_or(|value| {
                self.table.database.as_ref().map(Identifier::as_str) == Some(value)
            })
            && schema.is_none_or(|value| {
                self.table.schema.as_ref().map(Identifier::as_str) == Some(value)
            })
    }
}

/// A deterministic adapter backed by fixture metadata.
pub struct FakeDatabaseAdapter {
    engine: DatabaseEngine,
    dialect: &'static dyn SqlDialect,
    capabilities: DatabaseCapabilities,
    version: DatabaseVersionInfo,
    connected: AtomicBool,
    connect_count: AtomicUsize,
    ping_count: AtomicUsize,
    close_count: AtomicUsize,
    connect_error: Option<DatabaseError>,
    structure_execution_error: Option<DatabaseError>,
    skip_structure_execution: bool,
    structure_execution_count: AtomicUsize,
    read_only: bool,
    active_database: Option<String>,
    databases: Vec<DatabaseSummary>,
    schemas: Vec<SchemaSummary>,
    tables: Vec<TableSummary>,
    views: Vec<ViewSummary>,
    sequences: Vec<SequenceSummary>,
    routines: Vec<RoutineSummary>,
    triggers: Vec<TriggerSummary>,
    view_queries: HashMap<String, String>,
    routine_bodies: HashMap<String, String>,
    trigger_statements: HashMap<String, String>,
    trigger_functions: HashMap<String, String>,
    sequence_details: HashMap<String, SequenceDetails>,
    object_execution_error: Option<DatabaseError>,
    skip_object_execution: bool,
    object_execution_count: AtomicUsize,
    roles: Vec<RoleSummary>,
    role_grants: HashMap<String, RoleGrantData>,
    role_execution_error: Option<DatabaseError>,
    skip_role_execution: bool,
    role_execution_count: AtomicUsize,
    sessions: Vec<SessionSummary>,
    session_list_error: Option<DatabaseError>,
    session_execution_error: Option<DatabaseError>,
    skip_session_execution: bool,
    session_execution_count: AtomicUsize,
    columns: Vec<ColumnSummary>,
    created_tables: HashMap<String, Vec<ColumnSummary>>,
    indexes: HashMap<String, Vec<IndexSummary>>,
    constraints: HashMap<String, Vec<ConstraintSummary>>,
    dependencies: TableDependencies,
    detail_table: String,
    row_tables: Vec<FakeRowTable>,
    row_read_error: Option<DatabaseError>,
    row_write_error: Option<DatabaseError>,
    row_write_count: usize,
    row_rollback_count: usize,
    query_error: Option<DatabaseError>,
    query_execution_count: AtomicUsize,
}

impl FakeDatabaseAdapter {
    pub fn new(engine: DatabaseEngine, raw_version: Option<&str>) -> Self {
        let parsed_version = raw_version.and_then(DatabaseVersion::parse);
        let version = parsed_version.unwrap_or(DatabaseVersion::new(0, 0, 0));
        let version_text = raw_version
            .map(ToString::to_string)
            .unwrap_or_else(|| "unknown".to_string());
        let (dialect, fixtures) = match engine {
            DatabaseEngine::Mysql => (
                &mysql::MYSQL_DIALECT as &'static dyn SqlDialect,
                mysql_fixtures(),
            ),
            DatabaseEngine::Postgresql => (
                &postgres::POSTGRES_DIALECT as &'static dyn SqlDialect,
                postgres_fixtures(),
            ),
        };
        let capabilities = DatabaseCapabilities::with_version(engine, parsed_version);
        let detail_table = fixtures.detail_table.clone();
        let detail_columns = fixtures.columns.clone();
        let detail_indexes = fixtures.indexes.clone();
        let detail_constraints = fixtures.constraints.clone();

        Self {
            engine,
            dialect,
            capabilities,
            version: DatabaseVersionInfo {
                version,
                raw: version_text,
            },
            connected: AtomicBool::new(false),
            connect_count: AtomicUsize::new(0),
            ping_count: AtomicUsize::new(0),
            close_count: AtomicUsize::new(0),
            connect_error: None,
            structure_execution_error: None,
            skip_structure_execution: false,
            structure_execution_count: AtomicUsize::new(0),
            read_only: false,
            active_database: None,
            databases: fixtures.databases,
            schemas: fixtures.schemas,
            tables: fixtures.tables,
            views: fixtures.views,
            sequences: fixtures.sequences,
            routines: fixtures.routines,
            triggers: fixtures.triggers,
            view_queries: fixtures.view_queries,
            routine_bodies: fixtures.routine_bodies,
            trigger_statements: fixtures.trigger_statements,
            trigger_functions: fixtures.trigger_functions,
            sequence_details: fixtures.sequence_details,
            object_execution_error: None,
            skip_object_execution: false,
            object_execution_count: AtomicUsize::new(0),
            roles: fixtures.roles,
            role_grants: fixtures.role_grants,
            role_execution_error: None,
            skip_role_execution: false,
            role_execution_count: AtomicUsize::new(0),
            sessions: fixtures.sessions,
            session_list_error: None,
            session_execution_error: None,
            skip_session_execution: false,
            session_execution_count: AtomicUsize::new(0),
            columns: fixtures.columns,
            created_tables: HashMap::from([(detail_table.clone(), detail_columns)]),
            indexes: HashMap::from([(detail_table.clone(), detail_indexes)]),
            constraints: HashMap::from([(detail_table.clone(), detail_constraints)]),
            dependencies: fixtures.dependencies,
            detail_table,
            row_tables: Vec::new(),
            row_read_error: None,
            row_write_error: None,
            row_write_count: 0,
            row_rollback_count: 0,
            query_error: None,
            query_execution_count: AtomicUsize::new(0),
        }
    }

    /// Adds one deterministic row-browser table fixture.
    pub fn with_row_table(mut self, table: FakeRowTable) -> Self {
        self.row_tables.push(table);
        self
    }

    /// Makes every SQL execution fail with a typed database error.
    pub fn with_query_error(mut self, error: DatabaseError) -> Self {
        self.query_error = Some(error);
        self
    }

    /// Makes the next object action fail after its preview succeeds.
    pub fn with_object_execution_error(mut self, error: DatabaseError) -> Self {
        self.object_execution_error = Some(error);
        self
    }

    /// Simulates a server that accepts the DDL but does not apply it, so
    /// metadata verification reports an unverified outcome.
    pub fn with_skipped_object_execution(mut self) -> Self {
        self.skip_object_execution = true;
        self
    }

    pub fn object_execution_count(&self) -> usize {
        self.object_execution_count.load(Ordering::SeqCst)
    }

    /// Makes the next role or privilege action fail after its preview succeeds.
    pub fn with_role_execution_error(mut self, error: DatabaseError) -> Self {
        self.role_execution_error = Some(error);
        self
    }

    /// Simulates a server that accepts role DDL but does not apply it.
    pub fn with_skipped_role_execution(mut self) -> Self {
        self.skip_role_execution = true;
        self
    }

    pub fn role_execution_count(&self) -> usize {
        self.role_execution_count.load(Ordering::SeqCst)
    }

    /// Makes every session listing fail with a typed database error.
    pub fn with_session_list_error(mut self, error: DatabaseError) -> Self {
        self.session_list_error = Some(error);
        self
    }

    /// Makes the next session cancellation or termination fail with a typed
    /// database error.
    pub fn with_session_execution_error(mut self, error: DatabaseError) -> Self {
        self.session_execution_error = Some(error);
        self
    }

    /// Simulates a server that accepts the request but leaves the session or
    /// query running, so verification must report an unverified outcome.
    pub fn with_skipped_session_execution(mut self) -> Self {
        self.skip_session_execution = true;
        self
    }

    pub fn session_execution_count(&self) -> usize {
        self.session_execution_count.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn connect_for_test(&mut self) {
        self.connected.store(true, Ordering::SeqCst);
        self.active_database = Some("app".to_string());
        self.read_only = false;
    }

    #[cfg(test)]
    pub fn set_read_only_for_test(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    pub fn query_execution_count(&self) -> usize {
        self.query_execution_count.load(Ordering::SeqCst)
    }

    /// Makes every row read fail with a typed database error.
    pub fn with_row_read_error(mut self, error: DatabaseError) -> Self {
        self.row_read_error = Some(error);
        self
    }

    /// Makes the next row write fail before any memory mutation, simulating a
    /// database rejection followed by a rollback.
    pub fn with_row_write_error(mut self, error: DatabaseError) -> Self {
        self.row_write_error = Some(error);
        self
    }

    pub fn row_write_count(&self) -> usize {
        self.row_write_count
    }

    /// Full fixture rows for one table, including values the display grid would
    /// truncate. Used by CSV round-trip tests.
    pub fn row_table_rows(&self, table: &TableRef) -> Option<&[Vec<Option<String>>]> {
        self.row_tables
            .iter()
            .find(|fixture| fixture.matches(table))
            .map(|fixture| fixture.rows.as_slice())
    }

    pub fn row_rollback_count(&self) -> usize {
        self.row_rollback_count
    }

    /// Makes the next structure execution fail with a typed database error.
    pub fn with_structure_execution_error(mut self, error: DatabaseError) -> Self {
        self.structure_execution_error = Some(error);
        self
    }

    /// Simulates a driver that reports success without applying the change.
    /// Verification must detect the missing metadata instead of trusting the
    /// execution result.
    pub fn without_structure_execution(mut self) -> Self {
        self.skip_structure_execution = true;
        self
    }

    pub fn structure_execution_count(&self) -> usize {
        self.structure_execution_count.load(Ordering::SeqCst)
    }

    /// Makes the next `connect` call fail with a typed database error.
    pub fn with_connect_error(mut self, error: DatabaseError) -> Self {
        self.connect_error = Some(error);
        self
    }

    pub fn connect_count(&self) -> usize {
        self.connect_count.load(Ordering::SeqCst)
    }

    pub fn ping_count(&self) -> usize {
        self.ping_count.load(Ordering::SeqCst)
    }

    pub fn close_count(&self) -> usize {
        self.close_count.load(Ordering::SeqCst)
    }

    pub fn active_database(&self) -> Option<&str> {
        self.active_database.as_deref()
    }

    fn connection_info(&self, request: &DatabaseConnectRequest) -> DatabaseConnectionInfo {
        DatabaseConnectionInfo {
            engine: self.engine,
            version: self.version.clone(),
            database: self
                .active_database
                .clone()
                .or_else(|| request.database.clone()),
            read_only: self.read_only,
            capabilities: self.capabilities.snapshot(),
        }
    }

    fn paginate<T: Clone>(items: &[T], request: PageRequest) -> Page<T> {
        let total = items.len() as u64;
        let start = usize::try_from(request.offset)
            .unwrap_or(usize::MAX)
            .min(items.len());
        let end = start
            .saturating_add(request.limit as usize)
            .min(items.len());
        Page::from_items(items[start..end].to_vec(), request, Some(total))
    }

    fn table_matches(table: &TableSummary, reference: &TableRef) -> bool {
        let database = reference.database.as_ref().map(Identifier::as_str);
        let schema = reference.schema.as_ref().map(Identifier::as_str);

        table.name == reference.name.as_str()
            && database.is_none_or(|value| table.database.as_deref() == Some(value))
            && schema.is_none_or(|value| table.schema.as_deref() == Some(value))
    }

    fn object_key(object: &ObjectRef) -> String {
        format!(
            "{}::{}::{}::{}",
            object.kind.as_str(),
            object
                .database
                .as_ref()
                .map(Identifier::as_str)
                .unwrap_or(""),
            object.schema.as_ref().map(Identifier::as_str).unwrap_or(""),
            object.name.as_str()
        )
    }

    fn object_matches(
        object: &ObjectRef,
        database: Option<&str>,
        schema: Option<&str>,
        name: &str,
    ) -> bool {
        name == object.name.as_str()
            && object
                .database
                .as_ref()
                .is_none_or(|value| database == Some(value.as_str()))
            && object
                .schema
                .as_ref()
                .is_none_or(|value| schema == Some(value.as_str()))
    }

    fn structure_scope(&self, table: &TableRef) -> (String, String) {
        let database = table
            .database
            .as_ref()
            .map(Identifier::as_str)
            .or(self.active_database.as_deref())
            .unwrap_or("app")
            .to_string();
        let schema = table
            .schema
            .as_ref()
            .map(Identifier::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| match self.engine {
                DatabaseEngine::Mysql => database.clone(),
                DatabaseEngine::Postgresql => "public".to_string(),
            });
        (database, schema)
    }

    fn column_index(
        columns: &[ColumnSummary],
        name: &str,
        engine: DatabaseEngine,
    ) -> Option<usize> {
        columns
            .iter()
            .position(|column| column.name == name)
            .or_else(|| {
                (engine == DatabaseEngine::Mysql)
                    .then(|| {
                        columns
                            .iter()
                            .position(|column| column.name.eq_ignore_ascii_case(name))
                    })
                    .flatten()
            })
    }

    fn default_metadata(default: Option<&ColumnDefault>) -> MetadataValue<Option<String>> {
        MetadataValue::available(match default {
            None | Some(ColumnDefault::Null) => None,
            Some(ColumnDefault::String(value)) => Some(value.clone()),
            Some(ColumnDefault::Number(value)) => Some(value.clone()),
            Some(ColumnDefault::Boolean(value)) => Some(value.to_string()),
            Some(ColumnDefault::CurrentTimestamp) => Some("CURRENT_TIMESTAMP".to_string()),
        })
    }

    fn definition_summary(
        column: &ValidatedColumnDefinition,
        ordinal_position: u32,
    ) -> ColumnSummary {
        ColumnSummary {
            name: column.name.as_str().to_string(),
            data_type: column.data_type.clone(),
            length: MetadataValue::available(None),
            precision: MetadataValue::available(None),
            scale: MetadataValue::available(None),
            nullable: column.nullable,
            default_value: Self::default_metadata(column.default.as_ref()),
            ordinal_position,
            primary_key: false,
            identity: MetadataValue::available(column.identity),
            generated: MetadataValue::available(column.generated.is_some()),
            generation_expression: MetadataValue::available(
                column
                    .generated
                    .as_ref()
                    .map(|generated| generated.expression.clone()),
            ),
            comment: MetadataValue::available(column.comment.clone()),
            collation: MetadataValue::available(None),
        }
    }

    fn apply_definition(target: &mut ColumnSummary, definition: &ValidatedColumnDefinition) {
        target.data_type = definition.data_type.clone();
        target.nullable = definition.nullable;
        target.default_value = Self::default_metadata(definition.default.as_ref());
        target.comment = MetadataValue::available(definition.comment.clone());
        target.identity = MetadataValue::available(definition.identity);
        target.generated = MetadataValue::available(definition.generated.is_some());
        target.generation_expression = MetadataValue::available(
            definition
                .generated
                .as_ref()
                .map(|generated| generated.expression.clone()),
        );
    }

    fn apply_changes(target: &mut ColumnSummary, changes: &ValidatedColumnChanges) {
        if let Some(data_type) = changes.data_type.as_ref() {
            target.data_type = data_type.clone();
        }
        if let Some(nullable) = changes.nullable {
            target.nullable = nullable;
        }
        if changes.default.is_some() {
            target.default_value = Self::default_metadata(changes.default.as_ref());
        } else if changes.drop_default {
            target.default_value = Self::default_metadata(None);
        }
        if changes.comment.is_some() {
            target.comment = MetadataValue::available(changes.comment.clone());
        } else if changes.drop_comment {
            target.comment = MetadataValue::available(None);
        }
    }

    fn renumber_columns(columns: &mut [ColumnSummary]) {
        for (index, column) in columns.iter_mut().enumerate() {
            column.ordinal_position = u32::try_from(index + 1).unwrap_or(u32::MAX);
        }
    }

    fn record_from_values(
        metadata: &RowTableMetadata,
        plan: &crate::database::rows::RowReadPlan,
        values: &[Option<String>],
    ) -> Result<RowRecord, DatabaseError> {
        let identity_indices = metadata.identity_indices();
        let mut cells = Vec::with_capacity(plan.visible.len());
        for index in &plan.visible {
            let raw = values.get(*index).cloned().ok_or_else(|| {
                DatabaseError::internal("A fake row fixture is missing a column value.")
            })?;
            let (value, truncated) = if identity_indices.contains(index) {
                (raw, false)
            } else {
                truncate_fake_cell(raw)
            };
            cells.push(RowCell { value, truncated });
        }
        let identity = identity_indices
            .iter()
            .filter_map(|index| {
                let column = metadata.columns.get(*index)?.name.clone();
                let value = values.get(*index).cloned()?;
                Some(RowIdentityValue { column, value })
            })
            .collect();
        Ok(RowRecord { cells, identity })
    }

    fn row_matches(
        values: &[Option<String>],
        metadata: &RowTableMetadata,
        plan: &crate::database::rows::RowReadPlan,
    ) -> bool {
        for filter in &plan.filters {
            let Some(column) = metadata.columns.get(filter.column) else {
                return false;
            };
            let Some(cell) = values.get(filter.column) else {
                return false;
            };
            if !filter_matches(
                cell,
                column.kind,
                filter.operator,
                filter.value.as_ref(),
                &filter.values,
            ) {
                return false;
            }
        }

        if let Some(search) = &plan.search {
            let needle = search.query.to_lowercase();
            let matched = search.columns.iter().any(|index| {
                values
                    .get(*index)
                    .and_then(Option::as_deref)
                    .is_some_and(|value| value.to_lowercase().contains(&needle))
            });
            if !matched {
                return false;
            }
        }

        true
    }

    fn row_metadata(&self, fixture: &FakeRowTable) -> RowTableMetadata {
        RowTableMetadata {
            engine: self.engine,
            database: fixture
                .table
                .database
                .as_ref()
                .map(|value| value.as_str().to_string()),
            schema: fixture
                .table
                .schema
                .as_ref()
                .map(|value| value.as_str().to_string()),
            table: fixture.table.name.clone(),
            columns: fixture.columns.clone(),
            identity: fixture.identity.clone(),
            estimated_rows: fixture.estimated_rows.clone(),
        }
    }

    fn row_record(metadata: &RowTableMetadata, values: &[Option<String>]) -> RowRecord {
        let plan = crate::database::rows::RowReadPlan {
            visible: (0..metadata.columns.len()).collect(),
            sort: Vec::new(),
            filters: Vec::new(),
            search: None,
        };
        Self::record_from_values(metadata, &plan, values).expect("fake row record")
    }
}

fn row_cell_matches(column: &RowColumn, actual: Option<&Option<String>>, expected: &str) -> bool {
    let Some(actual) = actual.and_then(Option::as_deref) else {
        return false;
    };
    text_matches_column(column, expected, actual).unwrap_or_else(|| match column.kind {
        RowColumnKind::Text | RowColumnKind::Uuid | RowColumnKind::Temporal => expected == actual,
        RowColumnKind::Integer => expected.parse::<i64>().ok() == actual.parse::<i64>().ok(),
        RowColumnKind::Float | RowColumnKind::Decimal => {
            expected.parse::<f64>().ok() == actual.parse::<f64>().ok()
        }
        RowColumnKind::Boolean => parse_fake_bool(expected) == parse_fake_bool(actual),
        _ => expected == actual,
    })
}

fn truncate_fake_cell(value: Option<String>) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    if value.chars().count() <= MAX_ROW_CELL_CHARACTERS {
        return (Some(value), false);
    }
    (
        Some(value.chars().take(MAX_ROW_CELL_CHARACTERS).collect()),
        true,
    )
}

fn filter_matches(
    cell: &Option<String>,
    kind: RowColumnKind,
    operator: RowFilterOperator,
    value: Option<&RowFilterValue>,
    values: &[RowFilterValue],
) -> bool {
    match operator {
        RowFilterOperator::IsNull => cell.is_none(),
        RowFilterOperator::IsNotNull => cell.is_some(),
        RowFilterOperator::In => cell
            .as_deref()
            .is_some_and(|cell| values.iter().any(|value| cell_equals(cell, value, kind))),
        RowFilterOperator::Contains
        | RowFilterOperator::StartsWith
        | RowFilterOperator::EndsWith => {
            let (Some(cell), Some(RowFilterValue::Text(value))) = (cell.as_deref(), value) else {
                return false;
            };
            let cell = cell.to_lowercase();
            let value = value.to_lowercase();
            match operator {
                RowFilterOperator::Contains => cell.contains(&value),
                RowFilterOperator::StartsWith => cell.starts_with(&value),
                RowFilterOperator::EndsWith => cell.ends_with(&value),
                _ => false,
            }
        }
        _ => {
            let (Some(cell), Some(value)) = (cell.as_deref(), value) else {
                return false;
            };
            let Some(ordering) = compare_cell_to_value(cell, value, kind) else {
                return false;
            };
            match operator {
                RowFilterOperator::Equals => ordering == CmpOrdering::Equal,
                RowFilterOperator::NotEquals => ordering != CmpOrdering::Equal,
                RowFilterOperator::GreaterThan => ordering == CmpOrdering::Greater,
                RowFilterOperator::GreaterThanOrEqual => {
                    matches!(ordering, CmpOrdering::Greater | CmpOrdering::Equal)
                }
                RowFilterOperator::LessThan => ordering == CmpOrdering::Less,
                RowFilterOperator::LessThanOrEqual => {
                    matches!(ordering, CmpOrdering::Less | CmpOrdering::Equal)
                }
                _ => false,
            }
        }
    }
}

fn cell_equals(cell: &str, value: &RowFilterValue, kind: RowColumnKind) -> bool {
    compare_cell_to_value(cell, value, kind) == Some(CmpOrdering::Equal)
}

fn compare_cell_to_value(
    cell: &str,
    value: &RowFilterValue,
    kind: RowColumnKind,
) -> Option<CmpOrdering> {
    match (kind, value) {
        (RowColumnKind::Text, RowFilterValue::Text(value))
        | (RowColumnKind::Temporal, RowFilterValue::Text(value))
        | (RowColumnKind::Uuid, RowFilterValue::Text(value)) => Some(cell.cmp(value)),
        (RowColumnKind::Integer, RowFilterValue::Integer(value)) => {
            Some(cell.parse::<i64>().ok()?.cmp(value))
        }
        (RowColumnKind::Float, RowFilterValue::Integer(value)) => {
            Some(cell.parse::<f64>().ok()?.total_cmp(&(*value as f64)))
        }
        (RowColumnKind::Float, RowFilterValue::Float(value))
        | (RowColumnKind::Decimal, RowFilterValue::Float(value)) => {
            Some(cell.parse::<f64>().ok()?.total_cmp(value))
        }
        (RowColumnKind::Decimal, RowFilterValue::Integer(value)) => {
            Some(cell.parse::<f64>().ok()?.total_cmp(&(*value as f64)))
        }
        (RowColumnKind::Decimal, RowFilterValue::Text(value)) => Some(
            cell.parse::<f64>()
                .ok()?
                .total_cmp(&value.parse::<f64>().ok()?),
        ),
        (RowColumnKind::Boolean, RowFilterValue::Boolean(value)) => {
            Some(parse_fake_bool(cell)?.cmp(value))
        }
        _ => None,
    }
}

fn compare_cells(
    left: &Option<String>,
    right: &Option<String>,
    kind: RowColumnKind,
) -> CmpOrdering {
    match (left.as_deref(), right.as_deref()) {
        (None, None) => CmpOrdering::Equal,
        (None, Some(_)) => CmpOrdering::Greater,
        (Some(_), None) => CmpOrdering::Less,
        (Some(left), Some(right)) => match kind {
            RowColumnKind::Integer => match (left.parse::<i64>(), right.parse::<i64>()) {
                (Ok(left), Ok(right)) => left.cmp(&right),
                _ => left.cmp(right),
            },
            RowColumnKind::Float | RowColumnKind::Decimal => {
                match (left.parse::<f64>(), right.parse::<f64>()) {
                    (Ok(left), Ok(right)) => left.total_cmp(&right),
                    _ => left.cmp(right),
                }
            }
            RowColumnKind::Boolean => match (parse_fake_bool(left), parse_fake_bool(right)) {
                (Some(left), Some(right)) => left.cmp(&right),
                _ => left.cmp(right),
            },
            _ => left.cmp(right),
        },
    }
}

fn parse_fake_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "t" | "true" | "yes" => Some(true),
        "0" | "f" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn table_key(adapter: &FakeDatabaseAdapter, table: &TableRef) -> Result<String, DatabaseError> {
    adapter
        .tables
        .iter()
        .find(|summary| FakeDatabaseAdapter::table_matches(summary, table))
        .map(|summary| summary.name.clone())
        .ok_or_else(|| DatabaseError::query(format!("Table {} does not exist.", table.name)))
}

fn rename_column_in_list(columns: &mut [String], old: &str, new: &str) {
    for column in columns {
        if column.eq_ignore_ascii_case(old) {
            *column = new.to_string();
        }
    }
}

fn fake_constraint(constraint: &ValidatedConstraint, engine: DatabaseEngine) -> ConstraintSummary {
    let mut summary = ConstraintSummary {
        name: constraint.name().as_str().to_string(),
        kind: constraint.kind(),
        columns: constraint
            .columns()
            .iter()
            .map(|column| column.as_str().to_string())
            .collect(),
        referenced_schema: MetadataValue::available(None),
        referenced_table: MetadataValue::available(None),
        referenced_columns: Vec::new(),
        on_update: MetadataValue::available(None),
        on_delete: MetadataValue::available(None),
        check_expression: MetadataValue::available(None),
        deferrable: match engine {
            DatabaseEngine::Mysql => MetadataValue::Unavailable,
            DatabaseEngine::Postgresql => MetadataValue::available(false),
        },
        initially_deferred: match engine {
            DatabaseEngine::Mysql => MetadataValue::Unavailable,
            DatabaseEngine::Postgresql => MetadataValue::available(false),
        },
    };

    match constraint {
        ValidatedConstraint::ForeignKey {
            referenced_schema,
            referenced_table,
            referenced_columns,
            on_delete,
            on_update,
            ..
        } => {
            summary.referenced_schema = MetadataValue::available(
                referenced_schema
                    .as_ref()
                    .map(|schema| schema.as_str().to_string()),
            );
            summary.referenced_table =
                MetadataValue::available(Some(referenced_table.as_str().to_string()));
            summary.referenced_columns = referenced_columns
                .iter()
                .map(|column| column.as_str().to_string())
                .collect();
            summary.on_delete =
                MetadataValue::available(on_delete.map(|action| action.as_sql().to_string()));
            summary.on_update =
                MetadataValue::available(on_update.map(|action| action.as_sql().to_string()));
        }
        ValidatedConstraint::Check { expression, .. } => {
            summary.check_expression = MetadataValue::available(Some(expression.clone()));
        }
        ValidatedConstraint::PrimaryKey { .. } | ValidatedConstraint::Unique { .. } => {}
    }

    summary
}

#[async_trait]
impl DatabaseAdapter for FakeDatabaseAdapter {
    fn engine(&self) -> DatabaseEngine {
        self.engine
    }

    fn capabilities(&self) -> &DatabaseCapabilities {
        &self.capabilities
    }

    fn dialect(&self) -> &dyn SqlDialect {
        self.dialect
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn connect(
        &mut self,
        request: DatabaseConnectRequest,
    ) -> Result<DatabaseConnectionInfo, DatabaseError> {
        self.connect_count.fetch_add(1, Ordering::SeqCst);

        if let Some(error) = self.connect_error.take() {
            return Err(error);
        }
        if request.host.trim().is_empty() {
            return Err(DatabaseError::validation(
                "A database host or socket is required.",
            ));
        }
        if request.port == 0 {
            return Err(DatabaseError::validation(
                "A database port must be between 1 and 65535.",
            ));
        }
        if request.username.trim().is_empty() {
            return Err(DatabaseError::validation(
                "A database username is required.",
            ));
        }

        self.connected.store(true, Ordering::SeqCst);
        self.read_only = request.read_only;
        self.active_database = request.database.clone();
        Ok(self.connection_info(&request))
    }

    async fn ping(&self) -> Result<DatabasePing, DatabaseError> {
        self.require_capability(DatabaseAction::RunQuery)?;
        self.require_connection()?;
        self.ping_count.fetch_add(1, Ordering::SeqCst);
        Ok(DatabasePing {
            latency_ms: FAKE_LATENCY_MS,
        })
    }

    async fn version(&self) -> Result<DatabaseVersionInfo, DatabaseError> {
        self.require_connection()?;
        Ok(self.version.clone())
    }

    async fn close(&mut self) -> Result<(), DatabaseError> {
        self.close_count.fetch_add(1, Ordering::SeqCst);
        self.connected.store(false, Ordering::SeqCst);
        self.read_only = false;
        self.active_database = None;
        Ok(())
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    async fn list_databases(
        &self,
        page: PageRequest,
    ) -> Result<Page<DatabaseSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseDatabases)?;
        self.require_connection()?;
        Ok(Self::paginate(&self.databases, page))
    }

    async fn database_summary(
        &self,
        database: &Identifier,
    ) -> Result<Option<DatabaseSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseDatabases)?;
        self.require_connection()?;
        Ok(self
            .databases
            .iter()
            .find(|summary| summary.name == database.as_str())
            .cloned())
    }

    async fn preview_database_action(
        &self,
        action: &ValidatedDatabaseAction,
    ) -> Result<DatabaseDdlPreview, DatabaseError> {
        self.require_connection()?;
        if matches!(action, ValidatedDatabaseAction::Rename { .. })
            && self.engine == DatabaseEngine::Mysql
        {
            return mysql::actions::preview(action);
        }
        self.require_capability(action.capability())?;
        if let ValidatedDatabaseAction::Rename { name, .. } | ValidatedDatabaseAction::Drop { name } =
            action
            && self.active_database.as_deref() == Some(name.as_str())
        {
            return Err(DatabaseError::validation(
                "The active database cannot be renamed or dropped. Set another database active \
                 first.",
            ));
        }
        match self.engine {
            DatabaseEngine::Mysql => mysql::actions::preview(action),
            DatabaseEngine::Postgresql => postgres::actions::preview(action),
        }
    }

    async fn preview_structure_action(
        &self,
        action: &ValidatedStructureAction,
    ) -> Result<StructureDdlPreview, DatabaseError> {
        self.require_connection()?;
        action.ensure_supported(&self.capabilities)?;
        match self.engine {
            DatabaseEngine::Mysql => mysql::ddl::preview(action),
            DatabaseEngine::Postgresql => postgres::ddl::preview(action),
        }
    }

    async fn execute_database_action(
        &mut self,
        action: &ValidatedDatabaseAction,
    ) -> Result<Option<DatabaseConnectionInfo>, DatabaseError> {
        self.require_connection()?;
        self.preview_database_action(action).await?;

        match action {
            ValidatedDatabaseAction::Create(options) => {
                let summary = match self.engine {
                    DatabaseEngine::Mysql => DatabaseSummary {
                        name: options.name.as_str().to_string(),
                        system: false,
                        metadata: ObjectMetadata {
                            encoding: MetadataValue::available(
                                options
                                    .encoding
                                    .clone()
                                    .unwrap_or_else(|| "utf8mb4".to_string()),
                            ),
                            collation: MetadataValue::available(
                                options
                                    .collation
                                    .clone()
                                    .unwrap_or_else(|| "utf8mb4_unicode_ci".to_string()),
                            ),
                            size_bytes: MetadataValue::available(0),
                            ..ObjectMetadata::default()
                        },
                    },
                    DatabaseEngine::Postgresql => DatabaseSummary {
                        name: options.name.as_str().to_string(),
                        system: false,
                        metadata: ObjectMetadata {
                            owner: MetadataValue::available(
                                options
                                    .owner
                                    .as_ref()
                                    .map(|owner| owner.as_str().to_string())
                                    .unwrap_or_else(|| "app".to_string()),
                            ),
                            encoding: MetadataValue::available(
                                options
                                    .encoding
                                    .clone()
                                    .unwrap_or_else(|| "UTF8".to_string()),
                            ),
                            collation: MetadataValue::available(
                                options
                                    .collation
                                    .clone()
                                    .unwrap_or_else(|| "C.UTF-8".to_string()),
                            ),
                            size_bytes: MetadataValue::available(0),
                            ..ObjectMetadata::default()
                        },
                    },
                };
                self.databases.push(summary);
                Ok(None)
            }
            ValidatedDatabaseAction::Rename { name, new_name } => {
                if let Some(summary) = self
                    .databases
                    .iter_mut()
                    .find(|summary| summary.name == name.as_str())
                {
                    summary.name = new_name.as_str().to_string();
                }
                Ok(None)
            }
            ValidatedDatabaseAction::Drop { name } => {
                self.databases
                    .retain(|summary| summary.name != name.as_str());
                Ok(None)
            }
            ValidatedDatabaseAction::SetActive { name } => {
                self.active_database = Some(name.as_str().to_string());
                Ok(Some(DatabaseConnectionInfo {
                    engine: self.engine,
                    version: self.version.clone(),
                    database: self.active_database.clone(),
                    read_only: self.read_only,
                    capabilities: self.capabilities.snapshot(),
                }))
            }
        }
    }

    async fn list_schemas(&self, page: PageRequest) -> Result<Page<SchemaSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseSchemas)?;
        self.require_connection()?;
        Ok(Self::paginate(&self.schemas, page))
    }

    async fn list_tables(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<TableSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        self.require_connection()?;

        let database = scope.database.as_ref().map(|value| value.as_str());
        let schema = scope.schema.as_ref().map(|value| value.as_str());
        let tables = self
            .tables
            .iter()
            .filter(|table| {
                database.is_none_or(|value| table.database.as_deref() == Some(value))
                    && schema.is_none_or(|value| table.schema.as_deref() == Some(value))
            })
            .cloned()
            .collect::<Vec<_>>();

        Ok(Self::paginate(&tables, page))
    }

    async fn table_summary(&self, table: &TableRef) -> Result<Option<TableSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        self.require_connection()?;
        Ok(self
            .tables
            .iter()
            .find(|summary| Self::table_matches(summary, table))
            .cloned())
    }

    async fn list_views(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<ViewSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseViews)?;
        self.require_connection()?;

        let database = scope.database.as_ref().map(|value| value.as_str());
        let schema = scope.schema.as_ref().map(|value| value.as_str());
        let views = self
            .views
            .iter()
            .filter(|view| {
                database.is_none_or(|value| view.database.as_deref() == Some(value))
                    && schema.is_none_or(|value| view.schema.as_deref() == Some(value))
            })
            .cloned()
            .collect::<Vec<_>>();

        Ok(Self::paginate(&views, page))
    }

    async fn list_sequences(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<SequenceSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseSequences)?;
        self.require_connection()?;

        let database = scope.database.as_ref().map(|value| value.as_str());
        let schema = scope.schema.as_ref().map(|value| value.as_str());
        let sequences = self
            .sequences
            .iter()
            .filter(|sequence| {
                database.is_none_or(|value| sequence.database.as_deref() == Some(value))
                    && schema.is_none_or(|value| sequence.schema.as_deref() == Some(value))
            })
            .cloned()
            .collect::<Vec<_>>();

        Ok(Self::paginate(&sequences, page))
    }

    async fn list_routines(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<RoutineSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseRoutines)?;
        self.require_connection()?;
        let database = scope.database.as_ref().map(|value| value.as_str());
        let schema = scope.schema.as_ref().map(|value| value.as_str());
        let routines = self
            .routines
            .iter()
            .filter(|routine| {
                database.is_none_or(|value| routine.database.as_deref() == Some(value))
                    && schema.is_none_or(|value| routine.schema.as_deref() == Some(value))
            })
            .cloned()
            .collect::<Vec<_>>();
        Ok(Self::paginate(&routines, page))
    }

    async fn list_triggers(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<TriggerSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTriggers)?;
        self.require_connection()?;
        let database = scope.database.as_ref().map(|value| value.as_str());
        let schema = scope.schema.as_ref().map(|value| value.as_str());
        let triggers = self
            .triggers
            .iter()
            .filter(|trigger| {
                database.is_none_or(|value| trigger.database.as_deref() == Some(value))
                    && schema.is_none_or(|value| trigger.schema.as_deref() == Some(value))
            })
            .cloned()
            .collect::<Vec<_>>();
        Ok(Self::paginate(&triggers, page))
    }

    async fn object_definition(
        &self,
        object: &ObjectRef,
    ) -> Result<ObjectDefinition, DatabaseError> {
        self.require_connection()?;
        match object.kind {
            DatabaseObjectKind::View | DatabaseObjectKind::MaterializedView => {
                self.require_capability(DatabaseAction::BrowseViews)?;
                if object.kind == DatabaseObjectKind::MaterializedView {
                    self.require_capability(DatabaseAction::BrowseMaterializedViews)?;
                }
                let view = self
                    .views
                    .iter()
                    .find(|view| {
                        Self::object_matches(
                            object,
                            view.database.as_deref(),
                            view.schema.as_deref(),
                            &view.name,
                        )
                    })
                    .ok_or_else(|| DatabaseError::query("The view does not exist."))?;
                if object.kind == DatabaseObjectKind::MaterializedView
                    && view.kind != ViewKind::MaterializedView
                {
                    return Err(DatabaseError::validation(
                        "The selected object is a view, not a materialized view.",
                    ));
                }
                if object.kind == DatabaseObjectKind::View
                    && view.kind == ViewKind::MaterializedView
                {
                    return Err(DatabaseError::validation(
                        "The selected object is a materialized view, not a view.",
                    ));
                }
                let key = Self::object_key(object);
                let query = self
                    .view_queries
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| format!("SELECT * FROM {}", object.qualified_table()));
                let definition = format!(
                    "CREATE {}VIEW {}.{} AS\n{}",
                    if view.kind == ViewKind::MaterializedView {
                        "MATERIALIZED "
                    } else {
                        ""
                    },
                    object
                        .schema
                        .as_ref()
                        .map(Identifier::as_str)
                        .unwrap_or("public"),
                    object.name.as_str(),
                    query
                );
                Ok(ObjectDefinition {
                    kind: if view.kind == ViewKind::MaterializedView {
                        DatabaseObjectKind::MaterializedView
                    } else {
                        DatabaseObjectKind::View
                    },
                    database: view.database.clone(),
                    schema: view.schema.clone(),
                    name: view.name.clone(),
                    owner: view.metadata.owner.clone(),
                    language: MetadataValue::Unavailable,
                    routine_kind: MetadataValue::Unavailable,
                    arguments: MetadataValue::Unavailable,
                    return_type: MetadataValue::Unavailable,
                    view_query: MetadataValue::available(query),
                    routine_body: MetadataValue::Unavailable,
                    definition: MetadataValue::available(definition),
                    trigger: None,
                    sequence: None,
                    truncated: false,
                })
            }
            DatabaseObjectKind::Routine => {
                self.require_capability(DatabaseAction::BrowseRoutines)?;
                let routine = self
                    .routines
                    .iter()
                    .find(|routine| {
                        Self::object_matches(
                            object,
                            routine.database.as_deref(),
                            routine.schema.as_deref(),
                            &routine.name,
                        ) && object.routine_kind.is_none_or(|kind| kind == routine.kind)
                    })
                    .ok_or_else(|| DatabaseError::query("The routine does not exist."))?;
                let key = Self::object_key(object);
                let body = self
                    .routine_bodies
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| "BEGIN\n  -- fixture body\nEND".to_string());
                let definition = format!(
                    "CREATE {} {}({})\n{}",
                    match routine.kind {
                        RoutineKind::Function => "FUNCTION",
                        RoutineKind::Procedure => "PROCEDURE",
                    },
                    routine.name,
                    routine.arguments.value().cloned().unwrap_or_default(),
                    body
                );
                Ok(ObjectDefinition {
                    kind: DatabaseObjectKind::Routine,
                    database: routine.database.clone(),
                    schema: routine.schema.clone(),
                    name: routine.name.clone(),
                    owner: routine.owner.clone(),
                    language: routine.language.clone(),
                    routine_kind: MetadataValue::available(routine.kind),
                    arguments: routine.arguments.clone(),
                    return_type: routine.return_type.clone(),
                    view_query: MetadataValue::Unavailable,
                    routine_body: MetadataValue::available(body.clone()),
                    definition: MetadataValue::available(definition),
                    trigger: None,
                    sequence: None,
                    truncated: false,
                })
            }
            DatabaseObjectKind::Trigger => {
                self.require_capability(DatabaseAction::BrowseTriggers)?;
                let trigger = self
                    .triggers
                    .iter()
                    .find(|trigger| {
                        Self::object_matches(
                            object,
                            trigger.database.as_deref(),
                            trigger.schema.as_deref(),
                            &trigger.name,
                        ) && object
                            .table
                            .as_ref()
                            .is_none_or(|table| table.as_str() == trigger.table)
                    })
                    .ok_or_else(|| DatabaseError::query("The trigger does not exist."))?;
                let key = Self::object_key(object);
                let statement = self
                    .trigger_statements
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| "BEGIN\n  -- fixture trigger body\nEND".to_string());
                let function = self.trigger_functions.get(&key).cloned();
                let definition = function.clone().unwrap_or_else(|| {
                    format!(
                        "CREATE TRIGGER {} {} {} ON {}.{} FOR EACH ROW\n{}",
                        trigger.name,
                        trigger.timing.as_sql(),
                        trigger
                            .events
                            .iter()
                            .map(|event| event.as_sql())
                            .collect::<Vec<_>>()
                            .join(" OR "),
                        trigger.schema.as_deref().unwrap_or("public"),
                        trigger.table,
                        statement
                    )
                });
                Ok(ObjectDefinition {
                    kind: DatabaseObjectKind::Trigger,
                    database: trigger.database.clone(),
                    schema: trigger.schema.clone(),
                    name: trigger.name.clone(),
                    owner: trigger.owner.clone(),
                    language: MetadataValue::Unavailable,
                    routine_kind: MetadataValue::Unavailable,
                    arguments: MetadataValue::Unavailable,
                    return_type: MetadataValue::Unavailable,
                    view_query: MetadataValue::Unavailable,
                    routine_body: MetadataValue::Unavailable,
                    definition: MetadataValue::available(definition),
                    trigger: Some(TriggerDetails {
                        table: trigger.table.clone(),
                        timing: trigger.timing,
                        events: trigger.events.clone(),
                        statement: MetadataValue::available(statement),
                        function_name: function
                            .as_ref()
                            .and_then(|definition| {
                                definition
                                    .split("FUNCTION ")
                                    .nth(1)
                                    .and_then(|rest| rest.split('(').next())
                                    .map(|name| {
                                        MetadataValue::available(
                                            name.rsplit('.').next().unwrap_or(name).to_string(),
                                        )
                                    })
                            })
                            .unwrap_or(MetadataValue::Unavailable),
                        function: function.into(),
                    }),
                    sequence: None,
                    truncated: false,
                })
            }
            DatabaseObjectKind::Sequence => {
                self.require_capability(DatabaseAction::BrowseSequences)?;
                let sequence = self
                    .sequences
                    .iter()
                    .find(|sequence| {
                        Self::object_matches(
                            object,
                            sequence.database.as_deref(),
                            sequence.schema.as_deref(),
                            &sequence.name,
                        )
                    })
                    .ok_or_else(|| DatabaseError::query("The sequence does not exist."))?;
                let details = self
                    .sequence_details
                    .get(&Self::object_key(object))
                    .cloned()
                    .unwrap_or_else(|| SequenceDetails {
                        data_type: sequence.data_type.clone(),
                        start_value: sequence.start_value.clone(),
                        increment: sequence.increment.clone(),
                        minimum: MetadataValue::Unavailable,
                        maximum: MetadataValue::Unavailable,
                        current_value: MetadataValue::available("1".to_string()),
                        cache: MetadataValue::Unavailable,
                        cycle: MetadataValue::available(false),
                    });
                let definition = format!(
                    "CREATE SEQUENCE {}.{} AS {} INCREMENT BY {}",
                    sequence.schema.as_deref().unwrap_or("public"),
                    sequence.name,
                    sequence
                        .data_type
                        .value()
                        .cloned()
                        .unwrap_or_else(|| "bigint".to_string()),
                    sequence
                        .increment
                        .value()
                        .cloned()
                        .unwrap_or_else(|| "1".to_string())
                );
                Ok(ObjectDefinition {
                    kind: DatabaseObjectKind::Sequence,
                    database: sequence.database.clone(),
                    schema: sequence.schema.clone(),
                    name: sequence.name.clone(),
                    owner: sequence.metadata.owner.clone(),
                    language: MetadataValue::Unavailable,
                    routine_kind: MetadataValue::Unavailable,
                    arguments: MetadataValue::Unavailable,
                    return_type: MetadataValue::Unavailable,
                    view_query: MetadataValue::Unavailable,
                    routine_body: MetadataValue::Unavailable,
                    definition: MetadataValue::available(definition),
                    trigger: None,
                    sequence: Some(details),
                    truncated: false,
                })
            }
        }
    }

    async fn list_roles(&self, page: PageRequest) -> Result<Page<RoleSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseRoles)?;
        self.require_connection()?;
        Ok(Self::paginate(&self.roles, page))
    }

    async fn role_summary(&self, role: &RoleRef) -> Result<Option<RoleSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseRoles)?;
        self.require_connection()?;
        Ok(self
            .roles
            .iter()
            .find(|summary| summary.label() == role.label())
            .cloned())
    }

    async fn role_grants(&self, role: &RoleRef) -> Result<RoleGrantData, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseRoles)?;
        self.require_connection()?;
        Ok(self
            .role_grants
            .get(&role.label())
            .cloned()
            .unwrap_or_default())
    }

    async fn preview_role_action(
        &self,
        action: &ValidatedRoleAction,
    ) -> Result<RoleDdlPlan, DatabaseError> {
        self.require_connection()?;
        action.ensure_supported(&self.capabilities)?;
        match self.engine {
            DatabaseEngine::Mysql => mysql::roles::preview(action),
            DatabaseEngine::Postgresql => postgres::roles::preview(action),
        }
    }

    async fn execute_role_action(
        &mut self,
        action: &ValidatedRoleAction,
    ) -> Result<(), DatabaseError> {
        self.require_connection()?;
        self.preview_role_action(action).await?;
        self.role_execution_count.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.role_execution_error.take() {
            return Err(error);
        }
        if self.skip_role_execution {
            return Ok(());
        }

        match action {
            ValidatedRoleAction::CreateRole(options) => {
                if self
                    .roles
                    .iter()
                    .any(|role| role.label() == options.role.label())
                {
                    return Err(DatabaseError::query(format!(
                        "The role {} already exists.",
                        options.role.label()
                    )));
                }
                let can_login = match self.engine {
                    DatabaseEngine::Mysql => false,
                    DatabaseEngine::Postgresql => options
                        .changes
                        .can_login
                        .unwrap_or(options.kind == RoleKind::User),
                };
                self.roles.push(RoleSummary {
                    name: options.role.name.clone(),
                    host: options.role.host.clone(),
                    kind: options.kind,
                    can_login: MetadataValue::available(can_login),
                    superuser: match self.engine {
                        DatabaseEngine::Mysql => MetadataValue::Unavailable,
                        DatabaseEngine::Postgresql => {
                            MetadataValue::available(options.changes.superuser.unwrap_or(false))
                        }
                    },
                    create_database: match self.engine {
                        DatabaseEngine::Mysql => MetadataValue::Unavailable,
                        DatabaseEngine::Postgresql => MetadataValue::available(
                            options.changes.create_database.unwrap_or(false),
                        ),
                    },
                    create_role: match self.engine {
                        DatabaseEngine::Mysql => MetadataValue::Unavailable,
                        DatabaseEngine::Postgresql => {
                            MetadataValue::available(options.changes.create_role.unwrap_or(false))
                        }
                    },
                    inherit: match self.engine {
                        DatabaseEngine::Mysql => MetadataValue::Unavailable,
                        DatabaseEngine::Postgresql => {
                            MetadataValue::available(options.changes.inherit.unwrap_or(true))
                        }
                    },
                });
                self.role_grants.entry(options.role.label()).or_default();
            }
            ValidatedRoleAction::AlterRole { role, changes } => {
                let Some(summary) = self
                    .roles
                    .iter_mut()
                    .find(|summary| summary.label() == role.label())
                else {
                    return Err(DatabaseError::query(format!(
                        "The role {} does not exist.",
                        role.label()
                    )));
                };
                if let Some(value) = changes.can_login {
                    summary.can_login = MetadataValue::available(value);
                }
                if let Some(value) = changes.superuser {
                    summary.superuser = MetadataValue::available(value);
                }
                if let Some(value) = changes.create_database {
                    summary.create_database = MetadataValue::available(value);
                }
                if let Some(value) = changes.create_role {
                    summary.create_role = MetadataValue::available(value);
                }
                if let Some(value) = changes.inherit {
                    summary.inherit = MetadataValue::available(value);
                }
            }
            ValidatedRoleAction::GrantPrivileges(change) => {
                let data = self.role_grants.entry(change.grantee.label()).or_default();
                for privilege in &change.privileges {
                    let grant = PrivilegeGrant {
                        privilege: privilege.as_sql().to_string(),
                        target: change.target.clone(),
                        grantable: change.grant_option,
                    };
                    if !data.direct.contains(&grant) {
                        data.direct.push(grant);
                    }
                }
            }
            ValidatedRoleAction::RevokePrivileges(change) => {
                let data = self.role_grants.entry(change.grantee.label()).or_default();
                let mut next = Vec::new();
                for grant in std::mem::take(&mut data.direct) {
                    let matches = change
                        .privileges
                        .iter()
                        .any(|privilege| privilege.as_sql() == grant.privilege)
                        && grant.target == change.target;
                    if !matches {
                        next.push(grant);
                    } else if change.grant_option && grant.grantable {
                        let mut downgraded = grant;
                        downgraded.grantable = false;
                        next.push(downgraded);
                    }
                }
                data.direct = next;
            }
            ValidatedRoleAction::GrantRole { role, member } => {
                let data = self.role_grants.entry(member.label()).or_default();
                if !data.member_of.contains(role) {
                    data.member_of.push(role.clone());
                }
            }
            ValidatedRoleAction::RevokeRole { role, member } => {
                let data = self.role_grants.entry(member.label()).or_default();
                data.member_of.retain(|entry| entry != role);
            }
        }
        Ok(())
    }

    async fn list_sessions(
        &self,
        page: PageRequest,
        running_only: bool,
    ) -> Result<Page<SessionSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseSessions)?;
        self.require_connection()?;
        if let Some(error) = &self.session_list_error {
            return Err(error.clone());
        }
        let sessions = self
            .sessions
            .iter()
            .filter(|session| !running_only || session.is_running())
            .cloned()
            .collect::<Vec<_>>();
        Ok(Self::paginate(&sessions, page))
    }

    async fn session_summary(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseSessions)?;
        self.require_connection()?;
        Ok(self
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned())
    }

    async fn execute_session_action(
        &mut self,
        action: &ValidatedSessionAction,
    ) -> Result<bool, DatabaseError> {
        self.require_capability(action.action.capability())?;
        self.require_connection()?;
        self.session_execution_count.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.session_execution_error.take() {
            return Err(error);
        }
        if self.skip_session_execution {
            return Ok(true);
        }

        let Some(index) = self
            .sessions
            .iter()
            .position(|session| session.id == action.session_id)
        else {
            return Ok(false);
        };
        match action.action {
            crate::database::SessionActionKind::CancelQuery => {
                self.sessions[index].query = MetadataValue::unavailable();
            }
            crate::database::SessionActionKind::TerminateSession => {
                self.sessions.remove(index);
            }
        }
        Ok(true)
    }

    async fn preview_object_action(
        &self,
        action: &ValidatedObjectAction,
    ) -> Result<ObjectDdlPreview, DatabaseError> {
        self.require_connection()?;
        action.ensure_supported(&self.capabilities)?;
        match self.engine {
            DatabaseEngine::Mysql => mysql::objects::preview(action),
            DatabaseEngine::Postgresql => postgres::objects::preview(action),
        }
    }

    async fn execute_object_action(
        &mut self,
        action: &ValidatedObjectAction,
    ) -> Result<(), DatabaseError> {
        self.require_connection()?;
        self.preview_object_action(action).await?;
        self.object_execution_count.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.object_execution_error.take() {
            return Err(error);
        }
        if self.skip_object_execution {
            return Ok(());
        }

        match action {
            ValidatedObjectAction::CreateView {
                object,
                query,
                or_replace,
            } => {
                let key = Self::object_key(object);
                if !*or_replace
                    && self.views.iter().any(|view| {
                        Self::object_matches(
                            object,
                            view.database.as_deref(),
                            view.schema.as_deref(),
                            &view.name,
                        )
                    })
                {
                    return Err(DatabaseError::query("The view already exists."));
                }
                self.views.retain(|view| {
                    !Self::object_matches(
                        object,
                        view.database.as_deref(),
                        view.schema.as_deref(),
                        &view.name,
                    )
                });
                self.views.push(ViewSummary {
                    database: object.database.as_ref().map(ToString::to_string),
                    schema: object.schema.as_ref().map(ToString::to_string),
                    name: object.name.as_str().to_string(),
                    kind: ViewKind::View,
                    metadata: ObjectMetadata {
                        owner: MetadataValue::available("app".to_string()),
                        ..ObjectMetadata::default()
                    },
                });
                self.view_queries.insert(key, query.clone());
            }
            ValidatedObjectAction::DropView { object } => {
                self.views.retain(|view| {
                    !Self::object_matches(
                        object,
                        view.database.as_deref(),
                        view.schema.as_deref(),
                        &view.name,
                    )
                });
                self.view_queries.remove(&Self::object_key(object));
            }
            ValidatedObjectAction::CreateRoutine {
                object,
                routine_kind,
                arguments,
                return_type,
                language,
                body,
                or_replace,
            } => {
                if !*or_replace
                    && self.routines.iter().any(|routine| {
                        Self::object_matches(
                            object,
                            routine.database.as_deref(),
                            routine.schema.as_deref(),
                            &routine.name,
                        ) && routine.kind == *routine_kind
                    })
                {
                    return Err(DatabaseError::query("The routine already exists."));
                }
                self.routines.retain(|routine| {
                    !(Self::object_matches(
                        object,
                        routine.database.as_deref(),
                        routine.schema.as_deref(),
                        &routine.name,
                    ) && routine.kind == *routine_kind)
                });
                self.routines.push(RoutineSummary {
                    database: object.database.as_ref().map(ToString::to_string),
                    schema: object.schema.as_ref().map(ToString::to_string),
                    name: object.name.as_str().to_string(),
                    kind: *routine_kind,
                    language: language
                        .as_ref()
                        .map(|value| MetadataValue::available(value.as_str().to_string()))
                        .unwrap_or(MetadataValue::Unavailable),
                    return_type: return_type
                        .clone()
                        .map_or(MetadataValue::Unavailable, MetadataValue::available),
                    arguments: arguments
                        .clone()
                        .map_or(MetadataValue::Unavailable, MetadataValue::available),
                    owner: MetadataValue::available("app".to_string()),
                });
                self.routine_bodies
                    .insert(Self::object_key(object), body.clone());
            }
            ValidatedObjectAction::DropRoutine {
                object,
                routine_kind,
                ..
            } => {
                self.routines.retain(|routine| {
                    !(Self::object_matches(
                        object,
                        routine.database.as_deref(),
                        routine.schema.as_deref(),
                        &routine.name,
                    ) && routine.kind == *routine_kind)
                });
                self.routine_bodies.remove(&Self::object_key(object));
            }
            ValidatedObjectAction::CreateTrigger {
                object,
                table,
                timing,
                events,
                body,
                or_replace,
            } => {
                if !*or_replace
                    && self.triggers.iter().any(|trigger| {
                        Self::object_matches(
                            object,
                            trigger.database.as_deref(),
                            trigger.schema.as_deref(),
                            &trigger.name,
                        ) && trigger.table == table.as_str()
                    })
                {
                    return Err(DatabaseError::query("The trigger already exists."));
                }
                self.triggers.retain(|trigger| {
                    !(Self::object_matches(
                        object,
                        trigger.database.as_deref(),
                        trigger.schema.as_deref(),
                        &trigger.name,
                    ) && trigger.table == table.as_str())
                });
                self.triggers.push(TriggerSummary {
                    database: object.database.as_ref().map(ToString::to_string),
                    schema: object.schema.as_ref().map(ToString::to_string),
                    name: object.name.as_str().to_string(),
                    table: table.as_str().to_string(),
                    timing: *timing,
                    events: events.clone(),
                    owner: MetadataValue::available("app".to_string()),
                });
                let key = Self::object_key(object);
                match body {
                    ValidatedTriggerBody::Statement(statement) => {
                        self.trigger_statements.insert(key, statement.clone());
                    }
                    ValidatedTriggerBody::Function { name, .. } => {
                        self.trigger_functions.insert(
                            key,
                            format!(
                                "CREATE FUNCTION {name}() RETURNS trigger AS 'BEGIN RETURN NEW; END' \
                                 LANGUAGE plpgsql"
                            ),
                        );
                    }
                }
            }
            ValidatedObjectAction::DropTrigger { object, .. } => {
                self.triggers.retain(|trigger| {
                    !Self::object_matches(
                        object,
                        trigger.database.as_deref(),
                        trigger.schema.as_deref(),
                        &trigger.name,
                    )
                });
                let key = Self::object_key(object);
                self.trigger_statements.remove(&key);
                self.trigger_functions.remove(&key);
            }
            ValidatedObjectAction::CreateSequence {
                object,
                data_type,
                start,
                increment,
                minimum,
                maximum,
                cache,
                cycle,
            } => {
                let key = Self::object_key(object);
                self.sequences.retain(|sequence| {
                    !Self::object_matches(
                        object,
                        sequence.database.as_deref(),
                        sequence.schema.as_deref(),
                        &sequence.name,
                    )
                });
                let data_type_text = data_type
                    .map(|value| value.as_sql().to_string())
                    .unwrap_or_else(|| "bigint".to_string());
                self.sequences.push(SequenceSummary {
                    database: object.database.as_ref().map(ToString::to_string),
                    schema: object.schema.as_ref().map(ToString::to_string),
                    name: object.name.as_str().to_string(),
                    data_type: MetadataValue::available(data_type_text.clone()),
                    start_value: MetadataValue::available(start.unwrap_or(1).to_string()),
                    increment: MetadataValue::available(increment.unwrap_or(1).to_string()),
                    metadata: ObjectMetadata {
                        owner: MetadataValue::available("app".to_string()),
                        ..ObjectMetadata::default()
                    },
                });
                self.sequence_details.insert(
                    key,
                    SequenceDetails {
                        data_type: MetadataValue::available(data_type_text),
                        start_value: MetadataValue::available(start.unwrap_or(1).to_string()),
                        increment: MetadataValue::available(increment.unwrap_or(1).to_string()),
                        minimum: minimum.map_or(MetadataValue::Unavailable, |value| {
                            MetadataValue::available(value.to_string())
                        }),
                        maximum: maximum.map_or(MetadataValue::Unavailable, |value| {
                            MetadataValue::available(value.to_string())
                        }),
                        current_value: MetadataValue::available(start.unwrap_or(1).to_string()),
                        cache: cache.map_or(MetadataValue::Unavailable, |value| {
                            MetadataValue::available(value.to_string())
                        }),
                        cycle: MetadataValue::available(*cycle),
                    },
                );
            }
            ValidatedObjectAction::AlterSequence {
                object,
                increment,
                minimum,
                maximum,
                restart,
                cache,
                cycle,
            } => {
                let key = Self::object_key(object);
                let Some(sequence) = self.sequences.iter_mut().find(|sequence| {
                    Self::object_matches(
                        object,
                        sequence.database.as_deref(),
                        sequence.schema.as_deref(),
                        &sequence.name,
                    )
                }) else {
                    return Err(DatabaseError::query("The sequence does not exist."));
                };
                if let Some(increment) = increment {
                    sequence.increment = MetadataValue::available(increment.to_string());
                }
                let details = self
                    .sequence_details
                    .entry(key)
                    .or_insert_with(|| SequenceDetails {
                        data_type: sequence.data_type.clone(),
                        start_value: sequence.start_value.clone(),
                        increment: sequence.increment.clone(),
                        minimum: MetadataValue::Unavailable,
                        maximum: MetadataValue::Unavailable,
                        current_value: MetadataValue::available("1".to_string()),
                        cache: MetadataValue::Unavailable,
                        cycle: MetadataValue::available(false),
                    });
                if let Some(increment) = increment {
                    details.increment = MetadataValue::available(increment.to_string());
                }
                if let Some(minimum) = minimum {
                    details.minimum = MetadataValue::available(minimum.to_string());
                }
                if let Some(maximum) = maximum {
                    details.maximum = MetadataValue::available(maximum.to_string());
                }
                if let Some(restart) = restart {
                    details.current_value = MetadataValue::available(restart.to_string());
                }
                if let Some(cache) = cache {
                    details.cache = MetadataValue::available(cache.to_string());
                }
                if let Some(cycle) = cycle {
                    details.cycle = MetadataValue::available(*cycle);
                }
            }
            ValidatedObjectAction::DropSequence { object } => {
                self.sequences.retain(|sequence| {
                    !Self::object_matches(
                        object,
                        sequence.database.as_deref(),
                        sequence.schema.as_deref(),
                        &sequence.name,
                    )
                });
                self.sequence_details.remove(&Self::object_key(object));
            }
            ValidatedObjectAction::RefreshMaterializedView { object } => {
                let exists = self.views.iter().any(|view| {
                    view.kind == ViewKind::MaterializedView
                        && Self::object_matches(
                            object,
                            view.database.as_deref(),
                            view.schema.as_deref(),
                            &view.name,
                        )
                });
                if !exists {
                    return Err(DatabaseError::query(
                        "The materialized view does not exist.",
                    ));
                }
            }
            ValidatedObjectAction::DropMaterializedView { object } => {
                self.views.retain(|view| {
                    !(view.kind == ViewKind::MaterializedView
                        && Self::object_matches(
                            object,
                            view.database.as_deref(),
                            view.schema.as_deref(),
                            &view.name,
                        ))
                });
                self.view_queries.remove(&Self::object_key(object));
            }
        }
        Ok(())
    }

    async fn object_counts(&self, scope: &MetadataScope) -> Result<ObjectCounts, DatabaseError> {
        self.require_connection()?;
        let database = scope.database.as_ref().map(|value| value.as_str());
        let schema = scope.schema.as_ref().map(|value| value.as_str());
        let matches = |database_name: Option<&str>, schema_name: Option<&str>| {
            database.is_none_or(|value| database_name == Some(value))
                && schema.is_none_or(|value| schema_name == Some(value))
        };
        let tables = self
            .tables
            .iter()
            .filter(|table| matches(table.database.as_deref(), table.schema.as_deref()))
            .count() as u64;
        let views = self
            .views
            .iter()
            .filter(|view| matches(view.database.as_deref(), view.schema.as_deref()))
            .count() as u64;
        let materialized_views = if self
            .capabilities
            .supports(DatabaseAction::BrowseMaterializedViews)
        {
            MetadataValue::available(
                self.views
                    .iter()
                    .filter(|view| {
                        view.kind == ViewKind::MaterializedView
                            && matches(view.database.as_deref(), view.schema.as_deref())
                    })
                    .count() as u64,
            )
        } else {
            MetadataValue::Unavailable
        };
        let sequences = if self.capabilities.supports(DatabaseAction::BrowseSequences) {
            MetadataValue::available(
                self.sequences
                    .iter()
                    .filter(|sequence| {
                        matches(sequence.database.as_deref(), sequence.schema.as_deref())
                    })
                    .count() as u64,
            )
        } else {
            MetadataValue::Unavailable
        };

        let routines = if self.capabilities.supports(DatabaseAction::BrowseRoutines) {
            MetadataValue::available(
                self.routines
                    .iter()
                    .filter(|routine| {
                        matches(routine.database.as_deref(), routine.schema.as_deref())
                    })
                    .count() as u64,
            )
        } else {
            MetadataValue::Unavailable
        };
        let triggers = if self.capabilities.supports(DatabaseAction::BrowseTriggers) {
            MetadataValue::available(
                self.triggers
                    .iter()
                    .filter(|trigger| {
                        matches(trigger.database.as_deref(), trigger.schema.as_deref())
                    })
                    .count() as u64,
            )
        } else {
            MetadataValue::Unavailable
        };

        Ok(ObjectCounts {
            tables: MetadataValue::available(tables),
            views: MetadataValue::available(views),
            materialized_views,
            sequences,
            routines,
            triggers,
        })
    }

    async fn execute_structure_action(
        &mut self,
        action: &ValidatedStructureAction,
    ) -> Result<(), DatabaseError> {
        self.require_connection()?;
        // Re-run the planner so capability and DDL errors happen before a
        // simulated mutation, just like the live adapters.
        self.preview_structure_action(action).await?;
        self.structure_execution_count
            .fetch_add(1, Ordering::SeqCst);

        if let Some(error) = self.structure_execution_error.take() {
            return Err(error);
        }
        if self.skip_structure_execution {
            return Ok(());
        }

        match action {
            ValidatedStructureAction::CreateTable(options) => {
                let (database, schema) = self.structure_scope(&options.table);
                if self.tables.iter().any(|table| {
                    table.name == options.table.name.as_str()
                        && table.database.as_deref() == Some(database.as_str())
                }) {
                    return Err(DatabaseError::query(format!(
                        "Table {} already exists.",
                        options.table.name
                    )));
                }
                let summary = TableSummary {
                    database: Some(database.clone()),
                    schema: Some(schema.clone()),
                    name: options.table.name.as_str().to_string(),
                    kind: TableKind::Table,
                    estimated_rows: MetadataValue::available(0),
                    metadata_refreshed_at: MetadataValue::unavailable(),
                    metadata: ObjectMetadata {
                        size_bytes: MetadataValue::available(0),
                        engine: MetadataValue::available(match self.engine {
                            DatabaseEngine::Mysql => "InnoDB".to_string(),
                            DatabaseEngine::Postgresql => "heap".to_string(),
                        }),
                        ..ObjectMetadata::default()
                    },
                };
                let columns = options
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| ColumnSummary {
                        name: column.name.as_str().to_string(),
                        data_type: column.data_type.clone(),
                        length: MetadataValue::available(None),
                        precision: MetadataValue::available(None),
                        scale: MetadataValue::available(None),
                        nullable: column.nullable,
                        default_value: MetadataValue::available(None),
                        ordinal_position: u32::try_from(index + 1).unwrap_or(u32::MAX),
                        primary_key: options.primary_key.iter().any(|key| key == &column.name),
                        identity: MetadataValue::available(column.identity),
                        generated: MetadataValue::available(column.generated.is_some()),
                        generation_expression: MetadataValue::available(
                            column
                                .generated
                                .as_ref()
                                .map(|generated| generated.expression.clone()),
                        ),
                        comment: MetadataValue::available(column.comment.clone()),
                        collation: MetadataValue::available(None),
                    })
                    .collect::<Vec<_>>();
                let table_name = options.table.name.as_str().to_string();
                self.tables.push(summary);
                self.created_tables.insert(table_name.clone(), columns);
                self.indexes.insert(table_name.clone(), Vec::new());
                self.constraints.insert(table_name, Vec::new());
            }
            ValidatedStructureAction::RenameTable { table, new_name } => {
                let old_name = table.name.as_str().to_string();
                let Some(summary) = self
                    .tables
                    .iter_mut()
                    .find(|summary| Self::table_matches(summary, table))
                else {
                    return Err(DatabaseError::query(format!(
                        "Table {} does not exist.",
                        table.name
                    )));
                };
                summary.name = new_name.as_str().to_string();
                if let Some(columns) = self.created_tables.remove(&old_name) {
                    self.created_tables
                        .insert(new_name.as_str().to_string(), columns);
                }
                if let Some(indexes) = self.indexes.remove(&old_name) {
                    self.indexes.insert(new_name.as_str().to_string(), indexes);
                }
                if let Some(constraints) = self.constraints.remove(&old_name) {
                    self.constraints
                        .insert(new_name.as_str().to_string(), constraints);
                }
            }
            ValidatedStructureAction::DuplicateTable { table, new_name } => {
                let Some(source) = self
                    .tables
                    .iter()
                    .find(|summary| Self::table_matches(summary, table))
                    .cloned()
                else {
                    return Err(DatabaseError::query(format!(
                        "Table {} does not exist.",
                        table.name
                    )));
                };
                if self
                    .tables
                    .iter()
                    .any(|summary| summary.name == new_name.as_str())
                {
                    return Err(DatabaseError::query(format!(
                        "Table {new_name} already exists."
                    )));
                }
                let source_name = table.name.as_str().to_string();
                let mut duplicate = source;
                duplicate.name = new_name.as_str().to_string();
                duplicate.estimated_rows = MetadataValue::available(0);
                self.tables.push(duplicate);
                if let Some(columns) = self.created_tables.get(&source_name).cloned() {
                    self.created_tables
                        .insert(new_name.as_str().to_string(), columns);
                }
                if let Some(indexes) = self.indexes.get(&source_name).cloned() {
                    self.indexes.insert(new_name.as_str().to_string(), indexes);
                }
                if let Some(constraints) = self.constraints.get(&source_name).cloned() {
                    self.constraints
                        .insert(new_name.as_str().to_string(), constraints);
                }
            }
            ValidatedStructureAction::TruncateTable { table } => {
                let Some(summary) = self
                    .tables
                    .iter_mut()
                    .find(|summary| Self::table_matches(summary, table))
                else {
                    return Err(DatabaseError::query(format!(
                        "Table {} does not exist.",
                        table.name
                    )));
                };
                summary.estimated_rows = MetadataValue::available(0);
            }
            ValidatedStructureAction::DropTable { table } => {
                let before = self.tables.len();
                self.tables
                    .retain(|summary| !Self::table_matches(summary, table));
                if self.tables.len() == before {
                    return Err(DatabaseError::query(format!(
                        "Table {} does not exist.",
                        table.name
                    )));
                }
                self.created_tables.remove(table.name.as_str());
                self.indexes.remove(table.name.as_str());
                self.constraints.remove(table.name.as_str());
            }
            ValidatedStructureAction::AddColumn {
                table,
                column,
                position,
            } => {
                let engine = self.engine;
                let columns = self
                    .created_tables
                    .get_mut(table.name.as_str())
                    .ok_or_else(|| {
                        DatabaseError::query(format!("Table {} does not exist.", table.name))
                    })?;
                if Self::column_index(columns, column.name.as_str(), engine).is_some() {
                    return Err(DatabaseError::query(format!(
                        "Column {} already exists on table {}.",
                        column.name, table.name
                    )));
                }
                let insert_at = match position {
                    Some(ColumnPosition::First) => 0,
                    Some(ColumnPosition::After { column: after }) => {
                        Self::column_index(columns, after.as_str(), engine)
                            .map(|index| index + 1)
                            .ok_or_else(|| {
                                DatabaseError::query(format!(
                                    "Column {after} does not exist on table {}.",
                                    table.name
                                ))
                            })?
                    }
                    None => columns.len(),
                };
                let ordinal = u32::try_from(insert_at + 1).unwrap_or(u32::MAX);
                columns.insert(insert_at, Self::definition_summary(column, ordinal));
                Self::renumber_columns(columns);
            }
            ValidatedStructureAction::AlterColumn {
                table,
                name,
                changes,
            } => {
                let engine = self.engine;
                let columns = self
                    .created_tables
                    .get_mut(table.name.as_str())
                    .ok_or_else(|| {
                        DatabaseError::query(format!("Table {} does not exist.", table.name))
                    })?;
                let index =
                    Self::column_index(columns, name.as_str(), engine).ok_or_else(|| {
                        DatabaseError::query(format!(
                            "Column {} does not exist on table {}.",
                            name, table.name
                        ))
                    })?;
                Self::apply_changes(&mut columns[index], changes);
            }
            ValidatedStructureAction::RenameColumn {
                table,
                name,
                new_name,
            } => {
                let engine = self.engine;
                let columns = self
                    .created_tables
                    .get_mut(table.name.as_str())
                    .ok_or_else(|| {
                        DatabaseError::query(format!("Table {} does not exist.", table.name))
                    })?;
                let index =
                    Self::column_index(columns, name.as_str(), engine).ok_or_else(|| {
                        DatabaseError::query(format!(
                            "Column {} does not exist on table {}.",
                            name, table.name
                        ))
                    })?;
                if Self::column_index(columns, new_name.as_str(), engine).is_some() {
                    return Err(DatabaseError::query(format!(
                        "Column {} already exists on table {}.",
                        new_name, table.name
                    )));
                }
                columns[index].name = new_name.as_str().to_string();
                if let Some(indexes) = self.indexes.get_mut(table.name.as_str()) {
                    for index in indexes {
                        rename_column_in_list(&mut index.columns, name.as_str(), new_name.as_str());
                    }
                }
                if let Some(constraints) = self.constraints.get_mut(table.name.as_str()) {
                    for constraint in constraints {
                        rename_column_in_list(
                            &mut constraint.columns,
                            name.as_str(),
                            new_name.as_str(),
                        );
                        rename_column_in_list(
                            &mut constraint.referenced_columns,
                            name.as_str(),
                            new_name.as_str(),
                        );
                    }
                }
            }
            ValidatedStructureAction::DropColumn { table, name } => {
                let engine = self.engine;
                let columns = self
                    .created_tables
                    .get_mut(table.name.as_str())
                    .ok_or_else(|| {
                        DatabaseError::query(format!("Table {} does not exist.", table.name))
                    })?;
                let index =
                    Self::column_index(columns, name.as_str(), engine).ok_or_else(|| {
                        DatabaseError::query(format!(
                            "Column {} does not exist on table {}.",
                            name, table.name
                        ))
                    })?;
                columns.remove(index);
                Self::renumber_columns(columns);
                if let Some(indexes) = self.indexes.get_mut(table.name.as_str()) {
                    for index in indexes.iter_mut() {
                        index.columns.retain(|column| column != name.as_str());
                    }
                    indexes.retain(|index| !index.columns.is_empty());
                }
                if let Some(constraints) = self.constraints.get_mut(table.name.as_str()) {
                    for constraint in constraints.iter_mut() {
                        constraint.columns.retain(|column| column != name.as_str());
                    }
                    constraints.retain(|constraint| {
                        constraint.kind == ConstraintKind::Check || !constraint.columns.is_empty()
                    });
                }
            }
            ValidatedStructureAction::ReorderColumn {
                table,
                column,
                position,
            } => {
                let engine = self.engine;
                let columns = self
                    .created_tables
                    .get_mut(table.name.as_str())
                    .ok_or_else(|| {
                        DatabaseError::query(format!("Table {} does not exist.", table.name))
                    })?;
                let index =
                    Self::column_index(columns, column.name.as_str(), engine).ok_or_else(|| {
                        DatabaseError::query(format!(
                            "Column {} does not exist on table {}.",
                            column.name, table.name
                        ))
                    })?;
                let mut moved = columns.remove(index);
                Self::apply_definition(&mut moved, column);
                let insert_at = match position {
                    ColumnPosition::First => 0,
                    ColumnPosition::After { column: after } => {
                        Self::column_index(columns, after.as_str(), engine)
                            .map(|index| index + 1)
                            .ok_or_else(|| {
                                DatabaseError::query(format!(
                                    "Column {after} does not exist on table {}.",
                                    table.name
                                ))
                            })?
                    }
                };
                columns.insert(insert_at, moved);
                Self::renumber_columns(columns);
            }
            ValidatedStructureAction::CreateIndex(options) => {
                let table_name = table_key(self, &options.table)?;
                let indexes = self.indexes.entry(table_name).or_default();
                if indexes
                    .iter()
                    .any(|index| index.name.eq_ignore_ascii_case(options.name.as_str()))
                {
                    return Err(DatabaseError::query(format!(
                        "Index {} already exists on table {}.",
                        options.name, options.table.name
                    )));
                }
                indexes.push(IndexSummary {
                    name: options.name.as_str().to_string(),
                    columns: options
                        .columns
                        .iter()
                        .map(|column| column.as_str().to_string())
                        .collect(),
                    unique: options.unique,
                    primary: false,
                    method: MetadataValue::available(options.method.clone().unwrap_or_else(|| {
                        match self.engine {
                            DatabaseEngine::Mysql => "BTREE".to_string(),
                            DatabaseEngine::Postgresql => "btree".to_string(),
                        }
                    })),
                    predicate: match self.engine {
                        DatabaseEngine::Mysql => MetadataValue::Unavailable,
                        DatabaseEngine::Postgresql => {
                            MetadataValue::available(options.predicate.clone())
                        }
                    },
                });
            }
            ValidatedStructureAction::RenameIndex {
                table,
                name,
                new_name,
            } => {
                let table_name = table_key(self, table)?;
                let indexes = self.indexes.entry(table_name).or_default();
                let Some(position) = indexes
                    .iter()
                    .position(|index| index.name.eq_ignore_ascii_case(name.as_str()))
                else {
                    return Err(DatabaseError::query(format!(
                        "Index {} does not exist on table {}.",
                        name, table.name
                    )));
                };
                if indexes
                    .iter()
                    .any(|candidate| candidate.name.eq_ignore_ascii_case(new_name.as_str()))
                {
                    return Err(DatabaseError::query(format!(
                        "Index {} already exists on table {}.",
                        new_name, table.name
                    )));
                }
                indexes[position].name = new_name.as_str().to_string();
            }
            ValidatedStructureAction::DropIndex { table, name } => {
                let table_name = table_key(self, table)?;
                let indexes = self.indexes.entry(table_name).or_default();
                let Some(position) = indexes
                    .iter()
                    .position(|index| index.name.eq_ignore_ascii_case(name.as_str()))
                else {
                    return Err(DatabaseError::query(format!(
                        "Index {} does not exist on table {}.",
                        name, table.name
                    )));
                };
                indexes.remove(position);
            }
            ValidatedStructureAction::AddConstraint { table, constraint } => {
                let table_name = table_key(self, table)?;
                let constraints = self.constraints.entry(table_name.clone()).or_default();
                if constraints.iter().any(|existing| {
                    existing
                        .name
                        .eq_ignore_ascii_case(constraint.name().as_str())
                }) {
                    return Err(DatabaseError::query(format!(
                        "Constraint {} already exists on table {}.",
                        constraint.name(),
                        table.name
                    )));
                }
                if constraint.kind() == ConstraintKind::PrimaryKey
                    && constraints
                        .iter()
                        .any(|existing| existing.kind == ConstraintKind::PrimaryKey)
                {
                    return Err(DatabaseError::query(format!(
                        "Table {} already has a primary key.",
                        table.name
                    )));
                }

                constraints.push(fake_constraint(constraint, self.engine));
                match constraint {
                    ValidatedConstraint::PrimaryKey { name, columns } => {
                        if let Some(created) = self.created_tables.get_mut(&table_name) {
                            for column in created {
                                column.primary_key = columns
                                    .iter()
                                    .any(|key| key.as_str().eq_ignore_ascii_case(&column.name));
                            }
                        }
                        self.indexes
                            .entry(table_name)
                            .or_default()
                            .push(IndexSummary {
                                name: match self.engine {
                                    DatabaseEngine::Mysql => "PRIMARY".to_string(),
                                    DatabaseEngine::Postgresql => name.as_str().to_string(),
                                },
                                columns: columns
                                    .iter()
                                    .map(|column| column.as_str().to_string())
                                    .collect(),
                                unique: true,
                                primary: true,
                                method: MetadataValue::available(match self.engine {
                                    DatabaseEngine::Mysql => "BTREE".to_string(),
                                    DatabaseEngine::Postgresql => "btree".to_string(),
                                }),
                                predicate: MetadataValue::available(None),
                            });
                    }
                    ValidatedConstraint::Unique { name, columns } => {
                        self.indexes
                            .entry(table_name)
                            .or_default()
                            .push(IndexSummary {
                                name: name.as_str().to_string(),
                                columns: columns
                                    .iter()
                                    .map(|column| column.as_str().to_string())
                                    .collect(),
                                unique: true,
                                primary: false,
                                method: MetadataValue::available(match self.engine {
                                    DatabaseEngine::Mysql => "BTREE".to_string(),
                                    DatabaseEngine::Postgresql => "btree".to_string(),
                                }),
                                predicate: MetadataValue::available(None),
                            });
                    }
                    ValidatedConstraint::ForeignKey { .. } | ValidatedConstraint::Check { .. } => {}
                }
            }
            ValidatedStructureAction::DropConstraint { table, name, kind } => {
                let table_name = table_key(self, table)?;
                let constraints = self.constraints.entry(table_name.clone()).or_default();
                let Some(position) = constraints.iter().position(|constraint| {
                    constraint.kind == *kind
                        && (constraint.name.eq_ignore_ascii_case(name.as_str())
                            || (self.engine == DatabaseEngine::Mysql
                                && *kind == ConstraintKind::PrimaryKey))
                }) else {
                    return Err(DatabaseError::query(format!(
                        "Constraint {} does not exist on table {}.",
                        name, table.name
                    )));
                };
                constraints.remove(position);
                if matches!(kind, ConstraintKind::PrimaryKey | ConstraintKind::Unique)
                    && let Some(indexes) = self.indexes.get_mut(&table_name)
                {
                    indexes.retain(|index| match kind {
                        ConstraintKind::PrimaryKey => !index.primary,
                        ConstraintKind::Unique => !index.name.eq_ignore_ascii_case(name.as_str()),
                        _ => true,
                    });
                }
                if *kind == ConstraintKind::PrimaryKey
                    && let Some(columns) = self.created_tables.get_mut(&table_name)
                {
                    for column in columns {
                        column.primary_key = false;
                    }
                }
            }
        }

        Ok(())
    }

    async fn list_columns(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ColumnSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseColumns)?;
        self.require_connection()?;

        let Some(summary) = self
            .tables
            .iter()
            .find(|summary| Self::table_matches(summary, table))
        else {
            return Ok(Page::from_items(Vec::new(), page, Some(0)));
        };
        if let Some(columns) = self.created_tables.get(&summary.name) {
            return Ok(Self::paginate(columns, page));
        }
        if summary.name != self.detail_table {
            return Ok(Page::from_items(Vec::new(), page, Some(0)));
        }

        Ok(Self::paginate(&self.columns, page))
    }

    async fn list_indexes(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<IndexSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseIndexes)?;
        self.require_connection()?;

        let Some(summary) = self
            .tables
            .iter()
            .find(|summary| Self::table_matches(summary, table))
        else {
            return Ok(Page::from_items(Vec::new(), page, Some(0)));
        };

        Ok(Self::paginate(
            self.indexes
                .get(&summary.name)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            page,
        ))
    }

    async fn list_constraints(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ConstraintSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseConstraints)?;
        self.require_connection()?;

        let Some(summary) = self
            .tables
            .iter()
            .find(|summary| Self::table_matches(summary, table))
        else {
            return Ok(Page::from_items(Vec::new(), page, Some(0)));
        };

        Ok(Self::paginate(
            self.constraints
                .get(&summary.name)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            page,
        ))
    }

    async fn execute_query(
        &self,
        request: &QueryRequest,
        control: &QueryControl,
    ) -> Result<QueryExecutionResult, DatabaseError> {
        self.require_capability(DatabaseAction::RunQuery)?;
        self.require_connection()?;
        self.query_execution_count.fetch_add(1, Ordering::SeqCst);

        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The SQL execution was cancelled."));
        }
        if let Some(error) = &self.query_error {
            return Err(error.clone());
        }

        let analysis = request.analyze(self.engine)?;
        if self.read_only && analysis.has_writes {
            return Err(DatabaseError::permission(
                "The database session is read-only; the SQL attempted a write.",
            ));
        }

        let started = std::time::Instant::now();
        let statements = split_sql_statements(self.engine, &request.sql)?;
        let mut collector = QueryResultCollector::new(control);

        for statement in statements {
            let first = statement.tokens.iter().find_map(|token| match token {
                SqlToken::Word(word) => Some(word.as_str()),
                SqlToken::Symbol(_) => None,
            });
            match first {
                Some("select" | "show" | "describe" | "desc" | "values" | "table") => {
                    let many = statement.tokens.iter().any(|token| {
                        matches!(token, SqlToken::Word(word) if word == "many" || word == "series")
                    });
                    let values = if many {
                        (1..=6).map(|value| value.to_string()).collect::<Vec<_>>()
                    } else {
                        vec![
                            statement
                                .tokens
                                .get(1)
                                .and_then(|token| match token {
                                    SqlToken::Word(word) => Some(word.clone()),
                                    SqlToken::Symbol(_) => None,
                                })
                                .unwrap_or_else(|| "1".to_string()),
                        ]
                    };
                    let affected = values.len() as u64;
                    let columns = vec![QueryColumn {
                        name: "value".to_string(),
                        data_type: "text".to_string(),
                    }];
                    for value in values {
                        collector.push_row(columns.clone(), vec![Some(value)])?;
                    }
                    collector.push_command(affected, None);
                }
                Some("insert") => collector.push_command(1, Some(7)),
                Some("update" | "delete") => collector.push_command(1, None),
                _ => collector.push_command(0, None),
            }
        }

        let (results, truncated) = collector.finish();
        Ok(QueryExecutionResult {
            engine: self.engine,
            read_only: self.read_only,
            results,
            warnings: analysis.warnings,
            destructive: analysis.destructive,
            requires_confirmation: analysis.requires_confirmation,
            truncated,
            max_rows: control.max_rows,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    async fn read_rows(
        &self,
        request: &RowReadRequest,
        control: &RowQueryControl,
    ) -> Result<RowReadResult, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseRows)?;
        self.require_connection()?;
        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The row read was cancelled."));
        }
        if let Some(error) = &self.row_read_error {
            return Err(error.clone());
        }

        let fixture = self
            .row_tables
            .iter()
            .find(|table| table.matches(&request.table))
            .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;
        if fixture.columns.is_empty() {
            return Err(DatabaseError::query(
                "The selected table does not expose any columns.",
            ));
        }

        let metadata = RowTableMetadata {
            engine: self.engine,
            database: fixture
                .table
                .database
                .as_ref()
                .map(|value| value.as_str().to_string()),
            schema: fixture
                .table
                .schema
                .as_ref()
                .map(|value| value.as_str().to_string()),
            table: fixture.table.name.clone(),
            columns: fixture.columns.clone(),
            identity: fixture.identity.clone(),
            estimated_rows: fixture.estimated_rows.clone(),
        };
        let plan = resolve_read_plan(request, &metadata)?;
        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The row read was cancelled."));
        }

        let mut matched = Vec::new();
        for (position, values) in fixture.rows.iter().enumerate() {
            if values.len() != metadata.columns.len() {
                return Err(DatabaseError::internal(
                    "A fake row fixture has the wrong number of values.",
                ));
            }
            if Self::row_matches(values, &metadata, &plan) {
                matched.push((position, values));
            }
        }

        let mut order = plan.sort.clone();
        let mut seen: std::collections::HashSet<usize> =
            order.iter().map(|(index, _)| *index).collect();
        for index in metadata.identity_indices() {
            if seen.insert(index) {
                order.push((index, RowSortDirection::Asc));
            }
        }
        if !order.is_empty() {
            matched.sort_by(|left, right| {
                for (index, direction) in &order {
                    let ordering = compare_cells(
                        &left.1[*index],
                        &right.1[*index],
                        metadata.columns[*index].kind,
                    );
                    let ordering = match direction {
                        RowSortDirection::Asc => ordering,
                        RowSortDirection::Desc => ordering.reverse(),
                    };
                    if ordering != CmpOrdering::Equal {
                        return ordering;
                    }
                }
                left.0.cmp(&right.0)
            });
        }

        let total = matched.len() as u64;
        let start = usize::try_from(request.page.offset)
            .unwrap_or(usize::MAX)
            .min(matched.len());
        let end = start
            .saturating_add(request.page.limit as usize)
            .min(matched.len());
        let has_more = end < matched.len();
        let records = matched[start..end]
            .iter()
            .map(|(_, values)| Self::record_from_values(&metadata, &plan, values))
            .collect::<Result<Vec<_>, _>>()?;
        let exact_rows = request.include_exact_count.then(|| RowExactCount {
            value: total,
            filtered: !request.filters.is_empty()
                || request
                    .search
                    .as_ref()
                    .is_some_and(|search| !search.query.trim().is_empty()),
        });

        Ok(RowReadResult::new(
            metadata,
            &plan,
            records,
            request.page,
            has_more,
            exact_rows,
            self.read_only,
        ))
    }

    async fn export_csv(
        &mut self,
        source: &CsvExportSource,
        sink: &mut dyn CsvExportSink,
        control: &RowQueryControl,
    ) -> Result<CsvExportSummary, DatabaseError> {
        match source {
            CsvExportSource::Table { .. } => {
                self.require_capability(DatabaseAction::BrowseRows)?;
            }
            CsvExportSource::Query { .. } => {
                self.require_capability(DatabaseAction::RunQuery)?;
            }
        }
        self.require_connection()?;
        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The CSV export was cancelled."));
        }
        match source {
            CsvExportSource::Table { table } => {
                let fixture = self
                    .row_tables
                    .iter()
                    .find(|fixture| fixture.matches(table))
                    .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;
                let columns = fixture
                    .columns
                    .iter()
                    .map(|column| column.name.as_str().to_string())
                    .collect::<Vec<_>>();
                sink.write_header(&columns)?;
                let mut rows = 0u64;
                for row in &fixture.rows {
                    if control.is_cancelled() {
                        return Err(DatabaseError::cancelled("The CSV export was cancelled."));
                    }
                    if row.len() != columns.len() {
                        return Err(DatabaseError::internal(
                            "A fake row fixture has the wrong number of values.",
                        ));
                    }
                    sink.write_row(row)?;
                    rows = rows.saturating_add(1);
                }
                Ok(CsvExportSummary { columns, rows })
            }
            CsvExportSource::Query { sql } => {
                let request = QueryRequest::try_new(sql.clone(), None)?;
                let analysis = request.analyze(self.engine)?;
                if analysis.has_writes {
                    return Err(DatabaseError::validation(
                        "CSV export only runs read-only SQL. The statement would write data or schema.",
                    ));
                }
                let query_control = QueryControl::new(
                    control.timeout,
                    None,
                    crate::database::queries::DEFAULT_QUERY_RESULT_LIMIT,
                );
                let execution = self.execute_query(&request, &query_control).await?;
                let Some(result_set) = execution.results.first() else {
                    return Ok(CsvExportSummary {
                        columns: Vec::new(),
                        rows: 0,
                    });
                };
                let columns = result_set
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect::<Vec<_>>();
                sink.write_header(&columns)?;
                let mut rows = 0u64;
                for row in &result_set.rows {
                    if control.is_cancelled() {
                        return Err(DatabaseError::cancelled("The CSV export was cancelled."));
                    }
                    let values = row
                        .iter()
                        .map(|cell| cell.value.clone())
                        .collect::<Vec<_>>();
                    sink.write_row(&values)?;
                    rows = rows.saturating_add(1);
                }
                Ok(CsvExportSummary { columns, rows })
            }
        }
    }

    async fn import_csv(
        &mut self,
        request: &CsvImportRequest,
        source: &mut dyn CsvImportSource,
        observer: &mut dyn CsvImportObserver,
        control: &RowQueryControl,
    ) -> Result<CsvImportSummary, DatabaseError> {
        self.require_capability(DatabaseAction::InsertRow)?;
        self.require_connection()?;
        if self.read_only {
            return Err(DatabaseError::permission(
                "The database session is read-only; CSV imports are disabled.",
            ));
        }
        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The CSV import was cancelled."));
        }

        let fixture_index = self
            .row_tables
            .iter()
            .position(|fixture| fixture.matches(&request.table))
            .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;
        let metadata = {
            let fixture = &self.row_tables[fixture_index];
            self.row_metadata(fixture)
        };
        let mut mapping = Vec::with_capacity(request.mappings.len());
        let mut seen_sources = std::collections::HashSet::new();
        let mut seen_targets = std::collections::HashSet::new();
        for entry in &request.mappings {
            let source_index = usize::try_from(entry.source_index).unwrap_or(usize::MAX);
            if source_index >= source.headers().len() {
                return Err(DatabaseError::validation(format!(
                    "Source column {} does not exist in the CSV header.",
                    entry.source_index
                )));
            }
            if !seen_sources.insert(source_index) {
                return Err(DatabaseError::validation(format!(
                    "Source column {} is mapped more than once.",
                    entry.source_index
                )));
            }
            if !seen_targets.insert(entry.target_column.as_str()) {
                return Err(DatabaseError::validation(format!(
                    "Destination column {} is mapped more than once.",
                    entry.target_column
                )));
            }
            let target = metadata
                .columns
                .iter()
                .position(|column| column.name == entry.target_column)
                .ok_or_else(|| {
                    DatabaseError::validation(format!(
                        "Destination column {} does not exist in table {}.",
                        entry.target_column, metadata.table
                    ))
                })?;
            let column = metadata.column(target)?;
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
            mapping.push((source_index, target));
        }
        if mapping.is_empty() {
            return Err(DatabaseError::validation(
                "Map at least one source column to a destination column.",
            ));
        }

        let defaults = self.row_tables[fixture_index].defaults.clone();
        let mut staged = Vec::new();
        let mut warnings = Vec::new();
        let mut warnings_truncated = false;
        let mut conversion_skipped = 0u64;
        while let Some(row) = source.next_row()? {
            if control.is_cancelled() {
                return Err(DatabaseError::cancelled("The CSV import was cancelled."));
            }
            let values = mapping
                .iter()
                .map(|(source_index, _)| {
                    let field = &row.fields[*source_index];
                    if request.options.null_mode.is_null(field) {
                        None
                    } else {
                        Some(field.value.clone())
                    }
                })
                .collect::<Vec<_>>();
            let targets = mapping
                .iter()
                .map(|(_, target)| *target)
                .collect::<Vec<_>>();
            let resolved = match resolve_import_values(&metadata, &targets, &values) {
                Ok(resolved) => resolved,
                Err(error) => {
                    conversion_skipped = conversion_skipped.saturating_add(1);
                    let warning = CsvWarning {
                        row_number: row.record_number,
                        column_number: None,
                        column_name: None,
                        message: error.message,
                    };
                    if warnings.len() < MAX_CSV_REPORTED_ISSUES {
                        warnings.push(warning.clone());
                    } else {
                        warnings_truncated = true;
                    }
                    observer.row_skipped(&warning);
                    continue;
                }
            };
            let mut full_row = (0..metadata.columns.len())
                .map(|index| defaults.get(index).cloned().flatten())
                .collect::<Vec<_>>();
            let mut row_failed = false;
            for value in &resolved {
                let column = metadata.column(value.column)?;
                full_row[value.column] = match &value.value {
                    ResolvedWriteValue::Null => None,
                    ResolvedWriteValue::Default => defaults.get(value.column).cloned().flatten(),
                    ResolvedWriteValue::Value(_) => value.text.clone(),
                };
                if full_row[value.column].is_none() && !column.nullable {
                    row_failed = true;
                    break;
                }
            }
            if row_failed {
                conversion_skipped = conversion_skipped.saturating_add(1);
                let warning = CsvWarning {
                    row_number: row.record_number,
                    column_number: None,
                    column_name: None,
                    message: format!(
                        "Row {} leaves a non-nullable column without a value and was skipped.",
                        row.record_number
                    ),
                };
                if warnings.len() < MAX_CSV_REPORTED_ISSUES {
                    warnings.push(warning.clone());
                } else {
                    warnings_truncated = true;
                }
                observer.row_skipped(&warning);
                continue;
            }
            staged.push(full_row);
            observer.row_imported(row.record_number);
        }
        for issue in source.take_issues() {
            if warnings.len() < MAX_CSV_REPORTED_ISSUES {
                warnings.push(issue);
            } else {
                warnings_truncated = true;
            }
        }
        warnings_truncated |= source.issues_truncated();
        let imported = u64::try_from(staged.len()).unwrap_or(u64::MAX);
        let rows_skipped = source.skipped_rows().saturating_add(conversion_skipped);
        self.row_tables[fixture_index].rows.extend(staged);
        Ok(CsvImportSummary {
            table: request.table.clone(),
            engine: self.engine,
            rows_read: source.rows_read(),
            rows_imported: imported,
            rows_skipped,
            warnings,
            warnings_truncated,
        })
    }

    async fn insert_row(
        &mut self,
        request: &RowInsertRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError> {
        self.require_capability(DatabaseAction::InsertRow)?;
        self.require_connection()?;
        if self.read_only {
            return Err(DatabaseError::permission(
                "The database session is read-only; row inserts are disabled.",
            ));
        }
        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The row write was cancelled."));
        }
        self.row_write_count += 1;
        if let Some(error) = self.row_write_error.take() {
            self.row_rollback_count += 1;
            return Err(crate::database::row_writes::rolled_back(error));
        }

        let metadata = {
            let fixture = self
                .row_tables
                .iter()
                .find(|table| table.matches(&request.table))
                .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;
            self.row_metadata(fixture)
        };
        let plan = resolve_insert(request, &metadata)?;
        let defaults = self
            .row_tables
            .iter()
            .find(|table| table.matches(&request.table))
            .map(|table| table.defaults.clone())
            .unwrap_or_default();
        let default_for =
            |index: usize| -> Option<String> { defaults.get(index).cloned().flatten() };
        let mut values = (0..metadata.columns.len())
            .map(default_for)
            .collect::<Vec<_>>();

        for value in &plan.values {
            let column = metadata.column(value.column)?;
            values[value.column] = match &value.value {
                ResolvedWriteValue::Null => None,
                ResolvedWriteValue::Default => default_for(value.column),
                ResolvedWriteValue::Value(_) => value.text.clone(),
            };
            if values[value.column].is_none()
                && !column.nullable
                && column.identity != crate::database::ColumnIdentity::AutoIncrement
            {
                return Err(DatabaseError::validation(format!(
                    "Column {} is not nullable and has no default.",
                    column.name
                )));
            }
        }

        let identity_indices = metadata.identity_indices();
        if identity_indices.len() == 1 {
            let index = identity_indices[0];
            if metadata.columns[index].identity == crate::database::ColumnIdentity::AutoIncrement
                && values[index].is_none()
            {
                let next = self
                    .row_tables
                    .iter()
                    .find(|table| table.matches(&request.table))
                    .map(|table| {
                        table
                            .rows
                            .iter()
                            .filter_map(|row| row.get(index).and_then(|value| value.as_deref()))
                            .filter_map(|value| value.parse::<i64>().ok())
                            .max()
                            .unwrap_or(0)
                            + 1
                    })
                    .unwrap_or(1);
                values[index] = Some(next.to_string());
            }
        }

        for (index, column) in metadata.columns.iter().enumerate() {
            if values[index].is_none() && !column.nullable {
                return Err(DatabaseError::validation(format!(
                    "Column {} is not nullable and has no default.",
                    column.name
                )));
            }
        }

        let fixture = self
            .row_tables
            .iter_mut()
            .find(|table| table.matches(&request.table))
            .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;
        let duplicate = fixture.rows.iter().any(|row| {
            identity_indices
                .iter()
                .all(|index| row.get(*index) == values.get(*index))
        });
        if duplicate {
            self.row_rollback_count += 1;
            return Err(DatabaseError::dependency(
                "A row with the same stable identity already exists. No row was inserted.",
            ));
        }

        fixture.rows.push(values.clone());
        let row = Self::row_record(&metadata, &values);
        Ok(RowWriteResult::committed(
            RowWriteOperation::Insert,
            request.table.clone(),
            1,
            "Inserted 1 row into the fake fixture and re-read it by its stable identity."
                .to_string(),
            Some(row),
        ))
    }

    async fn update_row(
        &mut self,
        request: &RowUpdateRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError> {
        self.require_capability(DatabaseAction::EditRow)?;
        self.require_connection()?;
        if self.read_only {
            return Err(DatabaseError::permission(
                "The database session is read-only; row updates are disabled.",
            ));
        }
        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The row write was cancelled."));
        }
        self.row_write_count += 1;
        if let Some(error) = self.row_write_error.take() {
            self.row_rollback_count += 1;
            return Err(crate::database::row_writes::rolled_back(error));
        }

        let metadata = {
            let fixture = self
                .row_tables
                .iter()
                .find(|table| table.matches(&request.table))
                .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;
            self.row_metadata(fixture)
        };
        let plan = resolve_update(request, &metadata)?;
        let fixture = self
            .row_tables
            .iter_mut()
            .find(|table| table.matches(&request.table))
            .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;

        let Some(row_index) = fixture.rows.iter().position(|row| {
            plan.identity.iter().all(|(index, _, text)| {
                row_cell_matches(&metadata.columns[*index], row.get(*index), text)
            })
        }) else {
            self.row_rollback_count += 1;
            return Err(DatabaseError::dependency(
                "The row no longer exists. It may have been deleted by another session.",
            ));
        };

        for expected in &plan.expected {
            let column = metadata.column(expected.column)?;
            let actual = fixture.rows[row_index]
                .get(expected.column)
                .and_then(Option::as_deref);
            let matches = match (&expected.value, actual) {
                (None, None) => true,
                (Some(expected), Some(actual)) => {
                    text_matches_column(column, expected, actual) != Some(false)
                }
                _ => false,
            };
            if !matches {
                self.row_rollback_count += 1;
                return Err(DatabaseError::dependency(
                    "The row changed in another session before this update could be applied.",
                ));
            }
        }

        for value in &plan.values {
            let column = metadata.column(value.column)?;
            fixture.rows[row_index][value.column] = match &value.value {
                ResolvedWriteValue::Null => None,
                ResolvedWriteValue::Default => None,
                ResolvedWriteValue::Value(_) => value.text.clone(),
            };
            if fixture.rows[row_index][value.column].is_none() && !column.nullable {
                self.row_rollback_count += 1;
                return Err(DatabaseError::validation(format!(
                    "Column {} is not nullable.",
                    column.name
                )));
            }
        }

        let row = Self::row_record(&metadata, &fixture.rows[row_index]);
        Ok(RowWriteResult::committed(
            RowWriteOperation::Update,
            request.table.clone(),
            1,
            "Updated 1 row in the fake fixture and re-read the changed values.".to_string(),
            Some(row),
        ))
    }

    async fn delete_rows(
        &mut self,
        request: &RowDeleteRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError> {
        self.require_capability(DatabaseAction::DeleteRow)?;
        self.require_connection()?;
        if self.read_only {
            return Err(DatabaseError::permission(
                "The database session is read-only; row deletes are disabled.",
            ));
        }
        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The row write was cancelled."));
        }
        self.row_write_count += 1;
        if let Some(error) = self.row_write_error.take() {
            self.row_rollback_count += 1;
            return Err(crate::database::row_writes::rolled_back(error));
        }

        let metadata = {
            let fixture = self
                .row_tables
                .iter()
                .find(|table| table.matches(&request.table))
                .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;
            self.row_metadata(fixture)
        };
        let plan = resolve_delete(request, &metadata)?;
        let fixture = self
            .row_tables
            .iter_mut()
            .find(|table| table.matches(&request.table))
            .ok_or_else(|| DatabaseError::query("The selected table was not found."))?;

        let mut indices = Vec::with_capacity(plan.targets.len());
        for target in &plan.targets {
            let Some(index) = fixture.rows.iter().position(|row| {
                target.iter().all(|(index, _, text)| {
                    row_cell_matches(&metadata.columns[*index], row.get(*index), text)
                })
            }) else {
                self.row_rollback_count += 1;
                return Err(DatabaseError::dependency(
                    "One or more selected rows no longer exist or were changed in another session. No rows were deleted.",
                ));
            };
            indices.push(index);
        }
        indices.sort_unstable();
        indices.dedup();
        for index in indices.into_iter().rev() {
            fixture.rows.remove(index);
        }
        let affected = u64::try_from(plan.targets.len()).unwrap_or(u64::MAX);
        Ok(RowWriteResult::committed(
            RowWriteOperation::Delete,
            request.table.clone(),
            affected,
            format!(
                "Deleted {affected} row{} from the fake fixture and verified the targets are absent.",
                if affected == 1 { "" } else { "s" }
            ),
            None,
        ))
    }

    async fn table_dependencies(
        &self,
        table: &TableRef,
        limit: u32,
    ) -> Result<TableDependencies, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        self.require_connection()?;

        let limit = limit.clamp(1, crate::database::MAX_PAGE_LIMIT) as usize;
        if !self
            .tables
            .iter()
            .any(|summary| Self::table_matches(summary, table) && summary.name == self.detail_table)
        {
            return Ok(TableDependencies {
                incoming_foreign_keys: MetadataValue::available(Bounded::new(Vec::new(), limit)),
                dependent_views: MetadataValue::available(Bounded::new(Vec::new(), limit)),
                owned_sequences: MetadataValue::available(Bounded::new(Vec::new(), limit)),
                triggers: MetadataValue::available(Bounded::new(Vec::new(), limit)),
            });
        }

        let bound = |value: &MetadataValue<Bounded<DependencySummary>>| -> MetadataValue<
            Bounded<DependencySummary>,
        > {
            value.clone().map(|group| Bounded::new(group.items, limit))
        };

        Ok(TableDependencies {
            incoming_foreign_keys: bound(&self.dependencies.incoming_foreign_keys),
            dependent_views: bound(&self.dependencies.dependent_views),
            owned_sequences: bound(&self.dependencies.owned_sequences),
            triggers: bound(&self.dependencies.triggers),
        })
    }
}

struct FakeFixtures {
    databases: Vec<DatabaseSummary>,
    schemas: Vec<SchemaSummary>,
    tables: Vec<TableSummary>,
    views: Vec<ViewSummary>,
    sequences: Vec<SequenceSummary>,
    routines: Vec<RoutineSummary>,
    triggers: Vec<TriggerSummary>,
    view_queries: HashMap<String, String>,
    routine_bodies: HashMap<String, String>,
    trigger_statements: HashMap<String, String>,
    trigger_functions: HashMap<String, String>,
    sequence_details: HashMap<String, SequenceDetails>,
    roles: Vec<RoleSummary>,
    role_grants: HashMap<String, RoleGrantData>,
    sessions: Vec<SessionSummary>,
    columns: Vec<ColumnSummary>,
    indexes: Vec<IndexSummary>,
    constraints: Vec<ConstraintSummary>,
    dependencies: TableDependencies,
    detail_table: String,
}

fn identifier(value: &str) -> Identifier {
    Identifier::new(value).expect("fixture identifier")
}

fn mysql_table_target(database: &str, table: &str) -> PrivilegeTarget {
    PrivilegeTarget {
        kind: crate::database::PrivilegeTargetKind::Table,
        database: Some(identifier(database)),
        schema: None,
        object: Some(identifier(table)),
    }
}

fn postgres_table_target(schema: &str, table: &str) -> PrivilegeTarget {
    PrivilegeTarget {
        kind: crate::database::PrivilegeTargetKind::Table,
        database: None,
        schema: Some(identifier(schema)),
        object: Some(identifier(table)),
    }
}

fn postgres_schema_target(schema: &str) -> PrivilegeTarget {
    PrivilegeTarget {
        kind: crate::database::PrivilegeTargetKind::Schema,
        database: None,
        schema: Some(identifier(schema)),
        object: None,
    }
}

fn postgres_database_target(database: &str) -> PrivilegeTarget {
    PrivilegeTarget {
        kind: crate::database::PrivilegeTargetKind::Database,
        database: Some(identifier(database)),
        schema: None,
        object: None,
    }
}

fn grant(privilege: &str, target: PrivilegeTarget, grantable: bool) -> PrivilegeGrant {
    PrivilegeGrant {
        privilege: privilege.to_string(),
        target,
        grantable,
    }
}

struct FakeRoleFixture {
    name: &'static str,
    host: Option<&'static str>,
    kind: RoleKind,
    can_login: bool,
    superuser: bool,
    create_database: bool,
    create_role: bool,
    inherit: bool,
}

fn role_summary(fixture: FakeRoleFixture, engine: DatabaseEngine) -> RoleSummary {
    RoleSummary {
        name: fixture.name.to_string(),
        host: fixture.host.map(str::to_string),
        kind: fixture.kind,
        can_login: MetadataValue::available(fixture.can_login),
        superuser: match engine {
            DatabaseEngine::Mysql => MetadataValue::Unavailable,
            DatabaseEngine::Postgresql => MetadataValue::available(fixture.superuser),
        },
        create_database: match engine {
            DatabaseEngine::Mysql => MetadataValue::Unavailable,
            DatabaseEngine::Postgresql => MetadataValue::available(fixture.create_database),
        },
        create_role: match engine {
            DatabaseEngine::Mysql => MetadataValue::Unavailable,
            DatabaseEngine::Postgresql => MetadataValue::available(fixture.create_role),
        },
        inherit: match engine {
            DatabaseEngine::Mysql => MetadataValue::Unavailable,
            DatabaseEngine::Postgresql => MetadataValue::available(fixture.inherit),
        },
    }
}

fn optional_metadata(value: Option<&str>) -> MetadataValue<String> {
    value.map_or(MetadataValue::Unavailable, |value| {
        MetadataValue::available(value.to_string())
    })
}

fn session_query(
    text: &str,
    started_at: Option<&str>,
    elapsed_ms: Option<u64>,
) -> MetadataValue<SessionQuery> {
    MetadataValue::available(SessionQuery::from_raw(
        text,
        optional_metadata(started_at),
        elapsed_ms.map_or(MetadataValue::Unavailable, MetadataValue::available),
    ))
}

fn mysql_fixtures() -> FakeFixtures {
    FakeFixtures {
        databases: vec![
            DatabaseSummary {
                name: "app".to_string(),
                system: false,
                metadata: ObjectMetadata {
                    encoding: MetadataValue::available("utf8mb4".to_string()),
                    collation: MetadataValue::available("utf8mb4_unicode_ci".to_string()),
                    size_bytes: MetadataValue::available(4_096),
                    ..ObjectMetadata::default()
                },
            },
            DatabaseSummary {
                name: "sys".to_string(),
                system: true,
                metadata: ObjectMetadata {
                    encoding: MetadataValue::available("utf8mb4".to_string()),
                    collation: MetadataValue::available("utf8mb4_0900_ai_ci".to_string()),
                    size_bytes: MetadataValue::available(1_024),
                    ..ObjectMetadata::default()
                },
            },
        ],
        schemas: vec![
            SchemaSummary {
                name: "app".to_string(),
                system: false,
                metadata: ObjectMetadata {
                    encoding: MetadataValue::available("utf8mb4".to_string()),
                    collation: MetadataValue::available("utf8mb4_unicode_ci".to_string()),
                    size_bytes: MetadataValue::available(4_096),
                    ..ObjectMetadata::default()
                },
            },
            SchemaSummary {
                name: "sys".to_string(),
                system: true,
                metadata: ObjectMetadata {
                    encoding: MetadataValue::available("utf8mb4".to_string()),
                    collation: MetadataValue::available("utf8mb4_0900_ai_ci".to_string()),
                    size_bytes: MetadataValue::available(1_024),
                    ..ObjectMetadata::default()
                },
            },
        ],
        tables: vec![
            TableSummary {
                database: Some("app".to_string()),
                schema: Some("app".to_string()),
                name: "users".to_string(),
                kind: TableKind::Table,
                estimated_rows: MetadataValue::available(2),
                metadata_refreshed_at: MetadataValue::available("2026-01-02T03:04:05".to_string()),
                metadata: ObjectMetadata {
                    size_bytes: MetadataValue::available(16_384),
                    engine: MetadataValue::available("InnoDB".to_string()),
                    ..ObjectMetadata::default()
                },
            },
            TableSummary {
                database: Some("app".to_string()),
                schema: Some("app".to_string()),
                name: "orders".to_string(),
                kind: TableKind::Table,
                estimated_rows: MetadataValue::available(0),
                metadata_refreshed_at: MetadataValue::Unavailable,
                metadata: ObjectMetadata {
                    size_bytes: MetadataValue::available(16_384),
                    engine: MetadataValue::available("InnoDB".to_string()),
                    ..ObjectMetadata::default()
                },
            },
        ],
        views: vec![ViewSummary {
            database: Some("app".to_string()),
            schema: Some("app".to_string()),
            name: "active_users".to_string(),
            kind: ViewKind::View,
            metadata: ObjectMetadata {
                owner: MetadataValue::available("app@%".to_string()),
                ..ObjectMetadata::default()
            },
        }],
        sequences: Vec::new(),
        routines: vec![RoutineSummary {
            database: Some("app".to_string()),
            schema: Some("app".to_string()),
            name: "user_email".to_string(),
            kind: RoutineKind::Function,
            language: MetadataValue::available("SQL".to_string()),
            return_type: MetadataValue::available("varchar(255)".to_string()),
            arguments: MetadataValue::available("user_id bigint".to_string()),
            owner: MetadataValue::available("app@%".to_string()),
        }],
        triggers: vec![TriggerSummary {
            database: Some("app".to_string()),
            schema: Some("app".to_string()),
            name: "users_audit".to_string(),
            table: "users".to_string(),
            timing: TriggerTiming::After,
            events: vec![TriggerEvent::Insert],
            owner: MetadataValue::available("app@%".to_string()),
        }],
        view_queries: HashMap::from([(
            "view::app::app::active_users".to_string(),
            "SELECT id, email FROM users WHERE active = 1".to_string(),
        )]),
        routine_bodies: HashMap::from([(
            "routine::app::app::user_email".to_string(),
            "RETURN (SELECT email FROM users WHERE id = user_id)".to_string(),
        )]),
        trigger_statements: HashMap::from([(
            "trigger::app::app::users_audit".to_string(),
            "INSERT INTO audit (user_id) VALUES (NEW.id)".to_string(),
        )]),
        trigger_functions: HashMap::new(),
        sequence_details: HashMap::new(),
        columns: vec![
            ColumnSummary {
                name: "id".to_string(),
                data_type: "bigint(20) unsigned".to_string(),
                length: MetadataValue::available(None),
                precision: MetadataValue::available(Some(20)),
                scale: MetadataValue::available(Some(0)),
                nullable: false,
                default_value: MetadataValue::available(None),
                ordinal_position: 1,
                primary_key: true,
                identity: MetadataValue::available(ColumnIdentity::AutoIncrement),
                generated: MetadataValue::available(false),
                generation_expression: MetadataValue::available(None),
                comment: MetadataValue::available(None),
                collation: MetadataValue::available(None),
            },
            ColumnSummary {
                name: "email".to_string(),
                data_type: "varchar(255)".to_string(),
                length: MetadataValue::available(Some(255)),
                precision: MetadataValue::available(None),
                scale: MetadataValue::available(None),
                nullable: false,
                default_value: MetadataValue::available(None),
                ordinal_position: 2,
                primary_key: false,
                identity: MetadataValue::available(ColumnIdentity::None),
                generated: MetadataValue::available(false),
                generation_expression: MetadataValue::available(None),
                comment: MetadataValue::available(Some("Login address".to_string())),
                collation: MetadataValue::available(Some("utf8mb4_unicode_ci".to_string())),
            },
        ],
        indexes: vec![
            IndexSummary {
                name: "PRIMARY".to_string(),
                columns: vec!["id".to_string()],
                unique: true,
                primary: true,
                method: MetadataValue::available("BTREE".to_string()),
                predicate: MetadataValue::Unavailable,
            },
            IndexSummary {
                name: "users_email_unique".to_string(),
                columns: vec!["email".to_string()],
                unique: true,
                primary: false,
                method: MetadataValue::available("BTREE".to_string()),
                predicate: MetadataValue::Unavailable,
            },
        ],
        constraints: vec![
            ConstraintSummary {
                name: "PRIMARY".to_string(),
                kind: ConstraintKind::PrimaryKey,
                columns: vec!["id".to_string()],
                referenced_schema: MetadataValue::available(None),
                referenced_table: MetadataValue::available(None),
                referenced_columns: Vec::new(),
                on_update: MetadataValue::available(None),
                on_delete: MetadataValue::available(None),
                check_expression: MetadataValue::available(None),
                deferrable: MetadataValue::Unavailable,
                initially_deferred: MetadataValue::Unavailable,
            },
            ConstraintSummary {
                name: "users_email_unique".to_string(),
                kind: ConstraintKind::Unique,
                columns: vec!["email".to_string()],
                referenced_schema: MetadataValue::available(None),
                referenced_table: MetadataValue::available(None),
                referenced_columns: Vec::new(),
                on_update: MetadataValue::available(None),
                on_delete: MetadataValue::available(None),
                check_expression: MetadataValue::available(None),
                deferrable: MetadataValue::Unavailable,
                initially_deferred: MetadataValue::Unavailable,
            },
        ],
        dependencies: TableDependencies {
            incoming_foreign_keys: MetadataValue::available(Bounded::new(
                vec![incoming_key(
                    "orders_user_fk",
                    "app",
                    "orders",
                    "user_id",
                    "id",
                    "CASCADE",
                    "RESTRICT",
                )],
                100,
            )),
            dependent_views: MetadataValue::Unavailable,
            owned_sequences: MetadataValue::Unavailable,
            triggers: MetadataValue::available(Bounded::new(Vec::new(), 100)),
        },
        roles: vec![
            role_summary(
                FakeRoleFixture {
                    name: "app",
                    host: Some("%"),
                    kind: RoleKind::User,
                    can_login: true,
                    superuser: false,
                    create_database: true,
                    create_role: false,
                    inherit: true,
                },
                DatabaseEngine::Mysql,
            ),
            role_summary(
                FakeRoleFixture {
                    name: "reporting",
                    host: Some("%"),
                    kind: RoleKind::User,
                    can_login: true,
                    superuser: false,
                    create_database: false,
                    create_role: false,
                    inherit: true,
                },
                DatabaseEngine::Mysql,
            ),
            role_summary(
                FakeRoleFixture {
                    name: "reporting_reader",
                    host: Some("%"),
                    kind: RoleKind::Role,
                    can_login: false,
                    superuser: false,
                    create_database: false,
                    create_role: false,
                    inherit: true,
                },
                DatabaseEngine::Mysql,
            ),
            role_summary(
                FakeRoleFixture {
                    name: "auditor",
                    host: Some("%"),
                    kind: RoleKind::Role,
                    can_login: false,
                    superuser: false,
                    create_database: false,
                    create_role: false,
                    inherit: true,
                },
                DatabaseEngine::Mysql,
            ),
        ],
        role_grants: HashMap::from([
            (
                "app@%".to_string(),
                RoleGrantData {
                    direct: vec![grant(
                        "SELECT",
                        PrivilegeTarget {
                            kind: crate::database::PrivilegeTargetKind::Global,
                            database: None,
                            schema: None,
                            object: None,
                        },
                        false,
                    )],
                    member_of: Vec::new(),
                },
            ),
            (
                "reporting@%".to_string(),
                RoleGrantData {
                    direct: vec![grant("SELECT", mysql_table_target("app", "orders"), false)],
                    member_of: vec![RoleRef {
                        name: "reporting_reader".to_string(),
                        host: Some("%".to_string()),
                    }],
                },
            ),
            (
                "reporting_reader@%".to_string(),
                RoleGrantData {
                    direct: vec![grant("SELECT", mysql_table_target("app", "users"), false)],
                    member_of: Vec::new(),
                },
            ),
            (
                "auditor@%".to_string(),
                RoleGrantData {
                    direct: vec![grant("SELECT", mysql_table_target("app", "orders"), true)],
                    member_of: Vec::new(),
                },
            ),
        ]),
        sessions: vec![
            SessionSummary {
                id: 101,
                user: "app".to_string(),
                host: optional_metadata(Some("10.0.0.5:51234")),
                database: optional_metadata(Some("app")),
                application_name: MetadataValue::unavailable(),
                command: optional_metadata(Some("Query")),
                state: optional_metadata(Some("Sending data")),
                wait_event: MetadataValue::unavailable(),
                backend_type: MetadataValue::unavailable(),
                connected_at: MetadataValue::unavailable(),
                is_current: false,
                query: session_query(
                    "SELECT id, status FROM orders WHERE status = 'pending'",
                    None,
                    Some(2_500),
                ),
            },
            SessionSummary {
                id: 102,
                user: "reporting".to_string(),
                host: optional_metadata(Some("10.0.0.6:49876")),
                database: optional_metadata(Some("app")),
                application_name: MetadataValue::unavailable(),
                command: optional_metadata(Some("Sleep")),
                state: optional_metadata(Some("Sleep")),
                wait_event: MetadataValue::unavailable(),
                backend_type: MetadataValue::unavailable(),
                connected_at: MetadataValue::unavailable(),
                is_current: false,
                query: MetadataValue::unavailable(),
            },
            SessionSummary {
                id: 103,
                user: "app".to_string(),
                host: optional_metadata(Some("127.0.0.1:3306")),
                database: optional_metadata(Some("app")),
                application_name: MetadataValue::unavailable(),
                command: optional_metadata(Some("Query")),
                state: optional_metadata(Some("executing")),
                wait_event: MetadataValue::unavailable(),
                backend_type: MetadataValue::unavailable(),
                connected_at: MetadataValue::unavailable(),
                is_current: true,
                query: session_query("SELECT 1", None, Some(10)),
            },
            SessionSummary {
                id: 104,
                user: "admin".to_string(),
                host: optional_metadata(Some("10.0.0.7:55100")),
                database: optional_metadata(Some("app")),
                application_name: MetadataValue::unavailable(),
                command: optional_metadata(Some("Query")),
                state: optional_metadata(Some("executing")),
                wait_event: MetadataValue::unavailable(),
                backend_type: MetadataValue::unavailable(),
                connected_at: MetadataValue::unavailable(),
                is_current: false,
                query: session_query(
                    "ALTER USER 'app'@'%' IDENTIFIED BY 'fixture-session-secret'",
                    None,
                    Some(1_500),
                ),
            },
        ],
        detail_table: "users".to_string(),
    }
}

fn postgres_fixtures() -> FakeFixtures {
    FakeFixtures {
        databases: vec![
            DatabaseSummary {
                name: "app".to_string(),
                system: false,
                metadata: ObjectMetadata {
                    owner: MetadataValue::available("app".to_string()),
                    encoding: MetadataValue::available("UTF8".to_string()),
                    collation: MetadataValue::available("en_US.UTF-8".to_string()),
                    size_bytes: MetadataValue::available(8_192),
                    ..ObjectMetadata::default()
                },
            },
            DatabaseSummary {
                name: "postgres".to_string(),
                system: true,
                metadata: ObjectMetadata {
                    owner: MetadataValue::available("postgres".to_string()),
                    encoding: MetadataValue::available("UTF8".to_string()),
                    collation: MetadataValue::available("C.UTF-8".to_string()),
                    size_bytes: MetadataValue::available(7_168),
                    ..ObjectMetadata::default()
                },
            },
        ],
        schemas: vec![
            SchemaSummary {
                name: "public".to_string(),
                system: false,
                metadata: ObjectMetadata {
                    owner: MetadataValue::available("pg_database_owner".to_string()),
                    ..ObjectMetadata::default()
                },
            },
            SchemaSummary {
                name: "pg_catalog".to_string(),
                system: true,
                metadata: ObjectMetadata {
                    owner: MetadataValue::available("postgres".to_string()),
                    ..ObjectMetadata::default()
                },
            },
        ],
        tables: vec![
            TableSummary {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "users".to_string(),
                kind: TableKind::Table,
                estimated_rows: MetadataValue::available(2),
                metadata_refreshed_at: MetadataValue::available("2026-01-02T03:04:05Z".to_string()),
                metadata: ObjectMetadata {
                    owner: MetadataValue::available("app".to_string()),
                    size_bytes: MetadataValue::available(8_192),
                    engine: MetadataValue::available("heap".to_string()),
                    ..ObjectMetadata::default()
                },
            },
            TableSummary {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "orders".to_string(),
                kind: TableKind::Table,
                estimated_rows: MetadataValue::available(0),
                metadata_refreshed_at: MetadataValue::Unavailable,
                metadata: ObjectMetadata {
                    owner: MetadataValue::available("app".to_string()),
                    size_bytes: MetadataValue::available(8_192),
                    engine: MetadataValue::available("heap".to_string()),
                    ..ObjectMetadata::default()
                },
            },
        ],
        views: vec![
            ViewSummary {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "active_users".to_string(),
                kind: ViewKind::View,
                metadata: ObjectMetadata {
                    owner: MetadataValue::available("app".to_string()),
                    size_bytes: MetadataValue::available(0),
                    ..ObjectMetadata::default()
                },
            },
            ViewSummary {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "daily_totals".to_string(),
                kind: ViewKind::MaterializedView,
                metadata: ObjectMetadata {
                    owner: MetadataValue::available("app".to_string()),
                    size_bytes: MetadataValue::available(8_192),
                    ..ObjectMetadata::default()
                },
            },
        ],
        sequences: vec![SequenceSummary {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "users_id_seq".to_string(),
            data_type: MetadataValue::available("bigint".to_string()),
            start_value: MetadataValue::available("1".to_string()),
            increment: MetadataValue::available("1".to_string()),
            metadata: ObjectMetadata {
                owner: MetadataValue::available("app".to_string()),
                size_bytes: MetadataValue::available(8_192),
                ..ObjectMetadata::default()
            },
        }],
        routines: vec![RoutineSummary {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "user_email".to_string(),
            kind: RoutineKind::Function,
            language: MetadataValue::available("sql".to_string()),
            return_type: MetadataValue::available("text".to_string()),
            arguments: MetadataValue::available("user_id bigint".to_string()),
            owner: MetadataValue::available("app".to_string()),
        }],
        triggers: vec![TriggerSummary {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "users_audit".to_string(),
            table: "users".to_string(),
            timing: TriggerTiming::After,
            events: vec![TriggerEvent::Insert],
            owner: MetadataValue::available("app".to_string()),
        }],
        view_queries: HashMap::from([
            (
                "view::app::public::active_users".to_string(),
                "SELECT id, email FROM public.users WHERE active".to_string(),
            ),
            (
                "materialized_view::app::public::daily_totals".to_string(),
                "SELECT date_trunc('day', created_at) AS day, count(*) FROM public.orders GROUP BY 1"
                    .to_string(),
            ),
        ]),
        routine_bodies: HashMap::from([(
            "routine::app::public::user_email".to_string(),
            "SELECT email FROM public.users WHERE id = user_id".to_string(),
        )]),
        trigger_statements: HashMap::new(),
        trigger_functions: HashMap::from([(
            "trigger::app::public::users_audit".to_string(),
            "CREATE FUNCTION public.audit_users() RETURNS trigger AS $$ BEGIN RETURN NEW; END $$ LANGUAGE plpgsql"
                .to_string(),
        )]),
        sequence_details: HashMap::from([(
            "sequence::app::public::users_id_seq".to_string(),
            SequenceDetails {
                data_type: MetadataValue::available("bigint".to_string()),
                start_value: MetadataValue::available("1".to_string()),
                increment: MetadataValue::available("1".to_string()),
                minimum: MetadataValue::available("1".to_string()),
                maximum: MetadataValue::available("9223372036854775807".to_string()),
                current_value: MetadataValue::available("42".to_string()),
                cache: MetadataValue::available("1".to_string()),
                cycle: MetadataValue::available(false),
            },
        )]),
        columns: vec![
            ColumnSummary {
                name: "id".to_string(),
                data_type: "bigint".to_string(),
                length: MetadataValue::available(None),
                precision: MetadataValue::available(Some(64)),
                scale: MetadataValue::available(Some(0)),
                nullable: false,
                default_value: MetadataValue::available(None),
                ordinal_position: 1,
                primary_key: true,
                identity: MetadataValue::available(ColumnIdentity::Always),
                generated: MetadataValue::available(false),
                generation_expression: MetadataValue::available(None),
                comment: MetadataValue::available(None),
                collation: MetadataValue::available(None),
            },
            ColumnSummary {
                name: "email".to_string(),
                data_type: "text".to_string(),
                length: MetadataValue::available(None),
                precision: MetadataValue::available(None),
                scale: MetadataValue::available(None),
                nullable: false,
                default_value: MetadataValue::available(None),
                ordinal_position: 2,
                primary_key: false,
                identity: MetadataValue::available(ColumnIdentity::None),
                generated: MetadataValue::available(false),
                generation_expression: MetadataValue::available(None),
                comment: MetadataValue::available(Some("Login address".to_string())),
                collation: MetadataValue::available(Some("default".to_string())),
            },
        ],
        indexes: vec![
            IndexSummary {
                name: "users_pkey".to_string(),
                columns: vec!["id".to_string()],
                unique: true,
                primary: true,
                method: MetadataValue::available("btree".to_string()),
                predicate: MetadataValue::available(None),
            },
            IndexSummary {
                name: "users_email_key".to_string(),
                columns: vec!["email".to_string()],
                unique: true,
                primary: false,
                method: MetadataValue::available("btree".to_string()),
                predicate: MetadataValue::available(None),
            },
        ],
        constraints: vec![
            ConstraintSummary {
                name: "users_pkey".to_string(),
                kind: ConstraintKind::PrimaryKey,
                columns: vec!["id".to_string()],
                referenced_schema: MetadataValue::available(None),
                referenced_table: MetadataValue::available(None),
                referenced_columns: Vec::new(),
                on_update: MetadataValue::available(None),
                on_delete: MetadataValue::available(None),
                check_expression: MetadataValue::available(None),
                deferrable: MetadataValue::available(false),
                initially_deferred: MetadataValue::available(false),
            },
            ConstraintSummary {
                name: "users_email_key".to_string(),
                kind: ConstraintKind::Unique,
                columns: vec!["email".to_string()],
                referenced_schema: MetadataValue::available(None),
                referenced_table: MetadataValue::available(None),
                referenced_columns: Vec::new(),
                on_update: MetadataValue::available(None),
                on_delete: MetadataValue::available(None),
                check_expression: MetadataValue::available(None),
                deferrable: MetadataValue::available(false),
                initially_deferred: MetadataValue::available(false),
            },
        ],
        dependencies: TableDependencies {
            incoming_foreign_keys: MetadataValue::available(Bounded::new(
                vec![incoming_key(
                    "orders_user_fk",
                    "public",
                    "orders",
                    "user_id",
                    "id",
                    "CASCADE",
                    "RESTRICT",
                )],
                100,
            )),
            dependent_views: MetadataValue::available(Bounded::new(
                vec![dependent_view("active_users", "public", "VIEW")],
                100,
            )),
            owned_sequences: MetadataValue::available(Bounded::new(
                vec![owned_sequence("users_id_seq", "public", "id")],
                100,
            )),
            triggers: MetadataValue::available(Bounded::new(Vec::new(), 100)),
        },
        roles: vec![
            role_summary(
                FakeRoleFixture {
                    name: "app",
                    host: None,
                    kind: RoleKind::User,
                    can_login: true,
                    superuser: false,
                    create_database: true,
                    create_role: false,
                    inherit: true,
                },
                DatabaseEngine::Postgresql,
            ),
            role_summary(
                FakeRoleFixture {
                    name: "reporting",
                    host: None,
                    kind: RoleKind::User,
                    can_login: true,
                    superuser: false,
                    create_database: false,
                    create_role: false,
                    inherit: true,
                },
                DatabaseEngine::Postgresql,
            ),
            role_summary(
                FakeRoleFixture {
                    name: "reporting_reader",
                    host: None,
                    kind: RoleKind::Role,
                    can_login: false,
                    superuser: false,
                    create_database: false,
                    create_role: false,
                    inherit: true,
                },
                DatabaseEngine::Postgresql,
            ),
            role_summary(
                FakeRoleFixture {
                    name: "auditor",
                    host: None,
                    kind: RoleKind::Role,
                    can_login: false,
                    superuser: false,
                    create_database: false,
                    create_role: true,
                    inherit: true,
                },
                DatabaseEngine::Postgresql,
            ),
        ],
        role_grants: HashMap::from([
            (
                "app".to_string(),
                RoleGrantData {
                    direct: vec![
                        grant("CONNECT", postgres_database_target("app"), false),
                        grant("USAGE", postgres_schema_target("public"), false),
                        grant("SELECT", postgres_table_target("public", "orders"), false),
                    ],
                    member_of: Vec::new(),
                },
            ),
            (
                "reporting".to_string(),
                RoleGrantData {
                    direct: vec![grant(
                        "SELECT",
                        postgres_table_target("public", "orders"),
                        false,
                    )],
                    member_of: vec![RoleRef {
                        name: "reporting_reader".to_string(),
                        host: None,
                    }],
                },
            ),
            (
                "reporting_reader".to_string(),
                RoleGrantData {
                    direct: vec![grant(
                        "SELECT",
                        postgres_table_target("public", "users"),
                        false,
                    )],
                    member_of: Vec::new(),
                },
            ),
            (
                "auditor".to_string(),
                RoleGrantData {
                    direct: vec![grant(
                        "SELECT",
                        postgres_table_target("public", "orders"),
                        true,
                    )],
                    member_of: Vec::new(),
                },
            ),
        ]),
        sessions: vec![
            SessionSummary {
                id: 501,
                user: "app".to_string(),
                host: optional_metadata(Some("10.0.0.5")),
                database: optional_metadata(Some("app")),
                application_name: optional_metadata(Some("psql")),
                command: MetadataValue::unavailable(),
                state: optional_metadata(Some("active")),
                wait_event: MetadataValue::unavailable(),
                backend_type: MetadataValue::unavailable(),
                connected_at: optional_metadata(Some("2026-11-17T08:00:00Z")),
                is_current: false,
                query: session_query(
                    "SELECT pg_sleep(30)",
                    Some("2026-11-17T08:05:00Z"),
                    Some(30_000),
                ),
            },
            SessionSummary {
                id: 502,
                user: "reporting".to_string(),
                host: MetadataValue::unavailable(),
                database: optional_metadata(Some("app")),
                application_name: optional_metadata(Some("reporting-job")),
                command: MetadataValue::unavailable(),
                state: optional_metadata(Some("idle")),
                wait_event: MetadataValue::unavailable(),
                backend_type: MetadataValue::unavailable(),
                connected_at: optional_metadata(Some("2026-11-17T07:55:00Z")),
                is_current: false,
                query: MetadataValue::unavailable(),
            },
            SessionSummary {
                id: 503,
                user: "app".to_string(),
                host: optional_metadata(Some("127.0.0.1")),
                database: optional_metadata(Some("app")),
                application_name: optional_metadata(Some("serverdesk")),
                command: MetadataValue::unavailable(),
                state: optional_metadata(Some("active")),
                wait_event: MetadataValue::unavailable(),
                backend_type: MetadataValue::unavailable(),
                connected_at: optional_metadata(Some("2026-11-17T08:04:00Z")),
                is_current: true,
                query: session_query(
                    "SELECT 1",
                    Some("2026-11-17T08:05:00Z"),
                    Some(5),
                ),
            },
            SessionSummary {
                id: 504,
                user: "admin".to_string(),
                host: optional_metadata(Some("10.0.0.7")),
                database: optional_metadata(Some("app")),
                application_name: optional_metadata(Some("psql")),
                command: MetadataValue::unavailable(),
                state: optional_metadata(Some("active")),
                wait_event: MetadataValue::unavailable(),
                backend_type: MetadataValue::unavailable(),
                connected_at: optional_metadata(Some("2026-11-17T08:01:00Z")),
                is_current: false,
                query: session_query(
                    "ALTER ROLE reporting WITH PASSWORD 'fixture-role-session-secret'",
                    Some("2026-11-17T08:05:00Z"),
                    Some(1_200),
                ),
            },
        ],
        detail_table: "users".to_string(),
    }
}

fn incoming_key(
    name: &str,
    schema: &str,
    table: &str,
    column: &str,
    referenced_column: &str,
    on_delete: &str,
    on_update: &str,
) -> DependencySummary {
    DependencySummary {
        kind: DependencyKind::IncomingForeignKey,
        name: name.to_string(),
        database: Some("app".to_string()),
        schema: Some(schema.to_string()),
        table: Some(table.to_string()),
        columns: vec![column.to_string()],
        referenced_columns: vec![referenced_column.to_string()],
        on_update: MetadataValue::available(Some(on_update.to_string())),
        on_delete: MetadataValue::available(Some(on_delete.to_string())),
        details: MetadataValue::Unavailable,
    }
}

fn dependent_view(name: &str, schema: &str, details: &str) -> DependencySummary {
    DependencySummary {
        kind: DependencyKind::DependentView,
        name: name.to_string(),
        database: Some("app".to_string()),
        schema: Some(schema.to_string()),
        table: None,
        columns: Vec::new(),
        referenced_columns: Vec::new(),
        on_update: MetadataValue::Unavailable,
        on_delete: MetadataValue::Unavailable,
        details: MetadataValue::available(details.to_string()),
    }
}

fn owned_sequence(name: &str, schema: &str, column: &str) -> DependencySummary {
    DependencySummary {
        kind: DependencyKind::OwnedSequence,
        name: name.to_string(),
        database: Some("app".to_string()),
        schema: Some(schema.to_string()),
        table: Some("users".to_string()),
        columns: vec![column.to_string()],
        referenced_columns: Vec::new(),
        on_update: MetadataValue::Unavailable,
        on_delete: MetadataValue::Unavailable,
        details: MetadataValue::Unavailable,
    }
}
