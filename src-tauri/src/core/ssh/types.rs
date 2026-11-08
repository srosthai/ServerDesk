use std::{fmt, time::Duration};

use serde::{Deserialize, Serialize};

use crate::core::{AppError, ErrorCode};

pub const CONNECTION_STATE_EVENT: &str = "connection-state-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    HostKeyVerification,
    Connected,
    Reconnecting,
    Error,
}

impl ConnectionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::HostKeyVerification => "host-key-verification",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyDetails {
    pub key_type: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub server_id: String,
    pub state: ConnectionState,
    pub changed_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_key: Option<HostKeyDetails>,
}

impl ConnectionSnapshot {
    pub fn disconnected(server_id: impl Into<String>) -> Self {
        Self {
            server_id: server_id.into(),
            state: ConnectionState::Disconnected,
            changed_at: now_millis(),
            error: None,
            host_key: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedHostKey {
    pub key_type: String,
    pub fingerprint: String,
}

#[derive(Clone)]
pub enum Authentication {
    Password(String),
    PrivateKey {
        key: String,
        passphrase: Option<String>,
    },
}

impl fmt::Debug for Authentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => formatter.write_str("Password(<redacted>)"),
            Self::PrivateKey { passphrase, .. } => formatter
                .debug_struct("PrivateKey")
                .field("key", &"<redacted>")
                .field("passphrase", &passphrase.as_ref().map(|_| "<redacted>"))
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct ConnectionRequest {
    pub server_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: Authentication,
    pub trusted_host_keys: Vec<TrustedHostKey>,
}

impl fmt::Debug for ConnectionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionRequest")
            .field("server_id", &self.server_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("authentication", &self.authentication)
            .field("trusted_host_keys", &self.trusted_host_keys)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    HostKeyUnknown {
        key_type: String,
        fingerprint: String,
    },
    HostKeyChanged {
        key_type: String,
        expected_fingerprint: String,
        actual_fingerprint: String,
    },
    Authentication,
    Credential(String),
    Dns,
    Network,
    Timeout,
    Protocol,
    Remote(String),
    Cancelled,
    Internal(String),
}

impl TransportError {
    pub fn to_app_error(&self) -> AppError {
        match self {
            Self::HostKeyUnknown {
                key_type,
                fingerprint,
            } => AppError::host_key("The server host key is not trusted yet.").with_details(
                format!("Verify this fingerprint before connecting: {key_type} {fingerprint}"),
            ),
            Self::HostKeyChanged {
                key_type,
                expected_fingerprint,
                actual_fingerprint,
            } => AppError::host_key("The server host key has changed. The connection was blocked.")
                .with_details(format!(
                    "{key_type} expected {expected_fingerprint}, received {actual_fingerprint}"
                )),
            Self::Authentication => AppError::authentication(
                "SSH authentication failed. Check the username and credentials.",
            ),
            Self::Credential(message) => AppError::credential(message.clone()),
            Self::Dns => AppError::dns("Could not resolve the server hostname."),
            Self::Network => AppError::network("Could not reach the SSH server."),
            Self::Timeout => AppError::timeout("The SSH connection timed out."),
            Self::Protocol => AppError::connection("The SSH connection failed."),
            Self::Remote(message) => AppError::remote(message.clone()),
            Self::Cancelled => AppError::cancelled("The connection attempt was cancelled."),
            Self::Internal(message) => AppError::internal(message.clone()),
        }
    }

    pub fn error_code(&self) -> ErrorCode {
        self.to_app_error().code
    }
}

#[async_trait::async_trait]
pub trait SshSession: Send + Sync {
    async fn execute(&self, command: &str) -> Result<CommandOutput, TransportError>;
    async fn is_healthy(&self) -> bool;
    async fn close(&self) -> Result<(), TransportError>;
}

#[async_trait::async_trait]
pub trait SshTransport: Send + Sync {
    async fn connect(
        &self,
        request: &ConnectionRequest,
    ) -> Result<std::sync::Arc<dyn SshSession>, TransportError>;
}

pub trait ConnectionEventSink: Send + Sync {
    fn emit(&self, snapshot: &ConnectionSnapshot);
}

#[derive(Debug, Clone)]
pub struct ConnectionTimeouts {
    pub connect: Duration,
    pub command: Duration,
    pub close: Duration,
}

impl Default for ConnectionTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            command: Duration::from_secs(30),
            close: Duration::from_secs(5),
        }
    }
}

pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_connection_states_for_the_frontend() {
        let states = [
            (ConnectionState::Disconnected, "disconnected"),
            (ConnectionState::Connecting, "connecting"),
            (
                ConnectionState::HostKeyVerification,
                "host-key-verification",
            ),
            (ConnectionState::Connected, "connected"),
            (ConnectionState::Reconnecting, "reconnecting"),
            (ConnectionState::Error, "error"),
        ];

        for (state, expected) in states {
            assert_eq!(
                serde_json::to_value(state).expect("serialized state"),
                expected
            );
        }
    }

    #[test]
    fn debug_output_redacts_credentials() {
        let password_request = ConnectionRequest {
            server_id: "server-1".to_string(),
            host: "198.51.100.24".to_string(),
            port: 22,
            username: "deploy".to_string(),
            authentication: Authentication::Password("super-secret".to_string()),
            trusted_host_keys: Vec::new(),
        };
        let key_request = ConnectionRequest {
            authentication: Authentication::PrivateKey {
                key: "-----BEGIN OPENSSH PRIVATE KEY-----secret".to_string(),
                passphrase: Some("key-passphrase".to_string()),
            },
            ..password_request.clone()
        };

        for request in [password_request, key_request] {
            let debug = format!("{request:?}");
            assert!(debug.contains("<redacted>"));
            assert!(!debug.contains("super-secret"));
            assert!(!debug.contains("BEGIN OPENSSH PRIVATE KEY"));
            assert!(!debug.contains("key-passphrase"));
        }
    }

    #[test]
    fn authentication_errors_are_distinct_from_credentials() {
        assert_eq!(
            TransportError::Authentication.to_app_error().code,
            ErrorCode::Authentication
        );
        assert_eq!(
            TransportError::Credential("bad key".to_string())
                .to_app_error()
                .code,
            ErrorCode::Credential
        );
    }
}
