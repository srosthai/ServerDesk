use std::{
    collections::HashMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
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
use russh_sftp::{
    protocol::{
        Attrs, FileAttributes, FileType as SftpFileType, Handle, Name, Status, StatusCode, Version,
    },
    server::Handler as SftpHandler,
};
use serverdesk_lib::core::{
    ErrorCode,
    files::{MAX_FILE_ENTRIES, RemoteEntryType, browse_remote_files},
    ssh::{
        Authentication, ConnectionEventSink, ConnectionManager, ConnectionRequest,
        ConnectionSnapshot, ConnectionState, ConnectionTimeouts, RusshTransport, TrustedHostKey,
    },
};
use tokio::net::TcpListener;

const USERNAME: &str = "deploy";
const PASSWORD: &str = "fixture-password";

#[derive(Default)]
struct FixtureState {
    authenticated: AtomicUsize,
    subsystems: Mutex<Vec<String>>,
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
    async fn start(root: PathBuf, denied: Vec<PathBuf>) -> Self {
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
            root,
            denied,
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

    fn subsystems(&self) -> Vec<String> {
        self.state
            .subsystems
            .lock()
            .expect("subsystem lock")
            .clone()
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
    _root: PathBuf,
    denied: Vec<PathBuf>,
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        state.active_connections.fetch_add(1, Ordering::SeqCst);
        let config = Arc::clone(&config);
        let state = Arc::clone(&state);
        let denied = denied.clone();
        tokio::spawn(async move {
            let _guard = ConnectionGuard(Arc::clone(&state));
            let handler = FixtureHandler {
                state,
                denied,
                channels: HashMap::new(),
            };
            if let Ok(session) = server::run_stream(config, socket, handler).await {
                let _ = session.await;
            }
        });
    }
}

struct FixtureHandler {
    state: Arc<FixtureState>,
    denied: Vec<PathBuf>,
    channels: HashMap<u32, Channel<Msg>>,
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
        self.channels.insert(channel.id().number(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            session.channel_failure(channel_id)?;
            return Ok(());
        }

        let Some(channel) = self.channels.remove(&channel_id.number()) else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };

        self.state
            .subsystems
            .lock()
            .expect("subsystem lock")
            .push(name.to_string());
        session.channel_success(channel_id)?;
        russh_sftp::server::run(
            channel.into_stream(),
            FixtureSftpSession::new(self.denied.clone()),
        )
        .await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // The connection manager runs the fixed "true" health check before it
        // publishes the connected state. SFTP browsing itself never uses exec.
        session.channel_success(channel)?;
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

struct DirHandle {
    path: PathBuf,
    sent: bool,
}

struct FixtureSftpSession {
    denied: Vec<PathBuf>,
    handles: HashMap<String, DirHandle>,
}

impl FixtureSftpSession {
    fn new(denied: Vec<PathBuf>) -> Self {
        Self {
            denied,
            handles: HashMap::new(),
        }
    }

    fn check_path(&self, path: &str) -> Result<PathBuf, StatusCode> {
        let path = PathBuf::from(path);
        if self
            .denied
            .iter()
            .any(|denied| path == *denied || path.starts_with(denied))
        {
            return Err(StatusCode::PermissionDenied);
        }
        Ok(path)
    }

    fn symlink_attributes(&self, path: &Path) -> Result<FileAttributes, StatusCode> {
        let metadata = fs::symlink_metadata(path).map_err(map_io_error)?;
        let mut attributes = FileAttributes::from(&metadata);
        if metadata.file_type().is_symlink() {
            attributes.set_regular(false);
            attributes.set_symlink(true);
        }
        Ok(attributes)
    }

    fn metadata_attributes(&self, path: &Path) -> Result<FileAttributes, StatusCode> {
        let metadata = fs::metadata(path).map_err(map_io_error)?;
        Ok(FileAttributes::from(&metadata))
    }
}

impl SftpHandler for FixtureSftpSession {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-US".to_string(),
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let path = self.check_path(&path)?;
        let metadata = fs::metadata(&path).map_err(map_io_error)?;
        if !metadata.is_dir() {
            return Err(StatusCode::Failure);
        }
        let handle = format!("dir-{id}");
        self.handles
            .insert(handle.clone(), DirHandle { path, sent: false });
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let Some(entry) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        if entry.sent {
            return Err(StatusCode::Eof);
        }
        entry.sent = true;

        let mut files = Vec::new();
        for item in fs::read_dir(&entry.path).map_err(map_io_error)? {
            let item = item.map_err(map_io_error)?;
            let name = item.file_name().to_string_lossy().to_string();
            let attributes = self
                .symlink_attributes(&item.path())
                .unwrap_or_else(|_| FileAttributes::dummy());
            files.push(russh_sftp::protocol::File::new(name, attributes));
        }

        Ok(Name { id, files })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let path = self.check_path(&path)?;
        Ok(Attrs {
            id,
            attrs: self.metadata_attributes(&path)?,
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let path = self.check_path(&path)?;
        Ok(Attrs {
            id,
            attrs: self.symlink_attributes(&path)?,
        })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let path = self.check_path(&path)?;
        let canonical = fs::canonicalize(path).map_err(map_io_error)?;
        Ok(Name {
            id,
            files: vec![russh_sftp::protocol::File::dummy(
                canonical.to_string_lossy().to_string(),
            )],
        })
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let path = self.check_path(&path)?;
        let target = fs::read_link(path).map_err(map_io_error)?;
        Ok(Name {
            id,
            files: vec![russh_sftp::protocol::File::dummy(
                target.to_string_lossy().to_string(),
            )],
        })
    }
}

fn map_io_error(error: std::io::Error) -> StatusCode {
    match error.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
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

struct FixtureTree {
    _directory: tempfile::TempDir,
    root: PathBuf,
    denied: PathBuf,
}

fn fixture_tree() -> FixtureTree {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("root");
    fs::create_dir_all(root.join("nested/deeper")).expect("nested directories");
    fs::create_dir(root.join("empty")).expect("empty directory");
    fs::create_dir(root.join("denied")).expect("denied directory");
    fs::write(root.join("denied/private.txt"), b"no access").expect("denied file");
    fs::write(root.join("readme.txt"), b"hello world\n").expect("readme");
    fs::write(root.join("nested/deeper/secret.txt"), b"deep file").expect("deep file");

    fs::set_permissions(root.join("readme.txt"), fs::Permissions::from_mode(0o644))
        .expect("file permissions");
    fs::set_permissions(root.join("nested"), fs::Permissions::from_mode(0o755))
        .expect("directory permissions");
    fs::set_permissions(root.join("denied"), fs::Permissions::from_mode(0o700))
        .expect("denied permissions");

    std::os::unix::fs::symlink("nested", root.join("inside-link")).expect("inside symlink");
    std::os::unix::fs::symlink("/etc", root.join("escape-link")).expect("escaping symlink");
    std::os::unix::fs::symlink("missing-target", root.join("broken-link")).expect("broken symlink");

    FixtureTree {
        denied: root.join("denied"),
        root,
        _directory: directory,
    }
}

fn root_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

async fn connect(manager: &ConnectionManager, fixture: &FixtureServer) {
    manager
        .connect(fixture.request())
        .await
        .expect("connect fixture server");
}

#[tokio::test]
async fn browses_nested_directories_and_returns_ls_metadata() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let manager = manager();
    connect(&manager, &fixture).await;
    let root = root_string(&tree.root);

    let listing = browse_remote_files(&manager, "fixture-server", &root, &root)
        .await
        .expect("browse fixture root");

    assert_eq!(listing.server_id, "fixture-server");
    assert_eq!(listing.root, root);
    assert_eq!(listing.path, root);
    assert_eq!(listing.resolved_path, root);
    assert_eq!(listing.connection_state, ConnectionState::Connected);
    assert!(!listing.truncated);
    assert!(listing.sampled_at > 0);
    assert!(listing.duration_ms < 5_000);

    let names = listing
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"nested"));
    assert!(names.contains(&"readme.txt"));
    assert!(names.contains(&"inside-link"));
    assert!(names.contains(&"escape-link"));
    assert!(names.contains(&"broken-link"));

    let readme = listing
        .entries
        .iter()
        .find(|entry| entry.name == "readme.txt")
        .expect("readme entry");
    assert_eq!(readme.entry_type, RemoteEntryType::File);
    assert_eq!(readme.size, Some(12));
    assert_eq!(readme.permissions.as_deref(), Some("-rw-r--r--"));
    assert_eq!(readme.mode.map(|mode| mode & 0o777), Some(0o644));
    let local_metadata = fs::metadata(tree.root.join("readme.txt")).expect("local readme metadata");
    let local_modified = i64::try_from(
        local_metadata
            .modified()
            .expect("local modified time")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("modified after epoch")
            .as_secs(),
    )
    .expect("modified seconds");
    assert_eq!(readme.modified_at, Some(local_modified));

    let nested = listing
        .entries
        .iter()
        .find(|entry| entry.name == "nested")
        .expect("nested entry");
    assert_eq!(nested.entry_type, RemoteEntryType::Directory);
    assert_eq!(nested.permissions.as_deref(), Some("drwxr-xr-x"));

    let inside = listing
        .entries
        .iter()
        .find(|entry| entry.name == "inside-link")
        .expect("inside symlink entry");
    assert_eq!(inside.entry_type, RemoteEntryType::Symlink);
    assert_eq!(inside.symlink_target.as_deref(), Some("nested"));
    assert_eq!(inside.symlink_escapes_root, Some(false));
    assert_eq!(
        inside.resolved_path.as_deref(),
        Some(root_string(&tree.root.join("nested")).as_str())
    );
    assert!(!inside.broken_symlink);

    let nested_root = root_string(&tree.root.join("nested"));
    let nested_listing = browse_remote_files(&manager, "fixture-server", &root, &nested_root)
        .await
        .expect("browse nested directory");
    assert_eq!(nested_listing.root, root);
    assert_eq!(nested_listing.path, nested_root);
    assert!(
        nested_listing
            .entries
            .iter()
            .any(|entry| entry.name == "deeper")
    );

    let deeper_root = root_string(&tree.root.join("nested/deeper"));
    let deeper_listing = browse_remote_files(&manager, "fixture-server", &root, &deeper_root)
        .await
        .expect("browse deeper directory");
    let deep_file = deeper_listing
        .entries
        .iter()
        .find(|entry| entry.name == "secret.txt")
        .expect("deep file entry");
    assert_eq!(deep_file.entry_type, RemoteEntryType::File);
    assert_eq!(deep_file.size, Some(9));
    assert_eq!(deep_file.permissions.as_deref(), Some("-rw-r--r--"));

    assert_eq!(
        fixture.subsystems(),
        vec!["sftp".to_string(), "sftp".to_string(), "sftp".to_string()],
        "each browse opens one SFTP channel on the same SSH connection"
    );
    assert_eq!(fixture.state.authenticated.load(Ordering::SeqCst), 1);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn rejects_path_traversal_before_opening_sftp() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let manager = manager();
    connect(&manager, &fixture).await;
    let root = root_string(&tree.root);

    for path in [
        root.clone() + "/../etc",
        root.clone() + "/nested/../../etc",
        "/etc/passwd".to_string(),
        "relative/path".to_string(),
        format!("{root}-sibling"),
    ] {
        let error = browse_remote_files(&manager, "fixture-server", &root, &path)
            .await
            .expect_err("path outside the root must be rejected");
        assert_eq!(error.code, ErrorCode::Validation, "{path}");
    }

    assert!(
        fixture.subsystems().is_empty(),
        "path validation must happen before any SFTP channel is opened"
    );

    let listing = browse_remote_files(&manager, "fixture-server", &root, &root)
        .await
        .expect("valid browse after rejected paths");
    assert!(!listing.entries.is_empty());
    assert_eq!(fixture.subsystems(), vec!["sftp".to_string()]);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn marks_symlinks_that_escape_the_root_and_blocks_navigation() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let manager = manager();
    connect(&manager, &fixture).await;
    let root = root_string(&tree.root);

    let listing = browse_remote_files(&manager, "fixture-server", &root, &root)
        .await
        .expect("browse fixture root");

    let escape = listing
        .entries
        .iter()
        .find(|entry| entry.name == "escape-link")
        .expect("escaping symlink entry");
    assert_eq!(escape.entry_type, RemoteEntryType::Symlink);
    assert_eq!(escape.symlink_target.as_deref(), Some("/etc"));
    assert_eq!(escape.symlink_escapes_root, Some(true));
    assert!(
        escape.resolved_path.is_none(),
        "an escaping target must not be presented as a safe resolved path"
    );
    assert!(!escape.broken_symlink);

    let broken = listing
        .entries
        .iter()
        .find(|entry| entry.name == "broken-link")
        .expect("broken symlink entry");
    assert!(broken.broken_symlink);
    assert!(broken.symlink_escapes_root.is_none());

    for path in [
        root_string(&tree.root.join("escape-link")),
        root_string(&tree.root.join("escape-link/passwd")),
    ] {
        let error = browse_remote_files(&manager, "fixture-server", &root, &path)
            .await
            .expect_err("escaping symlink navigation must be blocked");
        assert_eq!(error.code, ErrorCode::Permission, "{path}");
        assert!(error.message.contains("resolves outside"));
    }

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn reports_permission_and_missing_paths_without_dropping_the_session() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let manager = manager();
    connect(&manager, &fixture).await;
    let root = root_string(&tree.root);

    let denied = root_string(&tree.denied);
    let error = browse_remote_files(&manager, "fixture-server", &root, &denied)
        .await
        .expect_err("permission denied must be reported");
    assert_eq!(error.code, ErrorCode::Permission);
    assert!(error.message.contains("permission"));

    let missing = root_string(&tree.root.join("missing-directory"));
    let error = browse_remote_files(&manager, "fixture-server", &root, &missing)
        .await
        .expect_err("a missing path must be reported");
    assert_eq!(error.code, ErrorCode::NotFound);

    assert_eq!(
        manager
            .connection_state("fixture-server")
            .expect("connection state")
            .state,
        ConnectionState::Connected,
        "a failed browse must not drop a healthy SSH session"
    );

    let listing = browse_remote_files(&manager, "fixture-server", &root, &root)
        .await
        .expect("browsing still works after a typed failure");
    assert!(!listing.entries.is_empty());

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn rejects_browsing_without_a_connected_session() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let manager = manager();
    let root = root_string(&tree.root);

    let error = browse_remote_files(&manager, "fixture-server", &root, &root)
        .await
        .expect_err("a disconnected server must not browse");
    assert_eq!(error.code, ErrorCode::Connection);
    assert!(error.message.contains("not connected"));

    connect(&manager, &fixture).await;
    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;

    let error = browse_remote_files(&manager, "fixture-server", &root, &root)
        .await
        .expect_err("a disconnected server must not browse after disconnect");
    assert_eq!(error.code, ErrorCode::Connection);
}

#[tokio::test]
async fn caps_directory_entries_at_the_safety_limit() {
    let tree = fixture_tree();
    for index in 0..(MAX_FILE_ENTRIES + 5) {
        fs::write(tree.root.join(format!("file-{index:04}.txt")), b"x").expect("extra file");
    }
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let manager = manager();
    connect(&manager, &fixture).await;
    let root = root_string(&tree.root);

    let listing = browse_remote_files(&manager, "fixture-server", &root, &root)
        .await
        .expect("browse large directory");
    assert_eq!(listing.entries.len(), MAX_FILE_ENTRIES);
    assert!(listing.truncated);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[test]
fn fixture_handles_symlink_types_consistently() {
    let tree = fixture_tree();
    let metadata = fs::symlink_metadata(tree.root.join("inside-link")).expect("link metadata");
    assert_eq!(
        SftpFileType::from(metadata.mode()),
        SftpFileType::Symlink,
        "the fixture must expose symlinks as links, not regular files"
    );
}
