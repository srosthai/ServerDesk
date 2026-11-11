//! PostgreSQL dialect and engine-specific SQL.
//!
//! The SQL constants stay inside the adapter layer. Commands and React receive
//! only typed metadata, capability snapshots, and stable error kinds.

use crate::storage::DatabaseEngine;

use super::{
    dialect::{
        Identifier, IdentifierQuoter, SqlDialect, ValueParameterizer, validate_parameter_position,
    },
    errors::DatabaseError,
};

pub mod capabilities;
pub mod sql;

/// PostgreSQL identifier quoting and positional parameter syntax.
#[derive(Debug, Clone, Copy, Default)]
pub struct PostgresDialect;

/// Shared static dialect used by adapters and capability documents.
pub static POSTGRES_DIALECT: PostgresDialect = PostgresDialect;

impl IdentifierQuoter for PostgresDialect {
    fn quote_identifier(&self, identifier: &Identifier) -> String {
        format!("\"{}\"", identifier.as_str().replace('"', "\"\""))
    }
}

impl ValueParameterizer for PostgresDialect {
    fn placeholder(&self, position: usize) -> Result<String, DatabaseError> {
        validate_parameter_position(position)?;
        Ok(format!("${position}"))
    }
}

impl SqlDialect for PostgresDialect {
    fn engine(&self) -> DatabaseEngine {
        DatabaseEngine::Postgresql
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::capabilities::DatabaseVersion;

    #[test]
    fn sql_is_kept_in_the_postgres_module() {
        assert!(sql::PING.contains("SELECT"));
        assert!(sql::LIST_TABLES.contains("information_schema"));
        assert!(sql::LIST_TABLES.contains("$1"));
        assert!(!sql::LIST_TABLES.contains('?'));
        assert!(!sql::LIST_TABLES.contains("mysql"));
    }

    #[test]
    fn dialect_reports_postgresql() {
        assert_eq!(PostgresDialect.engine(), DatabaseEngine::Postgresql);
    }

    #[test]
    fn capability_flags_are_versioned() {
        let flags = capabilities::flags_for(Some(DatabaseVersion::new(12, 4, 0)));
        assert!(flags.materialized_views);
        assert!(flags.identity_columns);
        assert!(flags.generated_columns);
        assert!(flags.database_rename);

        let old = capabilities::flags_for(Some(DatabaseVersion::new(9, 2, 0)));
        assert!(old.sequences);
        assert!(!old.materialized_views);
        assert!(!old.identity_columns);
        assert!(!old.generated_columns);
    }
}
