use std::fmt;

use serde::Serialize;

use crate::storage::StorageError;

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
    Cancelled,
    NotFound,
    Storage,
    Internal,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
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

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Cancelled, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
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
    fn maps_storage_errors_without_exposing_them_as_success() {
        let error = AppError::from(StorageError::NotFound);

        assert_eq!(error.code, ErrorCode::Storage);
        assert_eq!(error.message, "Local storage error");
    }
}
