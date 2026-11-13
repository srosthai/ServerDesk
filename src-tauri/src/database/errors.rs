//! Shared database error taxonomy.
//!
//! Adapters translate driver, tunnel, and permission failures into these
//! stable kinds. Commands convert the result into `AppError` at the Tauri
//! boundary, so React never depends on engine-specific driver errors or SQL.

use std::fmt;

use serde::Serialize;

use crate::core::{AppError, ErrorCode, ErrorScope, redaction::sanitize_text_bounded};

const MAX_MESSAGE_LENGTH: usize = 2_000;
const MAX_DETAILS_LENGTH: usize = 4_000;

/// Stable failure categories shared by every database engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseErrorKind {
    /// The SSH tunnel or forwarded listener failed before the driver connected.
    Tunnel,
    /// The database rejected the username, password, or authentication method.
    DatabaseAuth,
    /// A connect, query, metadata, or cancellation operation exceeded its bound.
    Timeout,
    /// The database account lacks the privilege required for the operation.
    Permission,
    /// A foreign key, view, sequence, or trigger blocks the operation.
    Dependency,
    /// The engine rejected SQL or a metadata request while executing it.
    Query,
    /// The engine or version does not support the requested action.
    Capability,
    /// The user cancelled the operation.
    Cancelled,
    /// Local validation rejected the request before it reached an engine.
    Validation,
    /// No live database connection exists for the requested operation.
    Connection,
    /// A local invariant failed without a safe engine-specific interpretation.
    Internal,
}

impl DatabaseErrorKind {
    pub const ALL: [Self; 11] = [
        Self::Tunnel,
        Self::DatabaseAuth,
        Self::Timeout,
        Self::Permission,
        Self::Dependency,
        Self::Query,
        Self::Capability,
        Self::Cancelled,
        Self::Validation,
        Self::Connection,
        Self::Internal,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tunnel => "tunnel",
            Self::DatabaseAuth => "database_auth",
            Self::Timeout => "timeout",
            Self::Permission => "permission",
            Self::Dependency => "dependency",
            Self::Query => "query",
            Self::Capability => "capability",
            Self::Cancelled => "cancelled",
            Self::Validation => "validation",
            Self::Connection => "connection",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for DatabaseErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Structured database failure with bounded, redacted diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatabaseError {
    pub kind: DatabaseErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// The database object the failure belongs to. Metadata commands set this
    /// so a permission error can stay attached to one table or schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<ErrorScope>,
}

impl DatabaseError {
    pub fn new(kind: DatabaseErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: sanitize_text_bounded(&message.into(), MAX_MESSAGE_LENGTH),
            details: None,
            scope: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(sanitize_text_bounded(&details.into(), MAX_DETAILS_LENGTH));
        self
    }

    pub fn with_scope(mut self, object: impl Into<String>, name: impl Into<String>) -> Self {
        self.scope = Some(ErrorScope::new(object, name));
        self
    }

    /// Adds an object scope without changing the error kind. Adapters use this
    /// for metadata reads, which can fail for an object the explorer has not
    /// even rendered yet.
    pub fn scoped_to(self, object: impl Into<String>, name: impl Into<String>) -> Self {
        self.with_scope(object, name)
    }

    /// Adds an object scope only when the failure is a permission error. Query
    /// and connection errors are not object-specific.
    pub fn scoped_permission(self, object: impl Into<String>, name: impl Into<String>) -> Self {
        if self.kind == DatabaseErrorKind::Permission {
            self.with_scope(object, name)
        } else {
            self
        }
    }

    pub fn tunnel(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Tunnel, message)
    }

    pub fn database_auth(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::DatabaseAuth, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Timeout, message)
    }

    pub fn permission(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Permission, message)
    }

    pub fn dependency(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Dependency, message)
    }

    pub fn query(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Query, message)
    }

    pub fn capability(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Capability, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Cancelled, message)
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Validation, message)
    }

    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Connection, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(DatabaseErrorKind::Internal, message)
    }

    pub const fn code(&self) -> &'static str {
        self.kind.as_str()
    }
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DatabaseError {}

impl From<DatabaseError> for AppError {
    fn from(error: DatabaseError) -> Self {
        let code = match error.kind {
            DatabaseErrorKind::Tunnel => ErrorCode::Tunnel,
            DatabaseErrorKind::DatabaseAuth => ErrorCode::DatabaseAuth,
            DatabaseErrorKind::Timeout => ErrorCode::Timeout,
            DatabaseErrorKind::Permission => ErrorCode::Permission,
            DatabaseErrorKind::Dependency => ErrorCode::Conflict,
            DatabaseErrorKind::Query => ErrorCode::Query,
            DatabaseErrorKind::Capability => ErrorCode::Capability,
            DatabaseErrorKind::Cancelled => ErrorCode::Cancelled,
            DatabaseErrorKind::Validation => ErrorCode::Validation,
            DatabaseErrorKind::Connection => ErrorCode::Connection,
            DatabaseErrorKind::Internal => ErrorCode::Internal,
        };

        let app_error = AppError::new(code, error.message);
        let app_error = match error.details {
            Some(details) => app_error.with_details(details),
            None => app_error,
        };
        match error.scope {
            Some(scope) => app_error.with_scope(scope.object, scope.name),
            None => app_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_stable_code_and_maps_to_the_app_error_taxonomy() {
        let expected = [
            (DatabaseErrorKind::Tunnel, "tunnel", ErrorCode::Tunnel),
            (
                DatabaseErrorKind::DatabaseAuth,
                "database_auth",
                ErrorCode::DatabaseAuth,
            ),
            (DatabaseErrorKind::Timeout, "timeout", ErrorCode::Timeout),
            (
                DatabaseErrorKind::Permission,
                "permission",
                ErrorCode::Permission,
            ),
            (
                DatabaseErrorKind::Dependency,
                "dependency",
                ErrorCode::Conflict,
            ),
            (DatabaseErrorKind::Query, "query", ErrorCode::Query),
            (
                DatabaseErrorKind::Capability,
                "capability",
                ErrorCode::Capability,
            ),
            (
                DatabaseErrorKind::Cancelled,
                "cancelled",
                ErrorCode::Cancelled,
            ),
            (
                DatabaseErrorKind::Validation,
                "validation",
                ErrorCode::Validation,
            ),
            (
                DatabaseErrorKind::Connection,
                "connection",
                ErrorCode::Connection,
            ),
            (DatabaseErrorKind::Internal, "internal", ErrorCode::Internal),
        ];

        assert_eq!(expected.len(), DatabaseErrorKind::ALL.len());
        for (kind, code, app_code) in expected {
            assert_eq!(kind.as_str(), code);
            assert_eq!(DatabaseError::new(kind, "failure").code(), code);
            assert_eq!(
                AppError::from(DatabaseError::new(kind, "failure")).code,
                app_code
            );
        }
    }

    #[test]
    fn diagnostics_are_bounded_and_redacted() {
        let error = DatabaseError::database_auth("Access denied for user app")
            .with_details("DB_PASSWORD=hunter2 and a very long diagnostic ".repeat(500));
        let details = error.details.expect("details");

        assert!(!details.contains("hunter2"));
        assert!(details.len() <= MAX_DETAILS_LENGTH);
    }
}
