//! Shared database adapter layer.
//!
//! The module deliberately keeps engine-specific SQL in `mysql::sql` and
//! `postgres::sql`. Commands use [`DatabaseAdapter`] and the typed metadata
//! types; React receives only serialized metadata, capability snapshots, and
//! stable error kinds.

pub mod capabilities;
pub mod dialect;
pub mod errors;
pub mod fake;
pub mod mysql;
pub mod pagination;
pub mod postgres;
pub mod traits;

pub use capabilities::{
    CapabilityFlags, CapabilitySnapshot, DatabaseAction, DatabaseCapabilities, DatabaseVersion,
};
pub use dialect::{
    Identifier, IdentifierQuoter, MAX_BIND_PARAMETERS, MAX_IDENTIFIER_LENGTH, ParameterizedQuery,
    QualifiedName, SqlDialect, SqlValue, ValueParameterizer,
};
pub use errors::{DatabaseError, DatabaseErrorKind};
pub use fake::FakeDatabaseAdapter;
pub use pagination::{
    Bounded, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, MAX_PAGE_OFFSET, Page, PageRequest,
};
pub use traits::{
    ColumnSummary, ConstraintKind, ConstraintSummary, DatabaseAdapter, DatabaseConnectRequest,
    DatabaseConnectionInfo, DatabasePassword, DatabasePing, DatabaseSummary, DatabaseVersionInfo,
    IndexSummary, MetadataScope, SchemaSummary, TableKind, TableRef, TableSummary,
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
