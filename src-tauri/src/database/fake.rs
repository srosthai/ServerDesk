//! In-memory adapter used by tests and capability demos.
//!
//! It implements the full shared interface without a database driver, network
//! connection, SSH tunnel, or engine SQL. Tests use it to prove that commands
//! can depend on `DatabaseAdapter` alone.

use std::{
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
    structure::{StructureDdlPreview, ValidatedStructureAction},
    traits::{
        ColumnIdentity, ColumnSummary, ConstraintKind, ConstraintSummary, DatabaseAdapter,
        DatabaseConnectRequest, DatabaseConnectionInfo, DatabasePing, DatabaseSummary,
        DatabaseVersionInfo, DependencyKind, DependencySummary, IndexSummary, MetadataScope,
        MetadataValue, ObjectCounts, ObjectMetadata, SchemaSummary, SequenceSummary,
        TableDependencies, TableKind, TableRef, TableSummary, ViewKind, ViewSummary,
    },
};

const FAKE_LATENCY_MS: u64 = 1;

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
    indexes: Vec<IndexSummary>,
    constraints: Vec<ConstraintSummary>,
    dependencies: TableDependencies,
    detail_table: String,
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
            created_tables: HashMap::from([(detail_table, detail_columns)]),
            indexes: fixtures.indexes,
            constraints: fixtures.constraints,
            dependencies: fixtures.dependencies,
            detail_table: fixtures.detail_table,
        }
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
                self.tables.push(summary);
                self.created_tables
                    .insert(options.table.name.as_str().to_string(), columns);
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
            }
            // Other structure actions are covered by the preview contract in
            // this fixture; only table lifecycle needs mutable metadata here.
            _ => {}
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

        if !self
            .tables
            .iter()
            .any(|summary| Self::table_matches(summary, table) && summary.name == self.detail_table)
        {
            return Ok(Page::from_items(Vec::new(), page, Some(0)));
        }

        Ok(Self::paginate(&self.indexes, page))
    }

    async fn list_constraints(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ConstraintSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseConstraints)?;
        self.require_connection()?;

        if !self
            .tables
            .iter()
            .any(|summary| Self::table_matches(summary, table) && summary.name == self.detail_table)
        {
            return Ok(Page::from_items(Vec::new(), page, Some(0)));
        }

        Ok(Self::paginate(&self.constraints, page))
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
