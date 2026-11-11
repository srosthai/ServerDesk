//! Shared database adapter contract.
//!
//! Commands depend on this trait and on typed metadata only. Engine-specific
//! SQL and driver errors stay inside the adapter implementation.

use std::{fmt, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::storage::DatabaseEngine;

use super::{
    capabilities::{CapabilitySnapshot, DatabaseAction, DatabaseCapabilities, DatabaseVersion},
    dialect::{Identifier, SqlDialect},
    errors::DatabaseError,
    pagination::{Page, PageRequest},
};

/// A database password that never appears in debug output or serialization.
#[derive(Clone, PartialEq, Eq)]
pub struct DatabasePassword(String);

impl DatabasePassword {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DatabasePassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DatabasePassword")
            .field(&"<redacted>")
            .finish()
    }
}

/// Connection input for a future MySQL or PostgreSQL driver.
#[derive(Debug, Clone)]
pub struct DatabaseConnectRequest {
    pub host: String,
    pub port: u16,
    pub socket: Option<String>,
    pub database: Option<String>,
    pub username: String,
    pub password: DatabasePassword,
    pub read_only: bool,
    pub ssl_mode: Option<String>,
    pub connect_timeout: Duration,
}

/// Version returned by the server after a successful connect or probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseVersionInfo {
    pub version: DatabaseVersion,
    pub raw: String,
}

/// Successful connection details. It contains no SQL, credentials, or driver
/// internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConnectionInfo {
    pub engine: DatabaseEngine,
    pub version: DatabaseVersionInfo,
    pub database: Option<String>,
    pub read_only: bool,
    pub capabilities: CapabilitySnapshot,
}

/// Latency sample from the adapter's lightweight ping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabasePing {
    pub latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSummary {
    pub name: String,
    pub owner: Option<String>,
    pub encoding: Option<String>,
    pub collation: Option<String>,
    pub size_bytes: Option<u64>,
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSummary {
    pub name: String,
    pub owner: Option<String>,
    pub system: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    Table,
    PartitionedTable,
    View,
    ForeignTable,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSummary {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: TableKind,
    pub estimated_rows: Option<u64>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSummary {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub ordinal_position: u32,
    pub primary_key: bool,
    pub generated: bool,
    pub identity: bool,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    pub method: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    PrimaryKey,
    Unique,
    ForeignKey,
    Check,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintSummary {
    pub name: String,
    pub kind: ConstraintKind,
    pub columns: Vec<String>,
    pub referenced_table: Option<String>,
    pub referenced_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

/// Optional database/schema filter for metadata browsing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataScope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Identifier>,
}

impl MetadataScope {
    pub fn new(database: Option<&str>, schema: Option<&str>) -> Result<Self, DatabaseError> {
        Ok(Self {
            database: database.map(Identifier::new).transpose()?,
            schema: schema.map(Identifier::new).transpose()?,
        })
    }
}

/// A validated table reference used by metadata operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Identifier>,
    pub name: Identifier,
}

impl TableRef {
    pub fn new(
        database: Option<&str>,
        schema: Option<&str>,
        name: &str,
    ) -> Result<Self, DatabaseError> {
        Ok(Self {
            database: database.map(Identifier::new).transpose()?,
            schema: schema.map(Identifier::new).transpose()?,
            name: Identifier::new(name)?,
        })
    }
}

/// The one interface MySQL and PostgreSQL share.
///
/// A command obtains an adapter from the future connection registry and calls
/// these methods. It cannot call engine SQL directly. Capability checks happen
/// both before the command is enabled in the UI and again before an adapter
/// executes an action.
#[async_trait]
pub trait DatabaseAdapter: Send + Sync {
    fn engine(&self) -> DatabaseEngine;

    fn capabilities(&self) -> &DatabaseCapabilities;

    fn dialect(&self) -> &dyn SqlDialect;

    fn is_connected(&self) -> bool;

    async fn connect(
        &mut self,
        request: DatabaseConnectRequest,
    ) -> Result<DatabaseConnectionInfo, DatabaseError>;

    async fn ping(&self) -> Result<DatabasePing, DatabaseError>;

    async fn version(&self) -> Result<DatabaseVersionInfo, DatabaseError>;

    async fn close(&mut self) -> Result<(), DatabaseError>;

    async fn list_databases(
        &self,
        page: PageRequest,
    ) -> Result<Page<DatabaseSummary>, DatabaseError>;

    async fn list_schemas(&self, page: PageRequest) -> Result<Page<SchemaSummary>, DatabaseError>;

    async fn list_tables(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<TableSummary>, DatabaseError>;

    async fn list_columns(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ColumnSummary>, DatabaseError>;

    async fn list_indexes(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<IndexSummary>, DatabaseError>;

    async fn list_constraints(
        &self,
        table: &TableRef,
        page: PageRequest,
    ) -> Result<Page<ConstraintSummary>, DatabaseError>;

    /// Shared capability gate for adapter operations.
    fn require_capability(&self, action: DatabaseAction) -> Result<(), DatabaseError> {
        self.capabilities().ensure(action)
    }

    /// Shared connection gate for ping and metadata operations.
    fn require_connection(&self) -> Result<(), DatabaseError> {
        if self.is_connected() {
            Ok(())
        } else {
            Err(DatabaseError::connection(
                "No live database connection is available.",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_validated_before_metadata_refs_are_created() {
        assert!(TableRef::new(Some("app"), Some("public"), "users").is_ok());
        assert!(TableRef::new(None, None, "users; DROP TABLE users").is_ok());
        assert!(TableRef::new(None, None, "").is_err());
        assert!(TableRef::new(None, None, "app.users").is_err());
        assert!(MetadataScope::new(Some("app"), Some("bad\nschema")).is_err());
    }

    #[test]
    fn password_debug_output_is_redacted() {
        let request = DatabaseConnectRequest {
            host: "127.0.0.1".to_string(),
            port: 3306,
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            password: DatabasePassword::new("hunter2"),
            read_only: true,
            ssl_mode: None,
            connect_timeout: Duration::from_secs(5),
        };

        let debug = format!("{request:?}");
        assert!(!debug.contains("hunter2"));
        assert!(debug.contains("<redacted>"));
    }
}
