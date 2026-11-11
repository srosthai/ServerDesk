//! In-memory adapter used by tests and capability demos.
//!
//! It implements the full shared interface without a database driver, network
//! connection, SSH tunnel, or engine SQL. Tests use it to prove that commands
//! can depend on `DatabaseAdapter` alone.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;

use crate::storage::DatabaseEngine;

use super::{
    capabilities::{DatabaseAction, DatabaseCapabilities, DatabaseVersion},
    dialect::{Identifier, SqlDialect},
    errors::DatabaseError,
    mysql,
    pagination::{Page, PageRequest},
    postgres,
    traits::{
        ColumnSummary, ConstraintKind, ConstraintSummary, DatabaseAdapter, DatabaseConnectRequest,
        DatabaseConnectionInfo, DatabasePing, DatabaseSummary, DatabaseVersionInfo, IndexSummary,
        MetadataScope, SchemaSummary, TableKind, TableRef, TableSummary,
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
    databases: Vec<DatabaseSummary>,
    schemas: Vec<SchemaSummary>,
    tables: Vec<TableSummary>,
    columns: Vec<ColumnSummary>,
    indexes: Vec<IndexSummary>,
    constraints: Vec<ConstraintSummary>,
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
            databases: fixtures.databases,
            schemas: fixtures.schemas,
            tables: fixtures.tables,
            columns: fixtures.columns,
            indexes: fixtures.indexes,
            constraints: fixtures.constraints,
            detail_table: fixtures.detail_table,
        }
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

    fn connection_info(&self, request: &DatabaseConnectRequest) -> DatabaseConnectionInfo {
        DatabaseConnectionInfo {
            engine: self.engine,
            version: self.version.clone(),
            database: request.database.clone(),
            read_only: request.read_only,
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
        Ok(())
    }

    async fn list_databases(
        &self,
        page: PageRequest,
    ) -> Result<Page<DatabaseSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseDatabases)?;
        self.require_connection()?;
        Ok(Self::paginate(&self.databases, page))
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
}

struct FakeFixtures {
    databases: Vec<DatabaseSummary>,
    schemas: Vec<SchemaSummary>,
    tables: Vec<TableSummary>,
    columns: Vec<ColumnSummary>,
    indexes: Vec<IndexSummary>,
    constraints: Vec<ConstraintSummary>,
    detail_table: String,
}

fn mysql_fixtures() -> FakeFixtures {
    FakeFixtures {
        databases: vec![
            DatabaseSummary {
                name: "app".to_string(),
                owner: Some("app".to_string()),
                encoding: Some("utf8mb4".to_string()),
                collation: Some("utf8mb4_unicode_ci".to_string()),
                size_bytes: Some(4_096),
                system: false,
            },
            DatabaseSummary {
                name: "sys".to_string(),
                owner: Some("mysql.sys".to_string()),
                encoding: Some("utf8mb4".to_string()),
                collation: Some("utf8mb4_0900_ai_ci".to_string()),
                size_bytes: Some(1_024),
                system: true,
            },
        ],
        schemas: vec![
            SchemaSummary {
                name: "app".to_string(),
                owner: Some("app".to_string()),
                system: false,
            },
            SchemaSummary {
                name: "sys".to_string(),
                owner: Some("mysql.sys".to_string()),
                system: true,
            },
        ],
        tables: vec![
            TableSummary {
                database: Some("app".to_string()),
                schema: Some("app".to_string()),
                name: "users".to_string(),
                kind: TableKind::Table,
                estimated_rows: Some(2),
                size_bytes: Some(16_384),
            },
            TableSummary {
                database: Some("app".to_string()),
                schema: Some("app".to_string()),
                name: "orders".to_string(),
                kind: TableKind::Table,
                estimated_rows: Some(0),
                size_bytes: Some(16_384),
            },
        ],
        columns: vec![
            ColumnSummary {
                name: "id".to_string(),
                data_type: "bigint unsigned".to_string(),
                nullable: false,
                default_value: None,
                ordinal_position: 1,
                primary_key: true,
                generated: false,
                identity: false,
                comment: None,
            },
            ColumnSummary {
                name: "email".to_string(),
                data_type: "varchar(255)".to_string(),
                nullable: false,
                default_value: None,
                ordinal_position: 2,
                primary_key: false,
                generated: false,
                identity: false,
                comment: Some("Login address".to_string()),
            },
        ],
        indexes: vec![
            IndexSummary {
                name: "PRIMARY".to_string(),
                columns: vec!["id".to_string()],
                unique: true,
                primary: true,
                method: Some("BTREE".to_string()),
            },
            IndexSummary {
                name: "users_email_unique".to_string(),
                columns: vec!["email".to_string()],
                unique: true,
                primary: false,
                method: Some("BTREE".to_string()),
            },
        ],
        constraints: vec![
            ConstraintSummary {
                name: "PRIMARY".to_string(),
                kind: ConstraintKind::PrimaryKey,
                columns: vec!["id".to_string()],
                referenced_table: None,
                referenced_columns: Vec::new(),
                on_update: None,
                on_delete: None,
            },
            ConstraintSummary {
                name: "users_email_unique".to_string(),
                kind: ConstraintKind::Unique,
                columns: vec!["email".to_string()],
                referenced_table: None,
                referenced_columns: Vec::new(),
                on_update: None,
                on_delete: None,
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
                owner: Some("app".to_string()),
                encoding: Some("UTF8".to_string()),
                collation: Some("en_US.UTF-8".to_string()),
                size_bytes: Some(8_192),
                system: false,
            },
            DatabaseSummary {
                name: "postgres".to_string(),
                owner: Some("postgres".to_string()),
                encoding: Some("UTF8".to_string()),
                collation: Some("C.UTF-8".to_string()),
                size_bytes: Some(7_168),
                system: true,
            },
        ],
        schemas: vec![
            SchemaSummary {
                name: "public".to_string(),
                owner: Some("pg_database_owner".to_string()),
                system: false,
            },
            SchemaSummary {
                name: "pg_catalog".to_string(),
                owner: Some("postgres".to_string()),
                system: true,
            },
        ],
        tables: vec![
            TableSummary {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "users".to_string(),
                kind: TableKind::Table,
                estimated_rows: Some(2),
                size_bytes: Some(8_192),
            },
            TableSummary {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "orders".to_string(),
                kind: TableKind::Table,
                estimated_rows: Some(0),
                size_bytes: Some(8_192),
            },
        ],
        columns: vec![
            ColumnSummary {
                name: "id".to_string(),
                data_type: "bigint".to_string(),
                nullable: false,
                default_value: Some("nextval('users_id_seq')".to_string()),
                ordinal_position: 1,
                primary_key: true,
                generated: false,
                identity: true,
                comment: None,
            },
            ColumnSummary {
                name: "email".to_string(),
                data_type: "text".to_string(),
                nullable: false,
                default_value: None,
                ordinal_position: 2,
                primary_key: false,
                generated: false,
                identity: false,
                comment: Some("Login address".to_string()),
            },
        ],
        indexes: vec![
            IndexSummary {
                name: "users_pkey".to_string(),
                columns: vec!["id".to_string()],
                unique: true,
                primary: true,
                method: Some("btree".to_string()),
            },
            IndexSummary {
                name: "users_email_key".to_string(),
                columns: vec!["email".to_string()],
                unique: true,
                primary: false,
                method: Some("btree".to_string()),
            },
        ],
        constraints: vec![
            ConstraintSummary {
                name: "users_pkey".to_string(),
                kind: ConstraintKind::PrimaryKey,
                columns: vec!["id".to_string()],
                referenced_table: None,
                referenced_columns: Vec::new(),
                on_update: None,
                on_delete: None,
            },
            ConstraintSummary {
                name: "users_email_key".to_string(),
                kind: ConstraintKind::Unique,
                columns: vec!["email".to_string()],
                referenced_table: None,
                referenced_columns: Vec::new(),
                on_update: None,
                on_delete: None,
            },
        ],
        detail_table: "users".to_string(),
    }
}
