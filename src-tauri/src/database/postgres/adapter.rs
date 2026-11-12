//! SQLx-backed PostgreSQL adapter.
//!
//! The adapter connects only to the loopback endpoint published by an SSH
//! tunnel. It keeps one authenticated connection, applies the profile's
//! read-only session mode, discovers the server version and current database,
//! and translates every driver, authentication, permission, timeout, and
//! protocol failure into the shared [`DatabaseError`] taxonomy. Raw driver
//! errors and connection strings never cross this module boundary.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use sqlx::{
    Connection, PgConnection, Postgres, Row,
    postgres::{PgConnectOptions, PgRow, PgSslMode},
};
use tokio::{
    sync::Mutex,
    time::{timeout, timeout_at},
};

use crate::storage::DatabaseEngine;

use super::{POSTGRES_DIALECT, actions as engine_actions, sql};
use crate::database::{
    actions::{DatabaseDdlPreview, ValidatedDatabaseAction},
    capabilities::{DatabaseAction, DatabaseCapabilities, DatabaseVersion},
    dialect::{Identifier, SqlDialect},
    errors::DatabaseError,
    pagination::{Bounded, Page, PageRequest},
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
const CROSS_DATABASE_MESSAGE: &str =
    "PostgreSQL metadata can only be browsed inside the connected database.";

/// SQLx-backed adapter for one PostgreSQL profile.
pub struct PostgresDatabaseAdapter {
    connection: Mutex<Option<PgConnection>>,
    connected: AtomicBool,
    capabilities: DatabaseCapabilities,
    version: Option<DatabaseVersionInfo>,
    current_database: Option<String>,
    read_only: bool,
    connect_request: Option<DatabaseConnectRequest>,
    operation_timeout: Duration,
}

impl Default for PostgresDatabaseAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl PostgresDatabaseAdapter {
    pub fn new() -> Self {
        Self {
            connection: Mutex::new(None),
            connected: AtomicBool::new(false),
            capabilities: DatabaseCapabilities::unknown(DatabaseEngine::Postgresql),
            version: None,
            current_database: None,
            read_only: false,
            connect_request: None,
            operation_timeout: Duration::from_secs(10),
        }
    }

    /// Returns the raw server version observed during the last successful
    /// connect, or `None` before the first connect.
    pub fn raw_version(&self) -> Option<&str> {
        self.version.as_ref().map(|version| version.raw.as_str())
    }

    /// Returns the database reported by the server after the last connect.
    pub fn current_database(&self) -> Option<&str> {
        self.current_database.as_deref()
    }

    async fn lock_connection(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<PgConnection>>, DatabaseError> {
        Ok(self.connection.lock().await)
    }

    async fn live_connection(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<PgConnection>>, DatabaseError> {
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

    #[cfg(test)]
    async fn execute_for_test(
        &self,
        statement: &str,
    ) -> Result<sqlx::postgres::PgQueryResult, DatabaseError> {
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

    async fn query_server_info(
        connection: &mut PgConnection,
        request_timeout: Duration,
    ) -> Result<(String, String), DatabaseError> {
        let row = timeout_at(
            tokio::time::Instant::now() + request_timeout,
            sqlx::query(sql::SERVER_INFO).fetch_one(&mut *connection),
        )
        .await
        .map_err(|_| {
            DatabaseError::timeout(format!(
                "The database did not report its identity within {} seconds.",
                request_timeout.as_secs().max(1)
            ))
        })?
        .map_err(map_sqlx_error)?;

        Ok((decode(&row, "version")?, decode(&row, "database_name")?))
    }

    /// Opens one authenticated read-only or read-write session and reads the
    /// server identity. The caller owns the connection and can close it if a
    /// later setup step fails.
    async fn open_connection(
        request: &DatabaseConnectRequest,
    ) -> Result<
        (
            PgConnection,
            DatabaseVersionInfo,
            String,
            DatabaseCapabilities,
        ),
        DatabaseError,
    > {
        validate_connect_request(request)?;
        let ssl_mode = ssl_mode(request)?;
        let connect_timeout = request.connect_timeout;

        let mut options = PgConnectOptions::new()
            .host(request.host.trim())
            .port(request.port)
            .username(request.username.trim())
            .password(request.password.expose_secret())
            .ssl_mode(ssl_mode);

        if let Some(database) = request.database.as_deref() {
            options = options.database(database);
        }

        let mut connection = timeout(connect_timeout, PgConnection::connect_with(&options))
            .await
            .map_err(|_| {
                DatabaseError::timeout(format!(
                    "The database connection timed out after {} seconds.",
                    connect_timeout.as_secs().max(1)
                ))
            })?
            .map_err(map_sqlx_error)?;

        if request.read_only {
            // PostgreSQL rejects writes to non-temporary objects inside a
            // read-only transaction. Failing to set the mode aborts the
            // connect instead of silently opening a writable session.
            if let Err(error) = timeout_at(
                tokio::time::Instant::now() + connect_timeout,
                sqlx::query(sql::SET_READ_ONLY).execute(&mut connection),
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

        let (raw_version, current_database) =
            match Self::query_server_info(&mut connection, connect_timeout).await {
                Ok(info) => info,
                Err(error) => {
                    let _ = connection.close().await;
                    return Err(error);
                }
            };
        let parsed_version = DatabaseVersion::parse(&raw_version);
        let capabilities =
            DatabaseCapabilities::for_engine(DatabaseEngine::Postgresql, Some(&raw_version));
        let version = DatabaseVersionInfo {
            version: parsed_version.unwrap_or(DatabaseVersion::new(0, 0, 0)),
            raw: raw_version,
        };

        Ok((connection, version, current_database, capabilities))
    }

    fn connection_info(&self) -> Result<DatabaseConnectionInfo, DatabaseError> {
        let version = self
            .version
            .clone()
            .ok_or_else(|| DatabaseError::internal("The database version was not recorded."))?;
        Ok(DatabaseConnectionInfo {
            engine: DatabaseEngine::Postgresql,
            version,
            database: self.current_database.clone(),
            read_only: self.read_only,
            capabilities: self.capabilities.snapshot(),
        })
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

    fn ensure_database_matches(&self, database: Option<&str>) -> Result<(), DatabaseError> {
        if database.is_some_and(|database| self.current_database.as_deref() != Some(database)) {
            return Err(
                DatabaseError::validation(CROSS_DATABASE_MESSAGE).with_details(
                    "PostgreSQL stores databases as separate catalogs; reconnect with the target \
                 database selected before browsing its schemas.",
                ),
            );
        }
        Ok(())
    }

    fn schema_filter(&self, scope: &MetadataScope) -> Result<Option<String>, DatabaseError> {
        self.ensure_database_matches(scope.database.as_ref().map(Identifier::as_str))?;
        Ok(scope
            .schema
            .as_ref()
            .map(|schema| schema.as_str().to_string()))
    }

    fn table_schema_filter(&self, table: &TableRef) -> Result<Option<String>, DatabaseError> {
        self.ensure_database_matches(table.database.as_ref().map(Identifier::as_str))?;
        Ok(table
            .schema
            .as_ref()
            .map(|schema| schema.as_str().to_string()))
    }
}

#[async_trait]
impl DatabaseAdapter for PostgresDatabaseAdapter {
    fn engine(&self) -> DatabaseEngine {
        DatabaseEngine::Postgresql
    }

    fn capabilities(&self) -> &DatabaseCapabilities {
        &self.capabilities
    }

    fn dialect(&self) -> &dyn SqlDialect {
        &POSTGRES_DIALECT
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
        self.capabilities = DatabaseCapabilities::unknown(DatabaseEngine::Postgresql);

        let (connection, version, current_database, capabilities) =
            Self::open_connection(&request).await?;

        *self.connection.get_mut() = Some(connection);
        self.version = Some(version.clone());
        self.current_database = Some(current_database.clone());
        self.read_only = request.read_only;
        self.connect_request = Some(request);
        self.capabilities = capabilities.clone();
        self.connected.store(true, Ordering::SeqCst);

        Ok(DatabaseConnectionInfo {
            engine: DatabaseEngine::Postgresql,
            version,
            database: Some(current_database),
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
        self.connect_request = None;
        self.capabilities = DatabaseCapabilities::unknown(DatabaseEngine::Postgresql);

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
                let size_bytes = decode::<Option<i64>>(row, "size_bytes")?
                    .and_then(|size| u64::try_from(size).ok());
                Ok(DatabaseSummary {
                    system: is_system_database(&name),
                    name,
                    metadata: ObjectMetadata {
                        owner: decode::<Option<String>>(row, "owner")?.into(),
                        encoding: decode::<Option<String>>(row, "encoding")?.into(),
                        collation: decode::<Option<String>>(row, "collation")?.into(),
                        size_bytes: size_bytes.into(),
                        ..ObjectMetadata::default()
                    },
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
                let size_bytes = decode::<Option<i64>>(row, "size_bytes")?
                    .and_then(|size| u64::try_from(size).ok());
                Ok(DatabaseSummary {
                    system: is_system_database(&name),
                    name,
                    metadata: ObjectMetadata {
                        owner: decode::<Option<String>>(row, "owner")?.into(),
                        encoding: decode::<Option<String>>(row, "encoding")?.into(),
                        collation: decode::<Option<String>>(row, "collation")?.into(),
                        size_bytes: size_bytes.into(),
                        ..ObjectMetadata::default()
                    },
                })
            })
            .transpose()
    }

    async fn preview_database_action(
        &self,
        action: &ValidatedDatabaseAction,
    ) -> Result<DatabaseDdlPreview, DatabaseError> {
        self.require_connection()?;
        self.require_capability(action.capability())?;

        match action {
            ValidatedDatabaseAction::Rename { name, .. }
            | ValidatedDatabaseAction::Drop { name }
                if self.current_database.as_deref() == Some(name.as_str()) =>
            {
                return Err(DatabaseError::validation(
                    "PostgreSQL cannot rename or drop the database used by the current \
                     connection. Set another database active first.",
                ));
            }
            _ => {}
        }

        engine_actions::preview(action)
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
                        "The PostgreSQL connection settings were not recorded for a reconnect.",
                    )
                })?;
                request.database = Some(name.as_str().to_string());

                // Open the replacement session before closing the old one. A
                // failed switch leaves the current browsing context usable.
                let (connection, version, current_database, capabilities) =
                    Self::open_connection(&request).await?;
                if let Some(previous) = self.connection.get_mut().take() {
                    let _ = previous.close().await;
                }

                *self.connection.get_mut() = Some(connection);
                self.version = Some(version);
                self.current_database = Some(current_database);
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
                Ok(SchemaSummary {
                    name: decode(row, "name")?,
                    system: decode(row, "system")?,
                    metadata: ObjectMetadata {
                        owner: decode::<Option<String>>(row, "owner")?.into(),
                        ..ObjectMetadata::default()
                    },
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
        let schema = self.schema_filter(scope)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_TABLES)
                .bind(schema.as_deref())
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| {
            error.scoped_permission(
                "database",
                schema.as_deref().unwrap_or("connected database"),
            )
        })?;

        let tables = rows
            .iter()
            .map(PgTableRecord::from_row)
            .collect::<Result<Vec<_>, DatabaseError>>()?
            .into_iter()
            .map(PgTableRecord::into_summary)
            .collect::<Vec<_>>();

        Ok(Page::from_items(tables, page, None))
    }

    async fn list_views(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<ViewSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseViews)?;
        let schema = self.schema_filter(scope)?;
        let include_materialized = self
            .capabilities
            .supports(DatabaseAction::BrowseMaterializedViews);
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_VIEWS)
                .bind(schema.as_deref())
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .bind(include_materialized)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        let views = rows
            .iter()
            .map(|row| {
                let view_type: String = decode(row, "view_type")?;
                let size_bytes = decode::<Option<i64>>(row, "size_bytes")?
                    .and_then(|size| u64::try_from(size).ok());
                Ok(ViewSummary {
                    database: Some(decode(row, "database_name")?),
                    schema: Some(decode(row, "schema_name")?),
                    name: decode(row, "name")?,
                    kind: view_kind(&view_type),
                    metadata: ObjectMetadata {
                        owner: decode::<Option<String>>(row, "owner")?.into(),
                        size_bytes: size_bytes.into(),
                        ..ObjectMetadata::default()
                    },
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(views, page, None))
    }

    async fn list_sequences(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<SequenceSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseSequences)?;
        let schema = self.schema_filter(scope)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_SEQUENCES)
                .bind(schema.as_deref())
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        let sequences = rows
            .iter()
            .map(|row| {
                let size_bytes = decode::<Option<i64>>(row, "size_bytes")?
                    .and_then(|size| u64::try_from(size).ok());
                Ok(SequenceSummary {
                    database: Some(decode(row, "database_name")?),
                    schema: Some(decode(row, "schema_name")?),
                    name: decode(row, "name")?,
                    data_type: decode::<Option<String>>(row, "data_type")?.into(),
                    start_value: decode::<Option<String>>(row, "start_value")?.into(),
                    increment: decode::<Option<String>>(row, "increment_by")?.into(),
                    metadata: ObjectMetadata {
                        owner: decode::<Option<String>>(row, "owner")?.into(),
                        size_bytes: size_bytes.into(),
                        ..ObjectMetadata::default()
                    },
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(sequences, page, None))
    }

    async fn object_counts(&self, scope: &MetadataScope) -> Result<ObjectCounts, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        let schema = self.schema_filter(scope)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let row = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::OBJECT_COUNTS)
                .bind(schema.as_deref())
                .fetch_one(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        let count = |column: &str| -> Result<MetadataValue<u64>, DatabaseError> {
            let value = decode::<i64>(&row, column)?;
            Ok(u64::try_from(value).map_or(MetadataValue::Unavailable, MetadataValue::available))
        };
        let materialized_views = if self
            .capabilities
            .supports(DatabaseAction::BrowseMaterializedViews)
        {
            count("materialized_view_count")?
        } else {
            MetadataValue::Unavailable
        };
        let sequences = if self.capabilities.supports(DatabaseAction::BrowseSequences) {
            count("sequence_count")?
        } else {
            MetadataValue::Unavailable
        };

        Ok(ObjectCounts {
            tables: count("table_count")?,
            views: count("view_count")?,
            materialized_views,
            sequences,
        })
    }

    async fn list_columns(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ColumnSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseColumns)?;
        let schema = self.table_schema_filter(table)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_COLUMNS)
                .bind(schema.as_deref())
                .bind(table.name.as_str())
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table.name.as_str()))?;

        let generated_supported = self
            .capabilities
            .supports(DatabaseAction::CreateGeneratedColumn);
        let identity_supported = self
            .capabilities
            .supports(DatabaseAction::CreateIdentityColumn);
        let columns = rows
            .iter()
            .map(|row| {
                Ok(PgColumnRecord::from_row(row)?
                    .into_summary(generated_supported, identity_supported))
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(columns, page, None))
    }

    async fn list_indexes(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<IndexSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseIndexes)?;
        let schema = self.table_schema_filter(table)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_INDEXES)
                .bind(schema.as_deref())
                .bind(table.name.as_str())
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table.name.as_str()))?;

        let indexes = rows
            .iter()
            .map(|row| Ok(PgIndexRecord::from_row(row)?.into_summary()))
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(indexes, page, None))
    }

    async fn list_constraints(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ConstraintSummary>, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseConstraints)?;
        let schema = self.table_schema_filter(table)?;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_CONSTRAINTS)
                .bind(schema.as_deref())
                .bind(table.name.as_str())
                .bind(i64::from(page.limit) + 1)
                .bind(i64::try_from(page.offset).unwrap_or(i64::MAX))
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table.name.as_str()))?;

        let constraints = rows
            .iter()
            .map(|row| Ok(PgConstraintRecord::from_row(row)?.into_summary()))
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(constraints, page, None))
    }

    async fn table_dependencies(
        &self,
        table: &TableRef,
        limit: u32,
    ) -> Result<TableDependencies, DatabaseError> {
        self.require_capability(DatabaseAction::BrowseTables)?;
        let schema = self.table_schema_filter(table)?.ok_or_else(|| {
            DatabaseError::validation(
                "Select a PostgreSQL schema before inspecting table dependencies.",
            )
        })?;
        let table_name = table.name.as_str();
        let limit_i64 = i64::from(limit.clamp(1, crate::database::MAX_PAGE_LIMIT));
        let limit_usize = limit_i64 as usize;
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");

        let foreign_key_rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::TABLE_INCOMING_FOREIGN_KEYS)
                .bind(&schema)
                .bind(table_name)
                .bind(limit_i64)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        let incoming_foreign_keys = foreign_key_rows
            .iter()
            .map(|row| {
                Ok(DependencySummary {
                    kind: DependencyKind::IncomingForeignKey,
                    name: decode(row, "name")?,
                    database: Some(decode(row, "database_name")?),
                    schema: Some(decode(row, "schema_name")?),
                    table: Some(decode(row, "table_name")?),
                    columns: decode(row, "columns")?,
                    referenced_columns: decode(row, "referenced_columns")?,
                    on_update: MetadataValue::available(decode::<Option<String>>(
                        row,
                        "on_update",
                    )?),
                    on_delete: MetadataValue::available(decode::<Option<String>>(
                        row,
                        "on_delete",
                    )?),
                    details: MetadataValue::Unavailable,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        let view_rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::TABLE_DEPENDENT_VIEWS)
                .bind(&schema)
                .bind(table_name)
                .bind(limit_i64)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        let dependent_views = view_rows
            .iter()
            .map(|row| {
                let view_kind: String = decode(row, "view_kind")?;
                Ok(DependencySummary {
                    kind: if view_kind.eq_ignore_ascii_case("MATERIALIZED VIEW") {
                        DependencyKind::DependentMaterializedView
                    } else {
                        DependencyKind::DependentView
                    },
                    name: decode(row, "name")?,
                    database: Some(decode(row, "database_name")?),
                    schema: Some(decode(row, "schema_name")?),
                    table: None,
                    columns: Vec::new(),
                    referenced_columns: Vec::new(),
                    on_update: MetadataValue::Unavailable,
                    on_delete: MetadataValue::Unavailable,
                    details: MetadataValue::available(view_kind),
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        let sequence_rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::TABLE_OWNED_SEQUENCES)
                .bind(&schema)
                .bind(table_name)
                .bind(limit_i64)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        let owned_sequences = sequence_rows
            .iter()
            .map(|row| {
                let column: Option<String> = decode(row, "column_name")?;
                Ok(DependencySummary {
                    kind: DependencyKind::OwnedSequence,
                    name: decode(row, "name")?,
                    database: Some(decode(row, "database_name")?),
                    schema: Some(decode(row, "schema_name")?),
                    table: Some(table_name.to_string()),
                    columns: column.into_iter().collect(),
                    referenced_columns: Vec::new(),
                    on_update: MetadataValue::Unavailable,
                    on_delete: MetadataValue::Unavailable,
                    details: MetadataValue::Unavailable,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        let trigger_rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::TABLE_TRIGGERS)
                .bind(&schema)
                .bind(table_name)
                .bind(limit_i64)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)
        .map_err(|error| error.scoped_permission("table", table_name))?;

        let triggers = trigger_rows
            .iter()
            .map(|row| {
                Ok(DependencySummary {
                    kind: DependencyKind::Trigger,
                    name: decode(row, "name")?,
                    database: Some(decode(row, "database_name")?),
                    schema: Some(decode(row, "schema_name")?),
                    table: Some(decode(row, "table_name")?),
                    columns: Vec::new(),
                    referenced_columns: Vec::new(),
                    on_update: MetadataValue::Unavailable,
                    on_delete: MetadataValue::Unavailable,
                    details: MetadataValue::available(decode(row, "definition")?),
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(TableDependencies {
            incoming_foreign_keys: MetadataValue::available(Bounded::new(
                incoming_foreign_keys,
                limit_usize,
            )),
            dependent_views: MetadataValue::available(Bounded::new(dependent_views, limit_usize)),
            owned_sequences: MetadataValue::available(Bounded::new(owned_sequences, limit_usize)),
            triggers: MetadataValue::available(Bounded::new(triggers, limit_usize)),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PgTableRecord {
    database_name: String,
    schema_name: String,
    name: String,
    table_type: String,
    owner: Option<String>,
    size_bytes: Option<i64>,
    estimated_rows: Option<i64>,
    engine: Option<String>,
    metadata_refreshed_at: Option<String>,
}

impl PgTableRecord {
    fn from_row(row: &PgRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            database_name: decode(row, "database_name")?,
            schema_name: decode(row, "schema_name")?,
            name: decode(row, "name")?,
            table_type: decode(row, "table_type")?,
            owner: decode(row, "owner")?,
            size_bytes: decode(row, "size_bytes")?,
            estimated_rows: decode(row, "estimated_rows")?,
            engine: decode(row, "engine")?,
            metadata_refreshed_at: decode(row, "metadata_refreshed_at")?,
        })
    }

    fn into_summary(self) -> TableSummary {
        TableSummary {
            database: Some(self.database_name),
            schema: Some(self.schema_name),
            name: self.name,
            kind: table_kind(&self.table_type),
            estimated_rows: self
                .estimated_rows
                .and_then(|value| u64::try_from(value).ok())
                .into(),
            metadata_refreshed_at: self.metadata_refreshed_at.into(),
            metadata: ObjectMetadata {
                owner: self.owner.into(),
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
struct PgColumnRecord {
    name: String,
    data_type: String,
    length: Option<i64>,
    precision: Option<i64>,
    scale: Option<i64>,
    nullable: bool,
    column_expression: Option<String>,
    ordinal_position: i64,
    primary_key: bool,
    generated: bool,
    identity_generation: String,
    collation: Option<String>,
    comment: Option<String>,
}

impl PgColumnRecord {
    fn from_row(row: &PgRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            name: decode(row, "name")?,
            data_type: decode(row, "data_type")?,
            length: decode(row, "length")?,
            precision: decode(row, "precision")?,
            scale: decode(row, "scale")?,
            nullable: decode(row, "nullable")?,
            column_expression: decode(row, "column_expression")?,
            ordinal_position: decode(row, "ordinal_position")?,
            primary_key: decode(row, "primary_key")?,
            generated: decode(row, "generated")?,
            identity_generation: decode(row, "identity_generation")?,
            collation: decode(row, "collation")?,
            comment: decode(row, "comment")?,
        })
    }

    fn into_summary(self, generated_supported: bool, identity_supported: bool) -> ColumnSummary {
        let generation_expression = if generated_supported {
            MetadataValue::available(if self.generated {
                self.column_expression.clone()
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
        let identity = if identity_supported {
            MetadataValue::available(match self.identity_generation.as_str() {
                "a" => ColumnIdentity::Always,
                "d" => ColumnIdentity::ByDefault,
                _ => ColumnIdentity::None,
            })
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
            default_value: MetadataValue::available(if self.generated {
                None
            } else {
                self.column_expression
            }),
            ordinal_position: u32::try_from(self.ordinal_position).unwrap_or(u32::MAX),
            primary_key: self.primary_key,
            identity,
            generated,
            generation_expression,
            comment: MetadataValue::available(self.comment),
            collation: MetadataValue::available(self.collation),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PgIndexRecord {
    name: String,
    columns: Vec<String>,
    unique: bool,
    primary: bool,
    method: Option<String>,
    predicate: Option<String>,
}

impl PgIndexRecord {
    fn from_row(row: &PgRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            name: decode(row, "name")?,
            columns: decode(row, "columns")?,
            unique: decode(row, "unique_index")?,
            primary: decode(row, "primary_index")?,
            method: decode(row, "method")?,
            predicate: decode(row, "predicate")?,
        })
    }

    fn into_summary(self) -> IndexSummary {
        IndexSummary {
            name: self.name,
            columns: self.columns,
            unique: self.unique,
            primary: self.primary,
            method: self.method.into(),
            predicate: MetadataValue::available(self.predicate),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PgConstraintRecord {
    name: String,
    kind: String,
    columns: Vec<String>,
    referenced_schema: Option<String>,
    referenced_table: Option<String>,
    referenced_columns: Vec<String>,
    on_update: Option<String>,
    on_delete: Option<String>,
    check_expression: Option<String>,
    deferrable: bool,
    initially_deferred: bool,
}

impl PgConstraintRecord {
    fn from_row(row: &PgRow) -> Result<Self, DatabaseError> {
        Ok(Self {
            name: decode(row, "name")?,
            kind: decode(row, "kind")?,
            columns: decode(row, "columns")?,
            referenced_schema: decode(row, "referenced_schema")?,
            referenced_table: decode(row, "referenced_table")?,
            referenced_columns: decode(row, "referenced_columns")?,
            on_update: decode(row, "on_update")?,
            on_delete: decode(row, "on_delete")?,
            check_expression: decode(row, "check_expression")?,
            deferrable: decode(row, "deferrable")?,
            initially_deferred: decode(row, "initially_deferred")?,
        })
    }

    fn into_summary(self) -> ConstraintSummary {
        ConstraintSummary {
            kind: constraint_kind(&self.kind),
            name: self.name,
            columns: self.columns,
            referenced_schema: MetadataValue::available(self.referenced_schema),
            referenced_table: MetadataValue::available(self.referenced_table),
            referenced_columns: self.referenced_columns,
            on_update: MetadataValue::available(self.on_update),
            on_delete: MetadataValue::available(self.on_delete),
            check_expression: MetadataValue::available(self.check_expression),
            deferrable: MetadataValue::available(self.deferrable),
            initially_deferred: MetadataValue::available(self.initially_deferred),
        }
    }
}

fn ssl_mode(request: &DatabaseConnectRequest) -> Result<PgSslMode, DatabaseError> {
    match request
        .ssl_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        None | Some("prefer" | "preferred") => Ok(PgSslMode::Prefer),
        Some("disabled" | "disable" | "off" | "none") => Ok(PgSslMode::Disable),
        Some("allow") => Ok(PgSslMode::Allow),
        Some("required" | "require") => Ok(PgSslMode::Require),
        Some("verify-ca" | "verify_ca") => Ok(PgSslMode::VerifyCa),
        Some("verify-full" | "verify_full" | "verify_identity" | "verify-identity") => {
            Ok(PgSslMode::VerifyFull)
        }
        Some(_) => Err(DatabaseError::validation(
            "The configured database SSL mode is not supported.",
        )),
    }
}

fn is_system_database(name: &str) -> bool {
    matches!(name, "postgres" | "template0" | "template1")
}

fn table_kind(table_type: &str) -> TableKind {
    match table_type.to_ascii_uppercase().as_str() {
        "BASE TABLE" => TableKind::Table,
        "PARTITIONED TABLE" => TableKind::PartitionedTable,
        "VIEW" | "SYSTEM VIEW" => TableKind::View,
        "FOREIGN" | "FOREIGN TABLE" => TableKind::ForeignTable,
        _ => TableKind::Other,
    }
}

fn view_kind(view_type: &str) -> ViewKind {
    match view_type.to_ascii_uppercase().as_str() {
        "MATERIALIZED VIEW" => ViewKind::MaterializedView,
        _ => ViewKind::View,
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

fn decode<'r, T>(row: &'r PgRow, column: &str) -> Result<T, DatabaseError>
where
    T: sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column).map_err(|error| {
        map_sqlx_error(error).with_details(format!(
            "The database value in column {column} could not be decoded."
        ))
    })
}

/// Maps SQLx errors into the shared taxonomy without exposing the raw driver
/// display text. Authentication failures intentionally omit the server
/// message, which can echo the account, host, or connection string.
fn map_sqlx_error(error: sqlx::Error) -> DatabaseError {
    match error {
        sqlx::Error::Database(database_error) => {
            if let Some(postgres_error) =
                database_error.try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
            {
                return map_postgres_error(postgres_error.code());
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

fn map_postgres_error(sql_state: &str) -> DatabaseError {
    let error = match sql_state {
        // 28P01 invalid password, 28000 invalid authorization specification.
        "28P01" | "28000" => DatabaseError::database_auth(
            "The database rejected the username, password, or authentication method.",
        ),
        // 42501 insufficient privilege, 25006 read-only SQL transaction.
        "42501" => DatabaseError::permission(
            "The database account does not have permission for this operation.",
        ),
        "25006" => DatabaseError::permission(
            "The database session is read-only; the statement attempted a write.",
        ),
        // 57014 statement cancelled by a timeout or cancellation request,
        // 55P03 lock not available, 40001 serialization failure.
        "57014" | "55P03" => DatabaseError::timeout("The database operation timed out."),
        // 3D000 invalid catalog name, 57P03 cannot connect now,
        // 53300 too many connections.
        "3D000" => {
            DatabaseError::connection("The database server rejected the selected database name.")
        }
        "57P03" | "53300" => {
            DatabaseError::connection("The database server is not accepting connections.")
        }
        // Class 08 connection exceptions, 57P01 administrator shutdown,
        // 57P02 crash shutdown.
        state if state.starts_with("08") || matches!(state, "57P01" | "57P02") => {
            DatabaseError::connection(CONNECTION_LOST_MESSAGE)
        }
        // 0A000 feature not supported.
        "0A000" => DatabaseError::capability(
            "The database server does not support the requested operation.",
        ),
        _ => DatabaseError::query(DRIVER_FAILURE_MESSAGE),
    };

    // Diagnostics contain only the engine SQLSTATE. Server messages are
    // deliberately omitted because they can echo account names, SQL text,
    // connection details, or values from the statement.
    error.with_details(format!("PostgreSQL SQLSTATE {sql_state}."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{
        DatabasePassword, capabilities::CapabilityFlags, errors::DatabaseErrorKind,
        postgres::capabilities,
    };

    fn request() -> DatabaseConnectRequest {
        DatabaseConnectRequest {
            host: "127.0.0.1".to_string(),
            port: 5432,
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
            (None, "Prefer"),
            (Some("prefer"), "Prefer"),
            (Some("preferred"), "Prefer"),
            (Some("disabled"), "Disable"),
            (Some("allow"), "Allow"),
            (Some("required"), "Require"),
            (Some("verify-ca"), "VerifyCa"),
            (Some("verify_full"), "VerifyFull"),
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
        for (sql_state, expected) in [
            ("28P01", DatabaseErrorKind::DatabaseAuth),
            ("28000", DatabaseErrorKind::DatabaseAuth),
            ("42501", DatabaseErrorKind::Permission),
            ("25006", DatabaseErrorKind::Permission),
            ("57014", DatabaseErrorKind::Timeout),
            ("55P03", DatabaseErrorKind::Timeout),
            ("40001", DatabaseErrorKind::Query),
            ("3D000", DatabaseErrorKind::Connection),
            ("57P03", DatabaseErrorKind::Connection),
            ("53300", DatabaseErrorKind::Connection),
            ("08006", DatabaseErrorKind::Connection),
            ("0A000", DatabaseErrorKind::Capability),
            ("42601", DatabaseErrorKind::Query),
            ("23505", DatabaseErrorKind::Query),
        ] {
            let error = map_postgres_error(sql_state);
            assert_eq!(error.kind, expected, "PostgreSQL SQLSTATE {sql_state}");
        }

        let error = map_sqlx_error(sqlx::Error::Protocol(
            "postgresql://app:super-secret-token@127.0.0.1/app".to_string(),
        ));
        assert_eq!(error.kind, DatabaseErrorKind::Connection);
        assert!(!format!("{error:?}").contains("super-secret-token"));

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
        let error = map_postgres_error("28P01");
        let debug = format!("{error:?}");
        let serialized = serde_json::to_string(&error).expect("serialized error");

        assert_eq!(error.kind, DatabaseErrorKind::DatabaseAuth);
        assert!(!debug.contains("super-secret-token"));
        assert!(!serialized.contains("super-secret-token"));
        assert!(!debug.contains("postgres://"));
        assert!(debug.contains("28P01"));
        assert!(!error.message.contains("app"));
    }

    #[test]
    fn query_diagnostics_are_redacted_and_bounded() {
        let error = map_postgres_error("42601");
        let details = error.details.expect("details");

        assert_eq!(error.kind, DatabaseErrorKind::Query);
        assert!(!details.contains("super-secret-token"));
        assert!(details.contains("42601"));
    }

    #[test]
    fn permission_failures_are_scoped_without_driver_text_or_sql() {
        let error = map_postgres_error("42501").scoped_permission("table", "users");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(
            error.scope.as_ref().map(|scope| scope.name.as_str()),
            Some("users")
        );
        let debug = format!("{error:?}");
        assert!(!debug.contains("SELECT"));
        assert!(!debug.contains("pg_catalog"));
        assert!(!debug.contains("super-secret-token"));
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

    #[test]
    fn database_scope_is_limited_to_the_connected_catalog() {
        let mut adapter = PostgresDatabaseAdapter::new();
        adapter.current_database = Some("app".to_string());

        let matching = MetadataScope::new(Some("app"), Some("public")).expect("scope");
        assert_eq!(
            adapter.schema_filter(&matching).expect("matching database"),
            Some("public".to_string())
        );
        adapter
            .ensure_database_matches(None)
            .expect("no database filter");

        let other = MetadataScope::new(Some("analytics"), Some("public")).expect("scope");
        let error = adapter
            .schema_filter(&other)
            .expect_err("other database must be rejected");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);

        let table = TableRef::new(Some("analytics"), Some("public"), "events").expect("table ref");
        assert_eq!(
            adapter
                .table_schema_filter(&table)
                .expect_err("other database must be rejected")
                .kind,
            DatabaseErrorKind::Validation
        );
    }

    #[test]
    fn capability_flags_follow_the_reported_server_version() {
        let capabilities =
            DatabaseCapabilities::for_engine(DatabaseEngine::Postgresql, Some("PostgreSQL 16.2"));
        assert!(capabilities.supports(DatabaseAction::BrowseDatabases));
        assert!(capabilities.supports(DatabaseAction::BrowseSchemas));
        assert!(capabilities.supports(DatabaseAction::BrowseMaterializedViews));
        assert!(capabilities.supports(DatabaseAction::CreateIdentityColumn));

        let old =
            DatabaseCapabilities::for_engine(DatabaseEngine::Postgresql, Some("PostgreSQL 9.2"));
        assert!(!old.supports(DatabaseAction::BrowseMaterializedViews));
        assert!(!old.supports(DatabaseAction::CreateIdentityColumn));
    }

    #[test]
    fn capability_snapshot_represents_databases_and_schemas_without_sql() {
        let capabilities =
            DatabaseCapabilities::for_engine(DatabaseEngine::Postgresql, Some("PostgreSQL 16.2"));
        let snapshot = capabilities.snapshot();
        let json = serde_json::to_string(&snapshot).expect("capability json");

        assert!(snapshot.flags.schemas);
        assert!(snapshot.actions.contains(&DatabaseAction::BrowseDatabases));
        assert!(snapshot.actions.contains(&DatabaseAction::BrowseSchemas));
        assert!(json.contains("\"browse_databases\""));
        assert!(json.contains("\"browse_schemas\""));
        assert!(!json.contains("SELECT"));
        assert!(!json.contains("pg_catalog"));
    }

    #[test]
    fn metadata_helpers_distinguish_system_and_user_objects() {
        for name in ["postgres", "template0", "template1"] {
            assert!(is_system_database(name));
        }
        assert!(!is_system_database("app"));

        assert_eq!(table_kind("BASE TABLE"), TableKind::Table);
        assert_eq!(table_kind("VIEW"), TableKind::View);
        assert_eq!(table_kind("FOREIGN"), TableKind::ForeignTable);
        assert_eq!(table_kind("LOCAL TEMPORARY"), TableKind::Other);

        assert_eq!(constraint_kind("PRIMARY KEY"), ConstraintKind::PrimaryKey);
        assert_eq!(constraint_kind("FOREIGN KEY"), ConstraintKind::ForeignKey);
        assert_eq!(constraint_kind("EXCLUDE"), ConstraintKind::Other);
        assert!(!CapabilityFlags::default().schemas);
    }

    #[test]
    fn version_parser_keeps_vendor_suffixes_out_of_capability_decisions() {
        assert_eq!(
            DatabaseVersion::parse("PostgreSQL 16.2 (Debian 16.2-1.pgdg120+2)"),
            Some(DatabaseVersion::new(16, 2, 0))
        );
        assert!(capabilities::flags_for(Some(DatabaseVersion::new(12, 0, 0))).generated_columns);
        assert!(!capabilities::flags_for(Some(DatabaseVersion::new(11, 9, 0))).generated_columns);
    }

    #[test]
    fn postgres_fixture_table_and_column_metadata_match_the_ddl() {
        let summary = PgTableRecord {
            database_name: "app".to_string(),
            schema_name: "public".to_string(),
            name: "users".to_string(),
            table_type: "BASE TABLE".to_string(),
            owner: Some("app".to_string()),
            size_bytes: Some(8_192),
            estimated_rows: Some(42),
            engine: Some("heap".to_string()),
            metadata_refreshed_at: Some("2026-01-02T03:04:05Z".to_string()),
        }
        .into_summary();
        assert_eq!(summary.database.as_deref(), Some("app"));
        assert_eq!(summary.schema.as_deref(), Some("public"));
        assert_eq!(summary.kind, TableKind::Table);
        assert_eq!(summary.estimated_rows, MetadataValue::available(42));
        assert_eq!(
            summary.metadata_refreshed_at,
            MetadataValue::available("2026-01-02T03:04:05Z".to_string())
        );
        assert_eq!(
            summary.metadata.owner,
            MetadataValue::available("app".to_string())
        );

        let id = PgColumnRecord {
            name: "id".to_string(),
            data_type: "bigint".to_string(),
            precision: Some(64),
            scale: Some(0),
            ordinal_position: 1,
            primary_key: true,
            identity_generation: "a".to_string(),
            ..fixture_column("id", "bigint")
        }
        .into_summary(true, true);
        assert_eq!(
            id.identity,
            MetadataValue::available(ColumnIdentity::Always)
        );
        assert!(id.primary_key);
        assert_eq!(id.precision, MetadataValue::available(Some(64)));

        let email = PgColumnRecord {
            name: "email".to_string(),
            data_type: "character varying(255)".to_string(),
            length: Some(255),
            ordinal_position: 2,
            collation: Some("default".to_string()),
            comment: Some("Login address".to_string()),
            ..fixture_column("email", "character varying(255)")
        }
        .into_summary(true, true);
        assert_eq!(email.length, MetadataValue::available(Some(255)));
        assert_eq!(
            email.collation,
            MetadataValue::available(Some("default".to_string()))
        );
        assert_eq!(
            email.comment,
            MetadataValue::available(Some("Login address".to_string()))
        );

        let status = PgColumnRecord {
            name: "status".to_string(),
            data_type: "character varying(16)".to_string(),
            length: Some(16),
            column_expression: Some("'active'::character varying".to_string()),
            ordinal_position: 3,
            ..fixture_column("status", "character varying(16)")
        }
        .into_summary(true, true);
        assert_eq!(
            status.default_value,
            MetadataValue::available(Some("'active'::character varying".to_string()))
        );

        let note = PgColumnRecord {
            name: "note".to_string(),
            data_type: "text".to_string(),
            ordinal_position: 5,
            nullable: true,
            generated: true,
            column_expression: Some("((email)::text || ':note'::text)".to_string()),
            ..fixture_column("note", "text")
        }
        .into_summary(true, true);
        assert_eq!(note.generated, MetadataValue::available(true));
        assert_eq!(
            note.generation_expression,
            MetadataValue::available(Some("((email)::text || ':note'::text)".to_string()))
        );
        assert_eq!(
            note.default_value,
            MetadataValue::<Option<String>>::available(None)
        );

        let old_server_note = PgColumnRecord {
            name: "note".to_string(),
            data_type: "text".to_string(),
            generated: true,
            column_expression: Some("((email)::text || ':note'::text)".to_string()),
            ..fixture_column("note", "text")
        }
        .into_summary(false, false);
        assert_eq!(old_server_note.generated, MetadataValue::Unavailable);
        assert_eq!(old_server_note.identity, MetadataValue::Unavailable);
        assert_eq!(
            old_server_note.generation_expression,
            MetadataValue::Unavailable
        );
    }

    #[test]
    fn postgres_fixture_indexes_and_constraints_match_the_ddl() {
        let primary = PgIndexRecord {
            name: "users_pkey".to_string(),
            columns: vec!["id".to_string()],
            unique: true,
            primary: true,
            method: Some("btree".to_string()),
            predicate: None,
        }
        .into_summary();
        assert!(primary.primary);
        assert_eq!(
            primary.method,
            MetadataValue::available("btree".to_string())
        );
        assert_eq!(
            primary.predicate,
            MetadataValue::<Option<String>>::available(None)
        );

        let partial = PgIndexRecord {
            name: "users_active_email_key".to_string(),
            columns: vec!["email".to_string()],
            unique: true,
            primary: false,
            method: Some("btree".to_string()),
            predicate: Some("(status = 'active'::text)".to_string()),
        }
        .into_summary();
        assert_eq!(
            partial.predicate,
            MetadataValue::available(Some("(status = 'active'::text)".to_string()))
        );

        let foreign_key = PgConstraintRecord {
            name: "users_parent_fk".to_string(),
            kind: "FOREIGN KEY".to_string(),
            columns: vec!["parent_id".to_string()],
            referenced_schema: Some("public".to_string()),
            referenced_table: Some("users".to_string()),
            referenced_columns: vec!["id".to_string()],
            on_update: Some("CASCADE".to_string()),
            on_delete: Some("SET NULL".to_string()),
            check_expression: None,
            deferrable: false,
            initially_deferred: false,
        }
        .into_summary();
        assert_eq!(foreign_key.kind, ConstraintKind::ForeignKey);
        assert_eq!(
            foreign_key.referenced_schema,
            MetadataValue::available(Some("public".to_string()))
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

        let check = PgConstraintRecord {
            name: "users_balance_check".to_string(),
            kind: "CHECK".to_string(),
            columns: Vec::new(),
            referenced_schema: None,
            referenced_table: None,
            referenced_columns: Vec::new(),
            on_update: None,
            on_delete: None,
            check_expression: Some("CHECK ((balance >= (0)::numeric))".to_string()),
            deferrable: true,
            initially_deferred: false,
        }
        .into_summary();
        assert_eq!(check.kind, ConstraintKind::Check);
        assert_eq!(
            check.check_expression,
            MetadataValue::available(Some("CHECK ((balance >= (0)::numeric))".to_string()))
        );
        assert_eq!(check.deferrable, MetadataValue::available(true));
        assert_eq!(check.initially_deferred, MetadataValue::available(false));
    }

    fn fixture_column(name: &str, data_type: &str) -> PgColumnRecord {
        PgColumnRecord {
            name: name.to_string(),
            data_type: data_type.to_string(),
            length: None,
            precision: None,
            scale: None,
            nullable: false,
            column_expression: None,
            ordinal_position: 1,
            primary_key: false,
            generated: false,
            identity_generation: String::new(),
            collation: None,
            comment: None,
        }
    }

    // Live tests are opt-in. They are skipped unless the container fixture
    // environment is present; see docs/database-architecture.md.
    struct LivePostgres {
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    }

    impl LivePostgres {
        fn from_env() -> Option<Self> {
            Some(Self {
                host: std::env::var("SERVERDESK_TEST_POSTGRES_HOST").ok()?,
                port: std::env::var("SERVERDESK_TEST_POSTGRES_PORT")
                    .ok()?
                    .parse::<u16>()
                    .ok()?,
                database: std::env::var("SERVERDESK_TEST_POSTGRES_DATABASE").ok()?,
                username: std::env::var("SERVERDESK_TEST_POSTGRES_USER").ok()?,
                password: std::env::var("SERVERDESK_TEST_POSTGRES_PASSWORD").ok()?,
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
        let Some(fixture) = LivePostgres::from_env() else {
            eprintln!("skipping live PostgreSQL read-only test: fixture environment is not set");
            return;
        };
        let mut adapter = PostgresDatabaseAdapter::new();
        let info = adapter
            .connect(fixture.request(true))
            .await
            .expect("connect to fixture");
        assert!(info.read_only);

        let error = adapter
            .execute_for_test("CREATE TABLE serverdesk_read_only_probe (id integer)")
            .await
            .expect_err("read-only session rejects writes");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert!(error.message.to_ascii_lowercase().contains("read"));

        adapter.close().await.expect("close fixture");
    }

    #[tokio::test]
    async fn live_writable_session_allows_and_cleans_up_a_probe_table() {
        let Some(fixture) = LivePostgres::from_env() else {
            eprintln!("skipping live PostgreSQL writable test: fixture environment is not set");
            return;
        };
        let mut adapter = PostgresDatabaseAdapter::new();
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
                "CREATE TABLE serverdesk_writable_probe (
                    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                    email text NOT NULL UNIQUE,
                    note text GENERATED ALWAYS AS (email || ':note') STORED,
                    CONSTRAINT serverdesk_note_check CHECK (length(note) > 0)
                )",
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
                "CREATE SEQUENCE IF NOT EXISTS serverdesk_probe_seq START WITH 7 INCREMENT BY 2",
            )
            .await
            .expect("create probe sequence");
        adapter
            .execute_for_test(
                "CREATE TABLE serverdesk_probe_child (
                    id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                    probe_id bigint NOT NULL,
                    CONSTRAINT serverdesk_probe_child_fk FOREIGN KEY (probe_id)
                        REFERENCES serverdesk_writable_probe(id)
                        ON DELETE CASCADE ON UPDATE RESTRICT
                )",
            )
            .await
            .expect("create probe child table");
        adapter
            .execute_for_test(
                "CREATE FUNCTION serverdesk_probe_trigger_fn() RETURNS trigger
                 LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$",
            )
            .await
            .expect("create probe trigger function");
        adapter
            .execute_for_test(
                "CREATE TRIGGER serverdesk_probe_trigger BEFORE INSERT
                 ON serverdesk_writable_probe FOR EACH ROW
                 EXECUTE FUNCTION serverdesk_probe_trigger_fn()",
            )
            .await
            .expect("create probe trigger");
        adapter
            .execute_for_test("ANALYZE serverdesk_writable_probe")
            .await
            .expect("analyze probe table");

        let scope = MetadataScope::new(Some(&fixture.database), Some("public")).expect("scope");
        let tables = adapter
            .list_tables(&scope, PageRequest::new(500, 0))
            .await
            .expect("list tables");
        let probe = tables
            .items
            .iter()
            .find(|table| table.name == "serverdesk_writable_probe")
            .expect("probe table metadata");
        assert!(probe.metadata.owner.is_available());
        assert!(probe.metadata.size_bytes.is_available());
        assert!(probe.metadata.engine.is_available());
        assert!(probe.estimated_rows.is_available());
        assert!(probe.metadata_refreshed_at.is_available());

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

        let sequences = adapter
            .list_sequences(&scope, PageRequest::new(500, 0))
            .await
            .expect("list sequences");
        let sequence = sequences
            .items
            .iter()
            .find(|sequence| sequence.name == "serverdesk_probe_seq")
            .expect("probe sequence metadata");
        assert_eq!(
            sequence.start_value,
            MetadataValue::available("7".to_string())
        );
        assert_eq!(
            sequence.increment,
            MetadataValue::available("2".to_string())
        );
        assert!(sequence.metadata.owner.is_available());

        let counts = adapter.object_counts(&scope).await.expect("object counts");
        assert!(counts.tables.value().is_some_and(|value| *value >= 1));
        assert!(counts.views.value().is_some_and(|value| *value >= 1));
        assert!(counts.sequences.value().is_some_and(|value| *value >= 1));
        assert!(counts.materialized_views.is_available());

        let table = TableRef::new(
            Some(&fixture.database),
            Some("public"),
            "serverdesk_writable_probe",
        )
        .expect("table ref");
        let columns = adapter
            .list_columns(&table, PageRequest::new(500, 0))
            .await
            .expect("list columns");
        assert_eq!(columns.items.len(), 3);
        assert!(columns.items[0].primary_key);
        assert_eq!(
            columns.items[0].identity,
            MetadataValue::available(ColumnIdentity::Always)
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
        assert!(
            constraints
                .items
                .iter()
                .any(|constraint| constraint.kind == ConstraintKind::Check)
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
                && dependency.on_delete == MetadataValue::available(Some("CASCADE".to_string()))
        }));
        let views = dependencies.dependent_views.value().expect("views");
        assert!(
            views
                .items
                .iter()
                .any(|dependency| dependency.name == "serverdesk_probe_view")
        );
        let sequences = dependencies
            .owned_sequences
            .value()
            .expect("owned sequences");
        assert!(sequences.items.iter().any(|dependency| {
            dependency.name == "serverdesk_writable_probe_id_seq" && dependency.columns == ["id"]
        }));
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
            .execute_for_test("DROP SEQUENCE serverdesk_probe_seq")
            .await
            .expect("drop probe sequence");
        adapter
            .execute_for_test("DROP TABLE serverdesk_writable_probe")
            .await
            .expect("drop probe table");
        adapter
            .execute_for_test("DROP FUNCTION serverdesk_probe_trigger_fn()")
            .await
            .expect("drop probe trigger function");
        adapter.close().await.expect("close fixture");
    }

    #[tokio::test]
    async fn live_version_and_capabilities_reflect_the_fixture() {
        let Some(fixture) = LivePostgres::from_env() else {
            eprintln!("skipping live PostgreSQL version test: fixture environment is not set");
            return;
        };
        let mut adapter = PostgresDatabaseAdapter::new();
        let info = adapter
            .connect(fixture.request(true))
            .await
            .expect("connect to fixture");

        assert_eq!(info.engine, DatabaseEngine::Postgresql);
        assert_eq!(info.database.as_deref(), Some(fixture.database.as_str()));
        assert!(!info.version.raw.is_empty());
        assert_eq!(
            info.version.version,
            DatabaseVersion::parse(&info.version.raw).expect("parsed version")
        );
        assert_eq!(adapter.capabilities().version(), Some(info.version.version));
        assert_eq!(adapter.raw_version(), Some(info.version.raw.as_str()));
        assert_eq!(adapter.current_database(), Some(fixture.database.as_str()));

        let ping = adapter.ping().await.expect("ping fixture");
        assert!(ping.latency_ms < 10_000);
        let version = adapter.version().await.expect("version fixture");
        assert_eq!(version, info.version);

        adapter.close().await.expect("close fixture");
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn live_wrong_credentials_produce_a_typed_authentication_error() {
        let Some(fixture) = LivePostgres::from_env() else {
            eprintln!(
                "skipping live PostgreSQL authentication test: fixture environment is not set"
            );
            return;
        };
        let mut adapter = PostgresDatabaseAdapter::new();
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
