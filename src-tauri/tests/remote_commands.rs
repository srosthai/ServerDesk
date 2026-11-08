use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Auth, ChannelOpenHandle, Msg, Session},
};
use serverdesk_lib::core::{
    AppError, ErrorCode,
    remote::{RemoteCommand, run_remote_command},
    ssh::{
        Authentication, ConnectionEventSink, ConnectionManager, ConnectionRequest,
        ConnectionSnapshot, ConnectionState, ConnectionTimeouts, RusshTransport, TrustedHostKey,
    },
};
use tokio::net::TcpListener;

const USERNAME: &str = "deploy";
const PASSWORD: &str = "fixture-password";
const HOSTNAME_OUTPUT: &str = "fixture-host\n";
const UPTIME_OUTPUT: &str =
    " 10:42:00 up 4 days,  2:03,  1 user,  load average: 0.01, 0.02, 0.03\n";
const UPTIME_FAILURE: &str = "uptime: fixture failure\n";

#[derive(Default)]
struct FixtureState {
    authenticated: AtomicUsize,
    executed: Mutex<Vec<String>>,
    active_connections: AtomicUsize,
}

struct FixtureServer {
    port: u16,
    host_key_type: String,
    host_fingerprint: String,
    state: Arc<FixtureState>,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    async fn start(fail_uptime: bool) -> Self {
        let host_key = generate_private_key();
        let host_key_type = host_key.public_key().algorithm().as_str().to_string();
        let host_fingerprint = host_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind fixture server");
        let port = listener.local_addr().expect("fixture address").port();
        let state = Arc::new(FixtureState::default());
        let config = Arc::new(server::Config {
            keys: vec![host_key],
            auth_rejection_time: Duration::from_millis(10),
            auth_rejection_time_initial: Some(Duration::ZERO),
            inactivity_timeout: Some(Duration::from_secs(30)),
            ..Default::default()
        });
        let task = tokio::spawn(run_fixture_server(
            listener,
            config,
            Arc::clone(&state),
            fail_uptime,
        ));

        Self {
            port,
            host_key_type,
            host_fingerprint,
            state,
            task,
        }
    }

    fn request(&self, trust_host_key: bool) -> ConnectionRequest {
        ConnectionRequest {
            server_id: "fixture-server".to_string(),
            host: "127.0.0.1".to_string(),
            port: self.port,
            username: USERNAME.to_string(),
            authentication: Authentication::Password(PASSWORD.to_string()),
            trusted_host_keys: if trust_host_key {
                vec![TrustedHostKey {
                    key_type: self.host_key_type.clone(),
                    fingerprint: self.host_fingerprint.clone(),
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn authenticated(&self) -> usize {
        self.state.authenticated.load(Ordering::SeqCst)
    }

    fn executed(&self) -> Vec<String> {
        self.state.executed.lock().expect("executed lock").clone()
    }

    async fn wait_until_idle(&self) {
        for _ in 0..200 {
            if self.state.active_connections.load(Ordering::SeqCst) == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(
            self.state.active_connections.load(Ordering::SeqCst),
            0,
            "the fixture server still has an open connection after disconnect"
        );
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct ConnectionGuard(Arc<FixtureState>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.active_connections.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn run_fixture_server(
    listener: TcpListener,
    config: Arc<server::Config>,
    state: Arc<FixtureState>,
    fail_uptime: bool,
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        state.active_connections.fetch_add(1, Ordering::SeqCst);
        let config = Arc::clone(&config);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _guard = ConnectionGuard(Arc::clone(&state));
            let handler = FixtureHandler { state, fail_uptime };
            if let Ok(session) = server::run_stream(config, socket, handler).await {
                let _ = session.await;
            }
        });
    }
}

struct FixtureHandler {
    state: Arc<FixtureState>,
    fail_uptime: bool,
}

impl server::Handler for FixtureHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, username: &str, password: &str) -> Result<Auth, Self::Error> {
        if username == USERNAME && password == PASSWORD {
            self.state.authenticated.fetch_add(1, Ordering::SeqCst);
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).to_string();
        self.state
            .executed
            .lock()
            .expect("executed lock")
            .push(command.clone());

        match command.as_str() {
            "true" => respond(session, channel, b"", b"", 0),
            "hostname" => respond(session, channel, HOSTNAME_OUTPUT.as_bytes(), b"", 0),
            "uptime" if self.fail_uptime => {
                respond(session, channel, b"", UPTIME_FAILURE.as_bytes(), 7)
            }
            "uptime" => respond(session, channel, UPTIME_OUTPUT.as_bytes(), b"", 0),
            _ => {
                session.channel_failure(channel)?;
                session.eof(channel)?;
                session.close(channel)?;
                Ok(())
            }
        }
    }
}

fn respond(
    session: &mut Session,
    channel: ChannelId,
    stdout: &[u8],
    stderr: &[u8],
    exit_code: u32,
) -> Result<(), russh::Error> {
    session.channel_success(channel)?;
    if !stdout.is_empty() {
        session.data(channel, stdout.to_vec())?;
    }
    if !stderr.is_empty() {
        session.extended_data(channel, 1, stderr.to_vec())?;
    }
    session.exit_status_request(channel, exit_code)?;
    session.eof(channel)?;
    session.close(channel)?;
    Ok(())
}

#[derive(Default)]
struct RecordingSink {
    snapshots: Mutex<Vec<ConnectionSnapshot>>,
}

impl ConnectionEventSink for RecordingSink {
    fn emit(&self, snapshot: &ConnectionSnapshot) {
        self.snapshots
            .lock()
            .expect("snapshot lock")
            .push(snapshot.clone());
    }
}

fn generate_private_key() -> PrivateKey {
    let mut rng = rand::rng();
    PrivateKey::random(&mut rng, Algorithm::Ed25519).expect("generate host key")
}

fn manager() -> Arc<ConnectionManager> {
    Arc::new(ConnectionManager::new(
        Arc::new(RusshTransport::new()),
        Arc::new(RecordingSink::default()),
        ConnectionTimeouts {
            connect: Duration::from_secs(5),
            command: Duration::from_secs(5),
            close: Duration::from_secs(1),
        },
    ))
}

#[tokio::test]
async fn verifies_the_host_key_authenticates_and_runs_hostname_and_uptime() {
    let fixture = FixtureServer::start(false).await;
    let manager = manager();

    let error: AppError = manager
        .connect(fixture.request(false))
        .await
        .expect_err("an unknown host key must not be trusted automatically");
    assert_eq!(error.code, ErrorCode::HostKey);
    assert_eq!(fixture.authenticated(), 0);
    let snapshot = manager
        .connection_state("fixture-server")
        .expect("connection state");
    assert_eq!(snapshot.state, ConnectionState::HostKeyVerification);
    let host_key = snapshot.host_key.expect("pending host key");
    assert_eq!(host_key.key_type, fixture.host_key_type);
    assert_eq!(host_key.fingerprint, fixture.host_fingerprint);

    manager
        .approve_host_key("fixture-server", &host_key.key_type, &host_key.fingerprint)
        .expect("approve the verified host key");
    assert_eq!(
        manager
            .connection_state("fixture-server")
            .expect("connection state")
            .state,
        ConnectionState::Disconnected
    );

    let snapshot = manager
        .connect(fixture.request(true))
        .await
        .expect("connect with the trusted host key");
    assert_eq!(snapshot.state, ConnectionState::Connected);
    assert_eq!(fixture.authenticated(), 1);

    let hostname = run_remote_command(&manager, "fixture-server", RemoteCommand::Hostname)
        .await
        .expect("run hostname");
    assert_eq!(hostname.command, RemoteCommand::Hostname);
    assert_eq!(hostname.template, "hostname");
    assert_eq!(hostname.stdout, HOSTNAME_OUTPUT);
    assert_eq!(hostname.stderr, "");
    assert_eq!(hostname.exit_code, Some(0));
    assert!(hostname.succeeded);
    assert_eq!(hostname.connection_state, ConnectionState::Connected);
    println!(
        "[fixture] hostname stdout={:?} stderr={:?} exit={:?}",
        hostname.stdout, hostname.stderr, hostname.exit_code
    );

    let uptime = run_remote_command(&manager, "fixture-server", RemoteCommand::Uptime)
        .await
        .expect("run uptime");
    assert_eq!(uptime.command, RemoteCommand::Uptime);
    assert_eq!(uptime.template, "uptime");
    assert_eq!(uptime.stdout, UPTIME_OUTPUT);
    assert_eq!(uptime.stderr, "");
    assert_eq!(uptime.exit_code, Some(0));
    assert!(uptime.succeeded);
    println!(
        "[fixture] uptime stdout={:?} stderr={:?} exit={:?}",
        uptime.stdout, uptime.stderr, uptime.exit_code
    );

    assert_eq!(
        fixture.executed(),
        vec![
            "true".to_string(),
            "hostname".to_string(),
            "uptime".to_string()
        ],
        "the structured path must send exactly the fixed command templates"
    );

    let snapshot = manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    assert_eq!(snapshot.state, ConnectionState::Disconnected);
    let error = run_remote_command(&manager, "fixture-server", RemoteCommand::Hostname)
        .await
        .expect_err("the session must be released after disconnect");
    assert_eq!(error.code, ErrorCode::NotFound);
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn a_failing_command_keeps_stdout_stderr_exit_code_and_a_usable_session() {
    let fixture = FixtureServer::start(true).await;
    let manager = manager();
    manager
        .connect(fixture.request(true))
        .await
        .expect("connect with the trusted host key");

    let uptime = run_remote_command(&manager, "fixture-server", RemoteCommand::Uptime)
        .await
        .expect("a non-zero exit status is a structured result");
    assert_eq!(uptime.stdout, "");
    assert_eq!(uptime.stderr, UPTIME_FAILURE);
    assert_eq!(uptime.exit_code, Some(7));
    assert!(!uptime.succeeded);
    assert_eq!(uptime.connection_state, ConnectionState::Connected);
    println!(
        "[fixture] failing uptime stdout={:?} stderr={:?} exit={:?}",
        uptime.stdout, uptime.stderr, uptime.exit_code
    );
    assert_eq!(
        manager
            .connection_state("fixture-server")
            .expect("connection state")
            .state,
        ConnectionState::Connected
    );

    let hostname = run_remote_command(&manager, "fixture-server", RemoteCommand::Hostname)
        .await
        .expect("the session remains usable after a non-zero command");
    assert_eq!(hostname.stdout, HOSTNAME_OUTPUT);
    assert!(hostname.succeeded);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}
