use std::fmt;

use serde::Serialize;

use crate::{core::redaction::redact_text, storage::StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Validation,
    Connection,
    Authentication,
    HostKey,
    Timeout,
    Dns,
    Network,
    Permission,
    Remote,
    Tunnel,
    DatabaseAuth,
    Query,
    Capability,
    Cancelled,
    Conflict,
    NotFound,
    Storage,
    Credential,
    CredentialUnavailable,
    CredentialLocked,
    CredentialMissing,
    CredentialInvalid,
    Internal,
}

/// The object a typed error is scoped to.
///
/// Metadata operations report permission failures with the database object the
/// caller tried to read, so the UI can keep the error attached to that object
/// instead of replacing the whole workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorScope {
    pub object: String,
    pub name: String,
}

impl ErrorScope {
    pub fn new(object: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            object: object.into(),
            name: name.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<ErrorScope>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: sanitize_diagnostic(message.into()),
            details: None,
            scope: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(sanitize_diagnostic(details.into()));
        self
    }

    pub fn with_scope(mut self, object: impl Into<String>, name: impl Into<String>) -> Self {
        self.scope = Some(ErrorScope::new(object, name));
        self
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Validation, message)
    }

    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Connection, message)
    }

    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Authentication, message)
    }

    pub fn host_key(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::HostKey, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Timeout, message)
    }

    pub fn dns(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Dns, message)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Network, message)
    }

    pub fn permission(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Permission, message)
    }

    pub fn remote(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Remote, message)
    }

    pub fn tunnel(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Tunnel, message)
    }

    pub fn database_auth(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DatabaseAuth, message)
    }

    pub fn query(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Query, message)
    }

    pub fn capability(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Capability, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Cancelled, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn credential(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Credential, message)
    }

    pub fn credential_unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::CredentialUnavailable, message)
    }

    pub fn credential_locked(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::CredentialLocked, message)
    }

    pub fn credential_missing(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::CredentialMissing, message)
    }

    pub fn credential_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::CredentialInvalid, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

/// Redacts known secret patterns at the error boundary so diagnostics and
/// crash-report-style dumps can never contain credentials.
fn sanitize_diagnostic(value: String) -> String {
    redact_text(&value).text.trim_end().to_string()
}

impl From<StorageError> for AppError {
    fn from(error: StorageError) -> Self {
        Self::new(ErrorCode::Storage, "Local storage error").with_details(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_structured_error() {
        let error = AppError::timeout("Connection timed out").with_details("Timed out after 10s");
        let value = serde_json::to_value(error).expect("serialized error");

        assert_eq!(value["code"], "timeout");
        assert_eq!(value["message"], "Connection timed out");
        assert_eq!(value["details"], "Timed out after 10s");
    }

    #[test]
    fn omits_empty_details() {
        let value = serde_json::to_value(AppError::validation("Invalid host")).expect("error");

        assert_eq!(value["code"], "validation");
        assert!(value.get("details").is_none());
    }

    #[test]
    fn redacts_secrets_at_the_error_boundary() {
        let error = AppError::remote("Remote command failed: DB_PASSWORD=hunter2")
            .with_details(
                "APP_KEY=base64:super-secret\nAuthorization: Bearer abc123\n                 https://deploy:url-secret@example.test/app\nghp_1234567890abcdef",
            );
        let debug = format!("{error:?}");
        let serialized = serde_json::to_string(&error).expect("serialized error");

        for secret in [
            "hunter2",
            "base64:super-secret",
            "abc123",
            "url-secret",
            "ghp_1234567890abcdef",
        ] {
            assert!(!debug.contains(secret), "{secret:?} leaked in debug output");
            assert!(
                !serialized.contains(secret),
                "{secret:?} leaked in serialized error"
            );
        }
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn maps_storage_errors_without_exposing_them_as_success() {
        let error = AppError::from(StorageError::NotFound);

        assert_eq!(error.code, ErrorCode::Storage);
        assert_eq!(error.message, "Local storage error");
    }
}
