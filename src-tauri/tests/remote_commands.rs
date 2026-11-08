use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Auth, ChannelOpenHandle, Msg, Session},
};
use serverdesk_lib::core::{
    AppError, ErrorCode,
    identity::sample_system_identity,
    monitoring::{MONITORING_COMMANDS, MetricStatus, sample_monitoring_snapshot},
    remote::{RemoteCommand, run_remote_command},
    services::{
        SYSTEMD_SERVICES_COMMAND, ServiceDiscoveryStatus, ServiceState, discover_systemd_services,
    },
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
const OS_RELEASE_OUTPUT: &str = r#"PRETTY_NAME="Ubuntu 24.04.1 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04.1 LTS (Noble Numbat)"
VERSION_CODENAME=noble
ID=ubuntu
ID_LIKE=debian
HOME_URL="https://www.ubuntu.com/"
SUPPORT_URL="https://help.ubuntu.com/"
BUG_REPORT_URL="https://bugs.launchpad.net/ubuntu/"
"#;
const UNAME_OUTPUT: &str = "Linux 6.8.0-45-generic x86_64\n";
const PROC_UPTIME_OUTPUT: &str = "372063.12 1234567.89\n";
const OS_RELEASE_MISSING: &str = "cat: /etc/os-release: No such file or directory\n";
const LOADAVG_OUTPUT: &str = "0.42 0.31 0.28 2/512 123456\n";
const NPROC_OUTPUT: &str = "8\n";
const MEMINFO_OUTPUT: &str = r#"MemTotal:       16316172 kB
MemFree:          845624 kB
MemAvailable:    9247520 kB
Buffers:          123456 kB
Cached:          5678901 kB
"#;
const DF_OUTPUT: &str = r#"Filesystem     1024-blocks      Used Available Capacity Mounted on
/dev/vda1         41020672  12345678  26566840      32% /
/dev/vdb1        103080624  23456789  74345678      24% /mnt/data
"#;
const NETDEV_OUTPUT: &str = r#"Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo:    1000      10    0    0    0     0          0         0     1000      10    0    0    0     0       0          0
  eth0: 123456789   98765    0    0    0     0          0         0 987654321   54321    0    0    0     0       0          0
"#;
const SYSTEMD_UNITS_OUTPUT: &str = r#"  nginx.service               loaded    active   running    A high performance web server
  apache2.service             loaded    failed   failed     The Apache HTTP Server
  mysql.service               loaded    inactive dead       MySQL Community Server
  mariadb.service             loaded    inactive dead       MariaDB database server
  postgresql.service          loaded    active   exited     PostgreSQL RDBMS
  redis-server.service        loaded    activating start     Redis persistent key-value database
  supervisor.service          loaded    inactive dead       Supervisor process control system for UNIX
  php8.3-fpm.service          loaded    active   running    The PHP 8.3 FastCGI Process Manager
  custom-worker.service       loaded    active   running    A service that is not in any common list
"#;
const SYSTEMD_MISSING: &str =
    "System has not been booted with systemd as init system (PID 1). Can't operate.\n";

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
        Self::start_with(fail_uptime, true, true).await
    }

    async fn start_without_os_release() -> Self {
        Self::start_with(false, false, true).await
    }

    async fn start_without_systemd() -> Self {
        Self::start_with(false, true, false).await
    }

    async fn start_with(fail_uptime: bool, has_os_release: bool, has_systemd: bool) -> Self {
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
            has_os_release,
            has_systemd,
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
    has_os_release: bool,
    has_systemd: bool,
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
            let handler = FixtureHandler {
                state,
                fail_uptime,
                has_os_release,
                has_systemd,
            };
            if let Ok(session) = server::run_stream(config, socket, handler).await {
                let _ = session.await;
            }
        });
    }
}

struct FixtureHandler {
    state: Arc<FixtureState>,
    fail_uptime: bool,
    has_os_release: bool,
    has_systemd: bool,
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
            "cat /etc/os-release" if self.has_os_release => {
                respond(session, channel, OS_RELEASE_OUTPUT.as_bytes(), b"", 0)
            }
            "cat /etc/os-release" => {
                respond(session, channel, b"", OS_RELEASE_MISSING.as_bytes(), 1)
            }
            "uname -srm" => respond(session, channel, UNAME_OUTPUT.as_bytes(), b"", 0),
            "cat /proc/uptime" => respond(session, channel, PROC_UPTIME_OUTPUT.as_bytes(), b"", 0),
            "cat /proc/loadavg" => respond(session, channel, LOADAVG_OUTPUT.as_bytes(), b"", 0),
            "nproc" => respond(session, channel, NPROC_OUTPUT.as_bytes(), b"", 0),
            "cat /proc/meminfo" => respond(session, channel, MEMINFO_OUTPUT.as_bytes(), b"", 0),
            "df -Pk" => respond(session, channel, DF_OUTPUT.as_bytes(), b"", 0),
            "cat /proc/net/dev" => respond(session, channel, NETDEV_OUTPUT.as_bytes(), b"", 0),
            command if command == SYSTEMD_SERVICES_COMMAND && self.has_systemd => {
                respond(session, channel, SYSTEMD_UNITS_OUTPUT.as_bytes(), b"", 0)
            }
            command if command == SYSTEMD_SERVICES_COMMAND => {
                respond(session, channel, b"", SYSTEMD_MISSING.as_bytes(), 1)
            }
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

#[tokio::test]
async fn discovers_system_identity_over_the_authenticated_session() {
    let fixture = FixtureServer::start(false).await;
    let manager = manager();
    manager
        .connect(fixture.request(true))
        .await
        .expect("connect with the trusted host key");

    let identity = sample_system_identity(&manager, "fixture-server")
        .await
        .expect("sample identity");

    assert_eq!(identity.server_id, "fixture-server");
    assert_eq!(identity.hostname.as_deref(), Some("fixture-host"));
    assert_eq!(identity.distribution.as_deref(), Some("Ubuntu"));
    assert_eq!(identity.distribution_version.as_deref(), Some("24.04"));
    assert_eq!(identity.kernel.as_deref(), Some("6.8.0-45-generic"));
    assert_eq!(identity.architecture.as_deref(), Some("x86_64"));
    assert_eq!(identity.uptime_seconds, Some(372_063));
    assert_eq!(identity.connection_state, ConnectionState::Connected);
    assert!(identity.sampled_at > 0);
    println!(
        "[fixture] identity hostname={:?} distribution={:?} version={:?} kernel={:?} arch={:?} uptime={:?}",
        identity.hostname,
        identity.distribution,
        identity.distribution_version,
        identity.kernel,
        identity.architecture,
        identity.uptime_seconds
    );

    assert_eq!(
        fixture.executed(),
        vec![
            "true".to_string(),
            "hostname".to_string(),
            "cat /etc/os-release".to_string(),
            "uname -srm".to_string(),
            "cat /proc/uptime".to_string(),
        ],
        "identity discovery must send exactly the fixed templates"
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn collects_monitoring_snapshot_with_samples_and_counters() {
    let fixture = FixtureServer::start(false).await;
    let manager = manager();
    manager
        .connect(fixture.request(true))
        .await
        .expect("connect with the trusted host key");

    let started = Instant::now();
    let snapshot = sample_monitoring_snapshot(&manager, "fixture-server")
        .await
        .expect("sample monitoring");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a fixture snapshot must complete within the command timeout"
    );

    assert_eq!(snapshot.server_id, "fixture-server");
    assert_eq!(snapshot.connection_state, ConnectionState::Connected);
    assert!(snapshot.sampled_at > 0);

    assert_eq!(snapshot.cpu.status, MetricStatus::Available);
    let cpu = snapshot.cpu.data.as_ref().expect("CPU data");
    assert!((cpu.load_1m - 0.42).abs() < f64::EPSILON);
    assert!((cpu.load_5m - 0.31).abs() < f64::EPSILON);
    assert!((cpu.load_15m - 0.28).abs() < f64::EPSILON);
    assert_eq!(cpu.logical_cpus, Some(8));

    assert_eq!(snapshot.memory.status, MetricStatus::Available);
    let memory = snapshot.memory.data.as_ref().expect("memory data");
    assert_eq!(memory.total_bytes, 16_316_172 * 1024);
    assert_eq!(memory.available_bytes, 9_247_520 * 1024);
    assert_eq!(
        memory.used_bytes,
        memory.total_bytes - memory.available_bytes
    );

    assert_eq!(snapshot.filesystems.status, MetricStatus::Available);
    let filesystems = snapshot.filesystems.data.as_ref().expect("filesystem data");
    assert_eq!(filesystems.len(), 2);
    assert_eq!(filesystems[0].mount, "/");
    assert_eq!(filesystems[1].mount, "/mnt/data");

    assert_eq!(snapshot.network.status, MetricStatus::Available);
    let network = snapshot.network.data.as_ref().expect("network data");
    assert_eq!(network.len(), 2);
    let eth0 = network
        .iter()
        .find(|interface| interface.interface == "eth0")
        .expect("eth0");
    assert_eq!(eth0.received_bytes, 123_456_789);
    assert_eq!(eth0.transmitted_bytes, 987_654_321);

    for group_time in [
        snapshot.cpu.sampled_at,
        snapshot.memory.sampled_at,
        snapshot.filesystems.sampled_at,
        snapshot.network.sampled_at,
    ] {
        assert!(group_time > 0);
        assert!(group_time <= snapshot.sampled_at);
    }

    let mut expected = vec!["true".to_string()];
    expected.extend(
        MONITORING_COMMANDS
            .iter()
            .map(|command| command.to_string()),
    );
    assert_eq!(
        fixture.executed(),
        expected,
        "monitoring must send exactly the fixed command templates"
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn unsupported_distribution_returns_partial_identity() {
    let fixture = FixtureServer::start_without_os_release().await;
    let manager = manager();
    manager
        .connect(fixture.request(true))
        .await
        .expect("connect with the trusted host key");

    let identity = sample_system_identity(&manager, "fixture-server")
        .await
        .expect("partial identity is still a successful sample");

    assert_eq!(identity.hostname.as_deref(), Some("fixture-host"));
    assert_eq!(identity.distribution, None);
    assert_eq!(identity.distribution_version, None);
    assert_eq!(identity.kernel.as_deref(), Some("6.8.0-45-generic"));
    assert_eq!(identity.architecture.as_deref(), Some("x86_64"));
    assert_eq!(identity.uptime_seconds, Some(372_063));
    assert_eq!(identity.connection_state, ConnectionState::Connected);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn discovers_systemd_units_and_distinguishes_their_states() {
    let fixture = FixtureServer::start(false).await;
    let manager = manager();
    manager
        .connect(fixture.request(true))
        .await
        .expect("connect with the trusted host key");

    let discovery = discover_systemd_services(&manager, "fixture-server")
        .await
        .expect("discover services");

    assert_eq!(discovery.server_id, "fixture-server");
    assert_eq!(discovery.status, ServiceDiscoveryStatus::Available);
    assert_eq!(discovery.connection_state, ConnectionState::Connected);
    assert!(!discovery.truncated);
    assert!(discovery.sampled_at > 0);
    assert_eq!(discovery.services.len(), 9);
    assert!(discovery.detail.is_none());

    let service = |name: &str| {
        discovery
            .services
            .iter()
            .find(|service| service.name == name)
            .unwrap_or_else(|| panic!("missing fixture unit {name}"))
    };

    assert_eq!(service("nginx.service").state, ServiceState::Active);
    assert_eq!(service("apache2.service").state, ServiceState::Failed);
    assert_eq!(service("mysql.service").state, ServiceState::Inactive);
    assert_eq!(service("mariadb.service").state, ServiceState::Inactive);
    assert_eq!(service("postgresql.service").state, ServiceState::Active);
    assert_eq!(service("redis-server.service").state, ServiceState::Unknown);
    assert_eq!(service("supervisor.service").state, ServiceState::Inactive);
    assert_eq!(service("php8.3-fpm.service").state, ServiceState::Active);
    assert_eq!(service("custom-worker.service").state, ServiceState::Active);
    assert_ne!(
        service("apache2.service").state,
        service("redis-server.service").state,
        "failed and unknown states must stay distinct"
    );

    assert_eq!(
        fixture.executed(),
        vec!["true".to_string(), SYSTEMD_SERVICES_COMMAND.to_string()],
        "discovery must send exactly the fixed command template"
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn systems_without_systemd_return_a_typed_unsupported_result() {
    let fixture = FixtureServer::start_without_systemd().await;
    let manager = manager();
    manager
        .connect(fixture.request(true))
        .await
        .expect("connect with the trusted host key");

    let discovery = discover_systemd_services(&manager, "fixture-server")
        .await
        .expect("an unsupported system is a typed result, not a transport failure");

    assert_eq!(discovery.status, ServiceDiscoveryStatus::Unsupported);
    assert_eq!(discovery.connection_state, ConnectionState::Connected);
    assert!(discovery.services.is_empty());
    assert!(!discovery.truncated);
    let detail = discovery.detail.expect("unsupported detail");
    assert!(detail.contains("Systemd is not available"));
    assert!(detail.contains("not been booted with systemd"));

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}
