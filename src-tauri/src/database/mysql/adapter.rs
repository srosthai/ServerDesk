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

use super::{MYSQL_DIALECT, sql};
use crate::database::{
    capabilities::{DatabaseAction, DatabaseCapabilities, DatabaseVersion},
    dialect::SqlDialect,
    errors::DatabaseError,
    pagination::{Page, PageRequest},
    traits::{
        ColumnSummary, ConstraintKind, ConstraintSummary, DatabaseAdapter, DatabaseConnectRequest,
        DatabaseConnectionInfo, DatabasePing, DatabaseSummary, DatabaseVersionInfo, IndexSummary,
        MetadataScope, MetadataValue, ObjectCounts, ObjectMetadata, SchemaSummary, SequenceSummary,
        TableKind, TableRef, TableSummary, ViewKind, ViewSummary,
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
            operation_timeout: Duration::from_secs(10),
        }
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

    async fn connect(
        &mut self,
        request: DatabaseConnectRequest,
    ) -> Result<DatabaseConnectionInfo, DatabaseError> {
        validate_connect_request(&request)?;
        let ssl_mode = ssl_mode(&request)?;
        let connect_timeout = request.connect_timeout;
        self.operation_timeout = connect_timeout.max(Duration::from_secs(1));

        // Replacing a live connection must not leave the previous driver
        // session open after the new attempt fails.
        if let Some(previous) = self.connection.get_mut().take() {
            let _ = previous.close().await;
        }
        self.connected.store(false, Ordering::SeqCst);
        self.version = None;
        self.capabilities = DatabaseCapabilities::unknown(DatabaseEngine::Mysql);

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

        *self.connection.get_mut() = Some(connection);
        self.version = Some(version.clone());
        self.capabilities = capabilities.clone();
        self.connected.store(true, Ordering::SeqCst);

        Ok(DatabaseConnectionInfo {
            engine: DatabaseEngine::Mysql,
            version,
            database: request.database,
            read_only: request.read_only,
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
        .map_err(map_sqlx_error)?;

        let tables = rows
            .iter()
            .map(|row| {
                let database: String = decode(row, "database_name")?;
                let table_type: String = decode(row, "table_type")?;
                Ok(TableSummary {
                    database: Some(database.clone()),
                    schema: Some(database),
                    name: decode(row, "name")?,
                    kind: table_kind(&table_type),
                    estimated_rows: decode::<Option<u64>>(row, "estimated_rows")?.into(),
                    metadata: ObjectMetadata {
                        size_bytes: decode::<Option<u64>>(row, "size_bytes")?.into(),
                        engine: decode::<Option<String>>(row, "engine")?.into(),
                        ..ObjectMetadata::default()
                    },
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Page::from_items(tables, page, None))
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
            tables: MetadataValue::available(decode::<u64>(&row, "table_count")?),
            views: MetadataValue::available(decode::<u64>(&row, "view_count")?),
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
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let statement = format!("{} LIMIT ? OFFSET ?", sql::LIST_COLUMNS);
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sqlx::AssertSqlSafe(statement))
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
        .map_err(map_sqlx_error)?;

        let columns = rows
            .iter()
            .map(|row| {
                let ordinal_position: u64 = decode(row, "ordinal_position")?;
                Ok(ColumnSummary {
                    name: decode(row, "name")?,
                    data_type: decode(row, "data_type")?,
                    nullable: decode_flag(row, "nullable")?,
                    default_value: decode(row, "default_value")?,
                    ordinal_position: u32::try_from(ordinal_position).unwrap_or(u32::MAX),
                    primary_key: decode_flag(row, "primary_key")?,
                    generated: decode_flag(row, "generated")?,
                    identity: false,
                    comment: decode(row, "comment")?,
                })
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
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        // MySQL returns one row per index column; group the rows before
        // applying the shared page bound.
        let mut indexes: Vec<IndexSummary> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        for row in rows {
            let name: String = decode(&row, "name")?;
            let position = match positions.get(&name) {
                Some(position) => *position,
                None => {
                    let position = indexes.len();
                    positions.insert(name.clone(), position);
                    indexes.push(IndexSummary {
                        name,
                        columns: Vec::new(),
                        unique: decode_flag(&row, "unique_index")?,
                        primary: decode_flag(&row, "primary_index")?,
                        method: decode(&row, "method")?,
                    });
                    position
                }
            };
            let column: Option<String> = decode(&row, "column_name")?;
            if let Some(column) = column {
                indexes[position].columns.push(column);
            }
        }

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
        let mut guard = self.live_connection().await?;
        let connection = guard.as_mut().expect("checked above");
        let rows = timeout_at(
            tokio::time::Instant::now() + self.operation_timeout,
            sqlx::query(sql::LIST_CONSTRAINTS)
                .bind(database.as_deref())
                .bind(database.as_deref())
                .bind(table_name)
                .bind(table_name)
                .fetch_all(&mut *connection),
        )
        .await
        .map_err(|_| self.operation_timeout_error())?
        .map_err(map_sqlx_error)?;

        let mut constraints: Vec<ConstraintSummary> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        for row in rows {
            let name: String = decode(&row, "name")?;
            let kind_text: String = decode(&row, "kind")?;
            let key = format!(
                "{}\u{1f}{}\u{1f}{}",
                decode::<String>(&row, "database_name")?,
                decode::<String>(&row, "table_name")?,
                name
            );
            let position = match positions.get(&key) {
                Some(position) => *position,
                None => {
                    let position = constraints.len();
                    positions.insert(key, position);
                    constraints.push(ConstraintSummary {
                        name,
                        kind: constraint_kind(&kind_text),
                        columns: Vec::new(),
                        referenced_table: decode(&row, "referenced_table")?,
                        referenced_columns: Vec::new(),
                        on_update: decode(&row, "on_update")?,
                        on_delete: decode(&row, "on_delete")?,
                    });
                    position
                }
            };
            if let Some(column) = decode::<Option<String>>(&row, "column_name")? {
                constraints[position].columns.push(column);
            }
            if let Some(column) = decode::<Option<String>>(&row, "referenced_column")? {
                constraints[position].referenced_columns.push(column);
            }
        }

        Ok(Page::from_items(constraints, page, None))
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
        size_bytes: decode::<Option<u64>>(row, "size_bytes")?.into(),
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

fn decode<'r, T>(row: &'r MySqlRow, column: &str) -> Result<T, DatabaseError>
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

/// Maps SQLx errors into the shared taxonomy without exposing the raw driver
/// display text. Authentication failures intentionally omit the server
/// message, which can echo the account and host.
fn map_sqlx_error(error: sqlx::Error) -> DatabaseError {
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
        assert!(columns.items[2].generated);

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
