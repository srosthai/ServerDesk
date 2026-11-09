use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use russh::{
    Channel, ChannelId, Pty,
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Auth, ChannelOpenHandle, Msg, Session},
};
use serverdesk_lib::core::{
    ssh::{
        Authentication, ConnectionEventSink, ConnectionManager, ConnectionRequest,
        ConnectionSnapshot, ConnectionTimeouts, RusshTransport, TrustedHostKey,
    },
    terminal::{
        TerminalEvent, TerminalEventSink, TerminalExitReason, TerminalManager, TerminalOptions,
        TerminalSize,
    },
};
use tokio::{net::TcpListener, sync::Notify};

const USERNAME: &str = "deploy";
const PASSWORD: &str = "fixture-password";
const ANSI_BANNER: &str = "\x1b[32mfixture-ready\x1b[0m\r\n";
const EXEC_OUTPUT: &str = "fixture-exec\n";

#[derive(Debug, Default)]
struct FixtureChannel {
    term: String,
    cols: u32,
    rows: u32,
    input: Vec<u8>,
    shell: bool,
    closed: bool,
}

#[derive(Default)]
struct FixtureState {
    authenticated: AtomicUsize,
    pty_requests: AtomicUsize,
    shell_requests: AtomicUsize,
    closed_shell_channels: AtomicUsize,
    active_connections: AtomicUsize,
    channels: Mutex<HashMap<u32, FixtureChannel>>,
    sizes: Mutex<Vec<(u32, u32)>>,
}

impl FixtureState {
    fn mark_shell_closed(&self, channel: ChannelId) {
        let mut channels = self.channels.lock().expect("channels lock");
        if let Some(entry) = channels.get_mut(&channel.number())
            && entry.shell
            && !entry.closed
        {
            entry.closed = true;
            self.closed_shell_channels.fetch_add(1, Ordering::SeqCst);
        }
    }
}

struct FixtureServer {
    port: u16,
    host_key_type: String,
    host_fingerprint: String,
    state: Arc<FixtureState>,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    async fn start() -> Self {
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
        let task = tokio::spawn(run_fixture_server(listener, config, Arc::clone(&state)));

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

    fn pty_requests(&self) -> usize {
        self.state.pty_requests.load(Ordering::SeqCst)
    }

    fn shell_requests(&self) -> usize {
        self.state.shell_requests.load(Ordering::SeqCst)
    }

    fn closed_shell_channels(&self) -> usize {
        self.state.closed_shell_channels.load(Ordering::SeqCst)
    }

    fn channels(&self) -> Vec<(u32, FixtureChannel)> {
        let mut channels = self
            .state
            .channels
            .lock()
            .expect("channels lock")
            .iter()
            .filter(|(_, channel)| channel.shell)
            .map(|(id, channel)| {
                (
                    *id,
                    FixtureChannel {
                        term: channel.term.clone(),
                        cols: channel.cols,
                        rows: channel.rows,
                        input: channel.input.clone(),
                        shell: channel.shell,
                        closed: channel.closed,
                    },
                )
            })
            .collect::<Vec<_>>();
        channels.sort_by_key(|(id, _)| *id);
        channels
    }

    fn sizes(&self) -> Vec<(u32, u32)> {
        self.state.sizes.lock().expect("sizes lock").clone()
    }

    async fn wait_for_closed_channels(&self, expected: usize) {
        for _ in 0..200 {
            if self.closed_shell_channels() >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            self.closed_shell_channels(),
            expected,
            "the fixture did not observe every closed shell channel"
        );
    }

    async fn wait_for_no_connections(&self) {
        for _ in 0..200 {
            if self.state.active_connections.load(Ordering::SeqCst) == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            self.state.active_connections.load(Ordering::SeqCst),
            0,
            "the fixture still has an active SSH connection"
        );
    }

    async fn wait_for_size(&self, expected: (u32, u32)) {
        for _ in 0..200 {
            if self.sizes().contains(&expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "the fixture did not receive the resize {:?}; observed {:?}",
            expected,
            self.sizes()
        );
    }

    async fn wait_for_input(&self, expected: &[u8]) {
        for _ in 0..200 {
            if self.channels().iter().any(|(_, channel)| {
                channel
                    .input
                    .windows(expected.len())
                    .any(|part| part == expected)
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "the fixture did not receive the expected input; observed {:?}",
            self.channels()
        );
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
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let config = Arc::clone(&config);
        let state = Arc::clone(&state);
        state.active_connections.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            let _guard = ConnectionGuard(Arc::clone(&state));
            let handler = FixtureHandler { state };
            if let Ok(session) = server::run_stream(config, socket, handler).await {
                let _ = session.await;
            }
        });
    }
}

struct ConnectionGuard(Arc<FixtureState>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.active_connections.fetch_sub(1, Ordering::SeqCst);
    }
}

struct FixtureHandler {
    state: Arc<FixtureState>,
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
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.state
            .channels
            .lock()
            .expect("channels lock")
            .insert(channel.id().number(), FixtureChannel::default());
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.state.pty_requests.fetch_add(1, Ordering::SeqCst);
        if let Some(entry) = self
            .state
            .channels
            .lock()
            .expect("channels lock")
            .get_mut(&channel.number())
        {
            entry.shell = true;
            entry.term = term.to_string();
            entry.cols = col_width;
            entry.rows = row_height;
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.state.shell_requests.fetch_add(1, Ordering::SeqCst);
        session.channel_success(channel)?;
        session.data(channel, ANSI_BANNER.as_bytes().to_vec())?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.state
            .sizes
            .lock()
            .expect("sizes lock")
            .push((col_width, row_height));
        if let Some(entry) = self
            .state
            .channels
            .lock()
            .expect("channels lock")
            .get_mut(&channel.number())
        {
            entry.cols = col_width;
            entry.rows = row_height;
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(entry) = self
            .state
            .channels
            .lock()
            .expect("channels lock")
            .get_mut(&channel.number())
        {
            entry.input.extend_from_slice(data);
        }

        if data.windows(4).any(|part| part == b"exit") {
            self.state.mark_shell_closed(channel);
            session.exit_status_request(channel, 7)?;
            session.eof(channel)?;
            session.close(channel)?;
        } else {
            session.data(
                channel,
                format!("ack:{}\r\n", channel.number()).into_bytes(),
            )?;
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        session.data(channel, EXEC_OUTPUT.as_bytes().to_vec())?;
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.state.mark_shell_closed(channel);
        Ok(())
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<TerminalEvent>>,
    notify: Notify,
}

impl RecordingSink {
    fn events(&self) -> Vec<TerminalEvent> {
        self.events.lock().expect("event lock").clone()
    }

    fn session_events(&self, session_id: &str) -> Vec<TerminalEvent> {
        self.events()
            .into_iter()
            .filter(|event| match event {
                TerminalEvent::Output { session_id: id, .. }
                | TerminalEvent::Exited { session_id: id, .. } => id == session_id,
            })
            .collect()
    }

    async fn wait_for_event<F>(&self, predicate: F) -> TerminalEvent
    where
        F: Fn(&TerminalEvent) -> bool,
    {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.notify.notified();
                if let Some(event) = self
                    .events
                    .lock()
                    .expect("event lock")
                    .iter()
                    .find(|event| predicate(event))
                    .cloned()
                {
                    return event;
                }
                notified.await;
            }
        })
        .await
        .expect("terminal event")
    }

    async fn wait_for_output(&self, session_id: &str, expected: &[u8]) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.notify.notified();
                let output = self
                    .session_events(session_id)
                    .into_iter()
                    .filter_map(|event| match event {
                        TerminalEvent::Output { data, .. } => {
                            Some(STANDARD.decode(data).expect("base64 output"))
                        }
                        TerminalEvent::Exited { .. } => None,
                    })
                    .flatten()
                    .collect::<Vec<u8>>();
                if output.windows(expected.len()).any(|part| part == expected) {
                    return output;
                }
                notified.await;
            }
        })
        .await
        .expect("terminal output")
    }

    async fn wait_for_exit(&self, session_id: &str) -> TerminalEvent {
        self.wait_for_event(|event| {
            matches!(
                event,
                TerminalEvent::Exited { session_id: id, .. } if id == session_id
            )
        })
        .await
    }
}

impl TerminalEventSink for RecordingSink {
    fn emit(&self, event: &TerminalEvent) {
        self.events.lock().expect("event lock").push(event.clone());
        self.notify.notify_waiters();
    }
}

#[derive(Default)]
struct ConnectionSink {
    snapshots: Mutex<Vec<ConnectionSnapshot>>,
}

impl ConnectionEventSink for ConnectionSink {
    fn emit(&self, snapshot: &ConnectionSnapshot) {
        self.snapshots
            .lock()
            .expect("snapshot lock")
            .push(snapshot.clone());
    }
}

struct Harness {
    fixture: FixtureServer,
    connections: Arc<ConnectionManager>,
    terminal: Arc<TerminalManager>,
    sink: Arc<RecordingSink>,
}

impl Harness {
    async fn start() -> Self {
        let fixture = FixtureServer::start().await;
        let connections = Arc::new(ConnectionManager::new(
            Arc::new(RusshTransport::new()),
            Arc::new(ConnectionSink::default()),
            ConnectionTimeouts {
                connect: Duration::from_secs(5),
                command: Duration::from_secs(5),
                close: Duration::from_secs(1),
            },
        ));
        connections
            .connect(fixture.request())
            .await
            .expect("connect fixture");
        let sink = Arc::new(RecordingSink::default());
        let terminal = Arc::new(TerminalManager::new(
            Arc::clone(&sink) as Arc<dyn TerminalEventSink>
        ));

        Self {
            fixture,
            connections,
            terminal,
            sink,
        }
    }

    fn options(&self, cols: u32, rows: u32) -> TerminalOptions {
        TerminalOptions {
            term: "xterm-256color".to_string(),
            size: TerminalSize {
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            },
        }
    }

    async fn open(&self, cols: u32, rows: u32) -> String {
        let options = self.options(cols, rows);
        let channel = self
            .connections
            .open_terminal("fixture-server", options.clone())
            .await
            .expect("open terminal channel");
        self.terminal
            .open("fixture-server", options, channel)
            .expect("register terminal")
            .session_id
    }
}

fn generate_private_key() -> PrivateKey {
    let mut rng = rand::rng();
    PrivateKey::random(&mut rng, Algorithm::Ed25519).expect("generate test key")
}

#[tokio::test]
async fn opens_a_pty_shell_and_streams_ansi_output() {
    let harness = Harness::start().await;
    let session_id = harness.open(100, 40).await;

    let output = harness
        .sink
        .wait_for_output(&session_id, ANSI_BANNER.as_bytes())
        .await;
    assert_eq!(output, ANSI_BANNER.as_bytes());
    assert_eq!(harness.fixture.pty_requests(), 1);
    assert_eq!(harness.fixture.shell_requests(), 1);

    let channels = harness.fixture.channels();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].1.term, "xterm-256color");
    assert_eq!(channels[0].1.cols, 100);
    assert_eq!(channels[0].1.rows, 40);

    let sessions = harness.terminal.list().expect("sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, session_id);

    harness
        .terminal
        .close(&session_id)
        .await
        .expect("close terminal");
    let TerminalEvent::Exited { reason, .. } = harness.sink.wait_for_exit(&session_id).await else {
        panic!("expected exit event");
    };
    assert_eq!(reason, TerminalExitReason::Closed);
    harness.fixture.wait_for_closed_channels(1).await;
    assert!(harness.terminal.list().expect("sessions").is_empty());
}

#[tokio::test]
async fn routes_input_and_resize_to_the_remote_pty() {
    let harness = Harness::start().await;
    let session_id = harness.open(80, 24).await;
    harness
        .sink
        .wait_for_output(&session_id, ANSI_BANNER.as_bytes())
        .await;

    harness
        .terminal
        .write(&session_id, b"echo secret-command\r")
        .await
        .expect("write input");
    harness
        .fixture
        .wait_for_input(b"echo secret-command\r")
        .await;

    harness
        .terminal
        .resize(
            &session_id,
            TerminalSize {
                cols: 132,
                rows: 43,
                pixel_width: 1_320,
                pixel_height: 860,
            },
        )
        .await
        .expect("resize terminal");
    harness.fixture.wait_for_size((132, 43)).await;

    let serialized = format!("{:?}", harness.sink.session_events(&session_id));
    assert!(!serialized.contains("secret-command"));
    assert!(
        harness
            .sink
            .session_events(&session_id)
            .iter()
            .all(|event| !matches!(event, TerminalEvent::Output { .. })
                || !format!("{event:?}").contains("secret-command"))
    );

    harness
        .terminal
        .close(&session_id)
        .await
        .expect("close terminal");
}

#[tokio::test]
async fn multiple_terminal_sessions_on_one_server_are_independent() {
    let harness = Harness::start().await;
    let first = harness.open(80, 24).await;
    let second = harness.open(90, 30).await;

    assert_ne!(first, second);
    assert_eq!(harness.fixture.pty_requests(), 2);
    assert_eq!(harness.fixture.shell_requests(), 2);
    assert_eq!(harness.terminal.list().expect("sessions").len(), 2);
    harness
        .sink
        .wait_for_output(&first, ANSI_BANNER.as_bytes())
        .await;
    harness
        .sink
        .wait_for_output(&second, ANSI_BANNER.as_bytes())
        .await;

    harness
        .terminal
        .close(&first)
        .await
        .expect("close first terminal");
    harness.sink.wait_for_exit(&first).await;
    harness.fixture.wait_for_closed_channels(1).await;
    assert_eq!(harness.terminal.list().expect("sessions").len(), 1);
    assert_eq!(
        harness.terminal.list().expect("sessions")[0].session_id,
        second
    );

    harness
        .terminal
        .close(&second)
        .await
        .expect("close second terminal");
    harness.sink.wait_for_exit(&second).await;
    harness.fixture.wait_for_closed_channels(2).await;
}

#[tokio::test]
async fn remote_termination_reports_the_exit_status_and_releases_the_channel() {
    let harness = Harness::start().await;
    let session_id = harness.open(80, 24).await;
    harness
        .sink
        .wait_for_output(&session_id, ANSI_BANNER.as_bytes())
        .await;

    harness
        .terminal
        .write(&session_id, b"exit\r")
        .await
        .expect("send exit");

    let event = harness.sink.wait_for_exit(&session_id).await;
    let TerminalEvent::Exited {
        reason, exit_code, ..
    } = event
    else {
        panic!("expected exit event");
    };
    assert_eq!(reason, TerminalExitReason::RemoteExit);
    assert_eq!(exit_code, Some(7));
    assert!(harness.terminal.list().expect("sessions").is_empty());
    harness.fixture.wait_for_closed_channels(1).await;
}

#[tokio::test]
async fn command_execution_still_works_while_a_terminal_is_open() {
    let harness = Harness::start().await;
    let session_id = harness.open(80, 24).await;
    harness
        .sink
        .wait_for_output(&session_id, ANSI_BANNER.as_bytes())
        .await;

    let output = harness
        .connections
        .execute("fixture-server", "uname -srm")
        .await
        .expect("execute command");
    assert_eq!(output.stdout, EXEC_OUTPUT);
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(harness.fixture.pty_requests(), 1);

    harness
        .terminal
        .close(&session_id)
        .await
        .expect("close terminal");
}

#[tokio::test]
async fn disconnect_ends_terminal_sessions_and_cleans_up_remote_channels() {
    let harness = Harness::start().await;
    let session_id = harness.open(80, 24).await;
    harness
        .sink
        .wait_for_output(&session_id, ANSI_BANNER.as_bytes())
        .await;

    harness
        .connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");

    let event = harness.sink.wait_for_exit(&session_id).await;
    let TerminalEvent::Exited { reason, .. } = event else {
        panic!("expected exit event");
    };
    assert_eq!(reason, TerminalExitReason::Disconnected);
    harness.fixture.wait_for_no_connections().await;
    assert!(harness.terminal.list().expect("sessions").is_empty());
}
