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

pub mod actions;
pub mod adapter;
pub mod capabilities;
pub mod ddl;
pub mod rows;
pub mod sql;

pub use adapter::PostgresDatabaseAdapter;

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
        assert!(sql::LIST_DATABASES.contains("pg_database"));
        assert!(sql::LIST_SCHEMAS.contains("pg_namespace"));
        assert!(sql::SET_READ_ONLY.contains("READ ONLY"));
        assert!(sql::SERVER_INFO.contains("current_database"));
        assert!(sql::LIST_TABLES.contains("LIMIT $2 OFFSET $3"));
        assert!(sql::LIST_TABLES.contains("pg_total_relation_size"));
        assert!(sql::LIST_VIEWS.contains("MATERIALIZED VIEW"));
        assert!(sql::LIST_VIEWS.contains("LIMIT $2 OFFSET $3"));
        assert!(sql::LIST_SEQUENCES.contains("information_schema.sequences"));
        assert!(sql::OBJECT_COUNTS.contains("sequence_count"));
        assert!(!sql::OBJECT_COUNTS.contains('?'));

        assert!(sql::LIST_TABLES.contains("pg_stat_user_tables"));
        assert!(sql::LIST_TABLES.contains("metadata_refreshed_at"));
        assert!(sql::LIST_COLUMNS.contains("character_maximum_length"));
        assert!(sql::LIST_COLUMNS.contains("attidentity"));
        assert!(sql::LIST_COLUMNS.contains("collation_name"));
        assert!(sql::LIST_INDEXES.contains("indpred"));
        assert!(sql::LIST_CONSTRAINTS.contains("CHECK"));
        assert!(sql::LIST_CONSTRAINTS.contains("condeferrable"));
        assert!(sql::TABLE_INCOMING_FOREIGN_KEYS.contains("confupdtype"));
        assert!(sql::TABLE_DEPENDENT_VIEWS.contains("pg_depend"));
        assert!(sql::TABLE_OWNED_SEQUENCES.contains("deptype IN ('a', 'i')"));
        assert!(sql::TABLE_TRIGGERS.contains("tgisinternal"));
        assert!(sql::ROW_TABLE_ESTIMATE.contains("reltuples"));
        assert!(sql::ROW_TABLE_COLUMNS.contains("format_type"));
        assert!(sql::ROW_UNIQUE_INDEXES.contains("indisvalid"));
        for statement in [
            sql::TABLE_INCOMING_FOREIGN_KEYS,
            sql::TABLE_DEPENDENT_VIEWS,
            sql::TABLE_OWNED_SEQUENCES,
            sql::TABLE_TRIGGERS,
        ] {
            assert!(statement.contains("LIMIT $3"));
        }
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
