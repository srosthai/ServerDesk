//! Typed database profile contract.
//!
//! Profiles contain only non-secret metadata. The password lives in the OS
//! credential store behind an opaque reference, and neither the password nor
//! the reference is serialized to React. Commands use this module for
//! validation, CRUD, credential coordination, and connection snapshots so the
//! Tauri boundary stays thin.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    core::{
        AppError, credential_index::CredentialIndex, credentials::CredentialStore,
        database_credentials::DatabaseCredentialService, ssh::TunnelSnapshot,
    },
    database::{CapabilitySnapshot, DatabaseConnectionInfo, DatabaseVersionInfo},
    storage::{Database, DatabaseConnection, DatabaseEngine, NewDatabaseConnection, StorageError},
};

pub const MAX_DATABASE_PROFILE_NAME_LENGTH: usize = 80;
pub const MAX_DATABASE_HOST_LENGTH: usize = 253;
pub const MAX_DATABASE_SOCKET_LENGTH: usize = 4096;
pub const MAX_DATABASE_NAME_LENGTH: usize = 128;
pub const MAX_DATABASE_USERNAME_LENGTH: usize = 128;
pub const MAX_DATABASE_SERVER_ID_LENGTH: usize = 128;

/// Supported TLS modes, normalized to one spelling per mode.
///
/// The adapters accept the canonical strings produced by [`Self::as_str`].
/// `None` keeps the engine default (`preferred` for MySQL, `prefer` for
/// PostgreSQL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DatabaseSslMode {
    Disabled,
    Preferred,
    Required,
    VerifyCa,
    VerifyIdentity,
}

impl DatabaseSslMode {
    pub const ALL: [Self; 5] = [
        Self::Disabled,
        Self::Preferred,
        Self::Required,
        Self::VerifyCa,
        Self::VerifyIdentity,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Preferred => "preferred",
            Self::Required => "required",
            Self::VerifyCa => "verify-ca",
            Self::VerifyIdentity => "verify-identity",
        }
    }

    /// Parses canonical and the aliases already used by saved profiles.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "disabled" | "disable" | "off" | "none" => Some(Self::Disabled),
            "preferred" | "prefer" => Some(Self::Preferred),
            "required" | "require" => Some(Self::Required),
            "verify-ca" | "verify_ca" => Some(Self::VerifyCa),
            "verify-identity" | "verify_identity" => Some(Self::VerifyIdentity),
            _ => None,
        }
    }
}

/// Non-secret profile returned to React.
///
/// `has_credential` replaces the opaque credential reference. The reference
/// and the secret itself never cross the Tauri boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseProfile {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub engine: DatabaseEngine,
    pub host: String,
    pub port: u16,
    pub socket: Option<String>,
    pub database_name: Option<String>,
    pub username: String,
    pub has_credential: bool,
    pub read_only: bool,
    pub ssl_mode: Option<DatabaseSslMode>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_used_at: Option<i64>,
}

impl From<DatabaseConnection> for DatabaseProfile {
    fn from(profile: DatabaseConnection) -> Self {
        Self {
            id: profile.id,
            server_id: profile.server_id,
            name: profile.name,
            engine: profile.engine,
            host: profile.host,
            port: profile.port,
            socket: profile.socket,
            database_name: profile.database_name,
            username: profile.username,
            has_credential: profile.credential_ref.is_some(),
            read_only: profile.read_only,
            ssl_mode: profile.ssl_mode.as_deref().and_then(DatabaseSslMode::parse),
            created_at: profile.created_at,
            updated_at: profile.updated_at,
            last_used_at: profile.last_used_at,
        }
    }
}

/// Create/update payload. It is deliberately `Deserialize` only so a password
/// can never be serialized by an accidental response type.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseProfileInput {
    pub server_id: String,
    pub name: String,
    pub engine: DatabaseEngine,
    pub host: String,
    pub port: u16,
    pub socket: Option<String>,
    pub database_name: Option<String>,
    pub username: String,
    pub read_only: bool,
    pub ssl_mode: Option<DatabaseSslMode>,
    /// New password. `None` keeps the saved credential on update and is
    /// allowed on create for metadata-only profiles.
    #[serde(default)]
    pub password: Option<String>,
}

impl fmt::Debug for DatabaseProfileInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatabaseProfileInput")
            .field("server_id", &self.server_id)
            .field("name", &self.name)
            .field("engine", &self.engine)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("socket", &self.socket)
            .field("database_name", &self.database_name)
            .field("username", &self.username)
            .field("read_only", &self.read_only)
            .field("ssl_mode", &self.ssl_mode)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl DatabaseProfileInput {
    /// Validates and normalizes every user-controlled field.
    pub fn validate(&self) -> Result<ValidatedDatabaseProfile, AppError> {
        let server_id = self.server_id.trim().to_string();
        if server_id.is_empty() || server_id.len() > MAX_DATABASE_SERVER_ID_LENGTH {
            return Err(AppError::validation("Select a valid server profile."));
        }

        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::validation("Database profile name is required."));
        }
        if name.len() > MAX_DATABASE_PROFILE_NAME_LENGTH {
            return Err(AppError::validation(format!(
                "Database profile name must be {MAX_DATABASE_PROFILE_NAME_LENGTH} characters or fewer."
            )));
        }
        if name.chars().any(char::is_control) {
            return Err(AppError::validation(
                "The database profile name contains invalid characters.",
            ));
        }

        let host = self.host.trim().to_string();
        if host.is_empty() {
            return Err(AppError::validation("Database host is required."));
        }
        if host.len() > MAX_DATABASE_HOST_LENGTH || host.chars().any(invalid_host_character) {
            return Err(AppError::validation(
                "Enter a valid database hostname or IP address.",
            ));
        }
        if self.port == 0 {
            return Err(AppError::validation(
                "Database port must be between 1 and 65535.",
            ));
        }

        let socket = normalize_optional(self.socket.clone(), MAX_DATABASE_SOCKET_LENGTH, "Socket")?;
        if let Some(socket) = socket.as_deref() {
            if !socket.starts_with('/') {
                return Err(AppError::validation(
                    "The database socket must be an absolute path.",
                ));
            }
            if socket.chars().any(char::is_control) {
                return Err(AppError::validation(
                    "The database socket path contains invalid characters.",
                ));
            }
        }

        let database_name = normalize_optional(
            self.database_name.clone(),
            MAX_DATABASE_NAME_LENGTH,
            "Database name",
        )?;
        if database_name
            .as_deref()
            .is_some_and(|name| name.chars().any(char::is_control))
        {
            return Err(AppError::validation(
                "The database name contains invalid characters.",
            ));
        }

        let username = self.username.trim().to_string();
        if username.is_empty()
            || username.len() > MAX_DATABASE_USERNAME_LENGTH
            || username
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(AppError::validation("Enter a valid database username."));
        }

        if self.password.as_deref() == Some("") {
            return Err(AppError::validation(
                "The database password cannot be empty.",
            ));
        }

        Ok(ValidatedDatabaseProfile {
            server_id,
            name,
            engine: self.engine,
            host,
            port: self.port,
            socket,
            database_name,
            username,
            read_only: self.read_only,
            ssl_mode: self.ssl_mode,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDatabaseProfile {
    pub server_id: String,
    pub name: String,
    pub engine: DatabaseEngine,
    pub host: String,
    pub port: u16,
    pub socket: Option<String>,
    pub database_name: Option<String>,
    pub username: String,
    pub read_only: bool,
    pub ssl_mode: Option<DatabaseSslMode>,
}

impl ValidatedDatabaseProfile {
    pub fn into_new(self, credential_ref: Option<String>) -> NewDatabaseConnection {
        NewDatabaseConnection {
            server_id: self.server_id,
            name: self.name,
            engine: self.engine,
            host: self.host,
            port: self.port,
            socket: self.socket,
            database_name: self.database_name,
            username: self.username,
            credential_ref,
            read_only: self.read_only,
            ssl_mode: self.ssl_mode.map(|mode| mode.as_str().to_string()),
        }
    }
}

/// CRUD and credential coordination for saved database profiles.
pub struct DatabaseProfileService<'a> {
    database: &'a Database,
    credentials: &'a dyn CredentialStore,
    index: &'a CredentialIndex,
}

impl<'a> DatabaseProfileService<'a> {
    pub fn new(
        database: &'a Database,
        credentials: &'a dyn CredentialStore,
        index: &'a CredentialIndex,
    ) -> Self {
        Self {
            database,
            credentials,
            index,
        }
    }

    pub fn list(&self, server_id: &str) -> Result<Vec<DatabaseProfile>, AppError> {
        Ok(self
            .database
            .list_database_connections(server_id)?
            .into_iter()
            .map(DatabaseProfile::from)
            .collect())
    }

    pub fn get(&self, id: &str) -> Result<Option<DatabaseProfile>, AppError> {
        Ok(self
            .database
            .get_database_connection(id)?
            .map(DatabaseProfile::from))
    }

    pub fn create(&self, input: DatabaseProfileInput) -> Result<DatabaseProfile, AppError> {
        let validated = input.validate()?;
        self.ensure_server_exists(&validated.server_id)?;
        self.ensure_unique_name(&validated.server_id, &validated.name, None)?;

        let credential_service = DatabaseCredentialService::new(self.credentials, self.index);
        let credential_ref = match input.password.as_deref() {
            Some(password) => Some(credential_service.create(password)?),
            None => None,
        };

        match self
            .database
            .create_database_connection(&validated.into_new(credential_ref.clone()))
        {
            Ok(profile) => Ok(profile.into()),
            Err(error) => {
                if let Some(reference) = credential_ref {
                    let _ = credential_service.delete(&reference);
                }
                Err(map_storage_error(error))
            }
        }
    }

    pub fn update(
        &self,
        id: &str,
        input: DatabaseProfileInput,
    ) -> Result<DatabaseProfile, AppError> {
        let existing = self
            .database
            .get_database_connection(id)?
            .ok_or_else(|| AppError::not_found("Database profile not found."))?;
        let validated = input.validate()?;
        self.ensure_server_exists(&validated.server_id)?;
        self.ensure_unique_name(&validated.server_id, &validated.name, Some(id))?;

        let credential_service = DatabaseCredentialService::new(self.credentials, self.index);
        let mut credential_ref = existing.credential_ref.clone();
        let mut created_reference = None;
        let mut previous_secret = None;

        if let Some(password) = input.password.as_deref() {
            match credential_ref.as_deref() {
                Some(reference) => {
                    // Keep the old secret in memory only long enough to undo a
                    // failed metadata update, then let it drop. A missing
                    // secret is healed by `replace` and simply has no value to
                    // restore.
                    previous_secret = credential_service.read(reference).ok();
                    credential_service.replace(reference, password)?;
                }
                None => {
                    let reference = credential_service.create(password)?;
                    credential_ref = Some(reference.clone());
                    created_reference = Some(reference);
                }
            }
        }

        match self
            .database
            .update_database_connection(id, &validated.into_new(credential_ref))
        {
            Ok(profile) => Ok(profile.into()),
            Err(error) => {
                if let Some(reference) = created_reference.as_deref() {
                    let _ = credential_service.delete(reference);
                }
                if let (Some(previous_secret), Some(reference)) = (
                    previous_secret.as_deref(),
                    existing.credential_ref.as_deref(),
                ) {
                    let _ = credential_service.replace(reference, previous_secret);
                }
                Err(map_storage_error(error))
            }
        }
    }

    /// Deletes the credential before the metadata row so a failure cannot
    /// leave an orphaned secret.
    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        if self.database.get_database_connection(id)?.is_none() {
            return Err(AppError::not_found("Database profile not found."));
        }

        DatabaseCredentialService::new(self.credentials, self.index)
            .delete_profile(self.database, id)
    }

    fn ensure_server_exists(&self, server_id: &str) -> Result<(), AppError> {
        if self.database.get_server(server_id)?.is_none() {
            return Err(AppError::not_found("Server profile not found."));
        }
        Ok(())
    }

    fn ensure_unique_name(
        &self,
        server_id: &str,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<(), AppError> {
        if self
            .database
            .database_connection_name_exists(server_id, name, exclude_id)?
        {
            return Err(AppError::conflict(
                "A database profile with this name already exists for the server.",
            ));
        }
        Ok(())
    }
}

fn map_storage_error(error: StorageError) -> AppError {
    match error {
        StorageError::Conflict(_) => {
            AppError::conflict("A database profile with this name already exists for the server.")
        }
        StorageError::NotFound => AppError::not_found("Database profile not found."),
        StorageError::InvalidValue(message) => {
            AppError::validation("The database profile contains an invalid value.")
                .with_details(message)
        }
        other => other.into(),
    }
}

fn normalize_optional(
    value: Option<String>,
    max_length: usize,
    label: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().to_string();

    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_length {
        return Err(AppError::validation(format!(
            "{label} must be {max_length} characters or fewer."
        )));
    }

    Ok(Some(value))
}

fn invalid_host_character(character: char) -> bool {
    character.is_control()
        || character.is_whitespace()
        || !matches!(character, 'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' | ':' | '%')
}

/// Observable state of one saved profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DatabaseConnectionState {
    Disconnected,
    Connecting,
    Reconnecting,
    Connected,
    Error,
}

impl DatabaseConnectionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Reconnecting => "reconnecting",
            Self::Connected => "connected",
            Self::Error => "error",
        }
    }
}

/// Typed connection state sent to React.
///
/// It includes the profile engine, the live server version, capabilities,
/// read-only mode, the tunnel snapshot, and a structured error. It never
/// includes credentials, SQL, driver text, or the opaque credential reference.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConnectionSnapshot {
    pub profile_id: String,
    pub server_id: String,
    pub name: String,
    pub engine: DatabaseEngine,
    pub state: DatabaseConnectionState,
    pub read_only: bool,
    pub database_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<DatabaseVersionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<CapabilitySnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<TunnelSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
    pub changed_at: i64,
}

impl DatabaseConnectionSnapshot {
    pub fn disconnected(profile: &DatabaseConnection) -> Self {
        Self::new(
            profile,
            DatabaseConnectionState::Disconnected,
            None,
            None,
            None,
        )
    }

    pub fn connected(
        profile: &DatabaseConnection,
        info: DatabaseConnectionInfo,
        tunnel: Option<TunnelSnapshot>,
    ) -> Self {
        Self::new(
            profile,
            DatabaseConnectionState::Connected,
            Some(info),
            tunnel,
            None,
        )
    }

    pub fn failed(
        profile: &DatabaseConnection,
        state: DatabaseConnectionState,
        error: AppError,
    ) -> Self {
        Self::new(profile, state, None, None, Some(error))
    }

    fn new(
        profile: &DatabaseConnection,
        state: DatabaseConnectionState,
        info: Option<DatabaseConnectionInfo>,
        tunnel: Option<TunnelSnapshot>,
        error: Option<AppError>,
    ) -> Self {
        let version = info.as_ref().map(|info| info.version.clone());
        let capabilities = info.as_ref().map(|info| info.capabilities.clone());
        let read_only = info
            .as_ref()
            .map(|info| info.read_only)
            .unwrap_or(profile.read_only);
        let database_name = info
            .as_ref()
            .and_then(|info| info.database.clone())
            .or_else(|| profile.database_name.clone());

        Self {
            profile_id: profile.id.clone(),
            server_id: profile.server_id.clone(),
            name: profile.name.clone(),
            engine: profile.engine,
            state,
            read_only,
            database_name,
            version,
            capabilities,
            tunnel,
            error,
            changed_at: now_millis(),
        }
    }

    pub fn with_tunnel(mut self, tunnel: Option<TunnelSnapshot>) -> Self {
        self.tunnel = tunnel;
        self
    }
}

/// Result of a transient connection test. The adapter and tunnel are closed
/// before the command returns, so this is metadata only.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseTestResult {
    pub profile_id: String,
    pub server_id: String,
    pub engine: DatabaseEngine,
    pub version: DatabaseVersionInfo,
    pub database_name: Option<String>,
    pub read_only: bool,
    pub capabilities: CapabilitySnapshot,
    pub latency_ms: u64,
    pub tested_at: i64,
}

impl DatabaseTestResult {
    pub fn new(
        profile: &DatabaseConnection,
        info: DatabaseConnectionInfo,
        latency_ms: u64,
    ) -> Self {
        Self {
            profile_id: profile.id.clone(),
            server_id: profile.server_id.clone(),
            engine: info.engine,
            version: info.version,
            database_name: info.database,
            read_only: info.read_only,
            capabilities: info.capabilities,
            latency_ms,
            tested_at: now_millis(),
        }
    }
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ErrorCode;

    fn input() -> DatabaseProfileInput {
        DatabaseProfileInput {
            server_id: " server-1 ".to_string(),
            name: " Primary ".to_string(),
            engine: DatabaseEngine::Postgresql,
            host: " 127.0.0.1 ".to_string(),
            port: 5432,
            socket: Some(" /var/run/postgresql/.s.PGSQL.5432 ".to_string()),
            database_name: Some(" app ".to_string()),
            username: " app ".to_string(),
            read_only: true,
            ssl_mode: Some(DatabaseSslMode::VerifyCa),
            password: Some("fixture-password".to_string()),
        }
    }

    #[test]
    fn normalizes_valid_profile_fields() {
        let validated = input().validate().expect("valid profile");

        assert_eq!(validated.server_id, "server-1");
        assert_eq!(validated.name, "Primary");
        assert_eq!(validated.host, "127.0.0.1");
        assert_eq!(
            validated.socket.as_deref(),
            Some("/var/run/postgresql/.s.PGSQL.5432")
        );
        assert_eq!(validated.database_name.as_deref(), Some("app"));
        assert_eq!(validated.username, "app");
        assert!(validated.read_only);
        assert_eq!(validated.ssl_mode, Some(DatabaseSslMode::VerifyCa));

        let new = validated.into_new(Some("serverdesk:db:test".to_string()));
        assert_eq!(new.ssl_mode.as_deref(), Some("verify-ca"));
        assert_eq!(new.credential_ref.as_deref(), Some("serverdesk:db:test"));
    }

    #[test]
    fn rejects_invalid_hosts_ports_sockets_and_names() {
        let cases = [
            ("host", " ".to_string()),
            ("host", "db internal".to_string()),
            ("host", "db;rm -rf".to_string()),
            ("host", "a".repeat(MAX_DATABASE_HOST_LENGTH + 1)),
            ("port", String::new()),
            ("socket", "relative.sock".to_string()),
            ("socket", "/var/run/db\n.sock".to_string()),
            ("name", " ".to_string()),
            ("name", "x".repeat(MAX_DATABASE_PROFILE_NAME_LENGTH + 1)),
            ("username", "app user".to_string()),
            ("username", "app\nuser".to_string()),
            ("database_name", "app\0".to_string()),
        ];

        for (field, value) in cases {
            let mut candidate = input();
            match field {
                "host" => candidate.host = value.clone(),
                "port" => candidate.port = 0,
                "socket" => candidate.socket = Some(value.clone()),
                "name" => candidate.name = value.clone(),
                "username" => candidate.username = value.clone(),
                "database_name" => candidate.database_name = Some(value.clone()),
                _ => unreachable!(),
            }
            assert!(
                candidate.validate().is_err(),
                "{field} value {value:?} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_empty_password_before_any_credential_write() {
        let mut candidate = input();
        candidate.password = Some(String::new());

        assert_eq!(
            candidate.validate().expect_err("empty password").code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn parses_legacy_and_canonical_ssl_modes() {
        for (raw, expected) in [
            ("require", DatabaseSslMode::Required),
            ("required", DatabaseSslMode::Required),
            ("prefer", DatabaseSslMode::Preferred),
            ("verify_ca", DatabaseSslMode::VerifyCa),
            ("verify-identity", DatabaseSslMode::VerifyIdentity),
            ("off", DatabaseSslMode::Disabled),
        ] {
            assert_eq!(DatabaseSslMode::parse(raw), Some(expected));
        }
        assert_eq!(DatabaseSslMode::parse("sometimes"), None);
    }

    #[test]
    fn serialized_snapshot_uses_camel_case_and_contains_no_credential_reference() {
        let profile = DatabaseConnection {
            id: "profile-1".to_string(),
            server_id: "server-1".to_string(),
            name: "Primary".to_string(),
            engine: DatabaseEngine::Mysql,
            host: "127.0.0.1".to_string(),
            port: 3306,
            socket: None,
            database_name: Some("app".to_string()),
            username: "app".to_string(),
            credential_ref: Some("serverdesk:db:secret-reference".to_string()),
            read_only: true,
            ssl_mode: Some("required".to_string()),
            created_at: 1,
            updated_at: 2,
            last_used_at: None,
        };

        let snapshot = DatabaseConnectionSnapshot::disconnected(&profile);
        let json = serde_json::to_value(snapshot).expect("snapshot json");

        assert_eq!(json["profileId"], "profile-1");
        assert_eq!(json["serverId"], "server-1");
        assert_eq!(json["readOnly"], true);
        assert_eq!(json["databaseName"], "app");
        assert_eq!(json["state"], "disconnected");
        assert!(json.get("version").is_none());
        assert!(json.get("capabilities").is_none());
        assert!(json.get("tunnel").is_none());
        assert!(!json.to_string().contains("secret-reference"));
        assert!(!json.to_string().contains("credentialRef"));
    }
}
