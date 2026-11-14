//! SQLx-backed MySQL adapter.
//!
//! The adapter connects only to the loopback endpoint published by an SSH
//! tunnel. It keeps one authenticated connection, applies the profile's
//! read-only session mode, discovers the server version, and translates every
//! driver, authentication, permission, timeout, and protocol failure into the
//! shared [`DatabaseError`] taxonomy. Raw driver errors and connection strings
//! never cross this module boundary.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use sqlx::{
    Connection, MySql, MySqlConnection, Row,
    mysql::{MySqlConnectOptions, MySqlRow, MySqlSslMode},
};
use tokio::{
    sync::Mutex,
    time::{timeout, timeout_at},
};

use crate::storage::DatabaseEngine;

use super::{
    MYSQL_DIALECT, actions as engine_actions, ddl as engine_ddl, rows as engine_rows, sql,
};
use crate::database::{
    actions::{DatabaseDdlPreview, ValidatedDatabaseAction},
    capabilities::{DatabaseAction, DatabaseCapabilities, DatabaseVersion},
    dialect::{Identifier, SqlDialect},
    errors::DatabaseError,
    pagination::{Bounded, Page, PageRequest},
    row_writes::{RowDeleteRequest, RowInsertRequest, RowUpdateRequest, RowWriteResult},
    rows::{RowQueryControl, RowReadRequest, RowReadResult},
    structure::{StructureDdlPreview, ValidatedStructureAction},
    traits::{
        ColumnIdentity, ColumnSummary, ConstraintKind, ConstraintSummary, DatabaseAdapter,
        DatabaseConnectRequest, DatabaseConnectionInfo, DatabasePing, DatabaseSummary,
        DatabaseVersionInfo, DependencyKind, DependencySummary, IndexSummary, MetadataScope,
        MetadataValue, ObjectCounts, ObjectMetadata, SchemaSummary, SequenceSummary,
        TableDependencies, TableKind, TableRef, TableSummary, ViewKind, ViewSummary,
    },
    validation::validate_connect_request,
};

const CONNECTION_LOST_MESSAGE: &str = "The database connection is no longer available.";
const DRIVER_FAILURE_MESSAGE: &str = "The database operation failed.";

/// SQLx-backed adapter for one MySQL or MariaDB profile.
pub struct MysqlDatabaseAdapter {
    connection: Mutex<Option<MySqlConnection>>,
    connected: AtomicBool,
    capabilities: DatabaseCapabilities,
    version: Option<DatabaseVersionInfo>,
    current_database: Option<String>,
    read_only: bool,
    connect_request: Option<DatabaseConnectRequest>,
    operation_timeout: Duration,
}

impl Default for MysqlDatabaseAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl MysqlDatabaseAdapter {
    pub fn new() -> Self {
        Self {
            connection: Mutex::new(None),
            connected: AtomicBool::new(false),
            capabilities: DatabaseCapabilities::unknown(DatabaseEngine::Mysql),
            version: None,
            current_database: None,
            read_only: false,
            connect_request: None,
            operation_timeout: Duration::from_secs(10),
        }
    }

    /// Returns the active database reported after the last connect or
    /// set-active action.
    pub fn current_database(&self) -> Option<&str> {
        self.current_database.as_deref()
    }

    /// Returns the raw server version observed during the last successful
    /// connect, or `None` before the first connect.
    pub fn raw_version(&self) -> Option<&str> {
        self.version.as_ref().map(|version| version.raw.as_str())
    }

    async fn lock_connection(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<MySqlConnection>>, DatabaseError> {
        Ok(self.connection.lock().await)
    }

    async fn live_connection(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<MySqlConnection>>, DatabaseError> {
        self.require_connection()?;
        let guard = self.lock_connection().await?;
        if guard.is_none() {
            return Err(DatabaseError::connection(CONNECTION_LOST_MESSAGE));
        }
        Ok(guard)
    }

    fn operation_timeout_error(&self) -> DatabaseError {
        DatabaseError::timeout(format!(
            "The database did not respond within {} seconds.",
            self.operation_timeout.as_secs().max(1)
        ))
    }

    async fn execute_statement(&self, statement: &str) -> Result<(), DatabaseError> {
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::raw_sql(sqlx::AssertSqlSafe(statement.to_string())).execute(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    fn connection_info(&self) -> Result<DatabaseConnectionInfo, DatabaseError> {
        let version = self
            .version
            .clone()
            .ok_or_else(|| DatabaseError::internal("The database version was not recorded."))?;
        Ok(DatabaseConnectionInfo {
            engine: DatabaseEngine::Mysql,
            version,
            database: self.current_database.clone(),
            read_only: self.read_only,
            capabilities: self.capabilities.snapshot(),
        })
    }

    #[cfg(test)]
    async fn execute_for_test(
        &self,
        statement: &str,
    ) -> Result<sqlx::mysql::MySqlQueryResult, DatabaseError> {
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sqlx::AssertSqlSafe(statement.to_string())).execute(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
    }

    async fn query_version(
        connection: &mut MySqlConnection,
        request_timeout: Duration,
    ) -> Result<String, DatabaseError> {
        let row = timeout_at(
            tokio::time::Instant::now() + request_timeout,
            sqlx::query(sql::SERVER_VERSION).fetch_one(&mut *connection),
        )
        .await
        .map_err(|_| {
            DatabaseError::timeout(format!(
                "The database did not report its version within {} seconds.",
                request_timeout.as_secs().max(1)
            ))
        })?
        .map_err(map_sqlx_error)?;

        decode(&row, "VERSION()")
    }

    async fn open_connection(
        request: &DatabaseConnectRequest,
    ) -> Result<(MySqlConnection, DatabaseVersionInfo, DatabaseCapabilities), DatabaseError> {
        validate_connect_request(request)?;
        let ssl_mode = ssl_mode(request)?;
        let connect_timeout = request.connect_timeout;

        let mut options = MySqlConnectOptions::new()
            .host(request.host.trim())
            .port(request.port)
            .username(request.username.trim())
            .password(request.password.expose_secret())
            .ssl_mode(ssl_mode);

        if let Some(database) = request.database.as_deref() {
            options = options.database(database);
        }

        let mut connection = timeout(connect_timeout, MySqlConnection::connect_with(&options))
            .await
            .map_err(|_| {
                DatabaseError::timeout(format!(
                    "The database connection timed out after {} seconds.",
                    connect_timeout.as_secs().max(1)
                ))
            })?
            .map_err(map_sqlx_error)?;

        if request.read_only {
            // MySQL and MariaDB reject writes inside a read-only transaction.
            // Failing to set the mode aborts the connect instead of silently
            // opening a writable session.
            if let Err(error) = timeout_at(
                tokio::time::Instant::now() + connect_timeout,
                sqlx::query("SET SESSION TRANSACTION READ ONLY").execute(&mut connection),
            )
            .await
            .map_err(|_| {
                DatabaseError::timeout(format!(
                    "The read-only database session could not be configured within {} seconds.",
                    connect_timeout.as_secs().max(1)
                ))
            })
            .and_then(|result| result.map_err(map_sqlx_error))
            {
                let _ = connection.close().await;
                return Err(error);
            }
        }

        let raw_version = match Self::query_version(&mut connection, connect_timeout).await {
            Ok(raw_version) => raw_version,
            Err(error) => {
                let _ = connection.close().await;
                return Err(error);
            }
        };
        let parsed_version = DatabaseVersion::parse(&raw_version);
        let capabilities =
            DatabaseCapabilities::for_engine(DatabaseEngine::Mysql, Some(&raw_version));
        let version = DatabaseVersionInfo {
            version: parsed_version.unwrap_or(DatabaseVersion::new(0, 0, 0)),
            raw: raw_version,
        };

        Ok((connection, version, capabilities))
    }

    fn scope_name(scope: &MetadataScope) -> Result<Option<String>, DatabaseError> {
        match (
            scope.database.as_ref().map(|value| value.as_str()),
            scope.schema.as_ref().map(|value| value.as_str()),
        ) {
            (Some(database), Some(schema)) if database != schema => Err(DatabaseError::validation(
                "MySQL uses one database/schema namespace; the scope values must match.",
            )),
            (Some(database), _) => Ok(Some(database.to_string())),
            (None, Some(schema)) => Ok(Some(schema.to_string())),
            (None, None) => Ok(None),
        }
    }

    fn table_name(table: &TableRef) -> Result<Option<String>, DatabaseError> {
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

    /// Reads InnoDB statistics refresh timestamps for the current page only.
    /// Every failure is treated as unavailable: the table list itself already
    /// succeeded and this optional table is commonly inaccessible.
    async fn table_statistics_refresh_times(
        &self,
        connection: &mut MySqlConnection,
        database: Option<&str>,
        records: &[MysqlTableRecord],
    ) -> HashMap<String, String> {
        let Some(database) = database else {
            return HashMap::new();
        };
        if records.is_empty() {
            return HashMap::new();
        }

        let names = records
            .iter()
            .map(|record| record.name.as_str())
            .collect::<Vec<_>>();
        let placeholders = std::iter::repeat_n("?", names.len())
            .collect::<Vec<_>>()
            .join(", ");
        let statement = format!(
            "{} AND table_name IN ({placeholders})",
            sql::TABLE_STATISTICS_REFRESHED_AT
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(statement)).bind(database);
        for name in names {
            query = query.bind(name);
        }

        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            query.fetch_all(&mut *connection),
        )
        .await;
        let Ok(Ok(rows)) = rows else {
            return HashMap::new();
        };

        rows.iter()
            .filter_map(|row| {
                let name: String = row.try_get("name").ok()?;
                let refreshed_at: Option<String> = row.try_get("refreshed_at").ok()?;
                Some((name, refreshed_at?))
            })
            .collect()
    }
}

#[async_trait]
impl DatabaseAdapter for MysqlDatabaseAdapter {
    fn engine(&self) -> DatabaseEngine {
        DatabaseEngine::Mysql
    }

    fn capabilities(&self) -> &DatabaseCapabilities {
        &self.capabilities
    }

    fn dialect(&self) -> &dyn SqlDialect {
        &MYSQL_DIALECT
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    async fn connect(
        &mut self,
        request: DatabaseConnectRequest,
    ) -> Result<DatabaseConnectionInfo, DatabaseError> {
        self.operation_timeout = request.connect_timeout.max(Duration::from_secs(1));

        // Replacing a live connection must not leave the previous driver
        // session open after the new attempt fails.
        if let Some(previous) = self.connection.get_mut().take() {
            let _ = previous.close().await;
        }
        self.connected.store(false, Ordering::SeqCst);
        self.version = None;
        self.current_database = None;
        self.read_only = false;
        self.connect_request = None;
        self.capabilities = DatabaseCapabilities::unknown(DatabaseEngine::Mysql);

        let (connection, version, capabilities) = Self::open_connection(&request).await?;

        *self.connection.get_mut() = Some(connection);
        self.version = Some(version.clone());
        self.current_database = request.database.clone();
        self.read_only = request.read_only;
        self.connect_request = Some(request);
        self.capabilities = capabilities.clone();
        self.connected.store(true, Ordering::SeqCst);

        Ok(DatabaseConnectionInfo {
            engine: DatabaseEngine::Mysql,
            version,
            database: self.current_database.clone(),
            read_only: self.read_only,
            capabilities: capabilities.snapshot(),
        })
    }

    async fn ping(&self) -> Result<DatabasePing, DatabaseError> {
        self.require_capability(DatabaseAction::RunQuery)?;
        let started = Instant::now();
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let result = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::PING).execute(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error);

        match result {
            Ok(_) => Ok(DatabasePing {
                latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            }),
            Err(error) => {
                self.connected.store(false, Ordering::SeqCst);
                Err(error)
            }
        }
    }

    async fn version(&self) -> Result<DatabaseVersionInfo, DatabaseError> {
        self.require_connection()?;
        self.version
            .clone()
            .ok_or_else(|| DatabaseError::internal("The database version was not recorded."))
    }

    async fn close(&mut self) -> Result<(), DatabaseError> {
        self.connected.store(false, Ordering::SeqCst);
        self.version = None;
        self.current_database = None;
        self.read_only = false;
        self.capabilities = DatabaseCapabilities::unknown(DatabaseEngine::Mysql);

        if let Some(connection) = self.connection.get_mut().take() {
            // Closing a broken socket is best effort. The session is gone from
            // the adapter regardless, so disconnect remains idempotent.
            let _ = connection.close().await;
        }

        Ok(())
    }

    async fn list_databases(
        &self,
        page: PageRequest,
    ) -> Result<Page<DatabaseSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseDatabases)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_DATABASES)
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        let databases = rows
            .iter()
            .map(|row| {
                let name: String = decode(row, "name")?;
                Ok(DatabaseSummary {
                    system: is_system_database(&name),
                    name,
                    metadata: schema_metadata(row)?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(databases, page, None))
    }

    async fn database_summary(
        &self,
        database: &Identifier,
    ) -> Result<Option<DatabaseSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseDatabases)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let row = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::DATABASE_SUMMARY)
                .bind(database.as_str())
                .fetch_optional(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        row.as_ref()
            .map(|row| {
                let name: String = decode(row, "name")?;
                Ok(DatabaseSummary {
                    system: is_system_database(&name),
                    name,
                    metadata: schema_metadata(row)?,
                })
            })
            .transpose()
    }

    async fn preview_database_action(
        &self,
        action: &ValidatedDatabaseAction,
    ) -> Result<DatabaseDdlPreview, DatabaseError> {
        self.require_connection()?;
        if matches!(action, ValidatedDatabaseAction::Rename { .. }) {
            // Keep the explanation specific instead of the generic capability
            // message; MySQL has no safe rename operation at any version.
            return engine_actions::preview(action);
        }
        self.require_capability(action.capability())?;
        if let ValidatedDatabaseAction::Drop { name } = action
            && self.current_database.as_deref() == Some(name.as_str())
        {
            return Err(DatabaseError::validation(
                "MySQL cannot drop the database used by the current connection. Set another \
                 database active first.",
            ));
        }
        engine_actions::preview(action)
    }

    async fn preview_structure_action(
        &self,
        action: &ValidatedStructureAction,
    ) -> Result<StructureDdlPreview, DatabaseError> {
        self.require_connection()?;
        action.ensure_supported(&self.capabilities)?;
        engine_ddl::preview(action)
    }

    async fn execute_database_action(
        &mut self,
        action: &ValidatedDatabaseAction,
    ) -> Result<Option<DatabaseConnectionInfo>, DatabaseError> {
        self.require_connection()?;
        self.require_capability(action.capability())?;
        let preview = self.preview_database_action(action).await?;

        match action {
            ValidatedDatabaseAction::SetActive { name } => {
                if self.current_database.as_deref() == Some(name.as_str()) {
                    return self.connection_info().map(Some);
                }

                let mut request = self.connect_request.clone().ok_or_else(|| {
                    DatabaseError::internal(
                        "The MySQL connection settings were not recorded for a reconnect.",
                    )
                })?;
                request.database = Some(name.as_str().to_string());

                // Open the replacement session before closing the old one. A
                // failed switch leaves the current browsing context usable.
                let (connection, version, capabilities) = Self::open_connection(&request).await?;
                if let Some(previous) = self.connection.get_mut().take() {
                    let _ = previous.close().await;
                }

                *self.connection.get_mut() = Some(connection);
                self.version = Some(version);
                self.current_database = request.database.clone();
                self.capabilities = capabilities;
                self.connect_request = Some(request);
                self.connection_info().map(Some)
            }
            _ => {
                for statement in &preview.statements {
                    self.execute_statement(statement).await?;
                }
                Ok(None)
            }
        }
    }

    async fn execute_structure_action(
        &mut self,
        action: &ValidatedStructureAction,
    ) -> Result<(), DatabaseError> {
        self.require_connection()?;
        let preview = self.preview_structure_action(action).await?;
        for statement in &preview.statements {
            self.execute_statement(statement).await?;
        }
        Ok(())
    }

    async fn list_schemas(&self, page: PageRequest) -> Result<Page<SchemaSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseSchemas)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_SCHEMAS)
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        let schemas = rows
            .iter()
            .map(|row| {
                let name: String = decode(row, "name")?;
                Ok(SchemaSummary {
                    system: is_system_database(&name),
                    name,
                    metadata: schema_metadata(row)?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(schemas, page, None))
    }

    async fn list_tables(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<TableSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        let database = Self::scope_name(scope)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_TABLES)
                .bind(database.as_deref())
                .bind(database.as_deref())
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| {
            error.scoped_permission("database", database.as_deref().unwrap_or("all databases"))
        })?;

        let records = rows
            .iter()
            .map(MysqlTableRecord::from_row)
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        // InnoDB statistics timestamps live in a mysql schema table that most
        // application accounts cannot read. Treat that failure as an
        // unavailable field, not as a failed table listing.
        let refresh_times = self
            .table_statistics_refresh_times(&mut *connection, database.as_deref(), &records)
            .await;

        let tables = records
            .into_iter()
            .map(|record| {
                let refreshed_at = refresh_times
                    .get(&record.name)
                    .cloned()
                    .map_or(MetadataValue::Unavailable, MetadataValue::available);
                record.into_summary(refreshed_at)
            })
            .collect::<Vec<_>>();

        Ok(Page::from_items(tables, page, None))
    }

    async fn table_summary(&self, table: &TableRef) -> Result<Option<TableSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        let database = Self::table_name(table)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let row = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::TABLE_SUMMARY)
                .bind(database.as_deref())
                .bind(table.name.as_str())
                .fetch_optional(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table.name.as_str()))?;

        row.as_ref()
            .map(MysqlTableRecord::from_row)
            .transpose()
            .map(|record| record.map(|record| record.into_summary(MetadataValue::Unavailable)))
    }

    async fn list_views(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<ViewSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseViews)?;
        let database = Self::scope_name(scope)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_VIEWS)
                .bind(database.as_deref())
                .bind(database.as_deref())
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        let views = rows
            .iter()
            .map(|row| {
                let database: String = decode(row, "database_name")?;
                Ok(ViewSummary {
                    database: Some(database.clone()),
                    schema: Some(database),
                    name: decode(row, "name")?,
                    kind: ViewKind::View,
                    metadata: ObjectMetadata {
                        owner: decode::<Option<String>>(row, "owner")?.into(),
                        ..ObjectMetadata::default()
                    },
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(views, page, None))
    }

    async fn list_sequences(
        &self,
        _scope: &MetadataScope,
        _page: PageRequest,
    ) -> Result<Page<SequenceSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseSequences)?;
        Err(DatabaseError::capability(
            "MySQL does not expose sequences.",
        ))
    }

    async fn object_counts(&self, scope: &MetadataScope) -> Result<ObjectCounts, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        let database = Self::scope_name(scope)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let row = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::OBJECT_COUNTS)
                .bind(database.as_deref())
                .bind(database.as_deref())
                .fetch_one(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        Ok(ObjectCounts {
            tables: MetadataValue::available(decode_count(&row, "table_count")?),
            views: MetadataValue::available(decode_count(&row, "view_count")?),
            materialized_views: MetadataValue::Unavailable,
            sequences: MetadataValue::Unavailable,
        })
    }

    async fn list_columns(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ColumnSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseColumns)?;
        let database = Self::table_name(table)?;
        let table_name = table.name.as_str();
        let generated_supported = self
            .capabilities
            .supports(DatabaseAction::CreateGeneratedColumn);
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_COLUMNS)
                .bind(database.as_deref())
                .bind(database.as_deref())
                .bind(table_name)
                .bind(table_name)
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        let columns = rows
            .iter()
            .map(|row| Ok(MysqlColumnRecord::from_row(row)?.into_summary(generated_supported)))
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(columns, page, None))
    }

    async fn list_indexes(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<IndexSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseIndexes)?;
        let database = Self::table_name(table)?;
        let table_name = table.name.as_str();
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_INDEXES)
                .bind(database.as_deref())
                .bind(database.as_deref())
                .bind(table_name)
                .bind(table_name)
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .bind(database.as_deref())
                .bind(database.as_deref())
                .bind(table_name)
                .bind(table_name)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        // MySQL returns one row per index column. The query already selected
        // at most one page of index names; group only that bounded set.
        let records = rows
            .iter()
            .map(MysqlIndexRow::from_row)
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let indexes = MysqlIndexRow::group(records);

        Ok(Page::from_items(indexes, page, None))
    }

    async fn list_constraints(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ConstraintSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseConstraints)?;
        let database = Self::table_name(table)?;
        let table_name = table.name.as_str();
        let check_supported = self
            .capabilities
            .supports(DatabaseAction::CreateCheckConstraint);
        let statement = if check_supported {
            sql::LIST_CONSTRAINTS
        } else {
            sql::LIST_CONSTRAINTS_WITHOUT_CHECKS
        };
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(statement)
                .bind(database.as_deref())
                .bind(database.as_deref())
                .bind(table_name)
                .bind(table_name)
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .bind(database.as_deref())
                .bind(database.as_deref())
                .bind(table_name)
                .bind(table_name)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        let records = rows
            .iter()
            .map(MysqlConstraintRow::from_row)
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(
            MysqlConstraintRow::group(records, check_supported),
            page,
            None,
        ))
    }

    async fn read_rows(
        &self,
        request: &RowReadRequest,
        control: &RowQueryControl,
    ) -> Result<RowReadResult, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseRows)?;
        if control.is_cancelled() {
            return Err(DatabaseError::cancelled("The row read was cancelled."));
        }
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let result = engine_rows::read_rows(connection, request, control, self.read_only)
            .await
            .map_err(|error| error.scoped_permission("table", request.table.name.as_str()));

        if matches!(&result, Err(error) if matches!(
            error.kind,
            crate::database::DatabaseErrorKind::Cancelled
                | crate::database::DatabaseErrorKind::Timeout
        )) {
            // Dropping a query future can leave the protocol stream in an
            // unknown state. Close the session instead of risking a later
            // statement reading leftover packets.
            self.connected.store(false, Ordering::SeqCst);
            drop(guard.take());
        }

        result
    }

    async fn insert_row(
        &mut self,
        request: &RowInsertRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError> {
        self.require_capability(DatabaseAction::InsertRow)?;
        if self.read_only {
            return Err(DatabaseError::permission(
                "The database session is read-only; row inserts are disabled.",
            ));
        }
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let result = engine_rows::insert_row(connection, request, control, self.read_only)
            .await
            .map_err(|error| error.scoped_permission("table", request.table.name.as_str()));

        if matches!(&result, Err(error) if matches!(
            error.kind,
            crate::database::DatabaseErrorKind::Cancelled
                | crate::database::DatabaseErrorKind::Timeout
                | crate::database::DatabaseErrorKind::Connection
        )) {
            self.connected.store(false, Ordering::SeqCst);
            drop(guard.take());
        }
        result
    }

    async fn update_row(
        &mut self,
        request: &RowUpdateRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError> {
        self.require_capability(DatabaseAction::EditRow)?;
        if self.read_only {
            return Err(DatabaseError::permission(
                "The database session is read-only; row updates are disabled.",
            ));
        }
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let result = engine_rows::update_row(connection, request, control, self.read_only)
            .await
            .map_err(|error| error.scoped_permission("table", request.table.name.as_str()));

        if matches!(&result, Err(error) if matches!(
            error.kind,
            crate::database::DatabaseErrorKind::Cancelled
                | crate::database::DatabaseErrorKind::Timeout
                | crate::database::DatabaseErrorKind::Connection
        )) {
            self.connected.store(false, Ordering::SeqCst);
            drop(guard.take());
        }
        result
    }

    async fn delete_rows(
        &mut self,
        request: &RowDeleteRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError> {
        self.require_capability(DatabaseAction::DeleteRow)?;
        if self.read_only {
            return Err(DatabaseError::permission(
                "The database session is read-only; row deletes are disabled.",
            ));
        }
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let result = engine_rows::delete_rows(connection, request, control, self.read_only)
            .await
            .map_err(|error| error.scoped_permission("table", request.table.name.as_str()));

        if matches!(&result, Err(error) if matches!(
            error.kind,
            crate::database::DatabaseErrorKind::Cancelled
                | crate::database::DatabaseErrorKind::Timeout
                | crate::database::DatabaseErrorKind::Connection
        )) {
            self.connected.store(false, Ordering::SeqCst);
            drop(guard.take());
        }
        result
    }

    async fn table_dependencies(
        &self,
        table: &TableRef,
        limit: u32,
    ) -> Result<TableDependencies, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        let database = Self::table_name(table)?
            .or_else(|| self.current_database.clone())
            .ok_or_else(|| {
                DatabaseError::validation(
                    "Select a MySQL database before inspecting table dependencies.",
                )
            })?;
        let table_name = table.name.as_str();
        let limit = i64::from(limit.clamp(1, crate::database::MAX_PAGE_LIMIT));
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");

        let foreign_key_rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::TABLE_INCOMING_FOREIGN_KEYS)
                .bind(&database)
                .bind(table_name)
                .bind(limit)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        let foreign_key_records = foreign_key_rows
            .iter()
            .map(MysqlForeignKeyRow::from_row)
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let foreign_keys = MysqlForeignKeyRow::group(foreign_key_records);

        let trigger_rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::TABLE_TRIGGERS)
                .bind(&database)
                .bind(table_name)
                .bind(limit)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        let triggers = trigger_rows
            .iter()
            .map(|row| MysqlTriggerRow::from_row(row, &database))
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(TableDependencies {
            incoming_foreign_keys: MetadataValue::available(Bounded::new(
                foreign_keys,
                limit as usize,
            )),
            // MariaDB 11.4 does not expose information_schema.VIEW_TABLE_USAGE,
            // so view dependencies are explicitly unavailable rather than
            // guessed from view definition text.
            dependent_views: MetadataValue::Unavailable,
            owned_sequences: MetadataValue::Unavailable,
            triggers: MetadataValue::available(Bounded::new(triggers, limit as usize)),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MysqlTableRecord {
    database_name: String,
    name: String,
    table_type: String,
    estimated_rows: Option<i64>,
    size_bytes: Option<i64>,
    engine: Option<String>,
}

impl MysqlTableRecord {
    fn from_row(row: &MySqlRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            database_name: decode(row, "database_name")?,
            name: decode(row, "name")?,
            table_type: decode(row, "table_type")?,
            estimated_rows: decode(row, "estimated_rows")?,
            size_bytes: decode(row, "size_bytes")?,
            engine: decode(row, "engine")?,
        })
    }

    fn into_summary(self, metadata_refreshed_at: MetadataValue<String>) -> TableSummary {
        TableSummary {
            database: Some(self.database_name.clone()),
            schema: Some(self.database_name),
            name: self.name,
            kind: table_kind(&self.table_type),
            estimated_rows: self
                .estimated_rows
                .and_then(|value| u64::try_from(value).ok())
                .into(),
            metadata_refreshed_at,
            metadata: ObjectMetadata {
                size_bytes: self
                    .size_bytes
                    .and_then(|value| u64::try_from(value).ok())
                    .into(),
                engine: self.engine.into(),
                ..ObjectMetadata::default()
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MysqlColumnRecord {
    name: String,
    data_type: String,
    length: Option<i64>,
    precision: Option<i64>,
    scale: Option<i64>,
    nullable: bool,
    default_value: Option<String>,
    ordinal_position: i64,
    primary_key: bool,
    generated: bool,
    generation_expression: Option<String>,
    auto_increment: bool,
    comment: String,
    collation: Option<String>,
}

impl MysqlColumnRecord {
    fn from_row(row: &MySqlRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            name: decode(row, "name")?,
            data_type: decode(row, "data_type")?,
            length: decode(row, "value_length")?,
            precision: decode(row, "numeric_precision")?,
            scale: decode(row, "numeric_scale")?,
            nullable: decode_flag(row, "nullable")?,
            default_value: decode(row, "default_value")?,
            ordinal_position: decode(row, "ordinal_position")?,
            primary_key: decode_flag(row, "primary_key")?,
            generated: decode_flag(row, "generated")?,
            generation_expression: decode(row, "generation_expression")?,
            auto_increment: decode_flag(row, "auto_increment")?,
            comment: decode(row, "comment")?,
            collation: decode(row, "collation")?,
        })
    }

    fn into_summary(self, generated_supported: bool) -> ColumnSummary {
        let generation_expression = if generated_supported {
            MetadataValue::available(if self.generated {
                self.generation_expression.filter(|value| !value.is_empty())
            } else {
                None
            })
        } else {
            MetadataValue::Unavailable
        };
        let generated = if generated_supported {
            MetadataValue::available(self.generated)
        } else {
            MetadataValue::Unavailable
        };

        ColumnSummary {
            name: self.name,
            data_type: self.data_type,
            length: MetadataValue::available(
                self.length.and_then(|value| u64::try_from(value).ok()),
            ),
            precision: MetadataValue::available(
                self.precision.and_then(|value| u32::try_from(value).ok()),
            ),
            scale: MetadataValue::available(self.scale.and_then(|value| u32::try_from(value).ok())),
            nullable: self.nullable,
            default_value: MetadataValue::available(self.default_value),
            ordinal_position: u32::try_from(self.ordinal_position).unwrap_or(u32::MAX),
            primary_key: self.primary_key,
            identity: MetadataValue::available(if self.auto_increment {
                ColumnIdentity::AutoIncrement
            } else {
                ColumnIdentity::None
            }),
            generated,
            generation_expression,
            comment: MetadataValue::available((!self.comment.is_empty()).then_some(self.comment)),
            collation: MetadataValue::available(self.collation),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MysqlIndexRow {
    name: String,
    column_name: Option<String>,
    unique: bool,
    primary: bool,
    method: Option<String>,
}

impl MysqlIndexRow {
    fn from_row(row: &MySqlRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            name: decode(row, "name")?,
            column_name: decode(row, "column_name")?,
            unique: decode_flag(row, "unique_index")?,
            primary: decode_flag(row, "primary_index")?,
            method: decode(row, "method")?,
        })
    }

    fn group(rows: Vec<Self>) -> Vec<IndexSummary> {
        let mut indexes: Vec<IndexSummary> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        for row in rows {
            let position = match positions.get(&row.name) {
                Some(position) => *position,
                None => {
                    let position = indexes.len();
                    positions.insert(row.name.clone(), position);
                    indexes.push(IndexSummary {
                        name: row.name,
                        columns: Vec::new(),
                        unique: row.unique,
                        primary: row.primary,
                        method: row.method.into(),
                        predicate: MetadataValue::available(None),
                    });
                    position
                }
            };
            if let Some(column) = row.column_name {
                indexes[position].columns.push(column);
            }
        }
        indexes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MysqlConstraintRow {
    name: String,
    kind: String,
    database_name: String,
    table_name: String,
    column_name: Option<String>,
    referenced_schema: Option<String>,
    referenced_table: Option<String>,
    referenced_column: Option<String>,
    on_update: Option<String>,
    on_delete: Option<String>,
    check_expression: Option<String>,
}

impl MysqlConstraintRow {
    fn from_row(row: &MySqlRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            name: decode(row, "name")?,
            kind: decode(row, "kind")?,
            database_name: decode(row, "database_name")?,
            table_name: decode(row, "table_name")?,
            column_name: decode(row, "column_name")?,
            referenced_schema: decode(row, "referenced_schema")?,
            referenced_table: decode(row, "referenced_table")?,
            referenced_column: decode(row, "referenced_column")?,
            on_update: decode(row, "on_update")?,
            on_delete: decode(row, "on_delete")?,
            check_expression: decode(row, "check_expression")?,
        })
    }

    fn group(rows: Vec<Self>, check_supported: bool) -> Vec<ConstraintSummary> {
        let mut constraints: Vec<ConstraintSummary> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        for row in rows {
            let key = format!(
                "{}\u{1f}{}\u{1f}{}",
                row.database_name, row.table_name, row.name
            );
            let position = match positions.get(&key) {
                Some(position) => *position,
                None => {
                    let position = constraints.len();
                    positions.insert(key, position);
                    let check_expression = if check_supported {
                        MetadataValue::available(row.check_expression.clone())
                    } else {
                        MetadataValue::Unavailable
                    };
                    constraints.push(ConstraintSummary {
                        name: row.name,
                        kind: constraint_kind(&row.kind),
                        columns: Vec::new(),
                        referenced_schema: MetadataValue::available(row.referenced_schema),
                        referenced_table: MetadataValue::available(row.referenced_table),
                        referenced_columns: Vec::new(),
                        on_update: MetadataValue::available(row.on_update),
                        on_delete: MetadataValue::available(row.on_delete),
                        check_expression,
                        deferrable: MetadataValue::Unavailable,
                        initially_deferred: MetadataValue::Unavailable,
                    });
                    position
                }
            };
            if let Some(column) = row.column_name {
                constraints[position].columns.push(column);
            }
            if let Some(column) = row.referenced_column {
                constraints[position].referenced_columns.push(column);
            }
        }
        constraints
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MysqlForeignKeyRow {
    name: String,
    database_name: String,
    table_name: String,
    column_name: Option<String>,
    referenced_column: Option<String>,
    on_update: Option<String>,
    on_delete: Option<String>,
}

impl MysqlForeignKeyRow {
    fn from_row(row: &MySqlRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            name: decode(row, "name")?,
            database_name: decode(row, "database_name")?,
            table_name: decode(row, "table_name")?,
            column_name: decode(row, "column_name")?,
            referenced_column: decode(row, "referenced_column")?,
            on_update: decode(row, "on_update")?,
            on_delete: decode(row, "on_delete")?,
        })
    }

    fn group(rows: Vec<Self>) -> Vec<DependencySummary> {
        let mut foreign_keys: Vec<DependencySummary> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        for row in rows {
            let key = format!(
                "{}\u{1f}{}\u{1f}{}",
                row.database_name, row.table_name, row.name
            );
            let position = match positions.get(&key) {
                Some(position) => *position,
                None => {
                    let position = foreign_keys.len();
                    positions.insert(key, position);
                    foreign_keys.push(DependencySummary {
                        kind: DependencyKind::IncomingForeignKey,
                        name: row.name,
                        database: Some(row.database_name.clone()),
                        schema: Some(row.database_name),
                        table: Some(row.table_name),
                        columns: Vec::new(),
                        referenced_columns: Vec::new(),
                        on_update: MetadataValue::available(row.on_update),
                        on_delete: MetadataValue::available(row.on_delete),
                        details: MetadataValue::Unavailable,
                    });
                    position
                }
            };
            if let Some(column) = row.column_name {
                foreign_keys[position].columns.push(column);
            }
            if let Some(column) = row.referenced_column {
                foreign_keys[position].referenced_columns.push(column);
            }
        }
        foreign_keys
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MysqlTriggerRow {
    name: String,
    database_name: String,
    table_name: String,
    timing: String,
    event: String,
}

impl MysqlTriggerRow {
    fn from_row(row: &MySqlRow, schema: &str) -> Result<DependencySummary, DatabaseError> {
        let row = Self {
            name: decode(row, "name")?,
            database_name: decode(row, "database_name")?,
            table_name: decode(row, "table_name")?,
            timing: decode(row, "timing")?,
            event: decode(row, "event")?,
        };
        Ok(DependencySummary {
            kind: DependencyKind::Trigger,
            name: row.name,
            database: Some(row.database_name),
            schema: Some(schema.to_string()),
            table: Some(row.table_name),
            columns: Vec::new(),
            referenced_columns: Vec::new(),
            on_update: MetadataValue::Unavailable,
            on_delete: MetadataValue::Unavailable,
            details: MetadataValue::available(format!("{} {}", row.timing, row.event)),
        })
    }
}

fn ssl_mode(request: &DatabaseConnectRequest) -> Result<MySqlSslMode, DatabaseError> {
    match request
        .ssl_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        None | Some("preferred") => Ok(MySqlSslMode::Preferred),
        Some("disabled" | "disable" | "off" | "none") => Ok(MySqlSslMode::Disabled),
        Some("required" | "require") => Ok(MySqlSslMode::Required),
        Some("verify-ca" | "verify_ca") => Ok(MySqlSslMode::VerifyCa),
        Some("verify-full" | "verify_identity" | "verify-identity") => {
            Ok(MySqlSslMode::VerifyIdentity)
        }
        Some(_) => Err(DatabaseError::validation(
            "The configured database SSL mode is not supported.",
        )),
    }
}

fn schema_metadata(row: &MySqlRow) -> Result<ObjectMetadata, DatabaseError> {
    Ok(ObjectMetadata {
        encoding: decode::<Option<String>>(row, "encoding")?.into(),
        collation: decode::<Option<String>>(row, "collation")?.into(),
        size_bytes: decode::<Option<i64>>(row, "size_bytes")?
            .and_then(|value| u64::try_from(value).ok())
            .into(),
        ..ObjectMetadata::default()
    })
}

fn is_system_database(name: &str) -> bool {
    matches!(
        name,
        "information_schema" | "mysql" | "ndbinfo" | "performance_schema" | "sys"
    )
}

fn table_kind(table_type: &str) -> TableKind {
    match table_type.to_ascii_uppercase().as_str() {
        "BASE TABLE" => TableKind::Table,
        "VIEW" | "SYSTEM VIEW" => TableKind::View,
        "FOREIGN TABLE" => TableKind::ForeignTable,
        "SEQUENCE" => TableKind::Other,
        _ => TableKind::Other,
    }
}

fn constraint_kind(kind: &str) -> ConstraintKind {
    match kind.to_ascii_uppercase().as_str() {
        "PRIMARY KEY" => ConstraintKind::PrimaryKey,
        "UNIQUE" => ConstraintKind::Unique,
        "FOREIGN KEY" => ConstraintKind::ForeignKey,
        "CHECK" => ConstraintKind::Check,
        _ => ConstraintKind::Other,
    }
}

pub(crate) fn decode<'r, T>(row: &'r MySqlRow, column: &str) -> Result<T, DatabaseError>
where
    T: sqlx::Decode<'r, MySql> + sqlx::Type<MySql>,
{
    row.try_get(column).map_err(|error| {
        map_sqlx_error(error).with_details(format!(
            "The database value in column {column} could not be decoded."
        ))
    })
}

fn decode_flag(row: &MySqlRow, column: &str) -> Result<bool, DatabaseError> {
    Ok(decode::<i64>(row, column)? != 0)
}

fn decode_count(row: &MySqlRow, column: &str) -> Result<u64, DatabaseError> {
    let value: i64 = decode(row, column)?;
    u64::try_from(value).map_err(|_| {
        DatabaseError::query("The database returned a negative count.").with_details(format!(
            "The database value in column {column} could not be converted to an unsigned count."
        ))
    })
}

/// Maps SQLx errors into the shared taxonomy without exposing the raw driver
/// display text. Authentication failures intentionally omit the server
/// message, which can echo the account and host.
pub(crate) fn map_sqlx_error(error: sqlx::Error) -> DatabaseError {
    match error {
        sqlx::Error::Database(database_error) => {
            if let Some(mysql_error) =
                database_error.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
            {
                return map_mysql_error(Some(mysql_error.number()), mysql_error.code());
            }
            DatabaseError::query(DRIVER_FAILURE_MESSAGE)
                .with_details("The database driver reported an unmapped database error.")
        }
        sqlx::Error::PoolTimedOut => DatabaseError::timeout("The database connection timed out."),
        sqlx::Error::PoolClosed => DatabaseError::connection(CONNECTION_LOST_MESSAGE),
        sqlx::Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            DatabaseError::timeout("The database connection timed out.")
        }
        sqlx::Error::Io(_) => {
            DatabaseError::connection("The database endpoint could not be reached.")
        }
        sqlx::Error::Tls(_) => {
            DatabaseError::connection("The secure database connection could not be established.")
        }
        sqlx::Error::Protocol(_) => DatabaseError::connection(
            "The database server returned an unsupported protocol response.",
        ),
        sqlx::Error::Configuration(_) | sqlx::Error::InvalidArgument(_) => {
            DatabaseError::validation("The database connection settings are invalid.")
        }
        sqlx::Error::RowNotFound => {
            DatabaseError::query("The database did not return the expected row.")
        }
        sqlx::Error::TypeNotFound { .. }
        | sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::Encode(_) => {
            DatabaseError::query("The database returned a value ServerDesk could not decode.")
        }
        _ => DatabaseError::query(DRIVER_FAILURE_MESSAGE),
    }
}

fn map_mysql_error(number: Option<u16>, sql_state: Option<&str>) -> DatabaseError {
    let error = match number {
        // 1045 access denied, 1698 auth plugin rejection, 1130 host not
        // allowed, 1129 host blocked, 1251/1524 unsupported auth plugin,
        // 1862 expired password, 3118 locked account.
        Some(1045 | 1698 | 1130 | 1129 | 1251 | 1524 | 1862 | 3118) => {
            DatabaseError::database_auth(
                "The database rejected the username, password, or authentication method.",
            )
        }
        // 1044 database access denied, 1142 table denied, 1143 column denied,
        // 1227 specific privilege denied, 1370 routine denied, 1290 server
        // option prevents the statement.
        Some(1044 | 1142 | 1143 | 1227 | 1290 | 1370) => DatabaseError::permission(
            "The database account does not have permission for this operation.",
        ),
        Some(1792) => DatabaseError::permission(
            "The database session is read-only; the statement attempted a write.",
        ),
        Some(1836) => DatabaseError::permission(
            "The database server is in read-only mode; the statement attempted a write.",
        ),
        // 1217/1451 rows referenced by a foreign key, 3730 a table referenced
        // by a foreign key, 1553 an index required by a foreign key.
        Some(1217 | 1451 | 1553 | 3730) => DatabaseError::dependency(
            "The operation is blocked by a foreign key or another dependent object.",
        ),
        Some(1205) => DatabaseError::timeout("The database operation timed out."),
        // 1049 unknown database, 2006 server gone, 2013 lost connection.
        Some(1049) => {
            DatabaseError::connection("The database server rejected the selected database name.")
        }
        Some(2006 | 2013) => DatabaseError::connection(CONNECTION_LOST_MESSAGE),
        _ => DatabaseError::query(DRIVER_FAILURE_MESSAGE),
    };

    // Diagnostics contain only the engine error number and SQLSTATE. Server
    // messages are deliberately omitted because they can echo account names,
    // connection details, or values from the statement.
    error.with_details(driver_identifier(number, sql_state))
}

fn driver_identifier(number: Option<u16>, sql_state: Option<&str>) -> String {
    match (number, sql_state) {
        (Some(number), Some(state)) => format!("MySQL error {number} ({state})."),
        (Some(number), None) => format!("MySQL error {number}."),
        (None, Some(state)) => format!("MySQL error state {state}."),
        (None, None) => "The database driver reported an authentication failure.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{
        DatabasePassword, capabilities::CapabilityFlags, errors::DatabaseErrorKind,
        mysql::capabilities,
    };

    fn request() -> DatabaseConnectRequest {
        DatabaseConnectRequest {
            host: "127.0.0.1".to_string(),
            port: 3306,
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            password: DatabasePassword::new("fixture-password"),
            read_only: true,
            ssl_mode: Some("preferred".to_string()),
            connect_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn validates_loopback_requests_before_connecting() {
        validate_connect_request(&request()).expect("valid request");

        for mutate in [
            |request: &mut DatabaseConnectRequest| request.host = "db.internal".to_string(),
            |request: &mut DatabaseConnectRequest| request.host = "127.0.0.1 ".to_string(),
            |request: &mut DatabaseConnectRequest| request.port = 0,
            |request: &mut DatabaseConnectRequest| request.username = " ".to_string(),
            |request: &mut DatabaseConnectRequest| request.connect_timeout = Duration::ZERO,
        ] {
            let mut request = request();
            mutate(&mut request);
            assert!(validate_connect_request(&request).is_err());
        }
    }

    #[test]
    fn accepts_supported_ssl_mode_spellings() {
        for (value, expected) in [
            (None, "Preferred"),
            (Some("preferred"), "Preferred"),
            (Some("disabled"), "Disabled"),
            (Some("required"), "Required"),
            (Some("verify-ca"), "VerifyCa"),
            (Some("verify_identity"), "VerifyIdentity"),
        ] {
            let mut request = request();
            request.ssl_mode = value.map(str::to_string);
            assert_eq!(
                format!("{:?}", ssl_mode(&request).expect("ssl mode")),
                expected
            );
        }

        let mut request = request();
        request.ssl_mode = Some("starttls".to_string());
        assert_eq!(
            ssl_mode(&request).expect_err("unknown ssl mode").kind,
            DatabaseErrorKind::Validation
        );
    }

    #[test]
    fn maps_authentication_permission_timeout_and_protocol_failures() {
        for (number, expected) in [
            (1045, DatabaseErrorKind::DatabaseAuth),
            (1698, DatabaseErrorKind::DatabaseAuth),
            (1044, DatabaseErrorKind::Permission),
            (1142, DatabaseErrorKind::Permission),
            (1792, DatabaseErrorKind::Permission),
            (1217, DatabaseErrorKind::Dependency),
            (1451, DatabaseErrorKind::Dependency),
            (1553, DatabaseErrorKind::Dependency),
            (3730, DatabaseErrorKind::Dependency),
            (1205, DatabaseErrorKind::Timeout),
            (1049, DatabaseErrorKind::Connection),
        ] {
            let error = map_mysql_error(Some(number), Some("28000"));
            assert_eq!(error.kind, expected, "MySQL error {number}");
        }

        let error = map_sqlx_error(sqlx::Error::Protocol("bad packet".to_string()));
        assert_eq!(error.kind, DatabaseErrorKind::Connection);

        let error = map_sqlx_error(sqlx::Error::PoolTimedOut);
        assert_eq!(error.kind, DatabaseErrorKind::Timeout);

        let error = map_sqlx_error(sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        )));
        assert_eq!(error.kind, DatabaseErrorKind::Connection);
    }

    #[test]
    fn authentication_errors_do_not_echo_driver_text_or_credentials() {
        let error = map_mysql_error(Some(1045), Some("28000"));
        let debug = format!("{error:?}");
        let serialized = serde_json::to_string(&error).expect("serialized error");

        assert_eq!(error.kind, DatabaseErrorKind::DatabaseAuth);
        assert!(!debug.contains("super-secret-token"));
        assert!(!serialized.contains("super-secret-token"));
        assert!(!debug.contains("mysql://"));
        assert!(debug.contains("1045"));
    }

    #[test]
    fn query_diagnostics_are_redacted_and_bounded() {
        let error = map_mysql_error(Some(1064), Some("42000"));
        let details = error.details.expect("details");

        assert_eq!(error.kind, DatabaseErrorKind::Query);
        assert!(!details.contains("super-secret-token"));
        assert!(details.contains("1064"));
    }

    #[test]
    fn permission_failures_are_scoped_without_driver_text_or_sql() {
        let error = map_mysql_error(Some(1142), Some("42000")).scoped_permission("table", "users");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(
            error.scope.as_ref().map(|scope| scope.name.as_str()),
            Some("users")
        );
        let debug = format!("{error:?}");
        assert!(!debug.contains("SELECT"));
        assert!(!debug.contains("information_schema"));
        assert!(!debug.contains("super-secret-token"));
    }

    #[test]
    fn capability_flags_follow_the_reported_server_version() {
        let capabilities =
            DatabaseCapabilities::for_engine(DatabaseEngine::Mysql, Some("8.0.36-log"));
        assert!(capabilities.supports(DatabaseAction::BrowseRoles));
        assert!(capabilities.supports(DatabaseAction::CreateCheckConstraint));

        let old = DatabaseCapabilities::for_engine(DatabaseEngine::Mysql, Some("5.7.44-log"));
        assert!(!old.supports(DatabaseAction::BrowseRoles));
        assert!(!old.supports(DatabaseAction::CreateCheckConstraint));
    }

    #[test]
    fn version_parser_keeps_vendor_suffixes_out_of_capability_decisions() {
        assert_eq!(
            DatabaseVersion::parse("11.4.13-MariaDB-ubu2404"),
            Some(DatabaseVersion::new(11, 4, 13))
        );
        assert!(capabilities::flags_for(Some(DatabaseVersion::new(5, 7, 6))).generated_columns);
        assert!(!capabilities::flags_for(Some(DatabaseVersion::new(5, 7, 5))).generated_columns);
        assert!(!CapabilityFlags::default().schemas);
    }

    #[test]
    fn unsupported_ssl_mode_does_not_leak_the_value_through_a_driver_error() {
        let mut request = request();
        request.password = DatabasePassword::new("super-secret-token");
        request.ssl_mode = Some("super-secret-token".to_string());
        let error = ssl_mode(&request).expect_err("unsupported mode");
        let debug = format!("{error:?}");
        assert!(!debug.contains("super-secret-token"));
    }

    fn fixture_column(name: &str, data_type: &str) -> MysqlColumnRecord {
        MysqlColumnRecord {
            name: name.to_string(),
            data_type: data_type.to_string(),
            length: None,
            precision: None,
            scale: None,
            nullable: false,
            default_value: None,
            ordinal_position: 1,
            primary_key: false,
            generated: false,
            generation_expression: None,
            auto_increment: false,
            comment: String::new(),
            collation: None,
        }
    }

    #[test]
    fn mysql_fixture_table_and_column_metadata_match_the_ddl() {
        let summary = MysqlTableRecord {
            database_name: "app".to_string(),
            name: "users".to_string(),
            table_type: "BASE TABLE".to_string(),
            estimated_rows: Some(42),
            size_bytes: Some(16_384),
            engine: Some("InnoDB".to_string()),
        }
        .into_summary(MetadataValue::available("2026-01-02T03:04:05".to_string()));
        assert_eq!(summary.database.as_deref(), Some("app"));
        assert_eq!(summary.schema.as_deref(), Some("app"));
        assert_eq!(summary.kind, TableKind::Table);
        assert_eq!(summary.estimated_rows, MetadataValue::available(42));
        assert_eq!(
            summary.metadata_refreshed_at,
            MetadataValue::available("2026-01-02T03:04:05".to_string())
        );
        assert_eq!(
            summary.metadata.engine,
            MetadataValue::available("InnoDB".to_string())
        );

        let id = MysqlColumnRecord {
            name: "id".to_string(),
            data_type: "bigint(20) unsigned".to_string(),
            precision: Some(20),
            scale: Some(0),
            ordinal_position: 1,
            primary_key: true,
            auto_increment: true,
            ..fixture_column("id", "bigint(20) unsigned")
        }
        .into_summary(true);
        assert_eq!(
            id.identity,
            MetadataValue::available(ColumnIdentity::AutoIncrement)
        );
        assert!(id.primary_key);
        assert_eq!(id.precision, MetadataValue::available(Some(20)));
        assert_eq!(
            id.default_value,
            MetadataValue::<Option<String>>::available(None)
        );

        let email = MysqlColumnRecord {
            name: "email".to_string(),
            data_type: "varchar(255)".to_string(),
            length: Some(255),
            ordinal_position: 2,
            comment: "Login address".to_string(),
            collation: Some("utf8mb4_bin".to_string()),
            ..fixture_column("email", "varchar(255)")
        }
        .into_summary(true);
        assert_eq!(email.length, MetadataValue::available(Some(255)));
        assert_eq!(
            email.comment,
            MetadataValue::available(Some("Login address".to_string()))
        );
        assert_eq!(
            email.collation,
            MetadataValue::available(Some("utf8mb4_bin".to_string()))
        );

        let status = MysqlColumnRecord {
            name: "status".to_string(),
            data_type: "varchar(16)".to_string(),
            length: Some(16),
            default_value: Some("'active'".to_string()),
            ordinal_position: 3,
            ..fixture_column("status", "varchar(16)")
        }
        .into_summary(true);
        assert_eq!(
            status.default_value,
            MetadataValue::available(Some("'active'".to_string()))
        );

        let note = MysqlColumnRecord {
            name: "note".to_string(),
            data_type: "text".to_string(),
            length: Some(65_535),
            ordinal_position: 5,
            nullable: true,
            generated: true,
            generation_expression: Some("concat('mail:',`email`)".to_string()),
            ..fixture_column("note", "text")
        }
        .into_summary(true);
        assert_eq!(note.generated, MetadataValue::available(true));
        assert_eq!(
            note.generation_expression,
            MetadataValue::available(Some("concat('mail:',`email`)".to_string()))
        );
        assert_eq!(
            note.default_value,
            MetadataValue::<Option<String>>::available(None)
        );

        let old_server_note = MysqlColumnRecord {
            name: "note".to_string(),
            data_type: "text".to_string(),
            generated: true,
            generation_expression: Some("concat('mail:',`email`)".to_string()),
            ..fixture_column("note", "text")
        }
        .into_summary(false);
        assert_eq!(old_server_note.generated, MetadataValue::Unavailable);
        assert_eq!(
            old_server_note.generation_expression,
            MetadataValue::Unavailable
        );
    }

    #[test]
    fn mysql_fixture_indexes_and_constraints_group_in_ordinal_order() {
        let indexes = MysqlIndexRow::group(vec![
            MysqlIndexRow {
                name: "PRIMARY".to_string(),
                column_name: Some("id".to_string()),
                unique: true,
                primary: true,
                method: Some("BTREE".to_string()),
            },
            MysqlIndexRow {
                name: "users_email_unique".to_string(),
                column_name: Some("email".to_string()),
                unique: true,
                primary: false,
                method: Some("BTREE".to_string()),
            },
            MysqlIndexRow {
                name: "users_status_index".to_string(),
                column_name: Some("status".to_string()),
                unique: false,
                primary: false,
                method: Some("BTREE".to_string()),
            },
        ]);
        assert_eq!(indexes.len(), 3);
        assert!(indexes[0].primary);
        assert!(indexes[1].unique);
        assert_eq!(
            indexes[2].predicate,
            MetadataValue::<Option<String>>::available(None)
        );

        let constraints = MysqlConstraintRow::group(
            vec![
                MysqlConstraintRow {
                    name: "PRIMARY".to_string(),
                    kind: "PRIMARY KEY".to_string(),
                    database_name: "app".to_string(),
                    table_name: "users".to_string(),
                    column_name: Some("id".to_string()),
                    referenced_schema: None,
                    referenced_table: None,
                    referenced_column: None,
                    on_update: None,
                    on_delete: None,
                    check_expression: None,
                },
                MysqlConstraintRow {
                    name: "users_parent_fk".to_string(),
                    kind: "FOREIGN KEY".to_string(),
                    database_name: "app".to_string(),
                    table_name: "users".to_string(),
                    column_name: Some("parent_id".to_string()),
                    referenced_schema: Some("app".to_string()),
                    referenced_table: Some("users".to_string()),
                    referenced_column: Some("id".to_string()),
                    on_update: Some("CASCADE".to_string()),
                    on_delete: Some("SET NULL".to_string()),
                    check_expression: None,
                },
                MysqlConstraintRow {
                    name: "users_balance_check".to_string(),
                    kind: "CHECK".to_string(),
                    database_name: "app".to_string(),
                    table_name: "users".to_string(),
                    column_name: None,
                    referenced_schema: None,
                    referenced_table: None,
                    referenced_column: None,
                    on_update: None,
                    on_delete: None,
                    check_expression: Some("`balance` >= 0".to_string()),
                },
            ],
            true,
        );
        assert_eq!(constraints[0].kind, ConstraintKind::PrimaryKey);
        let foreign_key = &constraints[1];
        assert_eq!(foreign_key.kind, ConstraintKind::ForeignKey);
        assert_eq!(
            foreign_key.referenced_schema,
            MetadataValue::available(Some("app".to_string()))
        );
        assert_eq!(
            foreign_key.referenced_table,
            MetadataValue::available(Some("users".to_string()))
        );
        assert_eq!(foreign_key.referenced_columns, ["id"]);
        assert_eq!(
            foreign_key.on_update,
            MetadataValue::available(Some("CASCADE".to_string()))
        );
        assert_eq!(
            foreign_key.on_delete,
            MetadataValue::available(Some("SET NULL".to_string()))
        );
        assert_eq!(
            constraints[2].check_expression,
            MetadataValue::available(Some("`balance` >= 0".to_string()))
        );
        assert_eq!(constraints[2].deferrable, MetadataValue::Unavailable);

        let old_checks = MysqlConstraintRow::group(
            vec![MysqlConstraintRow {
                name: "users_balance_check".to_string(),
                kind: "CHECK".to_string(),
                database_name: "app".to_string(),
                table_name: "users".to_string(),
                column_name: None,
                referenced_schema: None,
                referenced_table: None,
                referenced_column: None,
                on_update: None,
                on_delete: None,
                check_expression: None,
            }],
            false,
        );
        assert_eq!(old_checks[0].check_expression, MetadataValue::Unavailable);
    }

    #[test]
    fn mysql_fixture_incoming_foreign_keys_keep_every_column() {
        let foreign_keys = MysqlForeignKeyRow::group(vec![
            MysqlForeignKeyRow {
                name: "orders_user_fk".to_string(),
                database_name: "app".to_string(),
                table_name: "orders".to_string(),
                column_name: Some("user_id".to_string()),
                referenced_column: Some("id".to_string()),
                on_update: Some("RESTRICT".to_string()),
                on_delete: Some("CASCADE".to_string()),
            },
            MysqlForeignKeyRow {
                name: "orders_user_fk".to_string(),
                database_name: "app".to_string(),
                table_name: "orders".to_string(),
                column_name: Some("tenant_id".to_string()),
                referenced_column: Some("tenant_id".to_string()),
                on_update: Some("RESTRICT".to_string()),
                on_delete: Some("CASCADE".to_string()),
            },
        ]);
        assert_eq!(foreign_keys.len(), 1);
        assert_eq!(foreign_keys[0].columns, ["user_id", "tenant_id"]);
        assert_eq!(foreign_keys[0].referenced_columns, ["id", "tenant_id"]);
        assert_eq!(
            foreign_keys[0].on_delete,
            MetadataValue::available(Some("CASCADE".to_string()))
        );
    }

    // Live tests are opt-in. They are skipped unless the container fixture
    // environment is present; see docs/database-architecture.md.
    struct LiveMysql {
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    }

    impl LiveMysql {
        fn from_env() -> Option<Self> {
            let host = std::env::var("SERVERDESK_TEST_MYSQL_HOST").ok()?;
            let port = std::env::var("SERVERDESK_TEST_MYSQL_PORT")
                .ok()?
                .parse::<u16>()
                .ok()?;
            let database = std::env::var("SERVERDESK_TEST_MYSQL_DATABASE").ok()?;
            let username = std::env::var("SERVERDESK_TEST_MYSQL_USER").ok()?;
            let password = std::env::var("SERVERDESK_TEST_MYSQL_PASSWORD").ok()?;
            Some(Self {
                host,
                port,
                database,
                username,
                password,
            })
        }

        fn request(&self, read_only: bool) -> DatabaseConnectRequest {
            DatabaseConnectRequest {
                host: self.host.clone(),
                port: self.port,
                socket: None,
                database: Some(self.database.clone()),
                username: self.username.clone(),
                password: DatabasePassword::new(self.password.clone()),
                read_only,
                ssl_mode: Some("preferred".to_string()),
                connect_timeout: Duration::from_secs(10),
            }
        }
    }

    #[tokio::test]
    async fn live_read_only_session_rejects_writes() {
        let Some(fixture) = LiveMysql::from_env() else {
            eprintln!("skipping live MySQL read-only test: fixture environment is not set");
            return;
        };
        let mut adapter = MysqlDatabaseAdapter::new();
        let info = adapter
            .connect(fixture.request(true))
            .await
            .expect("connect to fixture");
        assert!(info.read_only);

        let error = adapter
            .execute_for_test("CREATE TABLE serverdesk_read_only_probe (id INT)")
            .await
            .expect_err("read-only session rejects writes");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert!(error.message.to_ascii_lowercase().contains("read"));

        adapter.close().await.expect("close fixture");
    }

    #[tokio::test]
    async fn live_writable_session_allows_and_cleans_up_a_probe_table() {
        let Some(fixture) = LiveMysql::from_env() else {
            eprintln!("skipping live MySQL writable test: fixture environment is not set");
            return;
        };
        let mut adapter = MysqlDatabaseAdapter::new();
        adapter
            .connect(fixture.request(false))
            .await
            .expect("connect to fixture");

        adapter
            .execute_for_test("DROP VIEW IF EXISTS serverdesk_probe_view")
            .await
            .expect("drop stale probe view");
        adapter
            .execute_for_test("DROP TABLE IF EXISTS serverdesk_writable_probe")
            .await
            .expect("drop stale probe");
        adapter
            .execute_for_test(
                "CREATE TABLE serverdesk_writable_probe \
                 (id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY, \
                  email VARCHAR(255) NOT NULL UNIQUE, \
                  note VARCHAR(255) GENERATED ALWAYS AS (CONCAT('mail:', email)) STORED)",
            )
            .await
            .expect("create probe table");

        adapter
            .execute_for_test(
                "CREATE OR REPLACE VIEW serverdesk_probe_view AS \
                 SELECT id, email FROM serverdesk_writable_probe",
            )
            .await
            .expect("create probe view");
        adapter
            .execute_for_test(
                "CREATE TABLE serverdesk_probe_child (\
                    id BIGINT UNSIGNED NOT NULL PRIMARY KEY, \
                    probe_id BIGINT UNSIGNED NOT NULL, \
                    CONSTRAINT serverdesk_probe_child_fk FOREIGN KEY (probe_id) \
                        REFERENCES serverdesk_writable_probe(id) \
                        ON DELETE CASCADE ON UPDATE RESTRICT\
                 )",
            )
            .await
            .expect("create probe child table");
        adapter
            .execute_for_test(
                "CREATE TRIGGER serverdesk_probe_trigger BEFORE INSERT \
                 ON serverdesk_writable_probe FOR EACH ROW \
                 SET @serverdesk_probe_touched = 1",
            )
            .await
            .expect("create probe trigger");

        let scope = MetadataScope::new(Some(&fixture.database), None).expect("scope");
        let tables = adapter
            .list_tables(&scope, PageRequest::new(500, 0))
            .await
            .expect("list tables");
        assert!(
            tables
                .items
                .iter()
                .any(|table| table.name == "serverdesk_writable_probe")
        );
        let probe = tables
            .items
            .iter()
            .find(|table| table.name == "serverdesk_writable_probe")
            .expect("probe table metadata");
        assert!(probe.metadata.engine.is_available());
        assert!(probe.metadata.size_bytes.is_available());
        assert!(!probe.metadata.owner.is_available());

        let views = adapter
            .list_views(&scope, PageRequest::new(500, 0))
            .await
            .expect("list views");
        assert!(
            views
                .items
                .iter()
                .any(|view| view.name == "serverdesk_probe_view")
        );
        let counts = adapter.object_counts(&scope).await.expect("object counts");
        assert!(counts.tables.value().is_some_and(|value| *value >= 1));
        assert!(counts.views.value().is_some_and(|value| *value >= 1));
        assert_eq!(counts.sequences, MetadataValue::Unavailable);

        let databases = adapter
            .list_databases(PageRequest::new(500, 0))
            .await
            .expect("list databases");
        let database = databases
            .items
            .iter()
            .find(|database| database.name == fixture.database)
            .expect("fixture database");
        assert!(database.metadata.encoding.is_available());
        assert!(database.metadata.collation.is_available());
        assert!(database.metadata.size_bytes.is_available());
        assert!(!database.metadata.owner.is_available());

        let table = TableRef::new(Some(&fixture.database), None, "serverdesk_writable_probe")
            .expect("table ref");
        let columns = adapter
            .list_columns(&table, PageRequest::new(500, 0))
            .await
            .expect("list columns");
        assert_eq!(columns.items.len(), 3);
        assert!(columns.items[0].primary_key);
        assert_eq!(
            columns.items[0].identity,
            MetadataValue::available(ColumnIdentity::AutoIncrement)
        );
        assert_eq!(columns.items[2].generated, MetadataValue::available(true));

        let indexes = adapter
            .list_indexes(&table, PageRequest::new(500, 0))
            .await
            .expect("list indexes");
        assert!(indexes.items.iter().any(|index| index.primary));
        assert!(indexes.items.iter().any(|index| index.unique));

        let constraints = adapter
            .list_constraints(&table, PageRequest::new(500, 0))
            .await
            .expect("list constraints");
        assert!(
            constraints
                .items
                .iter()
                .any(|constraint| constraint.kind == ConstraintKind::PrimaryKey)
        );

        let dependencies = adapter
            .table_dependencies(&table, 50)
            .await
            .expect("table dependencies");
        let foreign_keys = dependencies
            .incoming_foreign_keys
            .value()
            .expect("incoming foreign keys");
        assert!(foreign_keys.items.iter().any(|dependency| {
            dependency.name == "serverdesk_probe_child_fk"
                && dependency.table.as_deref() == Some("serverdesk_probe_child")
                && dependency.columns == ["probe_id"]
                && dependency.referenced_columns == ["id"]
        }));
        assert_eq!(dependencies.dependent_views, MetadataValue::Unavailable);
        assert_eq!(dependencies.owned_sequences, MetadataValue::Unavailable);
        let triggers = dependencies.triggers.value().expect("triggers");
        assert!(
            triggers
                .items
                .iter()
                .any(|dependency| dependency.name == "serverdesk_probe_trigger")
        );

        adapter
            .execute_for_test("DROP TABLE serverdesk_probe_child")
            .await
            .expect("drop probe child table");
        adapter
            .execute_for_test("DROP VIEW serverdesk_probe_view")
            .await
            .expect("drop probe view");
        adapter
            .execute_for_test("DROP TABLE serverdesk_writable_probe")
            .await
            .expect("drop probe table");
        adapter.close().await.expect("close fixture");
    }

    #[tokio::test]
    async fn live_version_and_capabilities_reflect_the_fixture() {
        let Some(fixture) = LiveMysql::from_env() else {
            eprintln!("skipping live MySQL version test: fixture environment is not set");
            return;
        };
        let mut adapter = MysqlDatabaseAdapter::new();
        let info = adapter
            .connect(fixture.request(true))
            .await
            .expect("connect to fixture");

        assert_eq!(info.engine, DatabaseEngine::Mysql);
        assert!(!info.version.raw.is_empty());
        assert_eq!(
            info.version.version,
            DatabaseVersion::parse(&info.version.raw).expect("parsed version")
        );
        assert_eq!(adapter.capabilities().version(), Some(info.version.version));
        assert_eq!(adapter.raw_version(), Some(info.version.raw.as_str()));

        let ping = adapter.ping().await.expect("ping fixture");
        assert!(ping.latency_ms < 10_000);
        let version = adapter.version().await.expect("version fixture");
        assert_eq!(version, info.version);

        adapter.close().await.expect("close fixture");
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn live_wrong_credentials_produce_a_typed_authentication_error() {
        let Some(fixture) = LiveMysql::from_env() else {
            eprintln!("skipping live MySQL authentication test: fixture environment is not set");
            return;
        };
        let mut adapter = MysqlDatabaseAdapter::new();
        let mut request = fixture.request(true);
        let wrong_password = "serverdesk-wrong-password-token".to_string();
        request.password = DatabasePassword::new(wrong_password.clone());

        let error = adapter
            .connect(request)
            .await
            .expect_err("wrong credentials must fail");
        let debug = format!("{error:?}");
        let serialized = serde_json::to_string(&error).expect("serialized error");

        assert_eq!(error.kind, DatabaseErrorKind::DatabaseAuth);
        assert!(!debug.contains(&wrong_password));
        assert!(!serialized.contains(&wrong_password));
        assert!(!adapter.is_connected());
    }
}
