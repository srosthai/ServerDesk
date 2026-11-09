use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    Password,
    PrivateKey,
}

impl AuthType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PrivateKey => "private_key",
        }
    }
}

impl FromStr for AuthType {
    type Err = crate::storage::StorageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "password" => Ok(Self::Password),
            "private_key" => Ok(Self::PrivateKey),
            other => Err(crate::storage::StorageError::InvalidValue(format!(
                "unknown auth_type: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub credential_ref: Option<String>,
    pub group_name: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewServer {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub credential_ref: Option<String>,
    pub group_name: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownHost {
    pub id: String,
    pub server_id: String,
    pub key_type: String,
    pub fingerprint: String,
    pub first_seen_at: i64,
    pub verified_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub remote_path: String,
    pub project_type: String,
    pub laravel_version: Option<String>,
    pub php_version: Option<String>,
    pub queue_hint: Option<String>,
    pub scheduler_hint: Option<String>,
    pub detected_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    pub server_id: String,
    pub name: String,
    pub remote_path: String,
    pub project_type: String,
    pub laravel_version: Option<String>,
    pub php_version: Option<String>,
    pub queue_hint: Option<String>,
    pub scheduler_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCommand {
    pub id: String,
    pub name: String,
    pub category: String,
    pub command_template: String,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSavedCommand {
    pub name: String,
    pub category: String,
    pub command_template: String,
    pub requires_confirmation: bool,
}

/// Stable operation categories shown by the Command History page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationCategory {
    Command,
    Service,
    Artisan,
    Sftp,
    Queue,
}

impl OperationCategory {
    pub const ALL: [Self; 5] = [
        Self::Command,
        Self::Service,
        Self::Artisan,
        Self::Sftp,
        Self::Queue,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Service => "service",
            Self::Artisan => "artisan",
            Self::Sftp => "sftp",
            Self::Queue => "queue",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

impl FromStr for OperationCategory {
    type Err = crate::storage::StorageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_db(value).ok_or_else(|| {
            crate::storage::StorageError::InvalidValue(format!(
                "unknown operation category: {value}"
            ))
        })
    }
}

/// Final result of one recorded operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationResult {
    Success,
    Failure,
    Cancelled,
    Unknown,
}

impl OperationResult {
    pub const ALL: [Self; 4] = [Self::Success, Self::Failure, Self::Cancelled, Self::Unknown];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

impl FromStr for OperationResult {
    type Err = crate::storage::StorageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_db(value).ok_or_else(|| {
            crate::storage::StorageError::InvalidValue(format!("unknown operation result: {value}"))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecord {
    pub id: String,
    pub server_id: Option<String>,
    pub project_id: Option<String>,
    pub category: OperationCategory,
    pub operation: String,
    pub safe_summary: String,
    pub path: Option<String>,
    /// Short, sanitized output summary. Full command output is never stored.
    pub output_summary: Option<String>,
    pub result: OperationResult,
    pub exit_code: Option<i32>,
    pub started_at: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewOperationRecord {
    pub server_id: Option<String>,
    pub project_id: Option<String>,
    pub category: OperationCategory,
    pub operation: String,
    pub safe_summary: String,
    pub path: Option<String>,
    pub output_summary: Option<String>,
    pub result: OperationResult,
    pub exit_code: Option<i32>,
    pub started_at: i64,
    pub duration_ms: i64,
}

/// Optional filters for the Command History page. All filters are combined
/// with AND, and timestamps are Unix seconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationHistoryFilter {
    pub server_id: Option<String>,
    pub category: Option<OperationCategory>,
    pub result: Option<OperationResult>,
    pub started_after: Option<i64>,
    pub started_before: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForward {
    pub id: String,
    pub server_id: String,
    pub bind_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPortForward {
    pub server_id: String,
    pub bind_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preference {
    pub key: String,
    pub value: String,
    pub updated_at: i64,
}
