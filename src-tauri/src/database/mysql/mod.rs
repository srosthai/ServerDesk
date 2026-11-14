//! MySQL dialect and engine-specific SQL.
//!
//! Nothing in this module is returned by a Tauri command. Adapters consume the
//! dialect and SQL constants internally; commands and React only receive typed
//! metadata and capability snapshots.

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
pub mod queries;
pub mod rows;
pub mod sql;

pub use adapter::MysqlDatabaseAdapter;

/// MySQL identifier quoting and positional parameter syntax.
#[derive(Debug, Clone, Copy, Default)]
pub struct MysqlDialect;

/// Shared static dialect used by adapters and capability documents.
pub static MYSQL_DIALECT: MysqlDialect = MysqlDialect;

impl IdentifierQuoter for MysqlDialect {
    fn quote_identifier(&self, identifier: &Identifier) -> String {
        format!("`{}`", identifier.as_str().replace('`', "``"))
    }
}

impl ValueParameterizer for MysqlDialect {
    fn placeholder(&self, position: usize) -> Result<String, DatabaseError> {
        validate_parameter_position(position)?;
        Ok("?".to_string())
    }
}

impl SqlDialect for MysqlDialect {
    fn engine(&self) -> DatabaseEngine {
        DatabaseEngine::Mysql
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::capabilities::DatabaseVersion;

    #[test]
    fn sql_is_kept_in_the_mysql_module() {
        assert!(sql::PING.contains("SELECT"));
        assert!(sql::LIST_TABLES.contains("information_schema"));
        assert!(sql::LIST_TABLES.contains('?'));
        assert!(!sql::LIST_TABLES.contains("$1"));
        assert!(!sql::LIST_TABLES.contains("postgres"));
        assert!(sql::LIST_TABLES.contains("TABLE_TYPE = 'BASE TABLE'"));
        assert!(sql::LIST_TABLES.contains("LIMIT ? OFFSET ?"));
        assert!(sql::LIST_VIEWS.contains("information_schema.VIEWS"));
        assert!(sql::LIST_VIEWS.contains("LIMIT ? OFFSET ?"));
        assert!(sql::OBJECT_COUNTS.contains("table_count"));
        assert!(sql::OBJECT_COUNTS.contains("view_count"));
        assert!(!sql::OBJECT_COUNTS.contains("LIMIT"));

        assert!(sql::LIST_COLUMNS.contains("CHARACTER_MAXIMUM_LENGTH"));
        assert!(sql::LIST_COLUMNS.contains("GENERATION_EXPRESSION"));
        assert!(sql::LIST_COLUMNS.contains("COLLATION_NAME"));
        assert!(sql::LIST_COLUMNS.contains("LIMIT ? OFFSET ?"));

        // Index and constraint pages are selected before their detail rows,
        // so grouping never reads an unbounded number of index columns.
        assert!(sql::LIST_INDEXES.contains("JOIN ("));
        assert!(sql::LIST_INDEXES.contains("LIMIT ? OFFSET ?"));
        assert!(sql::LIST_CONSTRAINTS.contains("CHECK_CLAUSE"));
        assert!(sql::LIST_CONSTRAINTS.contains("JOIN ("));
        assert!(sql::LIST_CONSTRAINTS.contains("LIMIT ? OFFSET ?"));
        assert!(sql::LIST_CONSTRAINTS_WITHOUT_CHECKS.contains("NULL AS check_expression"));

        assert!(sql::TABLE_INCOMING_FOREIGN_KEYS.contains("REFERENCED_TABLE_NAME"));
        assert!(sql::TABLE_TRIGGERS.contains("information_schema.TRIGGERS"));
        assert!(sql::ROW_TABLE_ESTIMATE.contains("TABLE_ROWS"));
        assert!(sql::ROW_TABLE_COLUMNS.contains("COLUMN_TYPE"));
        assert!(sql::ROW_TABLE_COLUMNS.contains("LIMIT ?"));
        assert!(sql::ROW_UNIQUE_INDEXES.contains("SUB_PART"));
    }

    #[test]
    fn dialect_reports_mysql() {
        assert_eq!(MysqlDialect.engine(), DatabaseEngine::Mysql);
    }

    #[test]
    fn capability_flags_are_versioned() {
        let flags = capabilities::flags_for(Some(DatabaseVersion::new(8, 0, 16)));
        assert!(flags.roles);
        assert!(flags.check_constraints);
        assert!(flags.generated_columns);
        assert!(!flags.identity_columns);

        let old = capabilities::flags_for(Some(DatabaseVersion::new(5, 7, 44)));
        assert!(!old.roles);
        assert!(!old.check_constraints);
        assert!(old.generated_columns);
    }
}
