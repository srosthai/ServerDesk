//! Shared database adapter contract.
//!
//! Commands depend on this trait and on typed metadata only. Engine-specific
//! SQL and driver errors stay inside the adapter implementation.

use std::{fmt, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::storage::DatabaseEngine;

use super::{
    actions::{DatabaseDdlPreview, ValidatedDatabaseAction},
    capabilities::{CapabilitySnapshot, DatabaseAction, DatabaseCapabilities, DatabaseVersion},
    dialect::{Identifier, SqlDialect},
    errors::DatabaseError,
    pagination::{Bounded, Page, PageRequest},
    row_writes::{RowDeleteRequest, RowInsertRequest, RowUpdateRequest, RowWriteResult},
    rows::{RowQueryControl, RowReadRequest, RowReadResult},
    structure::{StructureDdlPreview, ValidatedStructureAction},
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

/// An optional metadata field reported by an engine.
///
/// `Unavailable` is deliberate: the engine has no concept for the field, the
/// account cannot read it, or the server version does not expose it. Adapters
/// must never substitute a guessed value. React can render unsupported fields
/// as unavailable without guessing from a missing/null field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "camelCase")]
pub enum MetadataValue<T> {
    Available(T),
    #[default]
    Unavailable,
}

impl<T> MetadataValue<T> {
    pub const fn available(value: T) -> Self {
        Self::Available(value)
    }

    pub const fn unavailable() -> Self {
        Self::Unavailable
    }

    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    pub const fn value(&self) -> Option<&T> {
        match self {
            Self::Available(value) => Some(value),
            Self::Unavailable => None,
        }
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> MetadataValue<U> {
        match self {
            Self::Available(value) => MetadataValue::Available(map(value)),
            Self::Unavailable => MetadataValue::Unavailable,
        }
    }
}

impl<T> From<Option<T>> for MetadataValue<T> {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Unavailable, Self::Available)
    }
}

/// Common database-object metadata. Every field is explicit about whether the
/// selected engine can report it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectMetadata {
    pub owner: MetadataValue<String>,
    pub encoding: MetadataValue<String>,
    pub collation: MetadataValue<String>,
    pub size_bytes: MetadataValue<u64>,
    pub engine: MetadataValue<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSummary {
    pub name: String,
    pub system: bool,
    #[serde(flatten)]
    pub metadata: ObjectMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSummary {
    pub name: String,
    pub system: bool,
    #[serde(flatten)]
    pub metadata: ObjectMetadata,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    View,
    MaterializedView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSummary {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: TableKind,
    pub estimated_rows: MetadataValue<u64>,
    /// Server-reported time when table statistics were last refreshed.
    /// `Unavailable` means the engine, storage engine, or account does not
    /// expose it; adapters never substitute the current time.
    pub metadata_refreshed_at: MetadataValue<String>,
    #[serde(flatten)]
    pub metadata: ObjectMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSummary {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: ViewKind,
    #[serde(flatten)]
    pub metadata: ObjectMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceSummary {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub data_type: MetadataValue<String>,
    pub start_value: MetadataValue<String>,
    pub increment: MetadataValue<String>,
    #[serde(flatten)]
    pub metadata: ObjectMetadata,
}

/// Counts for the object groups shown under one explorer node. Unsupported
/// groups are `Unavailable`, not zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectCounts {
    pub tables: MetadataValue<u64>,
    pub views: MetadataValue<u64>,
    pub materialized_views: MetadataValue<u64>,
    pub sequences: MetadataValue<u64>,
}

/// How a column produces its value when it is not supplied by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnIdentity {
    /// The column has no identity or auto-increment behavior.
    None,
    /// MySQL `AUTO_INCREMENT` or an equivalent engine-specific counter.
    AutoIncrement,
    /// SQL-standard `GENERATED ALWAYS AS IDENTITY`.
    Always,
    /// SQL-standard `GENERATED BY DEFAULT AS IDENTITY`.
    ByDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSummary {
    pub name: String,
    pub data_type: String,
    /// Maximum character length for character types. `Available(None)` means
    /// the type has no fixed length.
    pub length: MetadataValue<Option<u64>>,
    /// Numeric precision or fractional-seconds precision for temporal types.
    pub precision: MetadataValue<Option<u32>>,
    pub scale: MetadataValue<Option<u32>>,
    pub nullable: bool,
    /// `Available(None)` means the column has no default. `Unavailable` means
    /// the engine could not report the default at all.
    pub default_value: MetadataValue<Option<String>>,
    pub ordinal_position: u32,
    pub primary_key: bool,
    pub identity: MetadataValue<ColumnIdentity>,
    pub generated: MetadataValue<bool>,
    /// Expression for a generated column. `Available(None)` means the column
    /// is not generated; `Unavailable` means the engine cannot report it.
    pub generation_expression: MetadataValue<Option<String>>,
    pub comment: MetadataValue<Option<String>>,
    /// `Available(None)` means the column type is not collatable.
    pub collation: MetadataValue<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    pub method: MetadataValue<String>,
    /// Partial-index predicate where the engine supports one.
    pub predicate: MetadataValue<Option<String>>,
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
    pub referenced_schema: MetadataValue<Option<String>>,
    pub referenced_table: MetadataValue<Option<String>>,
    pub referenced_columns: Vec<String>,
    pub on_update: MetadataValue<Option<String>>,
    pub on_delete: MetadataValue<Option<String>>,
    /// Check constraint expression. `Available(None)` means the constraint is
    /// not a check constraint; `Unavailable` means the engine cannot report
    /// check expressions.
    pub check_expression: MetadataValue<Option<String>>,
    pub deferrable: MetadataValue<bool>,
    pub initially_deferred: MetadataValue<bool>,
}

/// A dependency that must be considered before dropping or rewriting a table
/// or one of its columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    IncomingForeignKey,
    DependentView,
    DependentMaterializedView,
    OwnedSequence,
    Trigger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencySummary {
    pub kind: DependencyKind,
    /// Constraint, view, sequence, or trigger name, depending on `kind`.
    pub name: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    /// Referencing table for a foreign key or trigger definition.
    pub table: Option<String>,
    pub columns: Vec<String>,
    pub referenced_columns: Vec<String>,
    pub on_update: MetadataValue<Option<String>>,
    pub on_delete: MetadataValue<Option<String>>,
    pub details: MetadataValue<String>,
}

/// Bounded dependency inventory for one table.
///
/// Each group is independently bounded so a table with thousands of incoming
/// foreign keys cannot make one metadata request unbounded. `Unavailable`
/// means the engine does not expose that dependency category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDependencies {
    pub incoming_foreign_keys: MetadataValue<Bounded<DependencySummary>>,
    pub dependent_views: MetadataValue<Bounded<DependencySummary>>,
    pub owned_sequences: MetadataValue<Bounded<DependencySummary>>,
    pub triggers: MetadataValue<Bounded<DependencySummary>>,
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

    /// True when the current session was opened in read-only mode. Structure
    /// actions are rejected before SQL runs; metadata browsing remains
    /// available.
    fn is_read_only(&self) -> bool;

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

    /// Re-reads one database summary after a lifecycle action. `None` means
    /// the engine does not list the database.
    async fn database_summary(
        &self,
        database: &Identifier,
    ) -> Result<Option<DatabaseSummary>, DatabaseError>;

    /// Generates engine-specific DDL and safe warnings without executing it.
    async fn preview_database_action(
        &self,
        action: &ValidatedDatabaseAction,
    ) -> Result<DatabaseDdlPreview, DatabaseError>;

    /// Generates engine-specific DDL for a table, column, index, or constraint
    /// change. This is a planning method only; it must never execute SQL.
    async fn preview_structure_action(
        &self,
        action: &ValidatedStructureAction,
    ) -> Result<StructureDdlPreview, DatabaseError>;

    /// Executes one validated lifecycle action. Only set-active returns updated
    /// connection metadata; create, rename, and drop return `None`.
    async fn execute_database_action(
        &mut self,
        action: &ValidatedDatabaseAction,
    ) -> Result<Option<DatabaseConnectionInfo>, DatabaseError>;

    /// Executes one validated structure action after re-running the same
    /// preview/capability path. The caller verifies metadata afterwards.
    async fn execute_structure_action(
        &mut self,
        action: &ValidatedStructureAction,
    ) -> Result<(), DatabaseError>;

    async fn list_schemas(&self, page: PageRequest) -> Result<Page<SchemaSummary>, DatabaseError>;

    async fn list_tables(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<TableSummary>, DatabaseError>;

    /// Re-reads one table summary after a structure action. `None` means the
    /// table is not present in the selected scope.
    async fn table_summary(&self, table: &TableRef) -> Result<Option<TableSummary>, DatabaseError>;

    async fn list_views(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<ViewSummary>, DatabaseError>;

    async fn list_sequences(
        &self,
        scope: &MetadataScope,
        page: PageRequest,
    ) -> Result<Page<SequenceSummary>, DatabaseError>;

    /// Returns bounded object counts for one explorer node in a single query.
    async fn object_counts(&self, scope: &MetadataScope) -> Result<ObjectCounts, DatabaseError>;

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

    /// Reads one hard-bounded page of table rows. Filters and search values are
    /// parameterized, sort columns are resolved against live metadata, and the
    /// result carries column metadata, stable identity, and exact/estimated
    /// counts. The control bounds the whole request with a timeout and an
    /// optional cancellation handle.
    async fn read_rows(
        &self,
        request: &RowReadRequest,
        control: &RowQueryControl,
    ) -> Result<RowReadResult, DatabaseError>;

    /// Inserts one row in a transaction, verifies the affected row, re-reads it
    /// where the identity is known, and commits only after verification. A
    /// failure rolls back and reports that no rows were changed.
    async fn insert_row(
        &mut self,
        request: &RowInsertRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError>;

    /// Updates one row addressed by a stable identity. `expected` values guard
    /// every changed column; a concurrent change affects zero rows and rolls the
    /// transaction back instead of overwriting newer data.
    async fn update_row(
        &mut self,
        request: &RowUpdateRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError>;

    /// Deletes one or more explicit row identities in one transaction. The
    /// affected count must match the target count or the transaction rolls back.
    async fn delete_rows(
        &mut self,
        request: &RowDeleteRequest,
        control: &RowQueryControl,
    ) -> Result<RowWriteResult, DatabaseError>;

    /// Returns a bounded dependency inventory for one table. Callers use it
    /// before destructive structure changes so the UI can name the foreign
    /// keys, views, sequences, and triggers that depend on the object.
    async fn table_dependencies(
        &self,
        table: &TableRef,
        limit: u32,
    ) -> Result<TableDependencies, DatabaseError>;

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
    fn metadata_values_distinguish_available_from_unsupported() {
        let available = MetadataValue::available("utf8mb4".to_string());
        let unavailable = MetadataValue::<String>::unavailable();

        assert!(available.is_available());
        assert_eq!(available.value().map(String::as_str), Some("utf8mb4"));
        assert!(!unavailable.is_available());
        assert_eq!(unavailable.value(), None);

        let json = serde_json::to_string(&available).expect("available json");
        assert_eq!(json, r#"{"status":"available","value":"utf8mb4"}"#);
        let json = serde_json::to_string(&unavailable).expect("unavailable json");
        assert_eq!(json, r#"{"status":"unavailable"}"#);

        assert_eq!(
            MetadataValue::from(Some(4_u64)),
            MetadataValue::Available(4)
        );
        assert_eq!(MetadataValue::<u64>::from(None), MetadataValue::Unavailable);
        assert_eq!(MetadataValue::<u64>::default(), MetadataValue::Unavailable);
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
