use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::core::AppError;

use super::{
    ConnectionEventSink, ConnectionRequest, ConnectionSnapshot, ConnectionState,
    ConnectionTimeouts, HostKeyDetails, SshSession, SshTransport, TransportError,
    types::now_millis,
};

const DISCOVERY_COMMAND: &str = "true";

#[derive(Clone)]
struct SessionEntry {
    session: Arc<dyn SshSession>,
}

#[derive(Default)]
struct ManagerInner {
    states: HashMap<String, ConnectionSnapshot>,
    sessions: HashMap<String, SessionEntry>,
    inflight: HashMap<String, u64>,
}

pub struct ConnectionManager {
    transport: Arc<dyn SshTransport>,
    events: Arc<dyn ConnectionEventSink>,
    timeouts: ConnectionTimeouts,
    inner: Mutex<ManagerInner>,
    next_generation: AtomicU64,
}

impl ConnectionManager {
    pub fn new(
        transport: Arc<dyn SshTransport>,
        events: Arc<dyn ConnectionEventSink>,
        timeouts: ConnectionTimeouts,
    ) -> Self {
        Self {
            transport,
            events,
            timeouts,
            inner: Mutex::new(ManagerInner::default()),
            next_generation: AtomicU64::new(0),
        }
    }

    pub fn connection_state(&self, server_id: &str) -> Result<ConnectionSnapshot, AppError> {
        let inner = self.lock()?;
        Ok(inner
            .states
            .get(server_id)
            .cloned()
            .unwrap_or_else(|| ConnectionSnapshot::disconnected(server_id)))
    }

    pub async fn connect(
        &self,
        request: ConnectionRequest,
    ) -> Result<ConnectionSnapshot, AppError> {
        validate_request(&request)?;

        let server_id = request.server_id.clone();
        let (generation, existing_session) = {
            let mut inner = self.lock()?;
            if inner.inflight.contains_key(&server_id) {
                return Ok(inner
                    .states
                    .get(&server_id)
                    .cloned()
                    .unwrap_or_else(|| ConnectionSnapshot::disconnected(&server_id)));
            }

            let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
            inner.inflight.insert(server_id.clone(), generation);
            (generation, inner.sessions.get(&server_id).cloned())
        };
        let had_session = existing_session.is_some();

        if let Some(entry) = existing_session {
            if entry.session.is_healthy().await {
                let snapshot = {
                    let mut inner = self.lock()?;
                    if !is_current(&inner, &server_id, generation) {
                        return Err(cancelled());
                    }
                    inner.inflight.remove(&server_id);
                    transition(
                        &mut inner,
                        &server_id,
                        ConnectionState::Connected,
                        None,
                        None,
                    )
                };
                self.events.emit(&snapshot);
                return Ok(snapshot);
            }

            let _ = entry.session.close().await;
            let mut inner = self.lock()?;
            if !is_current(&inner, &server_id, generation) {
                return Err(cancelled());
            }
            inner.sessions.remove(&server_id);
        }

        let previous_state = self.state_locked(&server_id)?;
        let next_state = if had_session || matches!(previous_state, ConnectionState::Error) {
            ConnectionState::Reconnecting
        } else {
            ConnectionState::Connecting
        };
        let snapshot = {
            let mut inner = self.lock()?;
            if !is_current(&inner, &server_id, generation) {
                return Err(cancelled());
            }
            transition(&mut inner, &server_id, next_state, None, None)
        };
        self.events.emit(&snapshot);

        let connect_result =
            tokio::time::timeout(self.timeouts.connect, self.transport.connect(&request)).await;

        match connect_result {
            Err(_) => {
                let error = AppError::timeout(format!(
                    "Connection timed out after {} seconds.",
                    timeout_seconds(self.timeouts.connect)
                ));
                self.finish_with_error(
                    &server_id,
                    generation,
                    error.clone(),
                    ConnectionState::Error,
                    None,
                )?;
                Err(error)
            }
            Ok(Err(TransportError::HostKeyUnknown {
                key_type,
                fingerprint,
            })) => {
                let details = HostKeyDetails {
                    key_type: key_type.clone(),
                    fingerprint: fingerprint.clone(),
                };
                let error = TransportError::HostKeyUnknown {
                    key_type,
                    fingerprint,
                }
                .to_app_error();
                self.finish_with_error(
                    &server_id,
                    generation,
                    error.clone(),
                    ConnectionState::HostKeyVerification,
                    Some(details),
                )?;
                Err(error)
            }
            Ok(Err(TransportError::HostKeyChanged {
                key_type,
                expected_fingerprint,
                actual_fingerprint,
            })) => {
                let details = HostKeyDetails {
                    key_type: key_type.clone(),
                    fingerprint: actual_fingerprint.clone(),
                };
                let error = TransportError::HostKeyChanged {
                    key_type,
                    expected_fingerprint,
                    actual_fingerprint,
                }
                .to_app_error();
                self.finish_with_error(
                    &server_id,
                    generation,
                    error.clone(),
                    ConnectionState::Error,
                    Some(details),
                )?;
                Err(error)
            }
            Ok(Err(error)) => {
                let app_error = error.to_app_error();
                let state = if matches!(error, TransportError::Cancelled) {
                    ConnectionState::Disconnected
                } else {
                    ConnectionState::Error
                };
                self.finish_with_error(&server_id, generation, app_error.clone(), state, None)?;
                Err(app_error)
            }
            Ok(Ok(session)) => {
                if !self.is_current(&server_id, generation)? {
                    let _ = session.close().await;
                    return Err(cancelled());
                }

                let discovery =
                    tokio::time::timeout(self.timeouts.command, session.execute(DISCOVERY_COMMAND))
                        .await;
                match discovery {
                    Err(_) => {
                        let _ = session.close().await;
                        let error = AppError::timeout(format!(
                            "The connection check timed out after {} seconds.",
                            timeout_seconds(self.timeouts.command)
                        ));
                        self.finish_with_error(
                            &server_id,
                            generation,
                            error.clone(),
                            ConnectionState::Error,
                            None,
                        )?;
                        Err(error)
                    }
                    Ok(Err(error)) => {
                        let _ = session.close().await;
                        let app_error = error.to_app_error();
                        self.finish_with_error(
                            &server_id,
                            generation,
                            app_error.clone(),
                            ConnectionState::Error,
                            None,
                        )?;
                        Err(app_error)
                    }
                    Ok(Ok(output)) if output.exit_code != Some(0) => {
                        let _ = session.close().await;
                        let detail = match output.exit_code {
                            Some(code) => format!("exit code {code}"),
                            None => "no exit status".to_string(),
                        };
                        let error = AppError::remote(format!(
                            "The SSH connection check failed ({detail})."
                        ));
                        self.finish_with_error(
                            &server_id,
                            generation,
                            error.clone(),
                            ConnectionState::Error,
                            None,
                        )?;
                        Err(error)
                    }
                    Ok(Ok(_)) => {
                        let snapshot = {
                            let mut inner = self.lock()?;
                            if !is_current(&inner, &server_id, generation) {
                                None
                            } else {
                                inner.sessions.insert(
                                    server_id.clone(),
                                    SessionEntry {
                                        session: Arc::clone(&session),
                                    },
                                );
                                inner.inflight.remove(&server_id);
                                Some(transition(
                                    &mut inner,
                                    &server_id,
                                    ConnectionState::Connected,
                                    None,
                                    None,
                                ))
                            }
                        };

                        let Some(snapshot) = snapshot else {
                            let _ = session.close().await;
                            return Err(cancelled());
                        };
                        self.events.emit(&snapshot);
                        Ok(snapshot)
                    }
                }
            }
        }
    }

    pub async fn disconnect(&self, server_id: &str) -> Result<ConnectionSnapshot, AppError> {
        let (snapshot, session) = {
            let mut inner = self.lock()?;
            inner.inflight.remove(server_id);
            let session = inner.sessions.remove(server_id).map(|entry| entry.session);
            let snapshot = transition(
                &mut inner,
                server_id,
                ConnectionState::Disconnected,
                None,
                None,
            );
            (snapshot, session)
        };
        self.events.emit(&snapshot);

        if let Some(session) = session {
            let _ = tokio::time::timeout(self.timeouts.close, session.close()).await;
        }

        Ok(snapshot)
    }

    pub async fn disconnect_all(&self) {
        let (snapshots, sessions) = {
            let mut inner = match self.inner.lock() {
                Ok(inner) => inner,
                Err(_) => return,
            };
            inner.inflight.clear();
            let sessions = std::mem::take(&mut inner.sessions);
            let states = inner
                .states
                .keys()
                .cloned()
                .chain(sessions.keys().cloned())
                .collect::<std::collections::HashSet<_>>();
            let mut snapshots = Vec::with_capacity(states.len());
            for server_id in states {
                snapshots.push(transition(
                    &mut inner,
                    &server_id,
                    ConnectionState::Disconnected,
                    None,
                    None,
                ));
            }
            (snapshots, sessions)
        };

        for snapshot in &snapshots {
            self.events.emit(snapshot);
        }

        for (_, entry) in sessions {
            let _ = tokio::time::timeout(self.timeouts.close, entry.session.close()).await;
        }
    }

    pub async fn execute(
        &self,
        server_id: &str,
        command: &str,
    ) -> Result<super::CommandOutput, AppError> {
        if command.trim().is_empty() {
            return Err(AppError::validation("Command cannot be empty."));
        }

        let session = {
            let inner = self.lock()?;
            inner
                .sessions
                .get(server_id)
                .map(|entry| Arc::clone(&entry.session))
                .ok_or_else(|| AppError::not_found("The server is not connected."))?
        };

        match tokio::time::timeout(self.timeouts.command, session.execute(command)).await {
            Err(_) => Err(AppError::timeout(format!(
                "The remote command timed out after {} seconds.",
                timeout_seconds(self.timeouts.command)
            ))),
            Ok(Err(error)) => {
                if !session.is_healthy().await {
                    let _ = self.mark_error(server_id, error.to_app_error());
                }
                Err(error.to_app_error())
            }
            Ok(Ok(output)) => Ok(output),
        }
    }

    fn state_locked(&self, server_id: &str) -> Result<ConnectionState, AppError> {
        let inner = self.lock()?;
        Ok(inner
            .states
            .get(server_id)
            .map(|snapshot| snapshot.state)
            .unwrap_or(ConnectionState::Disconnected))
    }

    fn is_current(&self, server_id: &str, generation: u64) -> Result<bool, AppError> {
        let inner = self.lock()?;
        Ok(is_current(&inner, server_id, generation))
    }

    fn finish_with_error(
        &self,
        server_id: &str,
        generation: u64,
        error: AppError,
        state: ConnectionState,
        host_key: Option<HostKeyDetails>,
    ) -> Result<(), AppError> {
        let snapshot = {
            let mut inner = self.lock()?;
            if !is_current(&inner, server_id, generation) {
                return Err(cancelled());
            }
            inner.inflight.remove(server_id);
            transition(&mut inner, server_id, state, Some(error), host_key)
        };
        self.events.emit(&snapshot);
        Ok(())
    }

    fn mark_error(&self, server_id: &str, error: AppError) -> Result<(), AppError> {
        let snapshot = {
            let mut inner = self.lock()?;
            transition(
                &mut inner,
                server_id,
                ConnectionState::Error,
                Some(error),
                None,
            )
        };
        self.events.emit(&snapshot);
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, ManagerInner>, AppError> {
        self.inner
            .lock()
            .map_err(|_| AppError::internal("Connection manager lock was poisoned."))
    }
}

fn validate_request(request: &ConnectionRequest) -> Result<(), AppError> {
    if request.server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }
    if request.host.trim().is_empty() {
        return Err(AppError::validation("Host is required."));
    }
    if request.port == 0 {
        return Err(AppError::validation(
            "SSH port must be between 1 and 65535.",
        ));
    }
    if request.username.trim().is_empty() {
        return Err(AppError::validation("SSH username is required."));
    }

    Ok(())
}

fn transition(
    inner: &mut ManagerInner,
    server_id: &str,
    state: ConnectionState,
    error: Option<AppError>,
    host_key: Option<HostKeyDetails>,
) -> ConnectionSnapshot {
    let snapshot = ConnectionSnapshot {
        server_id: server_id.to_string(),
        state,
        changed_at: now_millis(),
        error,
        host_key,
    };
    inner.states.insert(server_id.to_string(), snapshot.clone());
    snapshot
}

fn is_current(inner: &ManagerInner, server_id: &str, generation: u64) -> bool {
    inner.inflight.get(server_id) == Some(&generation)
}

fn cancelled() -> AppError {
    AppError::cancelled("The connection attempt was cancelled.")
}

fn timeout_seconds(duration: Duration) -> u64 {
    duration.as_secs().max(1)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use tokio::sync::Notify;

    use super::super::{
        Authentication, CommandOutput, ConnectionRequest, SshSession, SshTransport, TransportError,
        TrustedHostKey,
    };
    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        snapshots: Mutex<Vec<ConnectionSnapshot>>,
    }

    impl RecordingSink {
        fn states(&self) -> Vec<ConnectionState> {
            self.snapshots
                .lock()
                .expect("snapshot lock")
                .iter()
                .map(|snapshot| snapshot.state)
                .collect()
        }
    }

    impl ConnectionEventSink for RecordingSink {
        fn emit(&self, snapshot: &ConnectionSnapshot) {
            self.snapshots
                .lock()
                .expect("snapshot lock")
                .push(snapshot.clone());
        }
    }

    struct MockSession {
        command_delay_millis: AtomicU64,
        command_result: Result<CommandOutput, TransportError>,
        healthy: AtomicBool,
        closed: AtomicBool,
        close_count: AtomicUsize,
    }

    impl MockSession {
        fn new(
            command_delay: Duration,
            command_result: Result<CommandOutput, TransportError>,
        ) -> Self {
            Self {
                command_delay_millis: AtomicU64::new(command_delay.as_millis() as u64),
                command_result,
                healthy: AtomicBool::new(true),
                closed: AtomicBool::new(false),
                close_count: AtomicUsize::new(0),
            }
        }

        fn mark_unhealthy(&self) {
            self.healthy.store(false, Ordering::SeqCst);
        }

        fn set_command_delay(&self, delay: Duration) {
            self.command_delay_millis
                .store(delay.as_millis() as u64, Ordering::SeqCst);
        }

        fn close_count(&self) -> usize {
            self.close_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl SshSession for MockSession {
        async fn execute(&self, _command: &str) -> Result<CommandOutput, TransportError> {
            tokio::time::sleep(Duration::from_millis(
                self.command_delay_millis.load(Ordering::SeqCst),
            ))
            .await;
            self.command_result.clone()
        }

        async fn is_healthy(&self) -> bool {
            self.healthy.load(Ordering::SeqCst) && !self.closed.load(Ordering::SeqCst)
        }

        async fn close(&self) -> Result<(), TransportError> {
            self.closed.store(true, Ordering::SeqCst);
            self.close_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    enum ConnectResult {
        Success,
        Failure(TransportError),
    }

    struct MockTransport {
        connect_count: AtomicUsize,
        connect_delay: Duration,
        command_delay: Duration,
        command_result: Result<CommandOutput, TransportError>,
        result: Mutex<ConnectResult>,
        sessions: Mutex<Vec<Arc<MockSession>>>,
        started: Notify,
        gate: Mutex<Option<Arc<Notify>>>,
    }

    impl MockTransport {
        fn success() -> Self {
            Self {
                connect_count: AtomicUsize::new(0),
                connect_delay: Duration::ZERO,
                command_delay: Duration::ZERO,
                command_result: Ok(success_output()),
                result: Mutex::new(ConnectResult::Success),
                sessions: Mutex::new(Vec::new()),
                started: Notify::new(),
                gate: Mutex::new(None),
            }
        }

        fn failing(error: TransportError) -> Self {
            let transport = Self::success();
            *transport.result.lock().expect("result lock") = ConnectResult::Failure(error);
            transport
        }

        fn with_connect_delay(mut self, delay: Duration) -> Self {
            self.connect_delay = delay;
            self
        }

        fn with_command_delay(mut self, delay: Duration) -> Self {
            self.command_delay = delay;
            self
        }

        fn with_gate(self) -> (Self, Arc<Notify>) {
            let gate = Arc::new(Notify::new());
            *self.gate.lock().expect("gate lock") = Some(Arc::clone(&gate));
            (self, gate)
        }

        fn connect_count(&self) -> usize {
            self.connect_count.load(Ordering::SeqCst)
        }

        fn sessions(&self) -> Vec<Arc<MockSession>> {
            self.sessions.lock().expect("session lock").clone()
        }

        fn first_session(&self) -> Arc<MockSession> {
            self.sessions().into_iter().next().expect("session exists")
        }
    }

    #[async_trait]
    impl SshTransport for MockTransport {
        async fn connect(
            &self,
            _request: &ConnectionRequest,
        ) -> Result<Arc<dyn SshSession>, TransportError> {
            self.connect_count.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            let gate = self.gate.lock().expect("gate lock").clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            tokio::time::sleep(self.connect_delay).await;

            let result = self.result.lock().expect("result lock");
            match &*result {
                ConnectResult::Failure(error) => Err(error.clone()),
                ConnectResult::Success => {
                    let session = Arc::new(MockSession::new(
                        self.command_delay,
                        self.command_result.clone(),
                    ));
                    self.sessions
                        .lock()
                        .expect("session lock")
                        .push(Arc::clone(&session));
                    Ok(session)
                }
            }
        }
    }

    fn success_output() -> CommandOutput {
        CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
        }
    }

    fn request(server_id: &str) -> ConnectionRequest {
        ConnectionRequest {
            server_id: server_id.to_string(),
            host: "198.51.100.24".to_string(),
            port: 22,
            username: "deploy".to_string(),
            authentication: Authentication::Password("super-secret".to_string()),
            trusted_host_keys: vec![TrustedHostKey {
                key_type: "ssh-ed25519".to_string(),
                fingerprint: "SHA256:trusted".to_string(),
            }],
        }
    }

    fn test_timeouts() -> ConnectionTimeouts {
        ConnectionTimeouts {
            connect: Duration::from_secs(5),
            command: Duration::from_secs(5),
            close: Duration::from_secs(1),
        }
    }

    fn manager(transport: MockTransport, events: Arc<RecordingSink>) -> Arc<ConnectionManager> {
        Arc::new(ConnectionManager::new(
            Arc::new(transport),
            events,
            test_timeouts(),
        ))
    }

    #[tokio::test(start_paused = true)]
    async fn connect_emits_transitions_and_reuses_healthy_session() {
        let transport = MockTransport::success();
        let events = Arc::new(RecordingSink::default());
        let manager = manager(transport, Arc::clone(&events));

        let first = manager.connect(request("server-1")).await.expect("connect");
        assert_eq!(first.state, ConnectionState::Connected);
        assert_eq!(
            events.states(),
            vec![ConnectionState::Connecting, ConnectionState::Connected]
        );

        let second = manager
            .connect(request("server-1"))
            .await
            .expect("reuse connection");
        assert_eq!(second.state, ConnectionState::Connected);
        assert_eq!(manager.inner.lock().expect("inner lock").sessions.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn second_concurrent_connect_does_not_create_duplicate_session() {
        let (transport, gate) = MockTransport::success().with_gate();
        let transport = Arc::new(transport);
        let events = Arc::new(RecordingSink::default());
        let manager = Arc::new(ConnectionManager::new(
            Arc::clone(&transport) as Arc<dyn SshTransport>,
            Arc::clone(&events) as Arc<dyn ConnectionEventSink>,
            test_timeouts(),
        ));

        let first_manager = Arc::clone(&manager);
        let first = tokio::spawn(async move { first_manager.connect(request("server-1")).await });
        transport.started.notified().await;

        let second = manager
            .connect(request("server-1"))
            .await
            .expect("second connect");
        assert_eq!(second.state, ConnectionState::Connecting);
        assert_eq!(transport.connect_count(), 1);

        gate.notify_waiters();
        let first = first.await.expect("first task").expect("first connect");
        assert_eq!(first.state, ConnectionState::Connected);
        assert_eq!(transport.connect_count(), 1);
        assert_eq!(transport.sessions().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn host_key_verification_state_blocks_connection() {
        let transport = MockTransport::failing(TransportError::HostKeyUnknown {
            key_type: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:new".to_string(),
        });
        let events = Arc::new(RecordingSink::default());
        let manager = manager(transport, Arc::clone(&events));

        let error = manager
            .connect(request("server-1"))
            .await
            .expect_err("host key must be rejected");

        assert_eq!(error.code, crate::core::ErrorCode::HostKey);
        let snapshot = manager.connection_state("server-1").expect("state");
        assert_eq!(snapshot.state, ConnectionState::HostKeyVerification);
        assert_eq!(
            snapshot
                .host_key
                .as_ref()
                .map(|key| key.fingerprint.as_str()),
            Some("SHA256:new")
        );
        assert_eq!(
            events.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::HostKeyVerification
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn changed_host_key_sets_error_without_session() {
        let transport = MockTransport::failing(TransportError::HostKeyChanged {
            key_type: "ssh-ed25519".to_string(),
            expected_fingerprint: "SHA256:old".to_string(),
            actual_fingerprint: "SHA256:new".to_string(),
        });
        let events = Arc::new(RecordingSink::default());
        let manager = manager(transport, Arc::clone(&events));

        let error = manager
            .connect(request("server-1"))
            .await
            .expect_err("changed host key must be rejected");

        assert_eq!(error.code, crate::core::ErrorCode::HostKey);
        let snapshot = manager.connection_state("server-1").expect("state");
        assert_eq!(snapshot.state, ConnectionState::Error);
        assert_eq!(
            snapshot
                .host_key
                .as_ref()
                .map(|key| key.fingerprint.as_str()),
            Some("SHA256:new")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn authentication_failure_sets_error_state() {
        let transport = MockTransport::failing(TransportError::Authentication);
        let events = Arc::new(RecordingSink::default());
        let manager = manager(transport, Arc::clone(&events));

        let error = manager
            .connect(request("server-1"))
            .await
            .expect_err("authentication must fail");

        assert_eq!(error.code, crate::core::ErrorCode::Authentication);
        assert_eq!(
            manager.connection_state("server-1").expect("state").state,
            ConnectionState::Error
        );
    }

    #[tokio::test(start_paused = true)]
    async fn connection_timeout_returns_distinct_timeout_error() {
        let transport = MockTransport::success().with_connect_delay(Duration::from_secs(60));
        let events = Arc::new(RecordingSink::default());
        let manager = manager(transport, Arc::clone(&events));

        let error = manager
            .connect(request("server-1"))
            .await
            .expect_err("connection should time out");

        assert_eq!(error.code, crate::core::ErrorCode::Timeout);
        assert_eq!(
            manager.connection_state("server-1").expect("state").state,
            ConnectionState::Error
        );
    }

    #[tokio::test(start_paused = true)]
    async fn command_timeout_returns_distinct_timeout_error_and_closes_session() {
        let transport =
            Arc::new(MockTransport::success().with_command_delay(Duration::from_secs(60)));
        let events = Arc::new(RecordingSink::default());
        let manager = Arc::new(ConnectionManager::new(
            Arc::clone(&transport) as Arc<dyn SshTransport>,
            Arc::clone(&events) as Arc<dyn ConnectionEventSink>,
            test_timeouts(),
        ));

        let error = manager
            .connect(request("server-1"))
            .await
            .expect_err("command check should time out");

        assert_eq!(error.code, crate::core::ErrorCode::Timeout);
        assert_eq!(
            manager.connection_state("server-1").expect("state").state,
            ConnectionState::Error
        );
        assert_eq!(manager.inner.lock().expect("inner lock").sessions.len(), 0);
        assert_eq!(transport.sessions().len(), 1);
        assert_eq!(transport.first_session().close_count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_releases_session_and_updates_state() {
        let transport = MockTransport::success();
        let transport = Arc::new(transport);
        let events = Arc::new(RecordingSink::default());
        let manager = Arc::new(ConnectionManager::new(
            Arc::clone(&transport) as Arc<dyn SshTransport>,
            Arc::clone(&events) as Arc<dyn ConnectionEventSink>,
            test_timeouts(),
        ));

        manager.connect(request("server-1")).await.expect("connect");
        let snapshot = manager.disconnect("server-1").await.expect("disconnect");

        assert_eq!(snapshot.state, ConnectionState::Disconnected);
        assert_eq!(transport.first_session().close_count(), 1);
        assert_eq!(
            manager.connection_state("server-1").expect("state").state,
            ConnectionState::Disconnected
        );
        assert_eq!(
            manager
                .execute("server-1", "true")
                .await
                .expect_err("session is released")
                .code,
            crate::core::ErrorCode::NotFound
        );
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_all_releases_every_session() {
        let transport = MockTransport::success();
        let transport = Arc::new(transport);
        let events = Arc::new(RecordingSink::default());
        let manager = Arc::new(ConnectionManager::new(
            Arc::clone(&transport) as Arc<dyn SshTransport>,
            Arc::clone(&events) as Arc<dyn ConnectionEventSink>,
            test_timeouts(),
        ));

        manager.connect(request("server-1")).await.expect("connect");
        manager.connect(request("server-2")).await.expect("connect");
        manager.disconnect_all().await;

        let sessions = transport.sessions();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|session| session.close_count() == 1));
        assert_eq!(
            manager.connection_state("server-1").expect("state").state,
            ConnectionState::Disconnected
        );
        assert_eq!(
            manager.connection_state("server-2").expect("state").state,
            ConnectionState::Disconnected
        );
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_during_connect_cancels_and_closes_late_session() {
        let (transport, gate) = MockTransport::success().with_gate();
        let transport = Arc::new(transport);
        let events = Arc::new(RecordingSink::default());
        let manager = Arc::new(ConnectionManager::new(
            Arc::clone(&transport) as Arc<dyn SshTransport>,
            Arc::clone(&events) as Arc<dyn ConnectionEventSink>,
            test_timeouts(),
        ));

        let connect_manager = Arc::clone(&manager);
        let connect =
            tokio::spawn(async move { connect_manager.connect(request("server-1")).await });
        transport.started.notified().await;
        manager.disconnect("server-1").await.expect("disconnect");
        gate.notify_waiters();

        let error = connect
            .await
            .expect("connect task")
            .expect_err("connect must be cancelled");
        assert_eq!(error.code, crate::core::ErrorCode::Cancelled);
        assert_eq!(transport.first_session().close_count(), 1);
        assert_eq!(
            manager.connection_state("server-1").expect("state").state,
            ConnectionState::Disconnected
        );
    }

    #[tokio::test(start_paused = true)]
    async fn unhealthy_session_is_replaced() {
        let transport = MockTransport::success();
        let transport = Arc::new(transport);
        let events = Arc::new(RecordingSink::default());
        let manager = Arc::new(ConnectionManager::new(
            Arc::clone(&transport) as Arc<dyn SshTransport>,
            Arc::clone(&events) as Arc<dyn ConnectionEventSink>,
            test_timeouts(),
        ));

        manager.connect(request("server-1")).await.expect("connect");
        transport.first_session().mark_unhealthy();
        manager
            .connect(request("server-1"))
            .await
            .expect("reconnect");

        assert_eq!(transport.connect_count(), 2);
        assert_eq!(transport.sessions().len(), 2);
        assert_eq!(transport.sessions()[0].close_count(), 1);
        assert_eq!(
            manager.connection_state("server-1").expect("state").state,
            ConnectionState::Connected
        );
    }

    #[tokio::test(start_paused = true)]
    async fn execute_reports_command_timeout_without_dropping_healthy_session() {
        let transport = Arc::new(MockTransport::success());
        let events = Arc::new(RecordingSink::default());
        let manager = Arc::new(ConnectionManager::new(
            Arc::clone(&transport) as Arc<dyn SshTransport>,
            Arc::clone(&events) as Arc<dyn ConnectionEventSink>,
            test_timeouts(),
        ));

        manager.connect(request("server-1")).await.expect("connect");
        transport
            .first_session()
            .set_command_delay(Duration::from_secs(60));
        let error = manager
            .execute("server-1", "uptime")
            .await
            .expect_err("command should time out");

        assert_eq!(error.code, crate::core::ErrorCode::Timeout);
        assert_eq!(
            manager.connection_state("server-1").expect("state").state,
            ConnectionState::Connected
        );
    }
}
