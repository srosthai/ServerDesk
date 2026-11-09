use std::{fmt, time::Duration};

use serde::{Deserialize, Serialize};

use crate::core::{
    AppError, ErrorCode,
    terminal::{TerminalChannel, TerminalOptions},
};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_fingerprint: Option<String>,
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

/// Validates host-key metadata before it is persisted or compared.
///
/// Fingerprints use the standard OpenSSH SHA256 representation:
/// `SHA256:` followed by the unpadded base64 digest (43 characters).
pub fn validate_host_key_metadata(key_type: &str, fingerprint: &str) -> Result<(), AppError> {
    if key_type.is_empty()
        || key_type.len() > 64
        || !key_type.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '@')
        })
    {
        return Err(AppError::validation("The SSH host key type is not valid."));
    }

    let Some(encoded) = fingerprint.strip_prefix("SHA256:") else {
        return Err(AppError::validation(
            "Host key fingerprints must use the SHA256 format.",
        ));
    };

    if encoded.len() != 43
        || !encoded
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '/'))
    {
        return Err(AppError::validation(
            "The host key fingerprint is not a valid SHA256 fingerprint.",
        ));
    }

    Ok(())
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

    /// Executes a command while refusing to return more than `max_output_bytes`.
    ///
    /// Transports can override this to stop buffering as soon as the limit is
    /// crossed. The default implementation checks the completed output, so
    /// callers still get a hard bound even for simple test sessions.
    async fn execute_bounded(
        &self,
        command: &str,
        max_output_bytes: usize,
    ) -> Result<CommandOutput, TransportError> {
        let output = self.execute(command).await?;
        let output_bytes = output.stdout.len().saturating_add(output.stderr.len());
        if output_bytes > max_output_bytes {
            return Err(TransportError::Remote(
                "The remote command produced more output than ServerDesk allows.".to_string(),
            ));
        }
        Ok(output)
    }

    /// Opens a dedicated PTY-backed interactive shell channel on this session.
    ///
    /// Implementations that cannot provide an interactive terminal keep the
    /// default typed failure. The returned channel is independent from other
    /// command executions on the same SSH session.
    async fn open_terminal(
        &self,
        _options: TerminalOptions,
    ) -> Result<std::sync::Arc<dyn TerminalChannel>, TransportError> {
        Err(TransportError::Remote(
            "This SSH session does not support an interactive terminal.".to_string(),
        ))
    }

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

    #[test]
    fn accepts_standard_openssh_sha256_fingerprints() {
        let fingerprint = format!("SHA256:{}", "A".repeat(43));

        assert!(validate_host_key_metadata("ssh-ed25519", &fingerprint).is_ok());
        assert!(
            validate_host_key_metadata(
                "ecdsa-sha2-nistp256",
                &format!(
                    "SHA256:{}",
                    "abc+/".repeat(10).chars().take(43).collect::<String>()
                )
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_malformed_host_key_metadata() {
        for (key_type, fingerprint) in [
            ("", "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            (
                "ssh ed25519",
                "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            ),
            ("ssh-ed25519", "MD5:aa:bb:cc"),
            ("ssh-ed25519", "SHA256:short"),
            (
                "ssh-ed25519",
                "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            ),
        ] {
            assert!(
                validate_host_key_metadata(key_type, fingerprint).is_err(),
                "{key_type} {fingerprint} should be rejected"
            );
        }
    }
}
