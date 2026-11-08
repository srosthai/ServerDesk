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
    ErrorCode,
    services::{
        ServiceAction, ServiceState, inspect_service_status, run_service_action,
        validate_service_unit_name,
    },
    ssh::{
        Authentication, ConnectionEventSink, ConnectionManager, ConnectionRequest,
        ConnectionSnapshot, ConnectionState, ConnectionTimeouts, RusshTransport, TrustedHostKey,
    },
};
use tokio::net::TcpListener;

const USERNAME: &str = "deploy";
const PASSWORD: &str = "fixture-password";
const UNIT: &str = "nginx.service";
const STATUS_DESCRIPTION: &str = "A high performance web server";

#[derive(Clone, Copy, PartialEq, Eq)]
enum FixtureMode {
    Normal,
    DenySudo,
    FailAction,
}

#[derive(Default)]
struct FixtureState {
    authenticated: AtomicUsize,
    executed: Mutex<Vec<String>>,
    service_active: Mutex<bool>,
}

struct FixtureServer {
    port: u16,
    host_key_type: String,
    host_fingerprint: String,
    state: Arc<FixtureState>,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    async fn start(mode: FixtureMode, active: bool) -> Self {
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
        let state = Arc::new(FixtureState {
            authenticated: AtomicUsize::new(0),
            executed: Mutex::new(Vec::new()),
            service_active: Mutex::new(active),
        });
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
            mode,
        ));

        Self {
            port,
            host_key_type,
            host_fingerprint,
            state,
            task,
        }
    }

    fn request(&self) -> ConnectionRequest {
        ConnectionRequest {
            server_id: "fixture-server".to_string(),
            host: "127.0.0.1".to_string(),
            port: self.port,
            username: USERNAME.to_string(),
            authentication: Authentication::Password(PASSWORD.to_string()),
            trusted_host_keys: vec![TrustedHostKey {
                key_type: self.host_key_type.clone(),
                fingerprint: self.host_fingerprint.clone(),
            }],
        }
    }

    fn executed(&self) -> Vec<String> {
        self.state.executed.lock().expect("executed lock").clone()
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_fixture_server(
    listener: TcpListener,
    config: Arc<server::Config>,
    state: Arc<FixtureState>,
    mode: FixtureMode,
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let config = Arc::clone(&config);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let handler = FixtureHandler { state, mode };
            if let Ok(session) = server::run_stream(config, socket, handler).await {
                let _ = session.await;
            }
        });
    }
}

struct FixtureHandler {
    state: Arc<FixtureState>,
    mode: FixtureMode,
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

        if command == "true" {
            return respond(session, channel, b"", b"", 0);
        }

        let unit = validate_service_unit_name(UNIT).expect("fixture unit");
        let status_command = ServiceAction::Status.command(&unit);
        if command == status_command {
            let active = *self.state.service_active.lock().expect("service lock");
            let status = if active {
                format!(
                    "Id={UNIT}\nLoadState=loaded\nActiveState=active\nSubState=running\n\
                     UnitFileState=enabled\nDescription={STATUS_DESCRIPTION}\n"
                )
            } else {
                format!(
                    "Id={UNIT}\nLoadState=loaded\nActiveState=inactive\nSubState=dead\n\
                     UnitFileState=enabled\nDescription={STATUS_DESCRIPTION}\n"
                )
            };
            return respond(session, channel, status.as_bytes(), b"", 0);
        }

        if self.mode == FixtureMode::DenySudo {
            return respond(
                session,
                channel,
                b"",
                b"sudo: a password is required\nsudo: no tty present and no askpass program specified\n",
                1,
            );
        }

        if command == ServiceAction::Start.command(&unit) {
            if self.mode == FixtureMode::FailAction {
                return respond(
                    session,
                    channel,
                    b"",
                    b"Job for nginx.service failed because the control process exited with error code.\n",
                    5,
                );
            }
            *self.state.service_active.lock().expect("service lock") = true;
            return respond(session, channel, b"started\n", b"", 0);
        }

        if command == ServiceAction::Stop.command(&unit) {
            *self.state.service_active.lock().expect("service lock") = false;
            return respond(session, channel, b"stopped\n", b"", 0);
        }

        if command == ServiceAction::Restart.command(&unit) {
            *self.state.service_active.lock().expect("service lock") = true;
            return respond(session, channel, b"restarted\n", b"", 0);
        }

        session.channel_failure(channel)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
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

async fn connect(fixture: &FixtureServer) -> Arc<ConnectionManager> {
    let manager = manager();
    manager
        .connect(fixture.request())
        .await
        .expect("connect with the trusted host key");
    manager
}

fn status_command() -> String {
    ServiceAction::Status.command(&validate_service_unit_name(UNIT).expect("fixture unit"))
}

#[tokio::test]
async fn start_stop_and_restart_change_state_and_verify_each_action() {
    let fixture = FixtureServer::start(FixtureMode::Normal, false).await;
    let manager = connect(&fixture).await;

    let initial = inspect_service_status(&manager, "fixture-server", UNIT)
        .await
        .expect("inspect the initial state");
    assert_eq!(initial.state, ServiceState::Inactive);
    assert_eq!(initial.unit_file_state.as_deref(), Some("enabled"));

    let started = run_service_action(&manager, "fixture-server", UNIT, ServiceAction::Start)
        .await
        .expect("start the unit");
    assert!(started.succeeded);
    assert_eq!(started.action, ServiceAction::Start);
    assert_eq!(
        started.command,
        ServiceAction::Start.command(&validate_service_unit_name(UNIT).unwrap())
    );
    assert_eq!(started.stdout, "started\n");
    assert_eq!(started.stderr, "");
    assert_eq!(started.exit_code, Some(0));
    assert!(started.error.is_none());
    assert!(!started.permission_denied);
    assert_eq!(
        started.verified_status.as_ref().map(|status| status.state),
        Some(ServiceState::Active)
    );
    assert_eq!(
        started
            .verified_status
            .as_ref()
            .and_then(|status| status.sub_state.as_deref()),
        Some("running")
    );
    assert_eq!(started.state_verified, Some(true));
    assert_eq!(started.connection_state, ConnectionState::Connected);

    let stopped = run_service_action(&manager, "fixture-server", UNIT, ServiceAction::Stop)
        .await
        .expect("stop the unit");
    assert!(stopped.succeeded);
    assert_eq!(stopped.stdout, "stopped\n");
    assert_eq!(
        stopped.verified_status.as_ref().map(|status| status.state),
        Some(ServiceState::Inactive)
    );
    assert_eq!(stopped.state_verified, Some(true));

    let restarted = run_service_action(&manager, "fixture-server", UNIT, ServiceAction::Restart)
        .await
        .expect("restart the unit");
    assert!(restarted.succeeded);
    assert_eq!(restarted.stdout, "restarted\n");
    assert_eq!(
        restarted
            .verified_status
            .as_ref()
            .map(|status| status.state),
        Some(ServiceState::Active)
    );
    assert_eq!(restarted.state_verified, Some(true));

    let inspected = run_service_action(&manager, "fixture-server", UNIT, ServiceAction::Status)
        .await
        .expect("inspect through the typed action");
    assert!(inspected.succeeded);
    assert_eq!(inspected.action, ServiceAction::Status);
    assert_eq!(inspected.state_verified, None);
    assert_eq!(
        inspected
            .verified_status
            .as_ref()
            .map(|status| status.state),
        Some(ServiceState::Active)
    );

    assert_eq!(
        fixture.executed(),
        vec![
            "true".to_string(),
            status_command(),
            ServiceAction::Start.command(&validate_service_unit_name(UNIT).unwrap()),
            status_command(),
            ServiceAction::Stop.command(&validate_service_unit_name(UNIT).unwrap()),
            status_command(),
            ServiceAction::Restart.command(&validate_service_unit_name(UNIT).unwrap()),
            status_command(),
            status_command(),
        ]
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn permission_denial_is_a_typed_error_with_scoped_sudo_guidance_and_output() {
    let fixture = FixtureServer::start(FixtureMode::DenySudo, false).await;
    let manager = connect(&fixture).await;

    let denied = run_service_action(&manager, "fixture-server", UNIT, ServiceAction::Start)
        .await
        .expect("command-level denial is a structured result");

    assert!(!denied.succeeded);
    assert!(denied.permission_denied);
    assert_eq!(denied.exit_code, Some(1));
    assert_eq!(
        denied.stderr,
        "sudo: a password is required\nsudo: no tty present and no askpass program specified\n"
    );
    assert_eq!(denied.stdout, "");
    let error = denied.error.as_ref().expect("permission error");
    assert_eq!(error.code, ErrorCode::Permission);
    assert!(error.message.contains("narrowly scoped"));
    assert!(error.message.contains("nginx.service"));
    let details = error.details.as_deref().expect("guidance details");
    assert!(details.contains("NOPASSWD"));
    assert!(details.contains("/usr/bin/systemctl start -- nginx.service"));
    assert!(details.contains("sudo: a password is required"));
    assert!(details.contains("Exit code: 1"));
    let guidance = denied.sudo_guidance.as_deref().expect("sudo guidance");
    assert!(guidance.contains("NOPASSWD"));
    assert!(guidance.contains("Never grant ALL=(ALL) NOPASSWD: ALL"));

    assert_eq!(
        denied.verified_status.as_ref().map(|status| status.state),
        Some(ServiceState::Inactive),
        "the unit must be re-queried after a denied action"
    );
    assert_eq!(denied.state_verified, Some(false));
    assert_eq!(
        fixture.executed(),
        vec![
            "true".to_string(),
            ServiceAction::Start.command(&validate_service_unit_name(UNIT).unwrap()),
            status_command(),
        ]
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn failed_action_preserves_output_exit_status_and_verified_state() {
    let fixture = FixtureServer::start(FixtureMode::FailAction, false).await;
    let manager = connect(&fixture).await;

    let failed = run_service_action(&manager, "fixture-server", UNIT, ServiceAction::Start)
        .await
        .expect("command-level failure is a structured result");

    assert!(!failed.succeeded);
    assert!(!failed.permission_denied);
    assert_eq!(failed.exit_code, Some(5));
    assert_eq!(failed.stdout, "");
    assert!(failed.stderr.contains("Job for nginx.service failed"));
    let error = failed.error.as_ref().expect("failure error");
    assert_eq!(error.code, ErrorCode::Remote);
    assert!(error.message.contains("exit code 5"));
    let details = error.details.as_deref().expect("output details");
    assert!(details.contains("Job for nginx.service failed"));
    assert!(details.contains("Exit code: 5"));
    assert_eq!(
        failed.verified_status.as_ref().map(|status| status.state),
        Some(ServiceState::Inactive)
    );
    assert_eq!(failed.state_verified, Some(false));

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn invalid_unit_names_are_rejected_before_any_remote_command() {
    let fixture = FixtureServer::start(FixtureMode::Normal, false).await;
    let manager = connect(&fixture).await;

    for invalid in [
        "nginx.service; id",
        "nginx.service --force",
        "../nginx.service",
        "nginx.service\n",
        "nginx.socket",
        "$(reboot).service",
    ] {
        let error = run_service_action(&manager, "fixture-server", invalid, ServiceAction::Start)
            .await
            .expect_err("invalid units must never reach the server");
        assert_eq!(error.code, ErrorCode::Validation, "{invalid:?}");
    }

    assert_eq!(
        fixture.executed(),
        vec!["true".to_string()],
        "only the connection check may run before validation fails"
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}
