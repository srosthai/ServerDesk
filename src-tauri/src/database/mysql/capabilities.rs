//! MySQL action availability by server version.

use crate::database::capabilities::{CapabilityFlags, DatabaseVersion};

fn at_least(version: Option<DatabaseVersion>, major: u16, minor: u16, patch: u16) -> bool {
    version.is_some_and(|version| version.at_least(major, minor, patch))
}

/// MySQL baseline and version-gated capabilities.
///
/// Unknown versions keep the baseline features that every supported MySQL
/// release provides and disable version-dependent features.
pub fn flags_for(version: Option<DatabaseVersion>) -> CapabilityFlags {
    CapabilityFlags {
        // MySQL treats a schema as a database, so schema browsing and creation
        // map to the same catalog objects.
        schemas: true,
        sequences: false,
        materialized_views: false,
        routines: true,
        triggers: true,
        // Server roles were added in MySQL 8.0.
        roles: at_least(version, 8, 0, 0),
        sessions: true,
        transactions: true,
        savepoints: true,
        cancel_query: true,
        explain: true,
        // CHECK constraints are enforced from MySQL 8.0.16.
        check_constraints: at_least(version, 8, 0, 16),
        // GENERATED columns were added in MySQL 5.7.6.
        generated_columns: at_least(version, 5, 7, 6),
        // MySQL identity is expressed as AUTO_INCREMENT.
        identity_columns: false,
        auto_increment_columns: true,
        // ALTER TABLE ... MODIFY COLUMN always supports a type change.
        column_type_change: true,
        // ALTER TABLE ... RENAME COLUMN was added in MySQL 8.0. Older servers
        // need CHANGE COLUMN with the full definition, which the structure
        // planner does not guess from metadata.
        column_rename: at_least(version, 8, 0, 0),
        table_rename: true,
        // CREATE TABLE ... LIKE copies the table definition on every
        // supported MySQL and MariaDB release.
        table_duplicate: true,
        // MySQL supports FIRST/AFTER column positioning.
        column_reorder: true,
        // There is no safe, atomic RENAME DATABASE operation.
        database_rename: false,
        // ALTER TABLE ... RENAME INDEX was added in MySQL 5.7.
        index_rename: at_least(version, 5, 7, 0),
        // MySQL has no partial-index predicate.
        partial_indexes: false,
        // max_execution_time was added in MySQL 5.7.4. The adapter still
        // applies a client-side bound for older servers.
        query_timeout: at_least(version, 5, 7, 4),
    }
}
