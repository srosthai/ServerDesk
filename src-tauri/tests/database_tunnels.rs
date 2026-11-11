use std::{
    io,
    net::TcpListener as StdTcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use russh::{
    Channel, ChannelId, ChannelMsg, ChannelOpenFailure,
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Auth, ChannelOpenHandle, Msg, Session},
};
use serverdesk_lib::{
    core::{
        ErrorCode,
        ssh::{
            Authentication, ConnectionEventSink, ConnectionManager, ConnectionRequest,
            ConnectionSnapshot, ConnectionTimeouts, RusshTransport, TrustedHostKey,
            TunnelListenerFactory, TunnelManager, TunnelRequest, TunnelState, TunnelTimeouts,
        },
    },
    storage::{AuthType, Database, DatabaseEngine, NewDatabaseConnection, NewServer},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::Notify,
};

const USERNAME: &str = "deploy";
const PASSWORD: &str = "fixture-password";

#[derive(Default)]
struct FixtureState {
    targets: Mutex<Vec<(String, u32)>>,
    payloads: Mutex<Vec<Vec<u8>>>,
    active_channels: AtomicUsize,
    max_active_channels: AtomicUsize,
    direct_started: Notify,
}

struct FixtureServer {
    port: u16,
    host_key_type: String,
    host_fingerprint: String,
    state: Arc<FixtureState>,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    async fn start(reject_direct_tcpip: bool, direct_tcpip_delay: Duration) -> Self {
        let host_key = generate_private_key();
        let host_key_type = host_key.public_key().algorithm().as_str().to_string();
        let host_fingerprint = host_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
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
            reject_direct_tcpip,
            direct_tcpip_delay,
        ));

        Self {
            port,
            host_key_type,
            host_fingerprint,
            state,
            task,
        }
    }

    fn request(&self, server_id: &str) -> ConnectionRequest {
        ConnectionRequest {
            server_id: server_id.to_string(),
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

    fn targets(&self) -> Vec<(String, u32)> {
        self.state.targets.lock().expect("targets lock").clone()
    }

    fn payloads(&self) -> Vec<Vec<u8>> {
        self.state.payloads.lock().expect("payloads lock").clone()
    }

    fn active_channels(&self) -> usize {
        self.state.active_channels.load(Ordering::SeqCst)
    }

    fn max_active_channels(&self) -> usize {
        self.state.max_active_channels.load(Ordering::SeqCst)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_fixture_server(
    listener: tokio::net::TcpListener,
    config: Arc<server::Config>,
    state: Arc<FixtureState>,
    reject_direct_tcpip: bool,
    direct_tcpip_delay: Duration,
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let config = Arc::clone(&config);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let handler = FixtureHandler {
                state,
                reject_direct_tcpip,
                direct_tcpip_delay,
            };
            if let Ok(session) = server::run_stream(config, socket, handler).await {
                let _ = session.await;
            }
        });
    }
}

struct FixtureHandler {
    state: Arc<FixtureState>,
    reject_direct_tcpip: bool,
    direct_tcpip_delay: Duration,
}

impl server::Handler for FixtureHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, username: &str, password: &str) -> Result<Auth, Self::Error> {
        if username == USERNAME && password == PASSWORD {
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
        if data == b"true" {
            session.channel_success(channel)?;
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            session.close(channel)?;
        } else {
            session.channel_failure(channel)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        mut channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.state
            .targets
            .lock()
            .expect("targets lock")
            .push((host_to_connect.to_string(), port_to_connect));
        self.state.direct_started.notify_one();

        if !self.direct_tcpip_delay.is_zero() {
            tokio::time::sleep(self.direct_tcpip_delay).await;
        }
        if self.reject_direct_tcpip {
            reply.reject(ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        }

        reply.accept().await;
        self.state.active_channels.fetch_add(1, Ordering::SeqCst);
        self.state.max_active_channels.fetch_max(
            self.state.active_channels.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );

        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => {
                        state
                            .payloads
                            .lock()
                            .expect("payloads lock")
                            .push(data.to_vec());
                        if channel.data_bytes(data).await.is_err() {
                            break;
                        }
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            let _ = channel.eof().await;
            let _ = channel.close().await;
            state.active_channels.fetch_sub(1, Ordering::SeqCst);
        });

        Ok(())
    }
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

fn connection_manager() -> Arc<ConnectionManager> {
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

fn tunnel_timeouts() -> TunnelTimeouts {
    TunnelTimeouts {
        open: Duration::from_secs(2),
        close: Duration::from_secs(1),
    }
}

fn request(profile_id: &str, server_id: &str, port: u16) -> TunnelRequest {
    TunnelRequest {
        profile_id: profile_id.to_string(),
        server_id: server_id.to_string(),
        remote_host: "db.internal".to_string(),
        remote_port: port,
        socket: None,
    }
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    for _ in 0..100 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("condition was not met before the test timeout");
}

fn port_is_closed(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(100),
    )
    .is_err()
}

struct FailingListenerFactory;

impl TunnelListenerFactory for FailingListenerFactory {
    fn bind_loopback(&self) -> io::Result<StdTcpListener> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fixture bind failure",
        ))
    }
}

#[tokio::test]
async fn successful_tunnel_forwards_only_through_the_ssh_session() {
    let fixture = FixtureServer::start(false, Duration::ZERO).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::new(Arc::clone(&connections));

    let snapshot = tunnels
        .open(request("db-1", "fixture-server", 3306))
        .await
        .expect("open tunnel");
    assert_eq!(snapshot.state, TunnelState::Active);
    assert_eq!(snapshot.local_host, "127.0.0.1");
    assert_eq!(snapshot.remote_host, "db.internal");
    assert_eq!(snapshot.remote_port, 3306);
    let port = snapshot.local_port.expect("local port");

    let mut stream = tokio::time::timeout(
        Duration::from_secs(2),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .expect("connect timeout")
    .expect("connect forwarded listener");
    stream.write_all(b"select 1").await.expect("write payload");
    let mut echoed = vec![0_u8; 8];
    tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut echoed))
        .await
        .expect("read timeout")
        .expect("read echoed payload");
    assert_eq!(echoed, b"select 1");

    assert!(
        fixture
            .targets()
            .contains(&("db.internal".to_string(), 3306))
    );
    assert!(
        fixture
            .payloads()
            .iter()
            .any(|payload| payload == b"select 1")
    );

    tunnels.close_all().await;
    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn bind_failure_returns_a_typed_error_without_publishing_a_tunnel() {
    let fixture = FixtureServer::start(false, Duration::ZERO).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::with_options(
        Arc::clone(&connections),
        Arc::new(FailingListenerFactory),
        tunnel_timeouts(),
        4,
    );

    let error = tunnels
        .open(request("db-bind", "fixture-server", 5432))
        .await
        .expect_err("bind failure must surface");

    assert_eq!(error.code, ErrorCode::Network);
    assert!(error.message.contains("loopback"));
    assert!(tunnels.list().expect("list tunnels").is_empty());
    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn remote_refusal_returns_a_typed_error_and_leaves_no_listener() {
    let fixture = FixtureServer::start(true, Duration::ZERO).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::new(Arc::clone(&connections));

    let error = tunnels
        .open(request("db-refused", "fixture-server", 5432))
        .await
        .expect_err("remote refusal must surface");

    assert_eq!(error.code, ErrorCode::Remote);
    assert!(tunnels.list().expect("list tunnels").is_empty());
    assert!(
        fixture
            .targets()
            .contains(&("db.internal".to_string(), 5432))
    );
    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn remote_open_timeout_returns_timeout_and_cleans_up() {
    let fixture = FixtureServer::start(false, Duration::from_secs(1)).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::with_options(
        Arc::clone(&connections),
        Arc::new(serverdesk_lib::core::ssh::LoopbackTunnelListenerFactory),
        TunnelTimeouts {
            open: Duration::from_millis(100),
            close: Duration::from_secs(1),
        },
        4,
    );

    let error = tunnels
        .open(request("db-timeout", "fixture-server", 5432))
        .await
        .expect_err("remote open timeout must surface");

    assert_eq!(error.code, ErrorCode::Timeout);
    assert!(tunnels.list().expect("list tunnels").is_empty());
    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn duplicate_requests_reuse_active_tunnels_and_reject_conflicts() {
    let fixture = FixtureServer::start(false, Duration::from_millis(200)).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::new(Arc::clone(&connections));

    let first = tokio::spawn({
        let tunnels = Arc::clone(&tunnels);
        async move {
            tunnels
                .open(request("db-duplicate", "fixture-server", 5432))
                .await
        }
    });
    fixture.state.direct_started.notified().await;
    let opening_error = tunnels
        .open(request("db-duplicate", "fixture-server", 5432))
        .await
        .expect_err("an opening duplicate must be rejected deterministically");
    assert_eq!(opening_error.code, ErrorCode::Conflict);

    let first = first
        .await
        .expect("first open task")
        .expect("first tunnel opens");
    let reused = tunnels
        .open(request("db-duplicate", "fixture-server", 5432))
        .await
        .expect("duplicate reuses active tunnel");
    assert_eq!(reused.state, TunnelState::Active);
    assert_eq!(reused.local_port, first.local_port);
    assert_eq!(tunnels.list().expect("list tunnels").len(), 1);

    let conflict = tunnels
        .open(request("db-duplicate", "fixture-server", 3306))
        .await
        .expect_err("a different target must conflict");
    assert_eq!(conflict.code, ErrorCode::Conflict);
    assert_eq!(tunnels.list().expect("list tunnels").len(), 1);

    tunnels.close_all().await;
    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn tunnel_count_bound_is_enforced_and_duplicates_still_reuse() {
    let fixture = FixtureServer::start(false, Duration::ZERO).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::with_options(
        Arc::clone(&connections),
        Arc::new(serverdesk_lib::core::ssh::LoopbackTunnelListenerFactory),
        tunnel_timeouts(),
        1,
    );

    let first = tunnels
        .open(request("db-one", "fixture-server", 5432))
        .await
        .expect("first tunnel");
    let reused = tunnels
        .open(request("db-one", "fixture-server", 5432))
        .await
        .expect("duplicate reuses the active tunnel at capacity");
    assert_eq!(reused.local_port, first.local_port);

    let error = tunnels
        .open(request("db-two", "fixture-server", 5432))
        .await
        .expect_err("the tunnel bound must reject another profile");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(tunnels.list().expect("list tunnels").len(), 1);

    tunnels.close_all().await;
    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn ssh_disconnect_closes_tunnel_listener_and_forwarded_channels() {
    let fixture = FixtureServer::start(false, Duration::ZERO).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::new(Arc::clone(&connections));

    let snapshot = tunnels
        .open(request("db-disconnect", "fixture-server", 5432))
        .await
        .expect("open tunnel");
    let port = snapshot.local_port.expect("local port");

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect tunnel");
    stream.write_all(b"ping").await.expect("write ping");
    let mut echoed = vec![0_u8; 4];
    stream.read_exact(&mut echoed).await.expect("read pong");
    assert_eq!(echoed, b"ping");
    wait_until(|| fixture.active_channels() == 1).await;

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect server");

    wait_until(|| tunnels.snapshot("db-disconnect").expect("state").is_none()).await;
    wait_until(|| fixture.active_channels() == 0).await;
    assert!(port_is_closed(port));
}

#[tokio::test]
async fn application_exit_closes_every_tunnel_and_listener() {
    let fixture = FixtureServer::start(false, Duration::ZERO).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::new(Arc::clone(&connections));

    let first = tunnels
        .open(request("db-a", "fixture-server", 3306))
        .await
        .expect("first tunnel");
    let second = tunnels
        .open(request("db-b", "fixture-server", 5432))
        .await
        .expect("second tunnel");
    let first_port = first.local_port.expect("first port");
    let second_port = second.local_port.expect("second port");
    assert_eq!(tunnels.list().expect("list tunnels").len(), 2);

    // This is the same ordering used by release_connections on app exit.
    tunnels.close_all().await;
    connections.disconnect_all().await;

    assert!(tunnels.list().expect("list tunnels").is_empty());
    assert!(port_is_closed(first_port));
    assert!(port_is_closed(second_port));
    wait_until(|| fixture.active_channels() == 0).await;
}

#[tokio::test]
async fn profile_deletion_closes_the_tunnel_before_removing_metadata() {
    let fixture = FixtureServer::start(false, Duration::ZERO).await;
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&NewServer {
            name: "Fixture".to_string(),
            host: "127.0.0.1".to_string(),
            port: fixture.port,
            username: USERNAME.to_string(),
            auth_type: AuthType::Password,
            credential_ref: Some("serverdesk:ssh:fixture".to_string()),
            group_name: None,
            tags: Vec::new(),
            notes: None,
        })
        .expect("create server");
    let profile = database
        .create_database_connection(&NewDatabaseConnection {
            server_id: server.id.clone(),
            name: "Primary".to_string(),
            engine: DatabaseEngine::Postgresql,
            host: "db.internal".to_string(),
            port: 5432,
            socket: None,
            database_name: Some("app".to_string()),
            username: "app".to_string(),
            credential_ref: Some("serverdesk:db:fixture".to_string()),
            read_only: false,
            ssl_mode: None,
        })
        .expect("create database profile");

    let connections = connection_manager();
    connections
        .connect(fixture.request(&profile.server_id))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::new(Arc::clone(&connections));
    tunnels
        .open(TunnelRequest {
            profile_id: profile.id.clone(),
            server_id: profile.server_id.clone(),
            remote_host: profile.host.clone(),
            remote_port: profile.port,
            socket: profile.socket.clone(),
        })
        .await
        .expect("open profile tunnel");

    tunnels
        .close(&profile.id)
        .await
        .expect("close profile tunnel");
    database
        .delete_database_connection(&profile.id)
        .expect("delete profile");

    assert!(
        database
            .get_database_connection(&profile.id)
            .expect("read deleted profile")
            .is_none()
    );
    assert!(tunnels.snapshot(&profile.id).expect("state").is_none());
    connections.disconnect_all().await;
}

#[tokio::test]
async fn host_key_and_authentication_errors_precede_tunnel_setup() {
    let fixture = FixtureServer::start(false, Duration::ZERO).await;
    let connections = connection_manager();
    let tunnels = TunnelManager::new(Arc::clone(&connections));

    let mut untrusted = fixture.request("fixture-server");
    untrusted.trusted_host_keys.clear();
    let host_key_error = connections
        .connect(untrusted)
        .await
        .expect_err("an untrusted host key must fail");
    assert_eq!(host_key_error.code, ErrorCode::HostKey);

    let mut wrong_password = fixture.request("fixture-server");
    wrong_password.authentication = Authentication::Password("wrong-password".to_string());
    let authentication_error = connections
        .connect(wrong_password)
        .await
        .expect_err("wrong SSH credentials must fail");
    assert_eq!(authentication_error.code, ErrorCode::Authentication);

    let tunnel_error = tunnels
        .open(request("db-auth", "fixture-server", 5432))
        .await
        .expect_err("no tunnel may open after SSH authentication fails");
    assert_eq!(tunnel_error.code, ErrorCode::Connection);
    assert!(tunnels.list().expect("list tunnels").is_empty());
}

#[tokio::test]
async fn tunnel_open_requires_an_authenticated_session() {
    let connections = connection_manager();
    let tunnels = TunnelManager::new(Arc::clone(&connections));

    let error = tunnels
        .open(request("db-offline", "missing-server", 5432))
        .await
        .expect_err("no session must fail before any database client starts");

    assert_eq!(error.code, ErrorCode::Connection);
    assert!(tunnels.list().expect("list tunnels").is_empty());
}

#[tokio::test]
async fn forwarded_client_concurrency_is_bounded() {
    let fixture = FixtureServer::start(false, Duration::ZERO).await;
    let connections = connection_manager();
    connections
        .connect(fixture.request("fixture-server"))
        .await
        .expect("connect fixture");
    let tunnels = TunnelManager::new(Arc::clone(&connections));
    let snapshot = tunnels
        .open(request("db-bounded", "fixture-server", 5432))
        .await
        .expect("open tunnel");
    let port = snapshot.local_port.expect("local port");

    let mut clients = Vec::new();
    for _ in 0..12 {
        clients.push(
            TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("connect forwarded listener"),
        );
    }
    for client in &mut clients {
        client.write_all(b"x").await.expect("write payload");
    }
    wait_until(|| fixture.max_active_channels() >= 8).await;
    assert!(fixture.max_active_channels() <= 8);

    tunnels.close_all().await;
    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}
