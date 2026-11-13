//! PostgreSQL action availability by server version.

use crate::database::capabilities::{CapabilityFlags, DatabaseVersion};

fn at_least(version: Option<DatabaseVersion>, major: u16, minor: u16, patch: u16) -> bool {
    version.is_some_and(|version| version.at_least(major, minor, patch))
}

/// PostgreSQL baseline and version-gated capabilities.
///
/// Unknown versions keep baseline features and disable version-dependent
/// features. Version checks are conservative rather than assuming the newest
/// server.
pub fn flags_for(version: Option<DatabaseVersion>) -> CapabilityFlags {
    CapabilityFlags {
        schemas: true,
        sequences: true,
        // Materialized views were added in PostgreSQL 9.3.
        materialized_views: at_least(version, 9, 3, 0),
        routines: true,
        triggers: true,
        roles: true,
        // pg_stat_activity has been available since PostgreSQL 8.1, but keep
        // unknown-version detection conservative for the session tools.
        sessions: at_least(version, 9, 2, 0),
        transactions: true,
        savepoints: true,
        cancel_query: true,
        explain: true,
        check_constraints: true,
        // Generated columns were added in PostgreSQL 12.
        generated_columns: at_least(version, 12, 0, 0),
        // Identity columns were added in PostgreSQL 10.
        identity_columns: at_least(version, 10, 0, 0),
        // PostgreSQL does not use AUTO_INCREMENT.
        auto_increment_columns: false,
        column_type_change: true,
        // ALTER TABLE ... RENAME COLUMN has been available since before 9.2.
        column_rename: true,
        table_rename: true,
        // CREATE TABLE ... (LIKE ... INCLUDING ALL) is available on every
        // supported PostgreSQL release.
        table_duplicate: true,
        // PostgreSQL does not support reordering stored columns.
        column_reorder: false,
        database_rename: true,
        // ALTER INDEX ... RENAME was added in PostgreSQL 9.2.
        index_rename: at_least(version, 9, 2, 0),
        // Partial-index predicates are supported at every supported version.
        partial_indexes: true,
        query_timeout: true,
    }
}
