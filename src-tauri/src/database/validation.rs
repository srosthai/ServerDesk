//! Shared request validation for live database adapters.
//!
//! Both SQLx adapters accept only the loopback endpoint published by an SSH
//! tunnel and reject malformed input before any driver sees secret material.

use super::{errors::DatabaseError, traits::DatabaseConnectRequest};

pub const LOOPBACK_VALIDATION_MESSAGE: &str =
    "Database connections must target the loopback endpoint published by the SSH tunnel.";

/// Validates endpoint, account, and timeout fields shared by MySQL and
/// PostgreSQL.
pub fn validate_connect_request(request: &DatabaseConnectRequest) -> Result<(), DatabaseError> {
    let host = request.host.trim();
    if host.is_empty() {
        return Err(DatabaseError::validation("A database host is required."));
    }
    if request.host.as_str() != host
        || host.chars().any(char::is_control)
        || host.chars().any(char::is_whitespace)
    {
        return Err(DatabaseError::validation("The database host is not valid."));
    }
    if !is_loopback_host(host) {
        return Err(DatabaseError::validation(LOOPBACK_VALIDATION_MESSAGE));
    }
    if request.port == 0 {
        return Err(DatabaseError::validation(
            "A database port must be between 1 and 65535.",
        ));
    }

    let username = request.username.trim();
    if username.is_empty() {
        return Err(DatabaseError::validation(
            "A database username is required.",
        ));
    }
    if username.chars().any(char::is_control) {
        return Err(DatabaseError::validation(
            "The database username is not valid.",
        ));
    }

    if request
        .database
        .as_deref()
        .is_some_and(|database| database.chars().any(char::is_control))
    {
        return Err(DatabaseError::validation("The database name is not valid."));
    }

    if request.connect_timeout.is_zero() {
        return Err(DatabaseError::validation(
            "A database connect timeout is required.",
        ));
    }

    Ok(())
}

pub fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::database::DatabasePassword;

    fn request() -> DatabaseConnectRequest {
        DatabaseConnectRequest {
            host: "127.0.0.1".to_string(),
            port: 5432,
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            password: DatabasePassword::new("fixture-password"),
            read_only: true,
            ssl_mode: Some("preferred".to_string()),
            connect_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn accepts_only_loopback_endpoints() {
        assert!(validate_connect_request(&request()).is_ok());

        let mut loopback = request();
        loopback.host = "::1".to_string();
        assert!(validate_connect_request(&loopback).is_ok());

        for host in [
            "db.internal",
            "localhost",
            "0.0.0.0",
            "127.0.0.1 ",
            "127.0.0.1\n",
        ] {
            let mut candidate = request();
            candidate.host = host.to_string();
            assert!(
                validate_connect_request(&candidate).is_err(),
                "host {host:?} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_malformed_account_and_timeout_fields() {
        for mutate in [
            |request: &mut DatabaseConnectRequest| request.port = 0,
            |request: &mut DatabaseConnectRequest| request.username = " ".to_string(),
            |request: &mut DatabaseConnectRequest| request.username = "app\nuser".to_string(),
            |request: &mut DatabaseConnectRequest| request.database = Some("app\0".to_string()),
            |request: &mut DatabaseConnectRequest| request.connect_timeout = Duration::ZERO,
        ] {
            let mut candidate = request();
            mutate(&mut candidate);
            assert!(validate_connect_request(&candidate).is_err());
        }
    }

    #[test]
    fn loopback_check_is_exact() {
        for host in ["127.0.0.1", "::1"] {
            assert!(is_loopback_host(host));
        }
        for host in ["localhost", "127.0.0.2", "::", "0.0.0.0", ""] {
            assert!(!is_loopback_host(host));
        }
    }
}
