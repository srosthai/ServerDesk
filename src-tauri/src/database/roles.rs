//! Typed database users, roles, and privilege management.
//!
//! Requests cross the Tauri boundary as untrusted JSON. [`RoleActionRequest::validate`]
//! turns them into [`ValidatedRoleAction`] values with every account name,
//! host, target, and privilege checked before an engine planner sees it.
//!
//! Role attributes and grants are the most dangerous part of the database
//! manager: a single broadening grant can expose every row, and a revoke can
//! lock an application out. Every mutation therefore requires an explicit
//! confirmation, is rejected on a read-only profile, and is previewed with the
//! exact statements plus a before/after effective-privilege diff.
//!
//! No password, password hash, or authentication column is ever selected,
//! serialized, logged, or placed in history. ServerDesk does not set passwords
//! for accounts; authentication must be configured outside the app.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    time::Instant,
};

use serde::{Deserialize, Serialize};

use crate::{core::AppError, storage::DatabaseEngine};

use super::{
    capabilities::{DatabaseAction as CapabilityAction, DatabaseCapabilities},
    dialect::Identifier,
    errors::DatabaseError,
    pagination::{Page, PageRequest},
    service::DatabaseConnectionRegistry,
    traits::{DatabaseAdapter, MetadataValue},
};

/// Longest accepted account or role name.
pub const MAX_ROLE_NAME_LENGTH: usize = 63;
/// Longest accepted MySQL account host pattern.
pub const MAX_ROLE_HOST_LENGTH: usize = 255;
/// Maximum privileges accepted in one grant or revoke request.
pub const MAX_PRIVILEGES_PER_CHANGE: usize = 32;
/// Maximum role-membership graph depth and size expanded for an effective
/// privilege diff. A larger graph is rejected instead of being truncated.
pub const MAX_ROLE_GRAPH: usize = 128;

/// A user or role classification. PostgreSQL has one role concept; a login
/// role is shown as a user, and a non-login role as a role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    User,
    Role,
}

impl RoleKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Role => "role",
        }
    }
}

/// A validated account reference. MySQL accounts include a host pattern;
/// PostgreSQL roles do not.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

impl RoleRef {
    pub fn label(&self) -> String {
        match self.host.as_deref() {
            Some(host) => format!("{}@{}", self.name, host),
            None => self.name.clone(),
        }
    }

    pub fn key(&self) -> String {
        self.label()
    }
}

/// Untrusted role reference from React.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRefRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

impl RoleRefRequest {
    pub fn validate(&self, engine: DatabaseEngine) -> Result<RoleRef, DatabaseError> {
        let name = validate_role_name(&self.name)?;
        let host = match engine {
            DatabaseEngine::Mysql => Some(validate_role_host(self.host.as_deref().unwrap_or("%"))?),
            DatabaseEngine::Postgresql => {
                if self
                    .host
                    .as_deref()
                    .is_some_and(|host| !host.trim().is_empty())
                {
                    return Err(DatabaseError::validation(
                        "PostgreSQL roles do not have an account host.",
                    ));
                }
                None
            }
        };
        Ok(RoleRef { name, host })
    }
}

/// One account in the role list. It deliberately contains no password,
/// authentication plugin, password hash, or token field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleSummary {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub kind: RoleKind,
    pub can_login: MetadataValue<bool>,
    pub superuser: MetadataValue<bool>,
    pub create_database: MetadataValue<bool>,
    pub create_role: MetadataValue<bool>,
    pub inherit: MetadataValue<bool>,
}

impl RoleSummary {
    pub fn reference(&self) -> RoleRef {
        RoleRef {
            name: self.name.clone(),
            host: self.host.clone(),
        }
    }

    pub fn label(&self) -> String {
        self.reference().label()
    }
}

/// The object a privilege applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegeTargetKind {
    Global,
    Database,
    Schema,
    Table,
    Sequence,
}

impl PrivilegeTargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Database => "database",
            Self::Schema => "schema",
            Self::Table => "table",
            Self::Sequence => "sequence",
        }
    }
}

/// A validated privilege target. Each engine supports a different subset:
/// MySQL has global/database/table targets, while PostgreSQL has
/// database/schema/table/sequence targets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivilegeTarget {
    pub kind: PrivilegeTargetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<Identifier>,
}

impl PrivilegeTarget {
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(database) = &self.database {
            parts.push(database.as_str().to_string());
        }
        if let Some(schema) = &self.schema
            && self.database.as_ref().map(Identifier::as_str) != Some(schema.as_str())
        {
            parts.push(schema.as_str().to_string());
        }
        if let Some(object) = &self.object {
            parts.push(object.as_str().to_string());
        }
        if parts.is_empty() {
            return "All databases (*.*)".to_string();
        }
        format!("{} ({})", self.kind.as_str(), parts.join("."))
    }
}

/// Untrusted privilege target from React.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivilegeTargetRequest {
    pub kind: PrivilegeTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
}

impl PrivilegeTargetRequest {
    pub fn validate(&self, engine: DatabaseEngine) -> Result<PrivilegeTarget, DatabaseError> {
        let database = validate_optional_identifier("database name", self.database.as_deref())?;
        let schema = validate_optional_identifier("schema name", self.schema.as_deref())?;
        let object = validate_optional_identifier("object name", self.object.as_deref())?;

        match (engine, self.kind) {
            (DatabaseEngine::Mysql, PrivilegeTargetKind::Global) => {
                if database.is_some() || schema.is_some() || object.is_some() {
                    return Err(DatabaseError::validation(
                        "A global MySQL grant cannot name a database, schema, or table.",
                    ));
                }
            }
            (DatabaseEngine::Mysql, PrivilegeTargetKind::Database) => {
                if database.is_none() {
                    return Err(DatabaseError::validation(
                        "A MySQL database grant requires a database name.",
                    ));
                }
                if schema.is_some() || object.is_some() {
                    return Err(DatabaseError::validation(
                        "A MySQL database grant cannot include a schema or object name.",
                    ));
                }
            }
            (DatabaseEngine::Mysql, PrivilegeTargetKind::Table) => {
                if database.is_none() {
                    return Err(DatabaseError::validation(
                        "A MySQL table grant requires a database name.",
                    ));
                }
                if object.is_none() {
                    return Err(DatabaseError::validation(
                        "A MySQL table grant requires a table name.",
                    ));
                }
                if schema.is_some() {
                    return Err(DatabaseError::validation(
                        "MySQL does not have a separate schema target; use the database name.",
                    ));
                }
            }
            (
                DatabaseEngine::Mysql,
                PrivilegeTargetKind::Schema | PrivilegeTargetKind::Sequence,
            ) => {
                return Err(DatabaseError::capability(format!(
                    "MySQL does not support {} privilege targets.",
                    self.kind.as_str()
                )));
            }
            (DatabaseEngine::Postgresql, PrivilegeTargetKind::Database) => {
                if database.is_none() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL database grant requires a database name.",
                    ));
                }
                if schema.is_some() || object.is_some() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL database grant cannot include a schema or object name.",
                    ));
                }
            }
            (DatabaseEngine::Postgresql, PrivilegeTargetKind::Schema) => {
                if schema.is_none() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL schema grant requires a schema name.",
                    ));
                }
                if database.is_some() || object.is_some() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL schema grant cannot include a database or object name.",
                    ));
                }
            }
            (DatabaseEngine::Postgresql, PrivilegeTargetKind::Table) => {
                if schema.is_none() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL table grant requires a schema name.",
                    ));
                }
                if object.is_none() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL table grant requires a table name.",
                    ));
                }
                if database.is_some() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL table grant cannot include a database name.",
                    ));
                }
            }
            (DatabaseEngine::Postgresql, PrivilegeTargetKind::Sequence) => {
                if schema.is_none() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL sequence grant requires a schema name.",
                    ));
                }
                if object.is_none() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL sequence grant requires a sequence name.",
                    ));
                }
                if database.is_some() {
                    return Err(DatabaseError::validation(
                        "A PostgreSQL sequence grant cannot include a database name.",
                    ));
                }
            }
            (DatabaseEngine::Postgresql, PrivilegeTargetKind::Global) => {
                return Err(DatabaseError::capability(
                    "PostgreSQL has no global object target. Use role attributes or a database, \
                     schema, table, or sequence target.",
                ));
            }
        }

        Ok(PrivilegeTarget {
            kind: self.kind,
            database,
            schema,
            object,
        })
    }
}

/// The fixed privilege vocabulary supported by the typed editor. Database
/// administrators can still run arbitrary SQL in the SQL workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Privilege {
    Select,
    Insert,
    Update,
    Delete,
    Truncate,
    References,
    Trigger,
    Create,
    Drop,
    Alter,
    Index,
    Execute,
    Usage,
    Connect,
    Temporary,
    CreateTemporaryTables,
    LockTables,
    CreateView,
    ShowView,
    CreateRoutine,
    AlterRoutine,
    Event,
}

impl Privilege {
    pub const ALL: &'static [Self] = &[
        Self::Select,
        Self::Insert,
        Self::Update,
        Self::Delete,
        Self::Truncate,
        Self::References,
        Self::Trigger,
        Self::Create,
        Self::Drop,
        Self::Alter,
        Self::Index,
        Self::Execute,
        Self::Usage,
        Self::Connect,
        Self::Temporary,
        Self::CreateTemporaryTables,
        Self::LockTables,
        Self::CreateView,
        Self::ShowView,
        Self::CreateRoutine,
        Self::AlterRoutine,
        Self::Event,
    ];

    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Select => "SELECT",
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
            Self::References => "REFERENCES",
            Self::Trigger => "TRIGGER",
            Self::Create => "CREATE",
            Self::Drop => "DROP",
            Self::Alter => "ALTER",
            Self::Index => "INDEX",
            Self::Execute => "EXECUTE",
            Self::Usage => "USAGE",
            Self::Connect => "CONNECT",
            Self::Temporary => "TEMPORARY",
            Self::CreateTemporaryTables => "CREATE TEMPORARY TABLES",
            Self::LockTables => "LOCK TABLES",
            Self::CreateView => "CREATE VIEW",
            Self::ShowView => "SHOW VIEW",
            Self::CreateRoutine => "CREATE ROUTINE",
            Self::AlterRoutine => "ALTER ROUTINE",
            Self::Event => "EVENT",
        }
    }

    /// Rejects a privilege the selected engine cannot grant on the target.
    pub const fn supports(self, engine: DatabaseEngine, target: PrivilegeTargetKind) -> bool {
        match engine {
            DatabaseEngine::Mysql => match target {
                PrivilegeTargetKind::Global | PrivilegeTargetKind::Database => matches!(
                    self,
                    Self::Select
                        | Self::Insert
                        | Self::Update
                        | Self::Delete
                        | Self::Create
                        | Self::Drop
                        | Self::Alter
                        | Self::Index
                        | Self::References
                        | Self::Trigger
                        | Self::Execute
                        | Self::CreateTemporaryTables
                        | Self::LockTables
                        | Self::CreateView
                        | Self::ShowView
                        | Self::CreateRoutine
                        | Self::AlterRoutine
                        | Self::Event
                ),
                PrivilegeTargetKind::Table => matches!(
                    self,
                    Self::Select
                        | Self::Insert
                        | Self::Update
                        | Self::Delete
                        | Self::Create
                        | Self::Drop
                        | Self::Alter
                        | Self::Index
                        | Self::References
                        | Self::Trigger
                ),
                PrivilegeTargetKind::Schema | PrivilegeTargetKind::Sequence => false,
            },
            DatabaseEngine::Postgresql => match target {
                PrivilegeTargetKind::Database => {
                    matches!(self, Self::Connect | Self::Create | Self::Temporary)
                }
                PrivilegeTargetKind::Schema => matches!(self, Self::Usage | Self::Create),
                PrivilegeTargetKind::Table => matches!(
                    self,
                    Self::Select
                        | Self::Insert
                        | Self::Update
                        | Self::Delete
                        | Self::Truncate
                        | Self::References
                        | Self::Trigger
                ),
                PrivilegeTargetKind::Sequence => {
                    matches!(self, Self::Usage | Self::Select | Self::Update)
                }
                PrivilegeTargetKind::Global => false,
            },
        }
    }
}

/// One direct privilege grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivilegeGrant {
    pub privilege: String,
    pub target: PrivilegeTarget,
    pub grantable: bool,
}

/// One privilege after expanding role memberships. `sources` lists the direct
/// grant and any roles that contribute the same privilege.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectivePrivilege {
    pub privilege: String,
    pub target: PrivilegeTarget,
    pub grantable: bool,
    pub sources: Vec<String>,
}

/// Direct grants and memberships for one role. This is the adapter response
/// used to build an effective-privilege view without exposing SQL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleGrantData {
    pub direct: Vec<PrivilegeGrant>,
    pub member_of: Vec<RoleRef>,
}

/// Bounded effective-privilege snapshot for one role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolePrivilegeSnapshot {
    pub role: RoleRef,
    pub direct: Vec<PrivilegeGrant>,
    pub member_of: Vec<RoleRef>,
    pub effective: Vec<EffectivePrivilege>,
}

/// A before/after effective-privilege comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivilegeDiff {
    pub before: Vec<EffectivePrivilege>,
    pub after: Vec<EffectivePrivilege>,
    pub added: Vec<EffectivePrivilege>,
    pub removed: Vec<EffectivePrivilege>,
}

/// One role-attribute change shown before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleAttributeChange {
    pub attribute: String,
    pub before: String,
    pub after: String,
}

/// Role action names used by previews, results, and history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleActionKind {
    CreateRole,
    AlterRole,
    GrantPrivileges,
    RevokePrivileges,
    GrantRole,
    RevokeRole,
}

impl RoleActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreateRole => "create_role",
            Self::AlterRole => "alter_role",
            Self::GrantPrivileges => "grant_privileges",
            Self::RevokePrivileges => "revoke_privileges",
            Self::GrantRole => "grant_role",
            Self::RevokeRole => "revoke_role",
        }
    }
}

/// One typed role or privilege action requested by the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoleActionRequest {
    CreateRole {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
        #[serde(rename = "roleKind")]
        role_kind: RoleKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        can_login: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        superuser: Option<bool>,
        #[serde(
            default,
            rename = "createDatabase",
            skip_serializing_if = "Option::is_none"
        )]
        create_database: Option<bool>,
        #[serde(
            default,
            rename = "createRole",
            skip_serializing_if = "Option::is_none"
        )]
        create_role: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inherit: Option<bool>,
    },
    AlterRole {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        can_login: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        superuser: Option<bool>,
        #[serde(
            default,
            rename = "createDatabase",
            skip_serializing_if = "Option::is_none"
        )]
        create_database: Option<bool>,
        #[serde(
            default,
            rename = "createRole",
            skip_serializing_if = "Option::is_none"
        )]
        create_role: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inherit: Option<bool>,
    },
    GrantPrivileges {
        grantee: RoleRefRequest,
        privileges: Vec<Privilege>,
        target: PrivilegeTargetRequest,
        #[serde(default, rename = "withGrantOption")]
        with_grant_option: bool,
    },
    RevokePrivileges {
        grantee: RoleRefRequest,
        privileges: Vec<Privilege>,
        target: PrivilegeTargetRequest,
        #[serde(default, rename = "revokeGrantOption")]
        revoke_grant_option: bool,
    },
    GrantRole {
        role: RoleRefRequest,
        member: RoleRefRequest,
    },
    RevokeRole {
        role: RoleRefRequest,
        member: RoleRefRequest,
    },
}

impl RoleActionRequest {
    pub const fn kind(&self) -> RoleActionKind {
        match self {
            Self::CreateRole { .. } => RoleActionKind::CreateRole,
            Self::AlterRole { .. } => RoleActionKind::AlterRole,
            Self::GrantPrivileges { .. } => RoleActionKind::GrantPrivileges,
            Self::RevokePrivileges { .. } => RoleActionKind::RevokePrivileges,
            Self::GrantRole { .. } => RoleActionKind::GrantRole,
            Self::RevokeRole { .. } => RoleActionKind::RevokeRole,
        }
    }

    /// A safe label for history and errors. It contains no statement text.
    pub fn role_label(&self) -> String {
        match self {
            Self::CreateRole { name, host, .. } | Self::AlterRole { name, host, .. } => {
                role_label(name, host.as_deref())
            }
            Self::GrantPrivileges { grantee, .. } | Self::RevokePrivileges { grantee, .. } => {
                role_label(&grantee.name, grantee.host.as_deref())
            }
            Self::GrantRole { role, member } | Self::RevokeRole { role, member } => {
                format!(
                    "{} -> {}",
                    role_label(&role.name, role.host.as_deref()),
                    role_label(&member.name, member.host.as_deref())
                )
            }
        }
    }

    pub fn validate(&self, engine: DatabaseEngine) -> Result<ValidatedRoleAction, DatabaseError> {
        match self {
            Self::CreateRole {
                name,
                host,
                role_kind,
                can_login,
                superuser,
                create_database,
                create_role,
                inherit,
            } => {
                let role = RoleRefRequest {
                    name: name.clone(),
                    host: host.clone(),
                }
                .validate(engine)?;
                if engine == DatabaseEngine::Mysql && *can_login == Some(true) {
                    return Err(DatabaseError::validation(
                        "MySQL users are created locked because ServerDesk does not set \
                         passwords. Create the account locked, configure authentication outside \
                         the app, then unlock it here.",
                    ));
                }
                let changes = validate_role_changes(
                    engine,
                    RoleAttributeChanges {
                        can_login: *can_login,
                        superuser: *superuser,
                        create_database: *create_database,
                        create_role: *create_role,
                        inherit: *inherit,
                    },
                )?;
                Ok(ValidatedRoleAction::CreateRole(ValidatedCreateRole {
                    role,
                    kind: *role_kind,
                    changes,
                }))
            }
            Self::AlterRole {
                name,
                host,
                can_login,
                superuser,
                create_database,
                create_role,
                inherit,
            } => {
                let role = RoleRefRequest {
                    name: name.clone(),
                    host: host.clone(),
                }
                .validate(engine)?;
                let changes = validate_role_changes(
                    engine,
                    RoleAttributeChanges {
                        can_login: *can_login,
                        superuser: *superuser,
                        create_database: *create_database,
                        create_role: *create_role,
                        inherit: *inherit,
                    },
                )?;
                if changes.is_empty() {
                    return Err(DatabaseError::validation(
                        "A role change must set at least one attribute.",
                    ));
                }
                Ok(ValidatedRoleAction::AlterRole { role, changes })
            }
            Self::GrantPrivileges {
                grantee,
                privileges,
                target,
                with_grant_option,
            } => {
                let change = validate_privilege_change(
                    engine,
                    grantee,
                    privileges,
                    target,
                    *with_grant_option,
                )?;
                Ok(ValidatedRoleAction::GrantPrivileges(change))
            }
            Self::RevokePrivileges {
                grantee,
                privileges,
                target,
                revoke_grant_option,
            } => {
                let change = validate_privilege_change(
                    engine,
                    grantee,
                    privileges,
                    target,
                    *revoke_grant_option,
                )?;
                Ok(ValidatedRoleAction::RevokePrivileges(change))
            }
            Self::GrantRole { role, member } => Ok(ValidatedRoleAction::GrantRole {
                role: role.validate(engine)?,
                member: member.validate(engine)?,
            }),
            Self::RevokeRole { role, member } => Ok(ValidatedRoleAction::RevokeRole {
                role: role.validate(engine)?,
                member: member.validate(engine)?,
            }),
        }
    }
}

/// Role attributes accepted by create and alter actions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoleAttributeChanges {
    pub can_login: Option<bool>,
    pub superuser: Option<bool>,
    pub create_database: Option<bool>,
    pub create_role: Option<bool>,
    pub inherit: Option<bool>,
}

impl RoleAttributeChanges {
    pub const fn is_empty(&self) -> bool {
        self.can_login.is_none()
            && self.superuser.is_none()
            && self.create_database.is_none()
            && self.create_role.is_none()
            && self.inherit.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCreateRole {
    pub role: RoleRef,
    pub kind: RoleKind,
    pub changes: RoleAttributeChanges,
}

/// One validated grant or revoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPrivilegeChange {
    pub grantee: RoleRef,
    pub privileges: Vec<Privilege>,
    pub target: PrivilegeTarget,
    /// `WITH GRANT OPTION` for a grant, or `GRANT OPTION FOR` for a revoke.
    pub grant_option: bool,
}

/// A validated role or privilege action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedRoleAction {
    CreateRole(ValidatedCreateRole),
    AlterRole {
        role: RoleRef,
        changes: RoleAttributeChanges,
    },
    GrantPrivileges(ValidatedPrivilegeChange),
    RevokePrivileges(ValidatedPrivilegeChange),
    GrantRole {
        role: RoleRef,
        member: RoleRef,
    },
    RevokeRole {
        role: RoleRef,
        member: RoleRef,
    },
}

impl ValidatedRoleAction {
    pub const fn kind(&self) -> RoleActionKind {
        match self {
            Self::CreateRole(_) => RoleActionKind::CreateRole,
            Self::AlterRole { .. } => RoleActionKind::AlterRole,
            Self::GrantPrivileges(_) => RoleActionKind::GrantPrivileges,
            Self::RevokePrivileges(_) => RoleActionKind::RevokePrivileges,
            Self::GrantRole { .. } => RoleActionKind::GrantRole,
            Self::RevokeRole { .. } => RoleActionKind::RevokeRole,
        }
    }

    /// The account whose effective privileges are shown in the diff.
    pub fn primary_role(&self) -> &RoleRef {
        match self {
            Self::CreateRole(options) => &options.role,
            Self::AlterRole { role, .. } => role,
            Self::GrantPrivileges(change) | Self::RevokePrivileges(change) => &change.grantee,
            Self::GrantRole { member, .. } | Self::RevokeRole { member, .. } => member,
        }
    }

    pub fn target(&self) -> Option<&PrivilegeTarget> {
        match self {
            Self::GrantPrivileges(change) | Self::RevokePrivileges(change) => Some(&change.target),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::GrantRole { role, member } | Self::RevokeRole { role, member } => {
                format!("{} -> {}", role.label(), member.label())
            }
            _ => self.primary_role().label(),
        }
    }

    pub const fn requires_confirmation(&self) -> bool {
        true
    }

    pub const fn destructive(&self) -> bool {
        matches!(self, Self::RevokePrivileges(_) | Self::RevokeRole { .. })
    }

    pub fn ensure_supported(
        &self,
        capabilities: &DatabaseCapabilities,
    ) -> Result<(), DatabaseError> {
        capabilities.ensure(CapabilityAction::ManageRoles)
    }
}

/// Statements and warnings generated by one engine planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleDdlPlan {
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
    pub destructive: bool,
}

/// Generated statements plus the exact before/after privilege diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDdlPreview {
    pub action: RoleActionKind,
    pub engine: DatabaseEngine,
    pub role: RoleRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<PrivilegeTarget>,
    pub statements: Vec<String>,
    pub ddl: String,
    pub warnings: Vec<String>,
    pub destructive: bool,
    pub requires_confirmation: bool,
    pub privilege_diff: PrivilegeDiff,
    pub role_changes: Vec<RoleAttributeChange>,
}

/// Verified outcome of one role or privilege action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleActionResult {
    pub action: RoleActionKind,
    pub engine: DatabaseEngine,
    pub role: RoleRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<PrivilegeTarget>,
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
    pub verified: bool,
    pub verification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_error: Option<String>,
    pub privilege_diff: PrivilegeDiff,
    pub role_changes: Vec<RoleAttributeChange>,
    pub duration_ms: u64,
}

/// Stateless role service used by the database role commands.
pub struct DatabaseRoleService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseRoleService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn list(
        &self,
        profile_id: &str,
        page: PageRequest,
    ) -> Result<Page<RoleSummary>, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter
            .require_capability(CapabilityAction::BrowseRoles)
            .map_err(AppError::from)?;
        adapter.list_roles(page).await.map_err(AppError::from)
    }

    pub async fn inspect(
        &self,
        profile_id: &str,
        role: RoleRefRequest,
    ) -> Result<RolePrivilegeSnapshot, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        let role = role.validate(adapter.engine()).map_err(AppError::from)?;
        adapter
            .require_capability(CapabilityAction::BrowseRoles)
            .map_err(AppError::from)?;
        inspect_role_with_adapter(adapter.as_ref(), &role)
            .await
            .map_err(AppError::from)
    }

    pub async fn preview(
        &self,
        profile_id: &str,
        request: RoleActionRequest,
    ) -> Result<RoleDdlPreview, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        preview_role_with_adapter(adapter.as_ref(), &request)
            .await
            .map_err(AppError::from)
    }

    pub async fn execute(
        &self,
        profile_id: &str,
        request: RoleActionRequest,
        confirmed: bool,
    ) -> Result<RoleActionResult, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        execute_role_with_adapter(adapter.as_mut(), request, confirmed)
            .await
            .map_err(AppError::from)
    }
}

/// Reads one role and its effective privileges without mutating anything.
pub async fn inspect_role_with_adapter(
    adapter: &dyn DatabaseAdapter,
    role: &RoleRef,
) -> Result<RolePrivilegeSnapshot, DatabaseError> {
    adapter.require_connection()?;
    if adapter.role_summary(role).await?.is_none() {
        return Err(DatabaseError::query(format!(
            "The role {} does not exist or is not visible to this account.",
            role.label()
        )));
    }
    let graph = read_role_graph(adapter, role).await?;
    let data = graph.get(role).cloned().unwrap_or_default();
    let mut direct = canonicalize_grants(data.direct);
    direct.sort_by(|left, right| {
        (left.target.clone(), left.privilege.clone())
            .cmp(&(right.target.clone(), right.privilege.clone()))
    });
    let mut member_of = data.member_of;
    member_of.sort_by_key(RoleRef::label);
    member_of.dedup();
    let effective = effective_for(&graph, role);
    Ok(RolePrivilegeSnapshot {
        role: role.clone(),
        direct,
        member_of,
        effective,
    })
}

/// Validates a request and generates the exact statements plus a privilege
/// diff. Execution always calls this same path first.
pub async fn preview_role_with_adapter(
    adapter: &dyn DatabaseAdapter,
    request: &RoleActionRequest,
) -> Result<RoleDdlPreview, DatabaseError> {
    adapter.require_connection()?;
    let action = request.validate(adapter.engine())?;
    action.ensure_supported(adapter.capabilities())?;
    let plan = adapter.preview_role_action(&action).await?;
    build_preview(adapter, &action, plan).await
}

/// Executes one role or privilege action after re-running the preview path.
pub async fn execute_role_with_adapter(
    adapter: &mut dyn DatabaseAdapter,
    request: RoleActionRequest,
    confirmed: bool,
) -> Result<RoleActionResult, DatabaseError> {
    let started = Instant::now();
    let action = request.validate(adapter.engine())?;
    let preview = preview_role_with_adapter(adapter, &request).await?;

    if action.requires_confirmation() && !confirmed {
        return Err(DatabaseError::cancelled(format!(
            "The {} action for {} was cancelled because it was not confirmed.",
            action.kind().as_str(),
            action.label()
        ))
        .with_details(
            "No statement was executed. Review the exact target, privileges, warnings, and \
             before/after diff, then confirm the privilege change.",
        ));
    }

    if adapter.is_read_only() {
        return Err(DatabaseError::permission(
            "The database connection profile is read-only; users, roles, and grants require a \
             writable connection.",
        )
        .with_scope("role", action.primary_role().label()));
    }

    adapter.execute_role_action(&action).await?;
    let verification = verify_role_action(adapter, &action, &preview).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(RoleActionResult {
        action: preview.action,
        engine: adapter.engine(),
        role: preview.role,
        target: preview.target,
        statements: preview.statements,
        warnings: preview.warnings,
        verified: verification.verified,
        verification: verification.summary,
        verification_error: verification.error,
        privilege_diff: preview.privilege_diff,
        role_changes: preview.role_changes,
        duration_ms,
    })
}

struct Verification {
    verified: bool,
    summary: String,
    error: Option<String>,
}

async fn verify_role_action(
    adapter: &dyn DatabaseAdapter,
    action: &ValidatedRoleAction,
    preview: &RoleDdlPreview,
) -> Verification {
    match action {
        ValidatedRoleAction::CreateRole(options) => {
            match adapter.role_summary(&options.role).await {
                Ok(Some(summary)) => {
                    if role_matches_changes(&summary, &preview.role_changes) {
                        Verification {
                            verified: true,
                            summary: format!(
                                "Role metadata re-read confirms {} exists with the requested \
                                 attributes.",
                                options.role.label()
                            ),
                            error: None,
                        }
                    } else {
                        Verification {
                            verified: false,
                            summary: format!(
                                "The create action finished, but {} does not report the requested \
                                 attributes.",
                                options.role.label()
                            ),
                            error: Some(
                                "Role attributes changed after execution or were not applied."
                                    .to_string(),
                            ),
                        }
                    }
                }
                Ok(None) => Verification {
                    verified: false,
                    summary: format!(
                        "The create action finished, but {} is not present in role metadata.",
                        options.role.label()
                    ),
                    error: Some("The role was not present after the create action.".to_string()),
                },
                Err(error) => Verification {
                    verified: false,
                    summary: format!(
                        "The create action finished, but role metadata for {} could not be \
                         re-read.",
                        options.role.label()
                    ),
                    error: Some(error.message),
                },
            }
        }
        ValidatedRoleAction::AlterRole { role, .. } => match adapter.role_summary(role).await {
            Ok(Some(summary)) => {
                if role_matches_changes(&summary, &preview.role_changes) {
                    Verification {
                        verified: true,
                        summary: format!(
                            "Role metadata re-read confirms the requested attributes for {}.",
                            role.label()
                        ),
                        error: None,
                    }
                } else {
                    Verification {
                        verified: false,
                        summary: format!(
                            "The alter action finished, but {} does not report the requested \
                             attributes.",
                            role.label()
                        ),
                        error: Some(
                            "Role attributes changed after execution or were not applied."
                                .to_string(),
                        ),
                    }
                }
            }
            Ok(None) => Verification {
                verified: false,
                summary: format!(
                    "The alter action finished, but {} is not present in role metadata.",
                    role.label()
                ),
                error: Some("The role was not present after the alter action.".to_string()),
            },
            Err(error) => Verification {
                verified: false,
                summary: format!(
                    "The alter action finished, but role metadata for {} could not be re-read.",
                    role.label()
                ),
                error: Some(error.message),
            },
        },
        ValidatedRoleAction::GrantPrivileges(_)
        | ValidatedRoleAction::RevokePrivileges(_)
        | ValidatedRoleAction::GrantRole { .. }
        | ValidatedRoleAction::RevokeRole { .. } => {
            match inspect_role_with_adapter(adapter, action.primary_role()).await {
                Ok(snapshot) => {
                    let actual = canonicalize_effective(snapshot.effective);
                    let expected = canonicalize_effective(preview.privilege_diff.after.clone());
                    if effective_sets_match(&actual, &expected) {
                        Verification {
                            verified: true,
                            summary: format!(
                                "Privilege metadata re-read confirms the requested effective \
                                 privileges for {}.",
                                action.primary_role().label()
                            ),
                            error: None,
                        }
                    } else {
                        let missing = diff_effective(&expected, &actual);
                        let unexpected = diff_effective(&actual, &expected);
                        Verification {
                            verified: false,
                            summary: format!(
                                "The {} action finished, but effective privileges for {} do not \
                                 match the preview.",
                                action.kind().as_str(),
                                action.primary_role().label()
                            ),
                            error: Some(format!(
                                "Missing after execution: {}. Unexpected after execution: {}.",
                                describe_effective(&missing),
                                describe_effective(&unexpected)
                            )),
                        }
                    }
                }
                Err(error) => Verification {
                    verified: false,
                    summary: format!(
                        "The {} action finished, but privileges for {} could not be re-read.",
                        action.kind().as_str(),
                        action.primary_role().label()
                    ),
                    error: Some(error.message),
                },
            }
        }
    }
}

async fn build_preview(
    adapter: &dyn DatabaseAdapter,
    action: &ValidatedRoleAction,
    plan: RoleDdlPlan,
) -> Result<RoleDdlPreview, DatabaseError> {
    let ddl = plan.statements.join(";\n");
    let (privilege_diff, role_changes) = match action {
        ValidatedRoleAction::CreateRole(options) => {
            let before = adapter.role_summary(&options.role).await?;
            if before.is_some() {
                return Err(DatabaseError::validation(format!(
                    "The role {} already exists.",
                    options.role.label()
                )));
            }
            let after = projected_summary(adapter.engine(), action, None)?;
            (
                PrivilegeDiff::default(),
                role_attribute_changes(None, Some(&after)),
            )
        }
        ValidatedRoleAction::AlterRole { role, .. } => {
            let before = adapter.role_summary(role).await?;
            let Some(before) = before else {
                return Err(DatabaseError::query(format!(
                    "The role {} does not exist or is not visible to this account.",
                    role.label()
                )));
            };
            if adapter.engine() == DatabaseEngine::Mysql
                && before.kind == RoleKind::Role
                && matches!(action, ValidatedRoleAction::AlterRole { changes, .. } if changes.can_login == Some(true))
            {
                return Err(DatabaseError::validation(
                    "MySQL roles cannot log in. Grant the role to a user instead of unlocking the \
                     role account.",
                ));
            }
            let after = projected_summary(adapter.engine(), action, Some(&before))?;
            (
                PrivilegeDiff::default(),
                role_attribute_changes(Some(&before), Some(&after)),
            )
        }
        ValidatedRoleAction::GrantPrivileges(_)
        | ValidatedRoleAction::RevokePrivileges(_)
        | ValidatedRoleAction::GrantRole { .. }
        | ValidatedRoleAction::RevokeRole { .. } => {
            if adapter.role_summary(action.primary_role()).await?.is_none() {
                return Err(DatabaseError::query(format!(
                    "The role {} does not exist or is not visible to this account.",
                    action.primary_role().label()
                )));
            }
            if let ValidatedRoleAction::GrantRole { role, .. }
            | ValidatedRoleAction::RevokeRole { role, .. } = action
                && adapter.role_summary(role).await?.is_none()
            {
                return Err(DatabaseError::query(format!(
                    "The role {} does not exist or is not visible to this account.",
                    role.label()
                )));
            }
            let graph = read_role_graph(adapter, action.primary_role()).await?;
            let root_data = graph
                .get(action.primary_role())
                .cloned()
                .unwrap_or_default();
            match action {
                ValidatedRoleAction::RevokePrivileges(change) => {
                    let missing = change
                        .privileges
                        .iter()
                        .filter(|privilege| {
                            !root_data.direct.iter().any(|grant| {
                                grant.privilege == privilege.as_sql()
                                    && grant.target == change.target
                                    && (!change.grant_option || grant.grantable)
                            })
                        })
                        .map(|privilege| privilege.as_sql())
                        .collect::<Vec<_>>();
                    if !missing.is_empty() {
                        return Err(DatabaseError::validation(format!(
                            "These privileges are not direct grants for {}: {}. Revoke them from \
                             the role that grants them, or remove the role membership instead.",
                            change.grantee.label(),
                            missing.join(", ")
                        )));
                    }
                }
                ValidatedRoleAction::GrantRole { role, .. }
                    if root_data.member_of.contains(role) =>
                {
                    return Err(DatabaseError::validation(format!(
                        "{} is already a member of {}.",
                        action.primary_role().label(),
                        role.label()
                    )));
                }
                ValidatedRoleAction::RevokeRole { role, .. }
                    if !root_data.member_of.contains(role) =>
                {
                    return Err(DatabaseError::validation(format!(
                        "{} is not a member of {}.",
                        action.primary_role().label(),
                        role.label()
                    )));
                }
                _ => {}
            }
            let before = effective_for(&graph, action.primary_role());
            let projected = project_graph(&graph, action)?;
            let projected = complete_role_graph(adapter, projected).await?;
            let after = effective_for(&projected, action.primary_role());
            let added = diff_effective(&after, &before);
            let removed = diff_effective(&before, &after);
            (
                PrivilegeDiff {
                    before,
                    after,
                    added,
                    removed,
                },
                Vec::new(),
            )
        }
    };

    Ok(RoleDdlPreview {
        action: action.kind(),
        engine: adapter.engine(),
        role: action.primary_role().clone(),
        target: action.target().cloned(),
        statements: plan.statements,
        ddl,
        warnings: plan.warnings,
        destructive: plan.destructive || action.destructive(),
        requires_confirmation: action.requires_confirmation(),
        privilege_diff,
        role_changes,
    })
}

fn projected_summary(
    engine: DatabaseEngine,
    action: &ValidatedRoleAction,
    before: Option<&RoleSummary>,
) -> Result<RoleSummary, DatabaseError> {
    match action {
        ValidatedRoleAction::CreateRole(options) => {
            let can_login = match engine {
                // MySQL accounts are always created locked because this app
                // never accepts or generates a password.
                DatabaseEngine::Mysql => false,
                DatabaseEngine::Postgresql => options
                    .changes
                    .can_login
                    .unwrap_or(options.kind == RoleKind::User),
            };
            let superuser = options.changes.superuser.unwrap_or(false);
            let create_database = options.changes.create_database.unwrap_or(false);
            let create_role = options.changes.create_role.unwrap_or(false);
            let inherit = options.changes.inherit.unwrap_or(true);
            Ok(RoleSummary {
                name: options.role.name.clone(),
                host: options.role.host.clone(),
                kind: options.kind,
                can_login: MetadataValue::available(can_login),
                superuser: match engine {
                    DatabaseEngine::Mysql => MetadataValue::Unavailable,
                    DatabaseEngine::Postgresql => MetadataValue::available(superuser),
                },
                create_database: match engine {
                    DatabaseEngine::Mysql => MetadataValue::Unavailable,
                    DatabaseEngine::Postgresql => MetadataValue::available(create_database),
                },
                create_role: match engine {
                    DatabaseEngine::Mysql => MetadataValue::Unavailable,
                    DatabaseEngine::Postgresql => MetadataValue::available(create_role),
                },
                inherit: match engine {
                    DatabaseEngine::Mysql => MetadataValue::Unavailable,
                    DatabaseEngine::Postgresql => MetadataValue::available(inherit),
                },
            })
        }
        ValidatedRoleAction::AlterRole { changes, .. } => {
            let Some(before) = before else {
                return Err(DatabaseError::query(
                    "The role does not exist or is not visible to this account.",
                ));
            };
            let mut summary = before.clone();
            if let Some(value) = changes.can_login {
                summary.can_login = MetadataValue::available(value);
            }
            if let Some(value) = changes.superuser {
                summary.superuser = MetadataValue::available(value);
            }
            if let Some(value) = changes.create_database {
                summary.create_database = MetadataValue::available(value);
            }
            if let Some(value) = changes.create_role {
                summary.create_role = MetadataValue::available(value);
            }
            if let Some(value) = changes.inherit {
                summary.inherit = MetadataValue::available(value);
            }
            Ok(summary)
        }
        _ => Err(DatabaseError::internal(
            "A privilege action does not have a projected role summary.",
        )),
    }
}

fn role_matches_changes(actual: &RoleSummary, changes: &[RoleAttributeChange]) -> bool {
    changes.iter().all(|change| {
        let actual_value = match change.attribute.as_str() {
            "canLogin" => format_metadata_bool(&actual.can_login),
            "superuser" => format_metadata_bool(&actual.superuser),
            "createDatabase" => format_metadata_bool(&actual.create_database),
            "createRole" => format_metadata_bool(&actual.create_role),
            "inherit" => format_metadata_bool(&actual.inherit),
            _ => return true,
        };
        actual_value == change.after
    })
}

fn role_attribute_changes(
    before: Option<&RoleSummary>,
    after: Option<&RoleSummary>,
) -> Vec<RoleAttributeChange> {
    let Some(after) = after else {
        return Vec::new();
    };
    let attributes = [
        (
            "canLogin",
            after.can_login.clone(),
            before.map(|value| value.can_login.clone()),
        ),
        (
            "superuser",
            after.superuser.clone(),
            before.map(|value| value.superuser.clone()),
        ),
        (
            "createDatabase",
            after.create_database.clone(),
            before.map(|value| value.create_database.clone()),
        ),
        (
            "createRole",
            after.create_role.clone(),
            before.map(|value| value.create_role.clone()),
        ),
        (
            "inherit",
            after.inherit.clone(),
            before.map(|value| value.inherit.clone()),
        ),
    ];

    attributes
        .into_iter()
        .filter_map(|(attribute, after, before)| {
            let before_text = before
                .as_ref()
                .map(format_metadata_bool)
                .unwrap_or_else(|| "Not present".to_string());
            let after_text = format_metadata_bool(&after);
            (before_text != after_text).then_some(RoleAttributeChange {
                attribute: attribute.to_string(),
                before: before_text,
                after: after_text,
            })
        })
        .collect()
}

fn format_metadata_bool(value: &MetadataValue<bool>) -> String {
    match value.value() {
        Some(true) => "Yes".to_string(),
        Some(false) => "No".to_string(),
        None => "Unavailable".to_string(),
    }
}

async fn read_role_graph(
    adapter: &dyn DatabaseAdapter,
    root: &RoleRef,
) -> Result<HashMap<RoleRef, RoleGrantData>, DatabaseError> {
    let mut graph = HashMap::new();
    let mut queue = VecDeque::from([root.clone()]);
    while let Some(role) = queue.pop_front() {
        if graph.contains_key(&role) {
            continue;
        }
        if graph.len() >= MAX_ROLE_GRAPH {
            return Err(DatabaseError::validation(format!(
                "The role membership graph exceeds {MAX_ROLE_GRAPH} roles, so the effective \
                 privilege diff cannot be computed safely."
            )));
        }
        let data = adapter.role_grants(&role).await?;
        for member in &data.member_of {
            if !graph.contains_key(member) {
                queue.push_back(member.clone());
            }
        }
        graph.insert(role, data);
    }
    Ok(graph)
}

/// Loads any role introduced by a membership projection so the after-diff
/// includes the newly inherited direct grants, not just the membership edge.
async fn complete_role_graph(
    adapter: &dyn DatabaseAdapter,
    mut graph: HashMap<RoleRef, RoleGrantData>,
) -> Result<HashMap<RoleRef, RoleGrantData>, DatabaseError> {
    loop {
        let missing = graph
            .values()
            .flat_map(|data| data.member_of.iter())
            .filter(|role| !graph.contains_key(*role))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(graph);
        }
        for role in missing {
            if graph.len() >= MAX_ROLE_GRAPH {
                return Err(DatabaseError::validation(format!(
                    "The role membership graph exceeds {MAX_ROLE_GRAPH} roles, so the effective \
                     privilege diff cannot be computed safely."
                )));
            }
            let data = adapter.role_grants(&role).await?;
            graph.insert(role, data);
        }
    }
}

fn project_graph(
    graph: &HashMap<RoleRef, RoleGrantData>,
    action: &ValidatedRoleAction,
) -> Result<HashMap<RoleRef, RoleGrantData>, DatabaseError> {
    let mut projected = graph.clone();
    match action {
        ValidatedRoleAction::GrantPrivileges(change) => {
            let data = projected.entry(change.grantee.clone()).or_default();
            for privilege in &change.privileges {
                data.direct.push(PrivilegeGrant {
                    privilege: privilege.as_sql().to_string(),
                    target: change.target.clone(),
                    grantable: change.grant_option,
                });
            }
            data.direct = canonicalize_grants(std::mem::take(&mut data.direct));
        }
        ValidatedRoleAction::RevokePrivileges(change) => {
            let data = projected.entry(change.grantee.clone()).or_default();
            let mut next = Vec::new();
            for grant in std::mem::take(&mut data.direct) {
                let matches = change
                    .privileges
                    .iter()
                    .any(|privilege| privilege.as_sql() == grant.privilege)
                    && grant.target == change.target;
                if !matches {
                    next.push(grant);
                } else if change.grant_option && grant.grantable {
                    let mut downgraded = grant;
                    downgraded.grantable = false;
                    next.push(downgraded);
                }
            }
            data.direct = canonicalize_grants(next);
        }
        ValidatedRoleAction::GrantRole { role, member } => {
            let data = projected.entry(member.clone()).or_default();
            if !data.member_of.contains(role) {
                data.member_of.push(role.clone());
            }
        }
        ValidatedRoleAction::RevokeRole { role, member } => {
            let data = projected.entry(member.clone()).or_default();
            data.member_of.retain(|entry| entry != role);
        }
        ValidatedRoleAction::CreateRole(_) | ValidatedRoleAction::AlterRole { .. } => {}
    }
    Ok(projected)
}

fn effective_for(
    graph: &HashMap<RoleRef, RoleGrantData>,
    role: &RoleRef,
) -> Vec<EffectivePrivilege> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([(role.clone(), "direct".to_string())]);
    let mut grants = Vec::new();
    while let Some((current, source)) = queue.pop_front() {
        if !seen.insert(current.clone()) {
            continue;
        }
        let Some(data) = graph.get(&current) else {
            continue;
        };
        for grant in &data.direct {
            grants.push(EffectivePrivilege {
                privilege: grant.privilege.clone(),
                target: grant.target.clone(),
                grantable: grant.grantable,
                sources: vec![source.clone()],
            });
        }
        for member in &data.member_of {
            queue.push_back((member.clone(), member.label()));
        }
    }
    canonicalize_effective(grants)
}

fn canonicalize_grants(grants: Vec<PrivilegeGrant>) -> Vec<PrivilegeGrant> {
    let mut map: BTreeMap<(PrivilegeTarget, String), bool> = BTreeMap::new();
    for grant in grants {
        let entry = map.entry((grant.target, grant.privilege)).or_insert(false);
        *entry = *entry || grant.grantable;
    }
    map.into_iter()
        .map(|((target, privilege), grantable)| PrivilegeGrant {
            privilege,
            target,
            grantable,
        })
        .collect()
}

fn canonicalize_effective(grants: Vec<EffectivePrivilege>) -> Vec<EffectivePrivilege> {
    let mut map: BTreeMap<(PrivilegeTarget, String), (bool, BTreeSet<String>)> = BTreeMap::new();
    for grant in grants {
        let entry = map
            .entry((grant.target, grant.privilege))
            .or_insert_with(|| (false, BTreeSet::new()));
        entry.0 = entry.0 || grant.grantable;
        entry.1.extend(grant.sources);
    }
    map.into_iter()
        .map(
            |((target, privilege), (grantable, sources))| EffectivePrivilege {
                privilege,
                target,
                grantable,
                sources: sources.into_iter().collect(),
            },
        )
        .collect()
}

fn effective_sets_match(left: &[EffectivePrivilege], right: &[EffectivePrivilege]) -> bool {
    let keys = |grants: &[EffectivePrivilege]| {
        grants
            .iter()
            .map(|grant| {
                (
                    grant.target.clone(),
                    grant.privilege.clone(),
                    grant.grantable,
                )
            })
            .collect::<HashSet<_>>()
    };
    keys(left) == keys(right)
}

fn diff_effective(
    left: &[EffectivePrivilege],
    right: &[EffectivePrivilege],
) -> Vec<EffectivePrivilege> {
    let right_keys = right
        .iter()
        .map(|grant| {
            (
                grant.target.clone(),
                grant.privilege.clone(),
                grant.grantable,
            )
        })
        .collect::<HashSet<_>>();
    let mut difference = left
        .iter()
        .filter(|grant| {
            !right_keys.contains(&(
                grant.target.clone(),
                grant.privilege.clone(),
                grant.grantable,
            ))
        })
        .cloned()
        .collect::<Vec<_>>();
    difference.sort_by(|left, right| {
        (left.target.clone(), left.privilege.clone(), left.grantable).cmp(&(
            right.target.clone(),
            right.privilege.clone(),
            right.grantable,
        ))
    });
    difference
}

fn describe_effective(grants: &[EffectivePrivilege]) -> String {
    if grants.is_empty() {
        return "none".to_string();
    }
    grants
        .iter()
        .map(|grant| {
            format!(
                "{} on {}{}",
                grant.privilege,
                grant.target.label(),
                if grant.grantable {
                    " with grant option"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn validate_privilege_change(
    engine: DatabaseEngine,
    grantee: &RoleRefRequest,
    privileges: &[Privilege],
    target: &PrivilegeTargetRequest,
    grant_option: bool,
) -> Result<ValidatedPrivilegeChange, DatabaseError> {
    let grantee = grantee.validate(engine)?;
    let target = target.validate(engine)?;
    if privileges.is_empty() {
        return Err(DatabaseError::validation(
            "Select at least one privilege to grant or revoke.",
        ));
    }
    if privileges.len() > MAX_PRIVILEGES_PER_CHANGE {
        return Err(DatabaseError::validation(format!(
            "At most {MAX_PRIVILEGES_PER_CHANGE} privileges can be changed at once."
        )));
    }
    let mut unique = BTreeSet::new();
    for privilege in privileges {
        if !unique.insert(*privilege) {
            return Err(DatabaseError::validation(format!(
                "The privilege {} was selected more than once.",
                privilege.as_sql()
            )));
        }
        if !privilege.supports(engine, target.kind) {
            return Err(DatabaseError::validation(format!(
                "{} is not supported for a {} target on {}.",
                privilege.as_sql(),
                target.kind.as_str(),
                match engine {
                    DatabaseEngine::Mysql => "MySQL",
                    DatabaseEngine::Postgresql => "PostgreSQL",
                }
            )));
        }
    }
    Ok(ValidatedPrivilegeChange {
        grantee,
        privileges: unique.into_iter().collect(),
        target,
        grant_option,
    })
}

fn validate_role_changes(
    engine: DatabaseEngine,
    changes: RoleAttributeChanges,
) -> Result<RoleAttributeChanges, DatabaseError> {
    if engine == DatabaseEngine::Mysql
        && (changes.superuser.is_some()
            || changes.create_database.is_some()
            || changes.create_role.is_some()
            || changes.inherit.is_some())
    {
        return Err(DatabaseError::validation(
            "MySQL manages SUPER, CREATE DATABASE, CREATE USER, and role inheritance through \
             privileges or server configuration, not account attributes. Use a privilege grant.",
        ));
    }
    Ok(changes)
}

fn validate_role_name(value: &str) -> Result<String, DatabaseError> {
    if value.trim() != value {
        return Err(DatabaseError::validation(
            "The role name must not start or end with whitespace.",
        ));
    }
    if value.is_empty() || value.len() > MAX_ROLE_NAME_LENGTH {
        return Err(DatabaseError::validation(format!(
            "The role name must be between 1 and {MAX_ROLE_NAME_LENGTH} bytes."
        )));
    }
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '$')
    }) {
        return Err(DatabaseError::validation(
            "The role name contains unsupported characters.",
        )
        .with_details(
            "Use letters, digits, '_', '-', '.', or '$'. Quote-sensitive characters are rejected.",
        ));
    }
    Ok(value.to_string())
}

fn validate_role_host(value: &str) -> Result<String, DatabaseError> {
    if value.trim() != value {
        return Err(DatabaseError::validation(
            "The account host must not start or end with whitespace.",
        ));
    }
    if value.is_empty() || value.len() > MAX_ROLE_HOST_LENGTH {
        return Err(DatabaseError::validation(format!(
            "The account host must be between 1 and {MAX_ROLE_HOST_LENGTH} bytes."
        )));
    }
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '%' | '_' | '-' | '.' | ':' | '[' | ']')
    }) {
        return Err(
            DatabaseError::validation("The account host contains unsupported characters.")
                .with_details("Use letters, digits, '%', '_', '-', '.', ':', '[', or ']'."),
        );
    }
    Ok(value.to_string())
}

fn validate_optional_identifier(
    label: &str,
    value: Option<&str>,
) -> Result<Option<Identifier>, DatabaseError> {
    value
        .map(|value| {
            if value.trim() != value {
                return Err(DatabaseError::validation(format!(
                    "The {label} must not start or end with whitespace."
                )));
            }
            Identifier::new(value).map_err(|error| {
                DatabaseError::validation(format!("The {label} is not valid."))
                    .with_details(error.message)
            })
        })
        .transpose()
}

fn role_label(name: &str, host: Option<&str>) -> String {
    match host {
        Some(host) => format!("{name}@{host}"),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::database::{
        DatabaseConnectRequest, DatabaseErrorKind, DatabasePassword, FakeDatabaseAdapter,
    };

    async fn fake(engine: DatabaseEngine, version: &str, read_only: bool) -> FakeDatabaseAdapter {
        let mut adapter = FakeDatabaseAdapter::new(engine, Some(version));
        adapter
            .connect(DatabaseConnectRequest {
                host: "127.0.0.1".to_string(),
                port: 5432,
                socket: None,
                database: Some("app".to_string()),
                username: "app".to_string(),
                password: DatabasePassword::new("fixture-password"),
                read_only,
                ssl_mode: None,
                connect_timeout: Duration::from_secs(5),
            })
            .await
            .expect("fake connection");
        adapter
    }

    fn postgres_grant() -> RoleActionRequest {
        RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Insert],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: None,
                schema: Some("public".to_string()),
                object: Some("users".to_string()),
            },
            with_grant_option: false,
        }
    }

    #[tokio::test]
    async fn lists_roles_and_expands_effective_privileges() {
        let adapter = fake(DatabaseEngine::Postgresql, "16.2", false).await;
        let roles = adapter
            .list_roles(PageRequest::new(10, 0))
            .await
            .expect("roles");
        assert_eq!(roles.items.len(), 4);
        assert!(roles.items.iter().any(|role| role.name == "reporting"
            && role.kind == RoleKind::User
            && role.can_login.value() == Some(&true)));

        let snapshot = inspect_role_with_adapter(
            &adapter,
            &RoleRef {
                name: "reporting".to_string(),
                host: None,
            },
        )
        .await
        .expect("snapshot");
        assert_eq!(snapshot.direct.len(), 1);
        assert_eq!(snapshot.member_of.len(), 1);
        assert!(
            snapshot
                .effective
                .iter()
                .any(|grant| grant.privilege == "SELECT"
                    && grant.target.object.as_ref().map(Identifier::as_str) == Some("users")
                    && grant.sources.contains(&"reporting_reader".to_string()))
        );
    }

    #[tokio::test]
    async fn grant_preview_shows_exact_sql_and_before_after_diff() {
        let adapter = fake(DatabaseEngine::Postgresql, "16.2", false).await;
        let preview = preview_role_with_adapter(&adapter, &postgres_grant())
            .await
            .expect("preview");

        assert_eq!(
            preview.statements,
            vec!["GRANT INSERT ON TABLE \"public\".\"users\" TO \"reporting\""]
        );
        assert!(preview.requires_confirmation);
        assert_eq!(preview.privilege_diff.added.len(), 1);
        assert_eq!(preview.privilege_diff.added[0].privilege, "INSERT");
        assert!(preview.privilege_diff.removed.is_empty());
        assert!(
            preview
                .privilege_diff
                .before
                .iter()
                .all(|grant| grant.privilege != "INSERT")
        );
        assert!(
            preview
                .privilege_diff
                .after
                .iter()
                .any(|grant| grant.privilege == "INSERT")
        );
    }

    #[tokio::test]
    async fn revoke_preview_is_destructive_and_removes_the_privilege() {
        let adapter = fake(DatabaseEngine::Postgresql, "16.2", false).await;
        let request = RoleActionRequest::RevokePrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Select],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: None,
                schema: Some("public".to_string()),
                object: Some("orders".to_string()),
            },
            revoke_grant_option: false,
        };
        let preview = preview_role_with_adapter(&adapter, &request)
            .await
            .expect("revoke preview");
        assert!(preview.destructive);
        assert_eq!(
            preview.statements,
            vec!["REVOKE SELECT ON TABLE \"public\".\"orders\" FROM \"reporting\""]
        );
        assert!(
            preview
                .privilege_diff
                .removed
                .iter()
                .any(|grant| grant.privilege == "SELECT")
        );
        assert!(
            preview.privilege_diff.after.iter().all(|grant| grant
                .target
                .object
                .as_ref()
                .map(Identifier::as_str)
                != Some("orders"))
        );
    }

    #[tokio::test]
    async fn revoking_an_inherited_privilege_requires_the_granting_role() {
        let adapter = fake(DatabaseEngine::Postgresql, "16.2", false).await;
        let request = RoleActionRequest::RevokePrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Select],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: None,
                schema: Some("public".to_string()),
                object: Some("users".to_string()),
            },
            revoke_grant_option: false,
        };
        let error = preview_role_with_adapter(&adapter, &request)
            .await
            .expect_err("inherited privilege is not a direct grant");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);
        assert!(error.message.contains("not direct grants"));
    }

    #[tokio::test]
    async fn role_membership_grant_adds_inherited_privileges_to_the_diff() {
        let adapter = fake(DatabaseEngine::Postgresql, "16.2", false).await;
        let request = RoleActionRequest::GrantRole {
            role: RoleRefRequest {
                name: "auditor".to_string(),
                host: None,
            },
            member: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
        };
        let preview = preview_role_with_adapter(&adapter, &request)
            .await
            .expect("role grant preview");
        assert_eq!(
            preview.statements,
            vec!["GRANT \"auditor\" TO \"reporting\""]
        );
        assert!(preview.privilege_diff.added.iter().any(|grant| {
            grant.privilege == "SELECT"
                && grant.grantable
                && grant.sources.contains(&"auditor".to_string())
        }));
    }

    #[tokio::test]
    async fn create_and_alter_role_are_confirmed_and_verified() {
        let mut adapter = fake(DatabaseEngine::Postgresql, "16.2", false).await;
        let create = RoleActionRequest::CreateRole {
            name: "deploy".to_string(),
            host: None,
            role_kind: RoleKind::User,
            can_login: Some(true),
            superuser: Some(false),
            create_database: Some(false),
            create_role: Some(false),
            inherit: Some(true),
        };
        let preview = preview_role_with_adapter(&adapter, &create)
            .await
            .expect("create preview");
        assert_eq!(
            preview.statements,
            vec!["CREATE ROLE \"deploy\" LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT"]
        );
        assert!(
            preview
                .role_changes
                .iter()
                .any(|change| change.attribute == "canLogin")
        );

        let unconfirmed = execute_role_with_adapter(&mut adapter, create.clone(), false)
            .await
            .expect_err("confirmation required");
        assert_eq!(unconfirmed.kind, DatabaseErrorKind::Cancelled);
        assert_eq!(adapter.role_execution_count(), 0);

        let result = execute_role_with_adapter(&mut adapter, create, true)
            .await
            .expect("create result");
        assert!(result.verified);
        assert!(result.verification.contains("exists"));

        let alter = RoleActionRequest::AlterRole {
            name: "deploy".to_string(),
            host: None,
            can_login: Some(false),
            superuser: None,
            create_database: None,
            create_role: None,
            inherit: None,
        };
        let alter_preview = preview_role_with_adapter(&adapter, &alter)
            .await
            .expect("alter preview");
        assert_eq!(
            alter_preview.statements,
            vec!["ALTER ROLE \"deploy\" NOLOGIN"]
        );
        assert!(alter_preview.role_changes.iter().any(|change| {
            change.attribute == "canLogin" && change.before == "Yes" && change.after == "No"
        }));
        let result = execute_role_with_adapter(&mut adapter, alter, true)
            .await
            .expect("alter result");
        assert!(result.verified);
    }

    #[tokio::test]
    async fn read_only_profiles_reject_privilege_changes() {
        let mut adapter = fake(DatabaseEngine::Postgresql, "16.2", true).await;
        let error = execute_role_with_adapter(&mut adapter, postgres_grant(), true)
            .await
            .expect_err("read-only rejection");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert!(error.message.contains("read-only"));
        assert_eq!(adapter.role_execution_count(), 0);
    }

    #[tokio::test]
    async fn capability_checks_block_roles_on_old_mysql() {
        let adapter = fake(DatabaseEngine::Mysql, "5.7.44", false).await;
        let error = adapter
            .list_roles(PageRequest::new(10, 0))
            .await
            .expect_err("roles unsupported");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);

        let request = RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: Some("%".to_string()),
            },
            privileges: vec![Privilege::Select],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: Some("app".to_string()),
                schema: None,
                object: Some("orders".to_string()),
            },
            with_grant_option: false,
        };
        let error = preview_role_with_adapter(&adapter, &request)
            .await
            .expect_err("manage roles unsupported");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);
    }

    #[tokio::test]
    async fn permission_denial_is_reported_without_executing_success() {
        let mut adapter = fake(DatabaseEngine::Postgresql, "16.2", false)
            .await
            .with_role_execution_error(DatabaseError::permission(
                "The database account does not have permission for this operation.",
            ));
        let error = execute_role_with_adapter(&mut adapter, postgres_grant(), true)
            .await
            .expect_err("permission denied");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert!(error.message.contains("permission"));
    }

    #[tokio::test]
    async fn skipped_execution_is_reported_as_unverified() {
        let mut adapter = fake(DatabaseEngine::Postgresql, "16.2", false)
            .await
            .with_skipped_role_execution();
        let result = execute_role_with_adapter(&mut adapter, postgres_grant(), true)
            .await
            .expect("execution result");
        assert!(!result.verified);
        assert!(result.verification_error.is_some());
    }

    #[tokio::test]
    async fn previews_and_results_never_contain_password_material() {
        let adapter = fake(DatabaseEngine::Postgresql, "16.2", false).await;
        let preview = preview_role_with_adapter(&adapter, &postgres_grant())
            .await
            .expect("preview");
        let json = serde_json::to_string(&preview).expect("preview json");
        assert!(!json.contains("password"));
        assert!(!json.contains("authentication"));
        assert!(!json.contains("rolpassword"));
        assert!(!json.contains("pg_authid"));
    }

    fn request_grant() -> RoleActionRequest {
        RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Select, Privilege::Update],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: None,
                schema: Some("public".to_string()),
                object: Some("orders".to_string()),
            },
            with_grant_option: true,
        }
    }

    #[test]
    fn role_refs_validate_names_and_engine_hosts() {
        let postgres = RoleRefRequest {
            name: "reporting".to_string(),
            host: None,
        }
        .validate(DatabaseEngine::Postgresql)
        .expect("postgres role");
        assert_eq!(postgres.label(), "reporting");

        let mysql = RoleRefRequest {
            name: "reporting".to_string(),
            host: None,
        }
        .validate(DatabaseEngine::Mysql)
        .expect("mysql account");
        assert_eq!(mysql.label(), "reporting@%");

        assert!(
            RoleRefRequest {
                name: "bad name".to_string(),
                host: None,
            }
            .validate(DatabaseEngine::Postgresql)
            .is_err()
        );
        assert!(
            RoleRefRequest {
                name: "reporting".to_string(),
                host: Some("evil'host".to_string()),
            }
            .validate(DatabaseEngine::Mysql)
            .is_err()
        );
        assert!(
            RoleRefRequest {
                name: "reporting".to_string(),
                host: Some("localhost".to_string()),
            }
            .validate(DatabaseEngine::Postgresql)
            .is_err()
        );
    }

    #[test]
    fn grant_requests_validate_targets_and_privileges_per_engine() {
        let postgres = request_grant()
            .validate(DatabaseEngine::Postgresql)
            .expect("postgres grant");
        assert_eq!(postgres.kind(), RoleActionKind::GrantPrivileges);
        assert!(postgres.requires_confirmation());

        let mysql = RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Select, Privilege::Update],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: Some("app".to_string()),
                schema: None,
                object: Some("orders".to_string()),
            },
            with_grant_option: true,
        }
        .validate(DatabaseEngine::Mysql)
        .expect("mysql table grant");
        assert_eq!(mysql.primary_role().label(), "reporting@%");

        let schema_target = RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Usage],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Schema,
                database: None,
                schema: Some("public".to_string()),
                object: None,
            },
            with_grant_option: false,
        };
        assert!(schema_target.validate(DatabaseEngine::Mysql).is_err());
        assert!(schema_target.validate(DatabaseEngine::Postgresql).is_ok());

        let unsupported = RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Connect],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: None,
                schema: Some("public".to_string()),
                object: Some("orders".to_string()),
            },
            with_grant_option: false,
        };
        assert!(unsupported.validate(DatabaseEngine::Postgresql).is_err());
    }

    #[test]
    fn role_request_payloads_use_camel_case_tags_and_fields() {
        let create: RoleActionRequest = serde_json::from_value(serde_json::json!({
            "kind": "create_role",
            "name": "deploy",
            "host": "%",
            "roleKind": "user",
            "canLogin": false
        }))
        .expect("create role payload");
        assert_eq!(create.kind(), RoleActionKind::CreateRole);
        assert_eq!(create.role_label(), "deploy@%");

        let grant: RoleActionRequest = serde_json::from_value(serde_json::json!({
            "kind": "grant_privileges",
            "grantee": { "name": "reporting", "host": "%" },
            "privileges": ["select", "update"],
            "target": { "kind": "table", "database": "app", "object": "orders" },
            "withGrantOption": true
        }))
        .expect("grant payload");
        let validated = grant
            .validate(DatabaseEngine::Mysql)
            .expect("valid grant payload");
        match validated {
            ValidatedRoleAction::GrantPrivileges(change) => {
                assert_eq!(
                    change.privileges,
                    vec![Privilege::Select, Privilege::Update]
                );
                assert!(change.grant_option);
            }
            _ => panic!("expected a grant"),
        }
    }

    #[test]
    fn mysql_users_are_created_locked_without_password_support() {
        let request = RoleActionRequest::CreateRole {
            name: "deploy".to_string(),
            host: None,
            role_kind: RoleKind::User,
            can_login: Some(true),
            superuser: None,
            create_database: None,
            create_role: None,
            inherit: None,
        };
        let error = request
            .validate(DatabaseEngine::Mysql)
            .expect_err("passwordless login must be rejected");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);
        assert!(error.message.contains("created locked"));
    }

    #[test]
    fn mysql_rejects_postgres_role_attributes() {
        let request = RoleActionRequest::CreateRole {
            name: "reporting".to_string(),
            host: None,
            role_kind: RoleKind::User,
            can_login: Some(true),
            superuser: Some(true),
            create_database: None,
            create_role: None,
            inherit: None,
        };
        assert!(request.validate(DatabaseEngine::Mysql).is_err());
        assert!(request.validate(DatabaseEngine::Postgresql).is_ok());
    }

    #[test]
    fn effective_privileges_collapse_duplicate_sources_and_grants() {
        let table = PrivilegeTarget {
            kind: PrivilegeTargetKind::Table,
            database: None,
            schema: Some(Identifier::new("public").expect("schema")),
            object: Some(Identifier::new("orders").expect("table")),
        };
        let grants = vec![
            EffectivePrivilege {
                privilege: "SELECT".to_string(),
                target: table.clone(),
                grantable: false,
                sources: vec!["direct".to_string()],
            },
            EffectivePrivilege {
                privilege: "SELECT".to_string(),
                target: table.clone(),
                grantable: true,
                sources: vec!["reporting_reader".to_string()],
            },
        ];
        let canonical = canonicalize_effective(grants);
        assert_eq!(canonical.len(), 1);
        assert!(canonical[0].grantable);
        assert_eq!(
            canonical[0].sources,
            vec!["direct".to_string(), "reporting_reader".to_string()]
        );
    }

    #[test]
    fn role_summary_serialization_contains_no_password_fields() {
        let summary = RoleSummary {
            name: "reporting".to_string(),
            host: None,
            kind: RoleKind::Role,
            can_login: MetadataValue::available(false),
            superuser: MetadataValue::available(false),
            create_database: MetadataValue::available(false),
            create_role: MetadataValue::available(false),
            inherit: MetadataValue::available(true),
        };
        let json = serde_json::to_string(&summary).expect("summary json");
        assert!(!json.contains("password"));
        assert!(!json.contains("authentication"));
        assert!(!json.contains("rolpassword"));
        assert!(json.contains("\"kind\":\"role\""));
    }
}
