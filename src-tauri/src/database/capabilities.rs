//! Engine and version capability flags.
//!
//! The UI asks which actions are available and disables everything the
//! selected engine/version cannot do. A capability check is repeated before an
//! adapter executes an operation, so bypassing the UI cannot run an
//! unsupported action.

use std::{fmt, str::FromStr};

use serde::{Serialize, Serializer};

use crate::storage::DatabaseEngine;

use super::{errors::DatabaseError, mysql, postgres};

/// A parsed server version. Only the numeric `major.minor.patch` components are
/// used for capability decisions; vendor suffixes and build metadata are kept
/// separately by the adapter that reported the raw version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DatabaseVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl DatabaseVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parses the first numeric version in a server string.
    ///
    /// Examples: `8.0.36-log`, `PostgreSQL 16.2 (Ubuntu 16.2-1)`, and
    /// `5.7.44` all produce usable versions. A missing or malformed version
    /// returns `None`, which capability logic treats conservatively.
    pub fn parse(raw: &str) -> Option<Self> {
        let start = raw
            .char_indices()
            .find(|(_, value)| value.is_ascii_digit())?
            .0;
        let rest = &raw[start..];
        let mut components = Vec::with_capacity(3);
        let mut current = String::new();

        for value in rest.chars() {
            if value.is_ascii_digit() {
                current.push(value);
            } else if value == '.' && !current.is_empty() && components.len() < 2 {
                components.push(current.parse::<u16>().ok()?);
                current.clear();
            } else {
                break;
            }
        }

        if !current.is_empty() {
            components.push(current.parse::<u16>().ok()?);
        }
        if components.is_empty() {
            return None;
        }

        Some(Self::new(
            *components.first().unwrap_or(&0),
            *components.get(1).unwrap_or(&0),
            *components.get(2).unwrap_or(&0),
        ))
    }

    /// Returns true when `self` is at least `major.minor.patch`.
    pub const fn at_least(self, major: u16, minor: u16, patch: u16) -> bool {
        let required = Self::new(major, minor, patch);
        self.major > required.major
            || (self.major == required.major
                && (self.minor > required.minor
                    || (self.minor == required.minor && self.patch >= required.patch)))
    }
}

impl fmt::Display for DatabaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for DatabaseVersion {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or(())
    }
}

impl Serialize for DatabaseVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Boolean feature flags derived from the engine and, where required, the
/// server version. Flags not tied to a version are safe for every supported
/// release of that engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityFlags {
    pub schemas: bool,
    pub sequences: bool,
    pub materialized_views: bool,
    pub routines: bool,
    pub triggers: bool,
    pub roles: bool,
    pub sessions: bool,
    pub transactions: bool,
    pub savepoints: bool,
    pub cancel_query: bool,
    pub explain: bool,
    pub check_constraints: bool,
    pub generated_columns: bool,
    pub identity_columns: bool,
    pub auto_increment_columns: bool,
    pub column_type_change: bool,
    pub column_rename: bool,
    pub table_rename: bool,
    pub table_duplicate: bool,
    pub column_reorder: bool,
    pub database_rename: bool,
    pub index_rename: bool,
    pub partial_indexes: bool,
    pub query_timeout: bool,
}

/// A user-visible action whose availability depends on engine capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseAction {
    BrowseDatabases,
    BrowseSchemas,
    BrowseTables,
    BrowseColumns,
    BrowseIndexes,
    BrowseConstraints,
    BrowseViews,
    BrowseMaterializedViews,
    BrowseSequences,
    BrowseRoutines,
    BrowseTriggers,
    BrowseRoles,
    BrowseSessions,
    ShowObjectSize,
    CreateDatabase,
    RenameDatabase,
    DropDatabase,
    SetActiveDatabase,
    CreateSchema,
    DropSchema,
    CreateTable,
    RenameTable,
    DuplicateTable,
    TruncateTable,
    DropTable,
    AddColumn,
    EditColumn,
    RenameColumn,
    DropColumn,
    ReorderColumns,
    CreateIndex,
    RenameIndex,
    DropIndex,
    CreateCheckConstraint,
    CreateView,
    CreateRoutine,
    CreateTrigger,
    ManageRoles,
    KillSession,
    CancelQuery,
    ExplainQuery,
    RunQuery,
    CreateGeneratedColumn,
    CreateIdentityColumn,
    CreateAutoIncrementColumn,
    CreatePartialIndex,
    RefreshMaterializedView,
    InsertRow,
    EditRow,
    DeleteRow,
}

impl DatabaseAction {
    pub const ALL: &'static [Self] = &[
        Self::BrowseDatabases,
        Self::BrowseSchemas,
        Self::BrowseTables,
        Self::BrowseColumns,
        Self::BrowseIndexes,
        Self::BrowseConstraints,
        Self::BrowseViews,
        Self::BrowseMaterializedViews,
        Self::BrowseSequences,
        Self::BrowseRoutines,
        Self::BrowseTriggers,
        Self::BrowseRoles,
        Self::BrowseSessions,
        Self::ShowObjectSize,
        Self::CreateDatabase,
        Self::RenameDatabase,
        Self::DropDatabase,
        Self::SetActiveDatabase,
        Self::CreateSchema,
        Self::DropSchema,
        Self::CreateTable,
        Self::RenameTable,
        Self::DuplicateTable,
        Self::TruncateTable,
        Self::DropTable,
        Self::AddColumn,
        Self::EditColumn,
        Self::RenameColumn,
        Self::DropColumn,
        Self::ReorderColumns,
        Self::CreateIndex,
        Self::RenameIndex,
        Self::DropIndex,
        Self::CreateCheckConstraint,
        Self::CreateView,
        Self::CreateRoutine,
        Self::CreateTrigger,
        Self::ManageRoles,
        Self::KillSession,
        Self::CancelQuery,
        Self::ExplainQuery,
        Self::RunQuery,
        Self::CreateGeneratedColumn,
        Self::CreateIdentityColumn,
        Self::CreateAutoIncrementColumn,
        Self::CreatePartialIndex,
        Self::RefreshMaterializedView,
        Self::InsertRow,
        Self::EditRow,
        Self::DeleteRow,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BrowseDatabases => "browse_databases",
            Self::BrowseSchemas => "browse_schemas",
            Self::BrowseTables => "browse_tables",
            Self::BrowseColumns => "browse_columns",
            Self::BrowseIndexes => "browse_indexes",
            Self::BrowseConstraints => "browse_constraints",
            Self::BrowseViews => "browse_views",
            Self::BrowseMaterializedViews => "browse_materialized_views",
            Self::BrowseSequences => "browse_sequences",
            Self::BrowseRoutines => "browse_routines",
            Self::BrowseTriggers => "browse_triggers",
            Self::BrowseRoles => "browse_roles",
            Self::BrowseSessions => "browse_sessions",
            Self::ShowObjectSize => "show_object_size",
            Self::CreateDatabase => "create_database",
            Self::RenameDatabase => "rename_database",
            Self::DropDatabase => "drop_database",
            Self::SetActiveDatabase => "set_active_database",
            Self::CreateSchema => "create_schema",
            Self::DropSchema => "drop_schema",
            Self::CreateTable => "create_table",
            Self::RenameTable => "rename_table",
            Self::DuplicateTable => "duplicate_table",
            Self::TruncateTable => "truncate_table",
            Self::DropTable => "drop_table",
            Self::AddColumn => "add_column",
            Self::EditColumn => "edit_column",
            Self::RenameColumn => "rename_column",
            Self::DropColumn => "drop_column",
            Self::ReorderColumns => "reorder_columns",
            Self::CreateIndex => "create_index",
            Self::RenameIndex => "rename_index",
            Self::DropIndex => "drop_index",
            Self::CreateCheckConstraint => "create_check_constraint",
            Self::CreateView => "create_view",
            Self::CreateRoutine => "create_routine",
            Self::CreateTrigger => "create_trigger",
            Self::ManageRoles => "manage_roles",
            Self::KillSession => "kill_session",
            Self::CancelQuery => "cancel_query",
            Self::ExplainQuery => "explain_query",
            Self::RunQuery => "run_query",
            Self::CreateGeneratedColumn => "create_generated_column",
            Self::CreateIdentityColumn => "create_identity_column",
            Self::CreateAutoIncrementColumn => "create_auto_increment_column",
            Self::CreatePartialIndex => "create_partial_index",
            Self::RefreshMaterializedView => "refresh_materialized_view",
            Self::InsertRow => "insert_row",
            Self::EditRow => "edit_row",
            Self::DeleteRow => "delete_row",
        }
    }
}

/// Engine, parsed version, and feature flags for one database connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseCapabilities {
    engine: DatabaseEngine,
    version: Option<DatabaseVersion>,
    flags: CapabilityFlags,
}

impl DatabaseCapabilities {
    /// Builds capabilities from a raw server version. Unknown versions keep
    /// baseline features enabled and disable every version-dependent action.
    pub fn for_engine(engine: DatabaseEngine, version: Option<&str>) -> Self {
        let version = version.and_then(DatabaseVersion::parse);
        Self::with_version(engine, version)
    }

    pub fn with_version(engine: DatabaseEngine, version: Option<DatabaseVersion>) -> Self {
        let flags = match engine {
            DatabaseEngine::Mysql => mysql::capabilities::flags_for(version),
            DatabaseEngine::Postgresql => postgres::capabilities::flags_for(version),
        };
        Self {
            engine,
            version,
            flags,
        }
    }

    pub fn unknown(engine: DatabaseEngine) -> Self {
        Self::with_version(engine, None)
    }

    pub const fn engine(&self) -> DatabaseEngine {
        self.engine
    }

    pub const fn version(&self) -> Option<DatabaseVersion> {
        self.version
    }

    pub const fn flags(&self) -> CapabilityFlags {
        self.flags
    }

    pub fn supports(&self, action: DatabaseAction) -> bool {
        match action {
            DatabaseAction::BrowseSchemas
            | DatabaseAction::CreateSchema
            | DatabaseAction::DropSchema => self.flags.schemas,
            DatabaseAction::BrowseSequences => self.flags.sequences,
            DatabaseAction::BrowseMaterializedViews | DatabaseAction::RefreshMaterializedView => {
                self.flags.materialized_views
            }
            DatabaseAction::BrowseRoutines | DatabaseAction::CreateRoutine => self.flags.routines,
            DatabaseAction::BrowseTriggers | DatabaseAction::CreateTrigger => self.flags.triggers,
            DatabaseAction::BrowseRoles | DatabaseAction::ManageRoles => self.flags.roles,
            DatabaseAction::BrowseSessions | DatabaseAction::KillSession => self.flags.sessions,
            DatabaseAction::CancelQuery => self.flags.cancel_query,
            DatabaseAction::ExplainQuery => self.flags.explain,
            DatabaseAction::CreateCheckConstraint => self.flags.check_constraints,
            DatabaseAction::CreateGeneratedColumn => self.flags.generated_columns,
            DatabaseAction::CreateIdentityColumn => self.flags.identity_columns,
            DatabaseAction::CreateAutoIncrementColumn => self.flags.auto_increment_columns,
            DatabaseAction::CreatePartialIndex => self.flags.partial_indexes,
            DatabaseAction::EditColumn => self.flags.column_type_change,
            DatabaseAction::RenameColumn => self.flags.column_rename,
            DatabaseAction::RenameTable => self.flags.table_rename,
            DatabaseAction::DuplicateTable => self.flags.table_duplicate,
            DatabaseAction::ReorderColumns => self.flags.column_reorder,
            DatabaseAction::RenameDatabase => self.flags.database_rename,
            DatabaseAction::RenameIndex => self.flags.index_rename,
            _ => true,
        }
    }

    /// Rejects an unsupported action before an adapter touches the database.
    pub fn ensure(&self, action: DatabaseAction) -> Result<(), DatabaseError> {
        if self.supports(action) {
            return Ok(());
        }

        let version = self
            .version
            .map(|version| version.to_string())
            .unwrap_or_else(|| "an unknown version".to_string());
        Err(DatabaseError::capability(format!(
            "{} is not supported by {} {}.",
            action.as_str(),
            self.engine.as_str(),
            version
        )))
    }

    /// Returns the non-SQL capability document commands may send to React.
    pub fn snapshot(&self) -> CapabilitySnapshot {
        CapabilitySnapshot {
            engine: self.engine,
            version: self.version,
            flags: self.flags,
            actions: DatabaseAction::ALL
                .iter()
                .copied()
                .filter(|action| self.supports(*action))
                .collect(),
        }
    }
}

/// Serialized capability document. It deliberately contains no SQL text,
/// driver error strings, or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySnapshot {
    pub engine: DatabaseEngine,
    pub version: Option<DatabaseVersion>,
    pub flags: CapabilityFlags,
    pub actions: Vec<DatabaseAction>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vendor_version_strings() {
        assert_eq!(
            DatabaseVersion::parse("8.0.36-log"),
            Some(DatabaseVersion::new(8, 0, 36))
        );
        assert_eq!(
            DatabaseVersion::parse("PostgreSQL 16.2 (Ubuntu 16.2-1)"),
            Some(DatabaseVersion::new(16, 2, 0))
        );
        assert_eq!(
            DatabaseVersion::parse("5.7.44"),
            Some(DatabaseVersion::new(5, 7, 44))
        );
        assert_eq!(DatabaseVersion::parse("no version"), None);
    }

    #[test]
    fn mysql_version_gates_capabilities() {
        let mysql_57 = DatabaseCapabilities::for_engine(DatabaseEngine::Mysql, Some("5.7.44-log"));
        assert!(mysql_57.supports(DatabaseAction::BrowseDatabases));
        assert!(mysql_57.supports(DatabaseAction::ReorderColumns));
        assert!(mysql_57.supports(DatabaseAction::CreateAutoIncrementColumn));
        assert!(!mysql_57.supports(DatabaseAction::RenameColumn));
        assert!(!mysql_57.supports(DatabaseAction::CreatePartialIndex));
        assert!(!mysql_57.supports(DatabaseAction::BrowseRoles));
        assert!(!mysql_57.supports(DatabaseAction::CreateCheckConstraint));
        assert!(!mysql_57.supports(DatabaseAction::CreateIdentityColumn));

        let mysql_80 = DatabaseCapabilities::for_engine(DatabaseEngine::Mysql, Some("8.0.36"));
        assert!(mysql_80.supports(DatabaseAction::BrowseRoles));
        assert!(mysql_80.supports(DatabaseAction::RenameColumn));
        assert!(mysql_80.supports(DatabaseAction::RenameTable));
        assert!(mysql_80.supports(DatabaseAction::CreateGeneratedColumn));
        assert!(mysql_80.supports(DatabaseAction::CreateCheckConstraint));
        assert!(mysql_80.supports(DatabaseAction::RenameIndex));
        assert!(!mysql_80.supports(DatabaseAction::RenameDatabase));
        assert!(!mysql_80.supports(DatabaseAction::BrowseMaterializedViews));
    }

    #[test]
    fn postgresql_version_gates_capabilities() {
        let pg_92 =
            DatabaseCapabilities::for_engine(DatabaseEngine::Postgresql, Some("PostgreSQL 9.2"));
        assert!(pg_92.supports(DatabaseAction::BrowseSchemas));
        assert!(pg_92.supports(DatabaseAction::BrowseSequences));
        assert!(!pg_92.supports(DatabaseAction::BrowseMaterializedViews));
        assert!(!pg_92.supports(DatabaseAction::CreateIdentityColumn));

        let pg_12 =
            DatabaseCapabilities::for_engine(DatabaseEngine::Postgresql, Some("PostgreSQL 12.4"));
        assert!(pg_12.supports(DatabaseAction::BrowseMaterializedViews));
        assert!(pg_12.supports(DatabaseAction::RefreshMaterializedView));
        assert!(pg_12.supports(DatabaseAction::CreateIdentityColumn));
        assert!(pg_12.supports(DatabaseAction::CreateGeneratedColumn));
        assert!(pg_12.supports(DatabaseAction::CreatePartialIndex));
        assert!(pg_12.supports(DatabaseAction::RenameColumn));
        assert!(pg_12.supports(DatabaseAction::RenameTable));
        assert!(!pg_12.supports(DatabaseAction::CreateAutoIncrementColumn));
        assert!(pg_12.supports(DatabaseAction::RenameDatabase));
        assert!(!pg_12.supports(DatabaseAction::ReorderColumns));
    }

    #[test]
    fn unknown_versions_keep_baseline_actions_and_disable_gated_ones() {
        for engine in DatabaseEngine::ALL {
            let capabilities = DatabaseCapabilities::unknown(engine);

            assert!(capabilities.supports(DatabaseAction::BrowseTables));
            assert!(capabilities.supports(DatabaseAction::RunQuery));
            assert!(capabilities.supports(DatabaseAction::CreateTable));
            assert!(!capabilities.supports(DatabaseAction::BrowseMaterializedViews));
            assert!(!capabilities.supports(DatabaseAction::CreateGeneratedColumn));
        }
    }

    #[test]
    fn ensure_rejects_unsupported_actions_before_execution() {
        let capabilities = DatabaseCapabilities::for_engine(DatabaseEngine::Mysql, Some("5.7.44"));
        let error = capabilities
            .ensure(DatabaseAction::CreateCheckConstraint)
            .expect_err("check constraints require MySQL 8.0.16");

        assert_eq!(error.kind, crate::database::DatabaseErrorKind::Capability);
        assert!(error.message.contains("create_check_constraint"));
    }

    #[test]
    fn snapshot_contains_only_capability_metadata() {
        let snapshot =
            DatabaseCapabilities::for_engine(DatabaseEngine::Postgresql, Some("16.2")).snapshot();
        let json = serde_json::to_string(&snapshot).expect("snapshot json");

        assert!(json.contains("\"engine\":\"postgresql\""));
        assert!(json.contains("\"version\":\"16.2.0\""));
        assert!(json.contains("browse_materialized_views"));
        assert!(!json.contains("SELECT"));
        assert!(!json.contains("information_schema"));
    }
}
