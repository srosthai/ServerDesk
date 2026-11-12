//! Typed database lifecycle actions.
//!
//! Create, rename, drop, and set-active requests are validated once in this
//! module. Engine modules generate the exact DDL preview and execute it inside
//! the adapter; the shared service re-reads metadata afterwards so a command
//! never reports success from the click alone.
//!
//! PostgresSQL and MySQL do not share the same lifecycle features. Capability
//! errors are typed, and an unsupported action never reaches engine SQL.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{core::AppError, storage::DatabaseEngine};

use super::{
    capabilities::DatabaseAction as CapabilityAction,
    dialect::Identifier,
    errors::DatabaseError,
    service::DatabaseConnectionRegistry,
    traits::{DatabaseAdapter, DatabaseConnectionInfo, DatabaseSummary, MetadataValue},
};

/// Maximum length accepted for encoding, collation, and similar option values.
pub const MAX_DATABASE_OPTION_LENGTH: usize = 128;

/// The lifecycle action a request represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseActionKind {
    Create,
    Rename,
    Drop,
    SetActive,
}

impl DatabaseActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Rename => "rename",
            Self::Drop => "drop",
            Self::SetActive => "set_active",
        }
    }

    pub const fn capability(self) -> CapabilityAction {
        match self {
            Self::Create => CapabilityAction::CreateDatabase,
            Self::Rename => CapabilityAction::RenameDatabase,
            Self::Drop => CapabilityAction::DropDatabase,
            Self::SetActive => CapabilityAction::SetActiveDatabase,
        }
    }
}

/// Untrusted request payload received at the command boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DatabaseActionRequest {
    Create {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encoding: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        collation: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
    },
    Rename {
        name: String,
        #[serde(rename = "newName")]
        new_name: String,
    },
    Drop {
        name: String,
    },
    SetActive {
        name: String,
    },
}

impl DatabaseActionRequest {
    pub const fn kind(&self) -> DatabaseActionKind {
        match self {
            Self::Create { .. } => DatabaseActionKind::Create,
            Self::Rename { .. } => DatabaseActionKind::Rename,
            Self::Drop { .. } => DatabaseActionKind::Drop,
            Self::SetActive { .. } => DatabaseActionKind::SetActive,
        }
    }

    pub fn database_name(&self) -> &str {
        match self {
            Self::Create { name, .. }
            | Self::Rename { name, .. }
            | Self::Drop { name }
            | Self::SetActive { name } => name,
        }
    }

    pub fn target_database(&self) -> Option<&str> {
        match self {
            Self::Rename { new_name, .. } => Some(new_name),
            _ => None,
        }
    }

    pub const fn requires_confirmation(&self) -> bool {
        matches!(self, Self::Drop { .. })
    }

    /// Validates every field before an adapter sees engine SQL.
    pub fn validate(&self) -> Result<ValidatedDatabaseAction, DatabaseError> {
        match self {
            Self::Create {
                name,
                encoding,
                collation,
                owner,
                template,
            } => Ok(ValidatedDatabaseAction::Create(ValidatedCreateDatabase {
                name: validate_identifier("database name", name)?,
                encoding: validate_option("encoding", encoding.clone())?,
                collation: validate_option("collation", collation.clone())?,
                owner: owner
                    .as_deref()
                    .map(|value| validate_identifier("owner", value))
                    .transpose()?,
                template: template
                    .as_deref()
                    .map(|value| validate_identifier("template", value))
                    .transpose()?,
            })),
            Self::Rename { name, new_name } => {
                let name = validate_identifier("database name", name)?;
                let new_name = validate_identifier("new database name", new_name)?;
                if name == new_name {
                    return Err(DatabaseError::validation(
                        "The new database name must differ from the current name.",
                    ));
                }
                Ok(ValidatedDatabaseAction::Rename { name, new_name })
            }
            Self::Drop { name } => Ok(ValidatedDatabaseAction::Drop {
                name: validate_identifier("database name", name)?,
            }),
            Self::SetActive { name } => Ok(ValidatedDatabaseAction::SetActive {
                name: validate_identifier("database name", name)?,
            }),
        }
    }
}

/// A validated lifecycle action. Engine modules can rely on every identifier
/// and option value already being safe to quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedDatabaseAction {
    Create(ValidatedCreateDatabase),
    Rename {
        name: Identifier,
        new_name: Identifier,
    },
    Drop {
        name: Identifier,
    },
    SetActive {
        name: Identifier,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCreateDatabase {
    pub name: Identifier,
    pub encoding: Option<String>,
    pub collation: Option<String>,
    pub owner: Option<Identifier>,
    pub template: Option<Identifier>,
}

impl ValidatedDatabaseAction {
    pub const fn kind(&self) -> DatabaseActionKind {
        match self {
            Self::Create(_) => DatabaseActionKind::Create,
            Self::Rename { .. } => DatabaseActionKind::Rename,
            Self::Drop { .. } => DatabaseActionKind::Drop,
            Self::SetActive { .. } => DatabaseActionKind::SetActive,
        }
    }

    pub fn database_name(&self) -> &str {
        match self {
            Self::Create(options) => options.name.as_str(),
            Self::Rename { name, .. } | Self::Drop { name } | Self::SetActive { name } => {
                name.as_str()
            }
        }
    }

    pub fn target_database_name(&self) -> Option<&str> {
        match self {
            Self::Rename { new_name, .. } => Some(new_name.as_str()),
            _ => None,
        }
    }

    pub const fn capability(&self) -> CapabilityAction {
        self.kind().capability()
    }

    pub const fn requires_confirmation(&self) -> bool {
        matches!(self, Self::Drop { .. })
    }

    pub const fn mutates_structure(&self) -> bool {
        matches!(
            self,
            Self::Create(_) | Self::Rename { .. } | Self::Drop { .. }
        )
    }
}

/// Generated DDL and safe warnings for one prospective action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseDdlPreview {
    pub action: DatabaseActionKind,
    pub engine: DatabaseEngine,
    pub database: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_database: Option<String>,
    pub statements: Vec<String>,
    pub ddl: String,
    pub warnings: Vec<String>,
    pub destructive: bool,
    pub requires_confirmation: bool,
}

impl DatabaseDdlPreview {
    pub fn new(
        engine: DatabaseEngine,
        action: &ValidatedDatabaseAction,
        statements: Vec<String>,
        warnings: Vec<String>,
    ) -> Self {
        let ddl = statements.join(";\n");
        Self {
            action: action.kind(),
            engine,
            database: action.database_name().to_string(),
            target_database: action.target_database_name().map(str::to_string),
            statements,
            ddl,
            warnings,
            destructive: matches!(action, ValidatedDatabaseAction::Drop { .. }),
            requires_confirmation: action.requires_confirmation(),
        }
    }
}

/// Verified outcome of one lifecycle action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseActionResult {
    pub action: DatabaseActionKind,
    pub engine: DatabaseEngine,
    pub database: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_name: Option<String>,
    pub statements: Vec<String>,
    pub verified: bool,
    pub verification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<DatabaseSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_database: Option<String>,
    pub duration_ms: u64,
}

/// Stateless lifecycle service used by the database action commands.
pub struct DatabaseActionService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseActionService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn preview(
        &self,
        profile_id: &str,
        request: DatabaseActionRequest,
    ) -> Result<DatabaseDdlPreview, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        preview_with_adapter(adapter.as_ref(), &request)
            .await
            .map_err(AppError::from)
    }

    pub async fn execute(
        &self,
        profile_id: &str,
        request: DatabaseActionRequest,
        confirmed: bool,
    ) -> Result<DatabaseActionResult, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        let (result, connection_info) =
            execute_with_adapter_detailed(adapter.as_mut(), request, confirmed)
                .await
                .map_err(AppError::from)?;

        if let Some(info) = connection_info {
            self.connections
                .update_connection_info(profile_id, info)
                .await;
        }

        Ok(result)
    }
}

/// Generates a preview without executing anything.
pub async fn preview_with_adapter(
    adapter: &dyn DatabaseAdapter,
    request: &DatabaseActionRequest,
) -> Result<DatabaseDdlPreview, DatabaseError> {
    let validated = request.validate()?;
    adapter.preview_database_action(&validated).await
}

/// Executes one validated lifecycle action and re-reads metadata.
pub async fn execute_with_adapter(
    adapter: &mut dyn DatabaseAdapter,
    request: DatabaseActionRequest,
    confirmed: bool,
) -> Result<DatabaseActionResult, DatabaseError> {
    let (result, _) = execute_with_adapter_detailed(adapter, request, confirmed).await?;
    Ok(result)
}

async fn execute_with_adapter_detailed(
    adapter: &mut dyn DatabaseAdapter,
    request: DatabaseActionRequest,
    confirmed: bool,
) -> Result<(DatabaseActionResult, Option<DatabaseConnectionInfo>), DatabaseError> {
    let started = Instant::now();
    let validated = request.validate()?;
    let preview = adapter.preview_database_action(&validated).await?;

    if validated.requires_confirmation() && !confirmed {
        return Err(DatabaseError::cancelled(format!(
            "The {} database action for {} was cancelled because it was not confirmed.",
            validated.kind().as_str(),
            validated.database_name()
        ))
        .with_details(
            "No statement was executed. Review the server, engine, database, and exact action, \
             then confirm it.",
        ));
    }

    if validated.mutates_structure() && adapter.is_read_only() {
        return Err(DatabaseError::permission(
            "The database connection profile is read-only; create, rename, and drop require a \
             writable connection.",
        ));
    }

    let connection_info = adapter.execute_database_action(&validated).await?;
    let verification = verify_database_action(adapter, &validated, connection_info.as_ref()).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let result = DatabaseActionResult {
        action: validated.kind(),
        engine: adapter.engine(),
        database: validated.database_name().to_string(),
        previous_name: validated
            .target_database_name()
            .map(|_| validated.database_name().to_string()),
        statements: preview.statements,
        verified: verification.verified,
        verification: verification.summary,
        verification_error: verification.error,
        metadata: verification.metadata,
        active_database: verification.active_database,
        duration_ms,
    };

    Ok((result, connection_info))
}

struct Verification {
    verified: bool,
    summary: String,
    error: Option<String>,
    metadata: Option<DatabaseSummary>,
    active_database: Option<String>,
}

async fn verify_database_action(
    adapter: &dyn DatabaseAdapter,
    action: &ValidatedDatabaseAction,
    connection_info: Option<&DatabaseConnectionInfo>,
) -> Verification {
    match action {
        ValidatedDatabaseAction::Create(options) => {
            match adapter.database_summary(&options.name).await {
                Ok(Some(metadata)) => Verification {
                    verified: true,
                    summary: format!(
                        "Metadata re-read confirms database {} exists{}.",
                        options.name,
                        metadata_detail(&metadata)
                    ),
                    error: None,
                    metadata: Some(metadata),
                    active_database: None,
                },
                Ok(None) => Verification {
                    verified: false,
                    summary: format!(
                        "The create action finished, but metadata does not list database {}.",
                        options.name
                    ),
                    error: Some(
                        "The database was not present in metadata after the create action."
                            .to_string(),
                    ),
                    metadata: None,
                    active_database: None,
                },
                Err(error) => verification_error(&options.name, &error),
            }
        }
        ValidatedDatabaseAction::Rename { name, new_name } => {
            let renamed = adapter.database_summary(new_name).await;
            let previous = adapter.database_summary(name).await;

            match (renamed, previous) {
                (Ok(Some(metadata)), Ok(None)) => Verification {
                    verified: true,
                    summary: format!(
                        "Metadata re-read confirms database {} was renamed to {}{}.",
                        name,
                        new_name,
                        metadata_detail(&metadata)
                    ),
                    error: None,
                    metadata: Some(metadata),
                    active_database: None,
                },
                (Ok(None), _) => Verification {
                    verified: false,
                    summary: format!(
                        "The rename action finished, but metadata does not list database {}.",
                        new_name
                    ),
                    error: Some(
                        "The new database name was not present in metadata after the rename action."
                            .to_string(),
                    ),
                    metadata: None,
                    active_database: None,
                },
                (Ok(Some(_)), Ok(Some(_))) => Verification {
                    verified: false,
                    summary: format!(
                        "The rename action finished, but metadata still lists both {} and {}.",
                        name, new_name
                    ),
                    error: Some(
                        "Both the previous and new database names are present after the rename."
                            .to_string(),
                    ),
                    metadata: None,
                    active_database: None,
                },
                (Err(error), _) | (_, Err(error)) => verification_error(new_name, &error),
            }
        }
        ValidatedDatabaseAction::Drop { name } => match adapter.database_summary(name).await {
            Ok(None) => Verification {
                verified: true,
                summary: format!(
                    "Metadata re-read confirms database {} is no longer present.",
                    name
                ),
                error: None,
                metadata: None,
                active_database: None,
            },
            Ok(Some(_)) => Verification {
                verified: false,
                summary: format!(
                    "The drop action finished, but metadata still lists database {}.",
                    name
                ),
                error: Some(
                    "The database was still present in metadata after the drop action.".to_string(),
                ),
                metadata: None,
                active_database: None,
            },
            Err(error) => verification_error(name, &error),
        },
        ValidatedDatabaseAction::SetActive { name } => {
            let active = connection_info
                .and_then(|info| info.database.as_deref())
                .is_some_and(|database| database == name.as_str());
            match (active, adapter.database_summary(name).await) {
                (true, Ok(Some(metadata))) => Verification {
                    verified: true,
                    summary: format!(
                        "Metadata re-read confirms database {} is active for this connection{}.",
                        name,
                        metadata_detail(&metadata)
                    ),
                    error: None,
                    metadata: Some(metadata),
                    active_database: Some(name.as_str().to_string()),
                },
                (false, _) => Verification {
                    verified: false,
                    summary: format!(
                        "The set-active action finished, but the connection did not report {} as \
                         its active database.",
                        name
                    ),
                    error: Some(
                        "The adapter did not report the requested database as active.".to_string(),
                    ),
                    metadata: None,
                    active_database: connection_info.and_then(|info| info.database.clone()),
                },
                (true, Ok(None)) => Verification {
                    verified: false,
                    summary: format!(
                        "The connection reports {} as active, but metadata does not list it.",
                        name
                    ),
                    error: Some(
                        "The active database was not present in metadata after the action."
                            .to_string(),
                    ),
                    metadata: None,
                    active_database: Some(name.as_str().to_string()),
                },
                (true, Err(error)) => verification_error(name, &error),
            }
        }
    }
}

fn verification_error(name: &Identifier, error: &DatabaseError) -> Verification {
    Verification {
        verified: false,
        summary: format!(
            "The database action finished, but metadata for {} could not be re-read.",
            name
        ),
        error: Some(error.message.clone()),
        metadata: None,
        active_database: None,
    }
}

fn metadata_detail(metadata: &DatabaseSummary) -> String {
    let mut details = Vec::new();
    if let MetadataValue::Available(encoding) = &metadata.metadata.encoding {
        details.push(format!("encoding {encoding}"));
    }
    if let MetadataValue::Available(collation) = &metadata.metadata.collation {
        details.push(format!("collation {collation}"));
    }
    if let MetadataValue::Available(owner) = &metadata.metadata.owner {
        details.push(format!("owner {owner}"));
    }

    if details.is_empty() {
        String::new()
    } else {
        format!(" ({})", details.join(", "))
    }
}

fn validate_identifier(label: &str, value: &str) -> Result<Identifier, DatabaseError> {
    if value.trim() != value {
        return Err(DatabaseError::validation(format!(
            "The {label} must not start or end with whitespace."
        )));
    }
    Identifier::new(value).map_err(|error| {
        DatabaseError::validation(format!("The {label} is not valid.")).with_details(error.message)
    })
}

fn validate_option(label: &str, value: Option<String>) -> Result<Option<String>, DatabaseError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(DatabaseError::validation(format!(
            "The database {label} cannot be empty when it is provided."
        )));
    }
    if value.len() > MAX_DATABASE_OPTION_LENGTH
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '@')
        })
    {
        return Err(DatabaseError::validation(format!(
            "The database {label} contains unsupported characters."
        ))
        .with_details(format!(
            "Use letters, digits, '_', '-', '.', or '@' up to {MAX_DATABASE_OPTION_LENGTH} \
             characters."
        )));
    }
    Ok(Some(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_request() -> DatabaseActionRequest {
        DatabaseActionRequest::Create {
            name: "app".to_string(),
            encoding: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_unicode_ci".to_string()),
            owner: Some("deploy".to_string()),
            template: Some("template0".to_string()),
        }
    }

    #[test]
    fn validates_names_and_engine_options_before_sql_generation() {
        let validated = create_request().validate().expect("valid create");
        assert_eq!(validated.kind(), DatabaseActionKind::Create);
        assert_eq!(validated.database_name(), "app");
        assert!(validated.mutates_structure());
        assert!(!validated.requires_confirmation());

        for request in [
            DatabaseActionRequest::Create {
                name: " app".to_string(),
                encoding: None,
                collation: None,
                owner: None,
                template: None,
            },
            DatabaseActionRequest::Create {
                name: "app".to_string(),
                encoding: Some("utf8 mb4".to_string()),
                collation: None,
                owner: None,
                template: None,
            },
            DatabaseActionRequest::Rename {
                name: "app".to_string(),
                new_name: "app".to_string(),
            },
            DatabaseActionRequest::Drop {
                name: "app\nusers".to_string(),
            },
            DatabaseActionRequest::SetActive {
                name: String::new(),
            },
        ] {
            assert!(request.validate().is_err(), "{request:?} must be rejected");
        }
    }

    #[test]
    fn drop_is_the_only_action_that_requires_confirmation() {
        assert!(
            DatabaseActionRequest::Drop {
                name: "app".to_string()
            }
            .requires_confirmation()
        );
        assert!(
            !DatabaseActionRequest::SetActive {
                name: "app".to_string()
            }
            .requires_confirmation()
        );
        assert!(!create_request().requires_confirmation());
    }

    #[test]
    fn request_payloads_use_camel_case_and_never_echo_options_unsafely() {
        let request: DatabaseActionRequest = serde_json::from_value(serde_json::json!({
            "kind": "rename",
            "name": "app",
            "newName": "app_next"
        }))
        .expect("rename payload");
        assert_eq!(
            request,
            DatabaseActionRequest::Rename {
                name: "app".to_string(),
                new_name: "app_next".to_string(),
            }
        );

        let json = serde_json::to_value(DatabaseActionRequest::Create {
            name: "app".to_string(),
            encoding: Some("UTF8".to_string()),
            collation: Some("C.UTF-8".to_string()),
            owner: Some("deploy".to_string()),
            template: Some("template0".to_string()),
        })
        .expect("create json");
        assert_eq!(json["kind"], "create");
        assert_eq!(json["collation"], "C.UTF-8");
        assert_eq!(json["template"], "template0");
    }
}
