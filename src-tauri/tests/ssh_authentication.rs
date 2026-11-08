use std::{
    io::Write,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, EcdsaCurve, HashAlg, PrivateKey, PublicKey, ssh_key::LineEnding},
    server::{self, Auth, ChannelOpenHandle, Msg, Session, VerifiedPublicKey},
};
use serverdesk_lib::core::{
    AppError, ErrorCode,
    ssh::{
        Authentication, CommandOutput, ConnectionEventSink, ConnectionManager, ConnectionRequest,
        ConnectionSnapshot, ConnectionTimeouts, RusshTransport, TrustedHostKey,
    },
};
use tokio::net::TcpListener;

const USERNAME: &str = "deploy";
const PASSWORD: &str = "fixture-password";
const PASSPHRASE: &str = "fixture-passphrase";
const FIXTURE_COMMAND: &str = "fixture-command";
const FIXTURE_OUTPUT: &str = "fixture-output\n";

#[derive(Default)]
struct FixtureState {
    accepted: Mutex<Vec<String>>,
}

struct GeneratedKey {
    private: String,
    public: PublicKey,
}

struct FixtureKeys {
    ed25519: GeneratedKey,
    rsa: GeneratedKey,
    ecdsa: GeneratedKey,
    encrypted_ed25519: GeneratedKey,
    unauthorized: GeneratedKey,
}

struct FixtureServer {
    port: u16,
    host_key_type: String,
    host_fingerprint: String,
    state: Arc<FixtureState>,
    keys: &'static FixtureKeys,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    async fn start() -> Self {
        let host_key = generate_private_key(Algorithm::Ed25519);
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
        let keys = fixture_keys();
        let authorized_keys = vec![
            keys.ed25519.public.clone(),
            keys.rsa.public.clone(),
            keys.ecdsa.public.clone(),
            keys.encrypted_ed25519.public.clone(),
        ];
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
            authorized_keys,
        ));

        Self {
            port,
            host_key_type,
            host_fingerprint,
            state,
            keys,
            task,
        }
    }

    fn request(&self, username: &str, authentication: Authentication) -> ConnectionRequest {
        ConnectionRequest {
            server_id: "fixture-server".to_string(),
            host: "127.0.0.1".to_string(),
            port: self.port,
            username: username.to_string(),
            authentication,
            trusted_host_keys: vec![TrustedHostKey {
                key_type: self.host_key_type.clone(),
                fingerprint: self.host_fingerprint.clone(),
            }],
        }
    }

    fn accepted(&self) -> Vec<String> {
        self.state.accepted.lock().expect("accepted lock").clone()
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
    authorized_keys: Vec<PublicKey>,
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let config = Arc::clone(&config);
        let state = Arc::clone(&state);
        let authorized_keys = authorized_keys.clone();
        tokio::spawn(async move {
            let handler = FixtureHandler {
                state,
                authorized_keys,
            };
            if let Ok(session) = server::run_stream(config, socket, handler).await {
                let _ = session.await;
            }
        });
    }
}

struct FixtureHandler {
    state: Arc<FixtureState>,
    authorized_keys: Vec<PublicKey>,
}

impl FixtureHandler {
    fn authorized(&self, username: &str, key: &PublicKey) -> bool {
        username == USERNAME
            && self
                .authorized_keys
                .iter()
                .any(|authorized| authorized.key_data() == key.key_data())
    }
}

impl server::Handler for FixtureHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, username: &str, password: &str) -> Result<Auth, Self::Error> {
        if username == USERNAME && password == PASSWORD {
            self.state
                .accepted
                .lock()
                .expect("accepted lock")
                .push(format!("password:{username}"));
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        username: &str,
        offer: server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        if self.authorized(username, offer.public_key) {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        username: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if self.authorized(username, key.public_key) {
            self.state
                .accepted
                .lock()
                .expect("accepted lock")
                .push(format!("publickey:{username}"));
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
        if data == FIXTURE_COMMAND.as_bytes() || data == b"true" {
            session.channel_success(channel)?;
            session.data(channel, FIXTURE_OUTPUT.as_bytes().to_vec())?;
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
}

#[derive(Default)]
struct RecordingSink {
    snapshots: Mutex<Vec<ConnectionSnapshot>>,
}

impl RecordingSink {
    fn states(&self) -> Vec<serverdesk_lib::core::ssh::ConnectionState> {
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

fn fixture_keys() -> &'static FixtureKeys {
    static KEYS: OnceLock<FixtureKeys> = OnceLock::new();
    KEYS.get_or_init(|| FixtureKeys {
        ed25519: generate_key(Algorithm::Ed25519),
        rsa: generate_key(Algorithm::Rsa { hash: None }),
        ecdsa: generate_key(Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        }),
        encrypted_ed25519: generate_encrypted_key(Algorithm::Ed25519, PASSPHRASE),
        unauthorized: generate_key(Algorithm::Ed25519),
    })
}

fn generate_private_key(algorithm: Algorithm) -> PrivateKey {
    let mut rng = rand::rng();
    PrivateKey::random(&mut rng, algorithm).expect("generate private key")
}

fn generate_key(algorithm: Algorithm) -> GeneratedKey {
    let key = generate_private_key(algorithm);
    GeneratedKey {
        public: key.public_key().clone(),
        private: key
            .to_openssh(LineEnding::LF)
            .expect("encode private key")
            .to_string(),
    }
}

fn generate_encrypted_key(algorithm: Algorithm, passphrase: &str) -> GeneratedKey {
    let key = generate_private_key(algorithm);
    let mut rng = rand::rng();
    GeneratedKey {
        public: key.public_key().clone(),
        private: key
            .encrypt(&mut rng, passphrase)
            .expect("encrypt private key")
            .to_openssh(LineEnding::LF)
            .expect("encode encrypted private key")
            .to_string(),
    }
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

fn password_authentication(password: &str) -> Authentication {
    Authentication::Password(password.to_string())
}

fn private_key_authentication(key: &str, passphrase: Option<&str>) -> Authentication {
    Authentication::PrivateKey {
        key: key.to_string(),
        passphrase: passphrase.map(str::to_string),
    }
}

#[tokio::test]
async fn password_authentication_succeeds_on_a_custom_port_and_username() {
    let fixture = FixtureServer::start().await;
    assert_ne!(fixture.port, 22);
    let manager = manager();

    let snapshot = manager
        .connect(fixture.request(USERNAME, password_authentication(PASSWORD)))
        .await
        .expect("password authentication");

    assert_eq!(
        snapshot.state,
        serverdesk_lib::core::ssh::ConnectionState::Connected
    );
    assert_eq!(fixture.accepted(), vec!["password:deploy"]);

    let output = manager
        .execute("fixture-server", FIXTURE_COMMAND)
        .await
        .expect("execute command");
    assert_eq!(
        output,
        CommandOutput {
            stdout: FIXTURE_OUTPUT.to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        }
    );
}

#[tokio::test]
async fn wrong_password_returns_a_typed_authentication_error() {
    let fixture = FixtureServer::start().await;
    let manager = manager();

    let error = manager
        .connect(fixture.request(USERNAME, password_authentication("wrong-password")))
        .await
        .expect_err("wrong password must fail");

    assert_eq!(error.code, ErrorCode::Authentication);
    assert!(!format!("{error:?}").contains("wrong-password"));
}

#[tokio::test]
async fn wrong_username_returns_a_typed_authentication_error() {
    let fixture = FixtureServer::start().await;
    let manager = manager();

    let error = manager
        .connect(fixture.request("root", password_authentication(PASSWORD)))
        .await
        .expect_err("wrong username must fail");

    assert_eq!(error.code, ErrorCode::Authentication);
}

#[tokio::test]
async fn unencrypted_private_key_authentication_succeeds() {
    let fixture = FixtureServer::start().await;
    let manager = manager();

    manager
        .connect(fixture.request(
            USERNAME,
            private_key_authentication(&fixture.keys.ed25519.private, None),
        ))
        .await
        .expect("private key authentication");

    assert_eq!(fixture.accepted(), vec!["publickey:deploy"]);
}

#[tokio::test]
async fn passphrase_protected_private_key_authentication_succeeds() {
    let fixture = FixtureServer::start().await;
    let manager = manager();

    manager
        .connect(fixture.request(
            USERNAME,
            private_key_authentication(&fixture.keys.encrypted_ed25519.private, Some(PASSPHRASE)),
        ))
        .await
        .expect("passphrase-protected private key authentication");

    assert_eq!(fixture.accepted(), vec!["publickey:deploy"]);
}

#[tokio::test]
async fn private_key_authentication_accepts_supported_formats() {
    let fixture = FixtureServer::start().await;

    for (fixture_name, key) in [
        ("ed25519", &fixture.keys.ed25519.private),
        ("rsa", &fixture.keys.rsa.private),
        ("ecdsa", &fixture.keys.ecdsa.private),
    ] {
        let manager = manager();
        manager
            .connect(fixture.request(USERNAME, private_key_authentication(key, None)))
            .await
            .unwrap_or_else(|error| panic!("{fixture_name} authentication failed: {error:?}"));
    }

    assert_eq!(fixture.accepted(), vec!["publickey:deploy"; 3]);
}

#[tokio::test]
async fn encrypted_private_key_without_a_passphrase_returns_a_credential_error() {
    let fixture = FixtureServer::start().await;
    let manager = manager();

    let error = manager
        .connect(fixture.request(
            USERNAME,
            private_key_authentication(&fixture.keys.encrypted_ed25519.private, None),
        ))
        .await
        .expect_err("encrypted key must need a passphrase");

    assert_eq!(error.code, ErrorCode::Credential);
    assert!(error.message.contains("passphrase"));
    assert!(!format!("{error:?}").contains(PASSPHRASE));
}

#[tokio::test]
async fn wrong_private_key_passphrase_returns_a_credential_error() {
    let fixture = FixtureServer::start().await;
    let manager = manager();

    let error = manager
        .connect(fixture.request(
            USERNAME,
            private_key_authentication(&fixture.keys.encrypted_ed25519.private, Some("wrong")),
        ))
        .await
        .expect_err("wrong passphrase must fail");

    assert_eq!(error.code, ErrorCode::Credential);
    assert!(!format!("{error:?}").contains("wrong"));
}

#[tokio::test]
async fn unauthorized_private_key_returns_a_typed_authentication_error() {
    let fixture = FixtureServer::start().await;
    let manager = manager();

    let error = manager
        .connect(fixture.request(
            USERNAME,
            private_key_authentication(&fixture.keys.unauthorized.private, None),
        ))
        .await
        .expect_err("unauthorized key must fail");

    assert_eq!(error.code, ErrorCode::Authentication);
}

#[tokio::test]
async fn private_key_can_be_loaded_from_a_file_path() {
    let fixture = FixtureServer::start().await;
    let manager = manager();
    let mut key_file = tempfile::NamedTempFile::new().expect("create key file");
    key_file
        .write_all(fixture.keys.ed25519.private.as_bytes())
        .expect("write key file");
    let path = key_file.path().to_str().expect("key file path");

    manager
        .connect(fixture.request(USERNAME, private_key_authentication(path, None)))
        .await
        .expect("private key path authentication");

    assert_eq!(fixture.accepted(), vec!["publickey:deploy"]);
}

#[tokio::test]
async fn failed_connection_does_not_store_a_session() {
    let fixture = FixtureServer::start().await;
    let sink = Arc::new(RecordingSink::default());
    let manager = Arc::new(ConnectionManager::new(
        Arc::new(RusshTransport::new()),
        sink.clone(),
        ConnectionTimeouts {
            connect: Duration::from_secs(5),
            command: Duration::from_secs(5),
            close: Duration::from_secs(1),
        },
    ));

    let error: AppError = manager
        .connect(fixture.request(USERNAME, password_authentication("wrong")))
        .await
        .expect_err("authentication must fail");
    assert_eq!(error.code, ErrorCode::Authentication);
    assert_eq!(
        sink.states(),
        vec![
            serverdesk_lib::core::ssh::ConnectionState::Connecting,
            serverdesk_lib::core::ssh::ConnectionState::Error,
        ]
    );
    assert!(
        manager
            .execute("fixture-server", FIXTURE_COMMAND)
            .await
            .is_err()
    );
}
