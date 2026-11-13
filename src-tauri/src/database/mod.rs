//! Shared database adapter layer.
//!
//! The module deliberately keeps engine-specific SQL in `mysql::sql` and
//! `postgres::sql`. Commands use [`DatabaseAdapter`] and the typed metadata
//! types; React receives only serialized metadata, capability snapshots, and
//! stable error kinds.

pub mod actions;
pub mod capabilities;
pub mod dialect;
pub mod discovery;
pub mod errors;
pub mod fake;
pub mod mysql;
pub mod pagination;
pub mod postgres;
pub mod service;
pub mod structure;
pub mod traits;
pub mod validation;

pub use actions::{
    DatabaseActionKind, DatabaseActionRequest, DatabaseActionResult, DatabaseActionService,
    DatabaseDdlPreview, MAX_DATABASE_OPTION_LENGTH, ValidatedCreateDatabase,
    ValidatedDatabaseAction, execute_with_adapter, preview_with_adapter,
};
pub use capabilities::{
    CapabilityFlags, CapabilitySnapshot, DatabaseAction, DatabaseCapabilities, DatabaseVersion,
};
pub use dialect::{
    Identifier, IdentifierQuoter, MAX_BIND_PARAMETERS, MAX_IDENTIFIER_LENGTH, ParameterizedQuery,
    QualifiedName, SqlDialect, SqlValue, ValueParameterizer,
};
pub use discovery::{
    DatabaseDiscoveryService, ExplorerNode, ExplorerNodeKind, ExplorerNodePayload,
};
pub use errors::{DatabaseError, DatabaseErrorKind};
pub use fake::FakeDatabaseAdapter;
pub use mysql::MysqlDatabaseAdapter;
pub use pagination::{
    Bounded, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, MAX_PAGE_OFFSET, Page, PageRequest,
};
pub use postgres::PostgresDatabaseAdapter;
pub use service::{DEFAULT_DATABASE_CONNECT_TIMEOUT, DatabaseConnectionRegistry};
pub use structure::{
    AffectedObject, ColumnDefault, ColumnDefaultRequest, ColumnDefinitionRequest, ColumnPosition,
    ColumnPositionRequest, ConstraintRequest, DatabaseStructureService, GeneratedColumnRequest,
    MAX_COLUMN_COMMENT_LENGTH, MAX_COLUMN_EXPRESSION_LENGTH, MAX_COLUMN_LIST_LENGTH,
    MAX_DATA_TYPE_LENGTH, MAX_DEFAULT_STRING_LENGTH, MAX_INDEX_METHOD_LENGTH, ReferentialAction,
    StructureActionKind, StructureActionRequest, StructureActionResult, StructureDdlPreview,
    StructureObjectKind, TableOptionsRequest, UniqueKeyRequest, ValidatedColumnChanges,
    ValidatedColumnDefinition, ValidatedConstraint, ValidatedCreateIndex, ValidatedCreateTable,
    ValidatedGeneratedColumn, ValidatedStructureAction, ValidatedTableOptions, ValidatedUniqueKey,
    execute_structure_with_adapter, preview_structure_with_adapter,
};
pub use traits::{
    ColumnIdentity, ColumnSummary, ConstraintKind, ConstraintSummary, DatabaseAdapter,
    DatabaseConnectRequest, DatabaseConnectionInfo, DatabasePassword, DatabasePing,
    DatabaseSummary, DatabaseVersionInfo, DependencyKind, DependencySummary, IndexSummary,
    MetadataScope, MetadataValue, ObjectCounts, ObjectMetadata, SchemaSummary, SequenceSummary,
    TableDependencies, TableKind, TableRef, TableSummary, ViewKind, ViewSummary,
};

use crate::storage::DatabaseEngine;

/// Returns the dialect for an engine without exposing engine SQL.
pub fn dialect_for(engine: DatabaseEngine) -> &'static dyn SqlDialect {
    match engine {
        DatabaseEngine::Mysql => &mysql::MYSQL_DIALECT,
        DatabaseEngine::Postgresql => &postgres::POSTGRES_DIALECT,
    }
}

/// Builds the capability document for an engine and raw server version.
pub fn capabilities_for(engine: DatabaseEngine, version: Option<&str>) -> DatabaseCapabilities {
    DatabaseCapabilities::for_engine(engine, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_factory_matches_the_engine() {
        assert_eq!(
            dialect_for(DatabaseEngine::Mysql).engine(),
            DatabaseEngine::Mysql
        );
        assert_eq!(
            dialect_for(DatabaseEngine::Postgresql).engine(),
            DatabaseEngine::Postgresql
        );
    }
}
