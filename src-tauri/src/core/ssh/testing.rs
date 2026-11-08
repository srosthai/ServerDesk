use std::time::Instant;

use serde::Serialize;

use crate::core::AppError;

use super::{ConnectionRequest, ConnectionTimeouts, HostKeyDetails, SshTransport, TransportError};

const DISCOVERY_COMMAND: &str = "true";

/// The result of a one-off connection test.
///
/// A host-key verification result is not an error: the caller must present the
/// fingerprint to the user and retry with the accepted key before authentication
/// can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConnectionTestOutcome {
    Connected { duration_ms: u64 },
    HostKeyVerification { host_key: HostKeyDetails },
}

/// Tests host resolution, SSH handshake, host-key verification, authentication,
/// and a harmless remote command without creating a managed session.
pub async fn test_connection(
    transport: &dyn SshTransport,
    request: &ConnectionRequest,
    timeouts: &ConnectionTimeouts,
) -> Result<ConnectionTestOutcome, AppError> {
    let started = Instant::now();

    let session = match tokio::time::timeout(timeouts.connect, transport.connect(request)).await {
        Ok(Ok(session)) => session,
        Ok(Err(TransportError::HostKeyUnknown {
            key_type,
            fingerprint,
        })) => {
            return Ok(ConnectionTestOutcome::HostKeyVerification {
                host_key: HostKeyDetails {
                    key_type,
                    fingerprint,
                    expected_fingerprint: None,
                },
            });
        }
        Ok(Err(error)) => return Err(error.to_app_error()),
        Err(_) => {
            return Err(AppError::timeout(format!(
                "Connection timed out after {} seconds.",
                timeout_seconds(timeouts.connect)
            )));
        }
    };

    let check =
        match tokio::time::timeout(timeouts.command, session.execute(DISCOVERY_COMMAND)).await {
            Ok(Ok(output)) if output.exit_code == Some(0) => Ok(()),
            Ok(Ok(output)) => {
                let detail = match output.exit_code {
                    Some(code) => format!("exit code {code}"),
                    None => "no exit status".to_string(),
                };
                Err(AppError::remote(format!(
                    "The SSH connection check failed ({detail})."
                )))
            }
            Ok(Err(error)) => Err(error.to_app_error()),
            Err(_) => Err(AppError::timeout(format!(
                "The connection check timed out after {} seconds.",
                timeout_seconds(timeouts.command)
            ))),
        };

    let _ = tokio::time::timeout(timeouts.close, session.close()).await;
    check?;

    Ok(ConnectionTestOutcome::Connected {
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

fn timeout_seconds(duration: std::time::Duration) -> u64 {
    duration.as_secs().max(1)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;

    use super::super::{
        Authentication, CommandOutput, ConnectionRequest, SshSession, SshTransport,
    };
    use super::*;

    struct MockSession {
        result: Result<CommandOutput, TransportError>,
        closed: AtomicBool,
        close_count: AtomicUsize,
    }

    impl MockSession {
        fn new(result: Result<CommandOutput, TransportError>) -> Self {
            Self {
                result,
                closed: AtomicBool::new(false),
                close_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl SshSession for MockSession {
        async fn execute(&self, _command: &str) -> Result<CommandOutput, TransportError> {
            self.result.clone()
        }

        async fn is_healthy(&self) -> bool {
            !self.closed.load(Ordering::SeqCst)
        }

        async fn close(&self) -> Result<(), TransportError> {
            self.closed.store(true, Ordering::SeqCst);
            self.close_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct MockTransport {
        result: Result<Arc<dyn SshSession>, TransportError>,
        session: Option<Arc<MockSession>>,
        delay: Duration,
    }

    impl MockTransport {
        fn success(output: CommandOutput) -> Arc<Self> {
            let session = Arc::new(MockSession::new(Ok(output)));
            Arc::new(Self {
                result: Ok(Arc::clone(&session) as Arc<dyn SshSession>),
                session: Some(session),
                delay: Duration::ZERO,
            })
        }

        fn failure(error: TransportError) -> Arc<Self> {
            Arc::new(Self {
                result: Err(error),
                session: None,
                delay: Duration::ZERO,
            })
        }

        fn with_delay(mut self: Arc<Self>, delay: Duration) -> Arc<Self> {
            Arc::get_mut(&mut self).expect("single owner").delay = delay;
            self
        }
    }

    #[async_trait]
    impl SshTransport for MockTransport {
        async fn connect(
            &self,
            _request: &ConnectionRequest,
        ) -> Result<Arc<dyn SshSession>, TransportError> {
            tokio::time::sleep(self.delay).await;
            self.result.clone()
        }
    }

    fn request() -> ConnectionRequest {
        ConnectionRequest {
            server_id: "draft".to_string(),
            host: "198.51.100.24".to_string(),
            port: 22,
            username: "deploy".to_string(),
            authentication: Authentication::Password("super-secret".to_string()),
            trusted_host_keys: Vec::new(),
        }
    }

    fn timeouts() -> ConnectionTimeouts {
        ConnectionTimeouts {
            connect: Duration::from_secs(5),
            command: Duration::from_secs(5),
            close: Duration::from_secs(1),
        }
    }

    fn success_output() -> CommandOutput {
        CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
        }
    }

    #[test]
    fn serializes_test_results_for_the_frontend() {
        let connected = serde_json::to_value(ConnectionTestOutcome::Connected { duration_ms: 125 })
            .expect("connected result");
        assert_eq!(connected["status"], "connected");
        assert_eq!(connected["durationMs"], 125);

        let verification = serde_json::to_value(ConnectionTestOutcome::HostKeyVerification {
            host_key: HostKeyDetails {
                key_type: "ssh-ed25519".to_string(),
                fingerprint: "SHA256:new".to_string(),
                expected_fingerprint: None,
            },
        })
        .expect("verification result");
        assert_eq!(verification["status"], "hostKeyVerification");
        assert_eq!(verification["hostKey"]["keyType"], "ssh-ed25519");
        assert_eq!(verification["hostKey"]["fingerprint"], "SHA256:new");
        assert!(verification["hostKey"].get("expectedFingerprint").is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn returns_host_key_details_before_authentication_for_an_unknown_key() {
        let transport = MockTransport::failure(TransportError::HostKeyUnknown {
            key_type: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:new".to_string(),
        });

        let outcome = test_connection(transport.as_ref(), &request(), &timeouts())
            .await
            .expect("verification outcome");

        assert_eq!(
            outcome,
            ConnectionTestOutcome::HostKeyVerification {
                host_key: HostKeyDetails {
                    key_type: "ssh-ed25519".to_string(),
                    fingerprint: "SHA256:new".to_string(),
                    expected_fingerprint: None,
                }
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn successful_test_closes_the_temporary_session() {
        let transport = MockTransport::success(success_output());
        let session = Arc::clone(transport.session.as_ref().expect("session"));

        let outcome = test_connection(transport.as_ref(), &request(), &timeouts())
            .await
            .expect("connected");

        assert!(matches!(outcome, ConnectionTestOutcome::Connected { .. }));
        assert_eq!(session.close_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_remote_check_returns_a_remote_error() {
        let transport = MockTransport::success(CommandOutput {
            stdout: String::new(),
            stderr: "nope".to_string(),
            exit_code: Some(1),
        });

        let error = test_connection(transport.as_ref(), &request(), &timeouts())
            .await
            .expect_err("remote check failure");

        assert_eq!(error.code, crate::core::ErrorCode::Remote);
    }

    #[tokio::test(start_paused = true)]
    async fn connect_timeout_returns_a_timeout_error() {
        let transport =
            MockTransport::success(success_output()).with_delay(Duration::from_secs(60));

        let error = test_connection(transport.as_ref(), &request(), &timeouts())
            .await
            .expect_err("timeout");

        assert_eq!(error.code, crate::core::ErrorCode::Timeout);
    }

    #[tokio::test(start_paused = true)]
    async fn authentication_and_host_key_failures_keep_their_distinct_codes() {
        for (transport_error, expected) in [
            (
                TransportError::Authentication,
                crate::core::ErrorCode::Authentication,
            ),
            (
                TransportError::HostKeyChanged {
                    key_type: "ssh-ed25519".to_string(),
                    expected_fingerprint: "SHA256:old".to_string(),
                    actual_fingerprint: "SHA256:new".to_string(),
                },
                crate::core::ErrorCode::HostKey,
            ),
            (TransportError::Dns, crate::core::ErrorCode::Dns),
            (TransportError::Network, crate::core::ErrorCode::Network),
        ] {
            let transport = MockTransport::failure(transport_error);
            let error = test_connection(transport.as_ref(), &request(), &timeouts())
                .await
                .expect_err("typed failure");
            assert_eq!(error.code, expected);
        }
    }
}
