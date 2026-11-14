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
pub mod queries;
pub mod row_service;
pub mod row_writes;
pub mod rows;
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
pub use fake::FakeRowTable;
pub use mysql::MysqlDatabaseAdapter;
pub use pagination::{
    Bounded, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, MAX_PAGE_OFFSET, Page, PageRequest,
};
pub use postgres::PostgresDatabaseAdapter;
pub use queries::{
    DEFAULT_QUERY_RESULT_LIMIT, DEFAULT_QUERY_TIMEOUT, DatabaseQueryService,
    MAX_QUERY_CELL_CHARACTERS, MAX_QUERY_COLUMNS, MAX_QUERY_REQUEST_ID_LENGTH,
    MAX_QUERY_RESULT_CHARACTERS, MAX_QUERY_RESULT_LIMIT, MAX_QUERY_RESULT_SETS,
    MAX_QUERY_SQL_CHARACTERS, MAX_QUERY_TIMEOUT, QueryCancellation, QueryCell, QueryColumn,
    QueryControl, QueryExecutionResult, QueryGuard, QueryRegistry, QueryRequest, QueryResultSet,
    SqlAnalysis, SqlWarning, SqlWarningCode, analyze_sql, execute_query_with_adapter,
    query_timeout, validate_query_request_id,
};
pub use row_service::DatabaseRowService;
pub use row_writes::{
    MAX_ROW_WRITE_TARGETS, MAX_ROW_WRITE_TEXT_CHARACTERS, MAX_ROW_WRITE_VALUES, RowDeleteRequest,
    RowInsertRequest, RowTransactionState, RowUpdateRequest, RowValueMode, RowWriteOperation,
    RowWriteResult, RowWriteValue,
};
pub use rows::{
    DEFAULT_ROW_PAGE_LIMIT, DEFAULT_ROW_QUERY_TIMEOUT, MAX_ROW_CELL_CHARACTERS, MAX_ROW_COLUMNS,
    MAX_ROW_FILTER_VALUE_CHARACTERS, MAX_ROW_FILTER_VALUES, MAX_ROW_FILTERS, MAX_ROW_OFFSET,
    MAX_ROW_PAGE_LIMIT, MAX_ROW_QUERY_TIMEOUT, MAX_ROW_SEARCH_CHARACTERS, MAX_ROW_SEARCH_COLUMNS,
    MAX_ROW_SORTS, MAX_ROW_VISIBLE_COLUMNS, RowCell, RowColumn, RowColumnKind, RowCounts,
    RowExactCount, RowFilter, RowFilterOperator, RowFilterValue, RowIdentity, RowIdentityKind,
    RowIdentityValue, RowPageInfo, RowPageRequest, RowQueryCancellation, RowQueryControl,
    RowQueryGuard, RowQueryRegistry, RowReadOnlyReason, RowReadRequest, RowReadResult, RowRecord,
    RowSearch, RowSort, RowSortDirection, RowTableMetadata, classify_column_kind,
    row_query_timeout, validate_row_query_id,
};
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
