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
    dialect::{Identifier, SqlDialect},
    errors::DatabaseError,
    mysql,
    pagination::{Bounded, Page, PageRequest},
    postgres,
    rows::{
        MAX_ROW_CELL_CHARACTERS, RowCell, RowColumn, RowColumnKind, RowExactCount,
        RowFilterOperator, RowFilterValue, RowIdentity, RowIdentityValue, RowQueryControl,
        RowReadRequest, RowReadResult, RowRecord, RowSortDirection, RowTableMetadata,
        resolve_read_plan,
    },
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
        }
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
    columns: Vec<ColumnSummary>,
    created_tables: HashMap<String, Vec<ColumnSummary>>,
    indexes: HashMap<String, Vec<IndexSummary>>,
    constraints: HashMap<String, Vec<ConstraintSummary>>,
    dependencies: TableDependencies,
    detail_table: String,
    row_tables: Vec<FakeRowTable>,
    row_read_error: Option<DatabaseError>,
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
            columns: fixtures.columns,
            created_tables: HashMap::from([(detail_table.clone(), detail_columns)]),
            indexes: HashMap::from([(detail_table.clone(), detail_indexes)]),
            constraints: HashMap::from([(detail_table.clone(), detail_constraints)]),
            dependencies: fixtures.dependencies,
            detail_table,
            row_tables: Vec::new(),
            row_read_error: None,
        }
    }

    /// Adds one deterministic row-browser table fixture.
    pub fn with_row_table(mut self, table: FakeRowTable) -> Self {
        self.row_tables.push(table);
        self
    }

    /// Makes every row read fail with a typed database error.
    pub fn with_row_read_error(mut self, error: DatabaseError) -> Self {
        self.row_read_error = Some(error);
        self
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

        Ok(ObjectCounts {
            tables: MetadataValue::available(tables),
            views: MetadataValue::available(views),
            materialized_views,
            sequences,
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
    columns: Vec<ColumnSummary>,
    indexes: Vec<IndexSummary>,
    constraints: Vec<ConstraintSummary>,
    dependencies: TableDependencies,
    detail_table: String,
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
