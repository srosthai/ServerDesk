use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
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
        Attrs, Data, FileAttributes, FileType as SftpFileType, Handle, Name, OpenFlags, Status,
        StatusCode, Version,
    },
    server::Handler as SftpHandler,
};
use serverdesk_lib::core::{
    ErrorCode,
    files::{
        DownloadRequest, FileTransferDirection, FileTransferEventSink, FileTransferManager,
        FileTransferProgress, FileTransferResult, FileTransferStatus, MAX_FILE_ENTRIES,
        RemoteEntryType, RemoteMutationAction, UploadRequest, browse_remote_files,
        create_remote_folder, delete_remote_entry, rename_remote_entry,
    },
    ssh::{
        Authentication, ConnectionEventSink, ConnectionManager, ConnectionRequest,
        ConnectionSnapshot, ConnectionState, ConnectionTimeouts, RusshTransport, TrustedHostKey,
    },
};
use tokio::{net::TcpListener, sync::Notify};

const USERNAME: &str = "deploy";
const PASSWORD: &str = "fixture-password";

struct FixtureFaults {
    fail_upload_after: AtomicU64,
    truncate_upload_after: AtomicU64,
    fail_download_after: AtomicU64,
    uploaded_bytes: AtomicU64,
    downloaded_bytes: AtomicU64,
    write_delay_millis: AtomicU64,
    read_delay_millis: AtomicU64,
    fail_remove_after: AtomicU64,
    remove_count: AtomicU64,
}

impl FixtureFaults {
    fn new() -> Self {
        Self {
            fail_upload_after: AtomicU64::new(u64::MAX),
            truncate_upload_after: AtomicU64::new(u64::MAX),
            fail_download_after: AtomicU64::new(u64::MAX),
            uploaded_bytes: AtomicU64::new(0),
            downloaded_bytes: AtomicU64::new(0),
            write_delay_millis: AtomicU64::new(0),
            read_delay_millis: AtomicU64::new(0),
            fail_remove_after: AtomicU64::new(u64::MAX),
            remove_count: AtomicU64::new(0),
        }
    }
}

impl Default for FixtureFaults {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct FixtureState {
    authenticated: AtomicUsize,
    subsystems: Mutex<Vec<String>>,
    active_connections: AtomicUsize,
    faults: FixtureFaults,
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

    fn fail_upload_after(&self, bytes: u64) {
        self.state
            .faults
            .fail_upload_after
            .store(bytes, Ordering::SeqCst);
        self.state.faults.uploaded_bytes.store(0, Ordering::SeqCst);
    }

    fn fail_download_after(&self, bytes: u64) {
        self.state
            .faults
            .fail_download_after
            .store(bytes, Ordering::SeqCst);
        self.state
            .faults
            .downloaded_bytes
            .store(0, Ordering::SeqCst);
    }

    fn truncate_upload_after(&self, bytes: u64) {
        self.state
            .faults
            .truncate_upload_after
            .store(bytes, Ordering::SeqCst);
        self.state.faults.uploaded_bytes.store(0, Ordering::SeqCst);
    }

    fn fail_remove_after(&self, removals: u64) {
        self.state
            .faults
            .fail_remove_after
            .store(removals, Ordering::SeqCst);
        self.state.faults.remove_count.store(0, Ordering::SeqCst);
    }

    fn set_write_delay(&self, delay: Duration) {
        self.state
            .faults
            .write_delay_millis
            .store(delay.as_millis() as u64, Ordering::SeqCst);
    }

    fn set_read_delay(&self, delay: Duration) {
        self.state
            .faults
            .read_delay_millis
            .store(delay.as_millis() as u64, Ordering::SeqCst);
    }

    fn uploaded_bytes(&self) -> u64 {
        self.state.faults.uploaded_bytes.load(Ordering::SeqCst)
    }

    fn downloaded_bytes(&self) -> u64 {
        self.state.faults.downloaded_bytes.load(Ordering::SeqCst)
    }

    fn clear_faults(&self) {
        self.state
            .faults
            .fail_upload_after
            .store(u64::MAX, Ordering::SeqCst);
        self.state
            .faults
            .fail_download_after
            .store(u64::MAX, Ordering::SeqCst);
        self.state
            .faults
            .truncate_upload_after
            .store(u64::MAX, Ordering::SeqCst);
        self.state.faults.uploaded_bytes.store(0, Ordering::SeqCst);
        self.state
            .faults
            .downloaded_bytes
            .store(0, Ordering::SeqCst);
        self.state
            .faults
            .write_delay_millis
            .store(0, Ordering::SeqCst);
        self.state
            .faults
            .read_delay_millis
            .store(0, Ordering::SeqCst);
        self.state
            .faults
            .fail_remove_after
            .store(u64::MAX, Ordering::SeqCst);
        self.state.faults.remove_count.store(0, Ordering::SeqCst);
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
            FixtureSftpSession::new(self.denied.clone(), Arc::clone(&self.state)),
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

struct FileHandle {
    file: fs::File,
}

enum OpenHandle {
    Dir(DirHandle),
    File(FileHandle),
}

struct FixtureSftpSession {
    denied: Vec<PathBuf>,
    state: Arc<FixtureState>,
    handles: HashMap<String, OpenHandle>,
}

impl FixtureSftpSession {
    fn new(denied: Vec<PathBuf>, state: Arc<FixtureState>) -> Self {
        Self {
            denied,
            state,
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

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let path = self.check_path(&filename)?;
        let mut options = fs::OpenOptions::new();
        if pflags.contains(OpenFlags::READ) {
            options.read(true);
        }
        if pflags.contains(OpenFlags::WRITE) {
            options.write(true);
        }
        if pflags.contains(OpenFlags::CREATE) {
            options.create(true);
        }
        if pflags.contains(OpenFlags::TRUNCATE) {
            options.truncate(true);
        }
        let file = options.open(path).map_err(map_io_error)?;
        let handle = format!("file-{id}");
        self.handles
            .insert(handle.clone(), OpenHandle::File(FileHandle { file }));
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let delay = self.state.faults.read_delay_millis.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        let Some(OpenHandle::File(entry)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        entry
            .file
            .seek(SeekFrom::Start(offset))
            .map_err(map_io_error)?;

        let limit = self.state.faults.fail_download_after.load(Ordering::SeqCst);
        let mut available = len as usize;
        if limit != u64::MAX {
            let read_so_far = self.state.faults.downloaded_bytes.load(Ordering::SeqCst);
            if read_so_far >= limit {
                return Err(StatusCode::Failure);
            }
            available = available.min((limit - read_so_far) as usize);
        }

        let mut buffer = vec![0u8; available];
        let read = entry.file.read(&mut buffer).map_err(map_io_error)?;
        if read == 0 {
            return Err(StatusCode::Eof);
        }
        buffer.truncate(read);
        self.state
            .faults
            .downloaded_bytes
            .fetch_add(read as u64, Ordering::SeqCst);
        Ok(Data { id, data: buffer })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let delay = self.state.faults.write_delay_millis.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        let Some(OpenHandle::File(entry)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        entry
            .file
            .seek(SeekFrom::Start(offset))
            .map_err(map_io_error)?;

        let limit = self.state.faults.fail_upload_after.load(Ordering::SeqCst);
        let truncate = self
            .state
            .faults
            .truncate_upload_after
            .load(Ordering::SeqCst);
        let mut accepted = data.len();
        if limit != u64::MAX {
            let written_so_far = self.state.faults.uploaded_bytes.load(Ordering::SeqCst);
            if written_so_far >= limit {
                return Err(StatusCode::Failure);
            }
            accepted = accepted.min((limit - written_so_far) as usize);
        }
        if truncate != u64::MAX {
            let written_so_far = self.state.faults.uploaded_bytes.load(Ordering::SeqCst);
            accepted = accepted.min((truncate.saturating_sub(written_so_far)) as usize);
        }

        entry
            .file
            .write_all(&data[..accepted])
            .map_err(map_io_error)?;
        entry.file.flush().map_err(map_io_error)?;
        self.state
            .faults
            .uploaded_bytes
            .fetch_add(accepted as u64, Ordering::SeqCst);
        if limit != u64::MAX && accepted < data.len() {
            return Err(StatusCode::Failure);
        }

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
        self.handles.insert(
            handle.clone(),
            OpenHandle::Dir(DirHandle { path, sent: false }),
        );
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let Some(OpenHandle::Dir(entry)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        if entry.sent {
            return Err(StatusCode::Eof);
        }
        entry.sent = true;

        let mut files = Vec::new();
        // OpenSSH includes synthetic `.` and `..` entries in every directory
        // listing. Emit them so the client-side filtering is exercised.
        let directory_attributes = fs::metadata(&entry.path)
            .map(|metadata| FileAttributes::from(&metadata))
            .unwrap_or_else(|_| FileAttributes::dummy());
        files.push(russh_sftp::protocol::File::new(
            ".",
            directory_attributes.clone(),
        ));
        files.push(russh_sftp::protocol::File::new("..", directory_attributes));
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

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        let limit = self.state.faults.fail_remove_after.load(Ordering::SeqCst);
        if limit != u64::MAX && self.state.faults.remove_count.load(Ordering::SeqCst) >= limit {
            return Err(StatusCode::Failure);
        }
        self.state
            .faults
            .remove_count
            .fetch_add(1, Ordering::SeqCst);
        let path = self.check_path(&filename)?;
        fs::remove_file(path).map_err(map_io_error)?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-US".to_string(),
        })
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let path = self.check_path(&path)?;
        fs::create_dir(path).map_err(map_io_error)?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-US".to_string(),
        })
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let path = self.check_path(&path)?;
        fs::remove_dir(path).map_err(map_io_error)?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-US".to_string(),
        })
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        let oldpath = self.check_path(&oldpath)?;
        let newpath = self.check_path(&newpath)?;
        fs::rename(oldpath, newpath).map_err(map_io_error)?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-US".to_string(),
        })
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

#[derive(Default)]
struct RecordingTransferSink {
    events: Mutex<Vec<FileTransferProgress>>,
    notify: Notify,
}

impl RecordingTransferSink {
    async fn wait_for<F>(&self, predicate: F) -> FileTransferProgress
    where
        F: Fn(&FileTransferProgress) -> bool,
    {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.notify.notified();
                if let Some(event) = self
                    .events
                    .lock()
                    .expect("transfer event lock")
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
        .expect("transfer progress event")
    }
}

impl FileTransferEventSink for RecordingTransferSink {
    fn emit(&self, progress: &FileTransferProgress) {
        self.events
            .lock()
            .expect("transfer event lock")
            .push(progress.clone());
        self.notify.notify_waiters();
    }
}

fn transfer_manager(
    connections: Arc<ConnectionManager>,
) -> (Arc<FileTransferManager>, Arc<RecordingTransferSink>) {
    let sink = Arc::new(RecordingTransferSink::default());
    let manager = Arc::new(FileTransferManager::new(
        connections,
        Arc::clone(&sink) as Arc<dyn FileTransferEventSink>,
    ));
    (manager, sink)
}

fn upload_request(
    transfer_id: &str,
    root: &str,
    local_path: &Path,
    remote_path: &str,
    overwrite: bool,
) -> UploadRequest {
    UploadRequest {
        transfer_id: transfer_id.to_string(),
        server_id: "fixture-server".to_string(),
        root: root.to_string(),
        local_path: local_path.to_string_lossy().to_string(),
        remote_path: remote_path.to_string(),
        overwrite,
    }
}

fn download_request(
    transfer_id: &str,
    root: &str,
    remote_path: &str,
    local_path: &Path,
    overwrite: bool,
) -> DownloadRequest {
    DownloadRequest {
        transfer_id: transfer_id.to_string(),
        server_id: "fixture-server".to_string(),
        root: root.to_string(),
        remote_path: remote_path.to_string(),
        local_path: local_path.to_string_lossy().to_string(),
        overwrite,
    }
}

fn test_content(bytes: usize) -> Vec<u8> {
    (0..bytes)
        .map(|index| ((index * 31 + 7) % 251) as u8)
        .collect()
}

#[tokio::test]
async fn uploads_and_downloads_exact_file_contents() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let local_source = tree._directory.path().join("upload-source.bin");
    let content = test_content(180_000);
    fs::write(&local_source, &content).expect("local upload source");

    let upload = transfers
        .upload(upload_request(
            "upload-file-1",
            &root,
            &local_source,
            &root_string(&tree.root.join("uploaded.bin")),
            false,
        ))
        .await
        .expect("upload file");
    assert_eq!(upload.status, FileTransferStatus::Completed);
    assert_eq!(upload.direction, FileTransferDirection::Upload);
    assert_eq!(upload.bytes_transferred, content.len() as u64);
    assert_eq!(upload.total_bytes, Some(content.len() as u64));
    assert_eq!(upload.files_transferred, 1);
    assert!(!upload.may_be_incomplete);
    assert!(upload.partial_paths.is_empty());
    assert_eq!(
        fs::read(tree.root.join("uploaded.bin")).expect("uploaded file"),
        content
    );
    assert!(
        sink.wait_for(|event| event.status == FileTransferStatus::Completed)
            .await
            .transfer_id
            == "upload-file-1"
    );

    let local_destination = tree._directory.path().join("download-target.bin");
    let download = transfers
        .download(download_request(
            "download-file-1",
            &root,
            &root_string(&tree.root.join("uploaded.bin")),
            &local_destination,
            false,
        ))
        .await
        .expect("download file");
    assert_eq!(download.status, FileTransferStatus::Completed);
    assert_eq!(download.direction, FileTransferDirection::Download);
    assert_eq!(download.bytes_transferred, content.len() as u64);
    assert!(!download.may_be_incomplete);
    assert_eq!(
        fs::read(&local_destination).expect("downloaded file"),
        content
    );
    assert_eq!(fixture.uploaded_bytes(), content.len() as u64);
    assert_eq!(fixture.downloaded_bytes(), content.len() as u64);

    assert_eq!(
        fixture.state.authenticated.load(Ordering::SeqCst),
        1,
        "uploads and downloads reuse the single authenticated SSH connection"
    );
    assert_eq!(
        fixture.subsystems(),
        vec!["sftp".to_string(), "sftp".to_string()]
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn uploads_and_downloads_directories_recursively() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, _sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);

    let local_source = tree._directory.path().join("upload-tree");
    fs::create_dir_all(local_source.join("nested/deeper")).expect("local upload tree");
    fs::write(local_source.join("root.txt"), b"root file").expect("root file");
    fs::write(local_source.join("nested/child.txt"), b"child file").expect("child file");
    fs::write(
        local_source.join("nested/deeper/grandchild.txt"),
        b"grandchild file",
    )
    .expect("grandchild file");

    let remote_destination = root_string(&tree.root.join("upload-tree"));
    let upload = transfers
        .upload(upload_request(
            "upload-dir-1",
            &root,
            &local_source,
            &remote_destination,
            false,
        ))
        .await
        .expect("upload directory");
    assert_eq!(upload.status, FileTransferStatus::Completed);
    assert_eq!(upload.files_transferred, 3);
    assert_eq!(upload.directories_created, 3);
    assert_eq!(
        fs::read(tree.root.join("upload-tree/nested/deeper/grandchild.txt"))
            .expect("uploaded grandchild"),
        b"grandchild file"
    );

    let local_destination = tree._directory.path().join("download-tree");
    let remote_source = root_string(&tree.root.join("upload-tree"));
    let download = transfers
        .download(download_request(
            "download-dir-1",
            &root,
            &remote_source,
            &local_destination,
            false,
        ))
        .await
        .expect("download directory");
    assert_eq!(download.status, FileTransferStatus::Completed);
    assert_eq!(download.files_transferred, 3);
    assert_eq!(
        fs::read(local_destination.join("nested/child.txt")).expect("downloaded child"),
        b"child file"
    );
    assert_eq!(
        fs::read(local_destination.join("nested/deeper/grandchild.txt"))
            .expect("downloaded grandchild"),
        b"grandchild file"
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn overwrite_requires_explicit_confirmation_for_upload_and_download() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, _sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let local_source = tree._directory.path().join("replacement.bin");
    fs::write(&local_source, b"replacement contents").expect("replacement source");
    let remote_target = root_string(&tree.root.join("readme.txt"));

    let error = transfers
        .upload(upload_request(
            "upload-conflict-1",
            &root,
            &local_source,
            &remote_target,
            false,
        ))
        .await
        .expect_err("unconfirmed overwrite must fail");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(error.message.contains("overwritten"));
    assert_eq!(
        fs::read(tree.root.join("readme.txt")).expect("original file"),
        b"hello world\n",
        "an unconfirmed upload must not modify the existing file"
    );

    let upload = transfers
        .upload(upload_request(
            "upload-conflict-2",
            &root,
            &local_source,
            &remote_target,
            true,
        ))
        .await
        .expect("confirmed overwrite");
    assert_eq!(upload.status, FileTransferStatus::Completed);
    assert!(upload.overwrite);
    assert_eq!(
        fs::read(tree.root.join("readme.txt")).expect("replaced file"),
        b"replacement contents"
    );

    let local_destination = tree._directory.path().join("existing-download.txt");
    fs::write(&local_destination, b"old local file").expect("existing local file");
    let error = transfers
        .download(download_request(
            "download-conflict-1",
            &root,
            &root_string(&tree.root.join("nested/deeper/secret.txt")),
            &local_destination,
            false,
        ))
        .await
        .expect_err("unconfirmed download overwrite must fail");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(
        fs::read(&local_destination).expect("existing local content"),
        b"old local file"
    );

    let download = transfers
        .download(download_request(
            "download-conflict-2",
            &root,
            &root_string(&tree.root.join("nested/deeper/secret.txt")),
            &local_destination,
            true,
        ))
        .await
        .expect("confirmed download overwrite");
    assert_eq!(download.status, FileTransferStatus::Completed);
    assert_eq!(
        fs::read(&local_destination).expect("replaced local content"),
        b"deep file"
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn cancellation_stops_upload_and_preserves_the_partial_file() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    fixture.set_write_delay(Duration::from_millis(5));
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let local_source = tree._directory.path().join("cancel-source.bin");
    let content = test_content(1_500_000);
    fs::write(&local_source, &content).expect("cancel source");
    let remote_target = root_string(&tree.root.join("cancel-target.bin"));

    let upload_transfers = Arc::clone(&transfers);
    let request = upload_request(
        "cancel-upload-1",
        &root,
        &local_source,
        &remote_target,
        false,
    );
    let task = tokio::spawn(async move { upload_transfers.upload(request).await });

    sink.wait_for(|event| {
        event.transfer_id == "cancel-upload-1"
            && event.status == FileTransferStatus::Running
            && event.bytes_transferred > 0
    })
    .await;
    assert!(
        transfers
            .cancel("cancel-upload-1")
            .expect("cancel transfer"),
        "the running transfer must be cancellable"
    );

    let result: FileTransferResult = task
        .await
        .expect("upload task")
        .expect("cancelled upload result");
    assert_eq!(result.status, FileTransferStatus::Cancelled);
    assert!(result.may_be_incomplete);
    assert!(result.bytes_transferred > 0);
    assert!(result.bytes_transferred < content.len() as u64);
    assert!(!tree.root.join("cancel-target.bin").exists());
    assert_eq!(result.partial_paths.len(), 1);
    let partial = PathBuf::from(&result.partial_paths[0]);
    assert!(partial.exists(), "the partial upload must be preserved");
    assert_eq!(
        fs::metadata(&partial).expect("partial metadata").len(),
        result.bytes_transferred,
        "the partial file reports exactly the acknowledged bytes"
    );
    assert!(!transfers.cancel("cancel-upload-1").expect("late cancel"));

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn cancellation_stops_download_and_preserves_the_partial_file() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let remote_source = tree.root.join("cancel-download-source.bin");
    let content = test_content(1_500_000);
    fs::write(&remote_source, &content).expect("download source");
    let local_destination = tree._directory.path().join("cancel-download.bin");
    fixture.set_read_delay(Duration::from_millis(5));

    let download_transfers = Arc::clone(&transfers);
    let request = download_request(
        "cancel-download-1",
        &root,
        &root_string(&remote_source),
        &local_destination,
        false,
    );
    let task = tokio::spawn(async move { download_transfers.download(request).await });

    sink.wait_for(|event| {
        event.transfer_id == "cancel-download-1"
            && event.status == FileTransferStatus::Running
            && event.bytes_transferred > 0
    })
    .await;
    assert!(
        transfers
            .cancel("cancel-download-1")
            .expect("cancel download")
    );

    let result: FileTransferResult = task
        .await
        .expect("download task")
        .expect("cancelled download result");
    assert_eq!(result.status, FileTransferStatus::Cancelled);
    assert!(result.may_be_incomplete);
    assert!(result.bytes_transferred > 0);
    assert!(result.bytes_transferred < content.len() as u64);
    assert!(!local_destination.exists());
    assert_eq!(result.partial_paths.len(), 1);
    let partial = PathBuf::from(&result.partial_paths[0]);
    assert!(partial.exists(), "the partial download must be preserved");
    assert!(fs::metadata(&partial).expect("partial metadata").len() > 0);

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn interrupted_transfers_report_partial_state_and_can_be_retried() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, _sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let local_source = tree._directory.path().join("interrupted-source.bin");
    let content = test_content(220_000);
    fs::write(&local_source, &content).expect("interrupted source");
    let remote_target = root_string(&tree.root.join("interrupted-target.bin"));
    fixture.fail_upload_after(100_000);

    let failed = transfers
        .upload(upload_request(
            "interrupted-upload-1",
            &root,
            &local_source,
            &remote_target,
            false,
        ))
        .await
        .expect("interrupted upload returns a result");
    assert_eq!(failed.status, FileTransferStatus::Failed);
    assert!(failed.may_be_incomplete);
    assert!(failed.error.is_some());
    assert!(!tree.root.join("interrupted-target.bin").exists());
    assert_eq!(failed.partial_paths.len(), 1);
    let partial = PathBuf::from(&failed.partial_paths[0]);
    let partial_bytes = fs::metadata(&partial).expect("partial upload").len();
    assert!(partial_bytes > 0);
    assert!(partial_bytes <= 100_000);
    assert!(partial_bytes >= failed.bytes_transferred);
    assert!(partial_bytes < content.len() as u64);

    fixture.clear_faults();
    let retried = transfers
        .upload(upload_request(
            "interrupted-upload-2",
            &root,
            &local_source,
            &remote_target,
            false,
        ))
        .await
        .expect("retry after interruption");
    assert_eq!(retried.status, FileTransferStatus::Completed);
    assert_eq!(
        fs::read(tree.root.join("interrupted-target.bin")).expect("retried upload"),
        content,
        "a safe retry writes complete contents and replaces no partial destination"
    );

    let remote_source = root_string(&tree.root.join("interrupted-target.bin"));
    let local_destination = tree._directory.path().join("interrupted-download.bin");
    fixture.fail_download_after(50_000);
    let failed = transfers
        .download(download_request(
            "interrupted-download-1",
            &root,
            &remote_source,
            &local_destination,
            false,
        ))
        .await
        .expect("interrupted download returns a result");
    assert_eq!(failed.status, FileTransferStatus::Failed);
    assert!(failed.may_be_incomplete);
    assert!(!local_destination.exists());
    assert_eq!(failed.partial_paths.len(), 1);
    let partial = PathBuf::from(&failed.partial_paths[0]);
    let partial_bytes = fs::metadata(&partial).expect("partial download").len();
    assert!(partial_bytes > 0);
    assert!(partial_bytes <= 50_000);
    // The fixture can fail between the final read and its buffered local
    // write, so the reported counter may lead the on-disk partial size.
    assert!(partial_bytes <= failed.bytes_transferred);
    assert!(partial_bytes < content.len() as u64);

    fixture.clear_faults();
    let retried = transfers
        .download(download_request(
            "interrupted-download-2",
            &root,
            &remote_source,
            &local_destination,
            false,
        ))
        .await
        .expect("retry download");
    assert_eq!(retried.status, FileTransferStatus::Completed);
    assert_eq!(
        fs::read(&local_destination).expect("retried download"),
        content
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn uploads_reverify_the_remote_size_and_remove_a_truncated_destination() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, _sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let local_source = tree._directory.path().join("truncated-source.bin");
    let content = test_content(220_000);
    fs::write(&local_source, &content).expect("truncated source");
    let remote_target = root_string(&tree.root.join("truncated-target.bin"));
    fixture.truncate_upload_after(100_000);

    let result = transfers
        .upload(upload_request(
            "truncated-upload-1",
            &root,
            &local_source,
            &remote_target,
            false,
        ))
        .await
        .expect("a verification failure still returns transfer state");

    assert_eq!(result.status, FileTransferStatus::Failed);
    assert!(result.may_be_incomplete);
    assert!(result.error.is_some());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("rolled back")),
        "the result must disclose the rollback: {:?}",
        result.warnings
    );
    assert!(
        !tree.root.join("truncated-target.bin").exists(),
        "an unverified remote file must not be left in place as a completed upload"
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn failed_upload_verification_restores_an_existing_destination() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, _sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let local_source = tree
        ._directory
        .path()
        .join("overwrite-truncated-source.bin");
    let content = test_content(220_000);
    fs::write(&local_source, &content).expect("overwrite source");
    let remote_target = tree.root.join("overwrite-truncated-target.bin");
    fs::write(&remote_target, b"previous complete contents\n").expect("existing destination");
    fixture.truncate_upload_after(100_000);

    let result = transfers
        .upload(upload_request(
            "truncated-overwrite-1",
            &root,
            &local_source,
            &root_string(&remote_target),
            true,
        ))
        .await
        .expect("a verification failure still returns transfer state");

    assert_eq!(result.status, FileTransferStatus::Failed);
    assert!(result.may_be_incomplete);
    assert_eq!(
        fs::read(&remote_target).expect("destination restored"),
        b"previous complete contents\n",
        "a failed overwrite verification must restore the previous complete file"
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn duplicate_transfers_to_the_same_destination_are_rejected() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    fixture.set_write_delay(Duration::from_millis(5));
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let local_source = tree._directory.path().join("duplicate-source.bin");
    fs::write(&local_source, test_content(1_500_000)).expect("duplicate source");
    let remote_target = root_string(&tree.root.join("duplicate-target.bin"));

    let upload_transfers = Arc::clone(&transfers);
    let request = upload_request(
        "duplicate-upload-1",
        &root,
        &local_source,
        &remote_target,
        false,
    );
    let task = tokio::spawn(async move { upload_transfers.upload(request).await });
    sink.wait_for(|event| {
        event.transfer_id == "duplicate-upload-1"
            && event.status == FileTransferStatus::Running
            && event.bytes_transferred > 0
    })
    .await;

    let error = transfers
        .upload(upload_request(
            "duplicate-upload-2",
            &root,
            &local_source,
            &remote_target,
            false,
        ))
        .await
        .expect_err("a second transfer to the same destination must be rejected");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(
        error
            .details
            .as_deref()
            .is_some_and(|details| details.contains("network retry"))
    );

    assert!(
        transfers
            .cancel("duplicate-upload-1")
            .expect("cancel first")
    );
    let result = task
        .await
        .expect("upload task")
        .expect("cancelled upload result");
    assert_eq!(result.status, FileTransferStatus::Cancelled);
    assert!(
        !transfers
            .cancel("duplicate-upload-2")
            .expect("never started")
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn recursive_delete_reports_entries_already_removed_when_a_later_removal_fails() {
    let tree = fixture_tree();
    let partial = tree.root.join("partial-delete");
    fs::create_dir(&partial).expect("partial delete directory");
    fs::write(partial.join("first.txt"), b"first").expect("first partial file");
    fs::write(partial.join("second.txt"), b"second").expect("second partial file");
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let root = root_string(&tree.root);
    fixture.fail_remove_after(1);

    let error = delete_remote_entry(
        &connections,
        "fixture-server",
        &root,
        &root_string(&partial),
        true,
        true,
    )
    .await
    .expect_err("a partial recursive delete must not report success");
    assert_eq!(error.code, ErrorCode::Remote);
    let details = error.details.as_deref().expect("partial delete detail");
    assert!(details.contains("1 remote entry had already been deleted"));
    assert!(details.contains("verify what remains before retrying"));
    assert!(partial.exists(), "the partially deleted directory remains");
    let remaining = fs::read_dir(&partial).expect("remaining entries").count();
    assert_eq!(
        remaining, 1,
        "exactly one child was removed before the failure"
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn create_rename_and_delete_require_confirmation_and_are_verified() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let root = root_string(&tree.root);

    let created = create_remote_folder(&connections, "fixture-server", &root, &root, "created")
        .await
        .expect("create folder");
    assert_eq!(created.action, RemoteMutationAction::CreateFolder);
    assert!(created.verified);
    assert!(tree.root.join("created").is_dir());
    let target = created.target_path.as_deref().expect("created target");
    assert_eq!(target, root_string(&tree.root.join("created")));

    let error = create_remote_folder(&connections, "fixture-server", &root, &root, "created")
        .await
        .expect_err("creating a duplicate folder must conflict");
    assert_eq!(error.code, ErrorCode::Conflict);

    let original = root_string(&tree.root.join("readme.txt"));
    let error = rename_remote_entry(
        &connections,
        "fixture-server",
        &root,
        &original,
        "renamed.txt",
        false,
    )
    .await
    .expect_err("rename must require confirmation");
    assert_eq!(error.code, ErrorCode::Validation);
    assert!(tree.root.join("readme.txt").exists());

    let renamed = rename_remote_entry(
        &connections,
        "fixture-server",
        &root,
        &original,
        "renamed.txt",
        true,
    )
    .await
    .expect("rename with confirmation");
    assert_eq!(renamed.action, RemoteMutationAction::Rename);
    assert!(renamed.verified);
    assert!(!tree.root.join("readme.txt").exists());
    assert_eq!(
        fs::read(tree.root.join("renamed.txt")).expect("renamed file"),
        b"hello world\n"
    );

    let error = delete_remote_entry(
        &connections,
        "fixture-server",
        &root,
        &root_string(&tree.root.join("created")),
        false,
        true,
    )
    .await
    .expect_err("delete must require confirmation");
    assert_eq!(error.code, ErrorCode::Validation);
    assert!(tree.root.join("created").exists());

    let error = delete_remote_entry(
        &connections,
        "fixture-server",
        &root,
        &root_string(&tree.root.join("created")),
        true,
        false,
    )
    .await
    .expect_err("directory delete must require recursive confirmation");
    assert_eq!(error.code, ErrorCode::Validation);

    let deleted = delete_remote_entry(
        &connections,
        "fixture-server",
        &root,
        &root_string(&tree.root.join("created")),
        true,
        true,
    )
    .await
    .expect("delete directory with confirmation");
    assert_eq!(deleted.action, RemoteMutationAction::Delete);
    assert!(deleted.verified);
    assert!(!tree.root.join("created").exists());

    let deleted_file = delete_remote_entry(
        &connections,
        "fixture-server",
        &root,
        &root_string(&tree.root.join("renamed.txt")),
        true,
        false,
    )
    .await
    .expect("delete file with confirmation");
    assert!(deleted_file.verified);
    assert!(!tree.root.join("renamed.txt").exists());

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}

#[tokio::test]
async fn rejects_traversal_paths_before_any_transfer_or_mutation_request() {
    let tree = fixture_tree();
    let fixture = FixtureServer::start(tree.root.clone(), vec![tree.denied.clone()]).await;
    let connections = manager();
    connect(&connections, &fixture).await;
    let (transfers, _sink) = transfer_manager(Arc::clone(&connections));
    let root = root_string(&tree.root);
    let local_source = tree._directory.path().join("traversal-source.txt");
    fs::write(&local_source, b"content").expect("traversal source");

    for remote_path in [format!("{root}/../escape.txt"), "/etc/passwd".to_string()] {
        let error = transfers
            .upload(upload_request(
                "traversal-upload",
                &root,
                &local_source,
                &remote_path,
                false,
            ))
            .await
            .expect_err("traversal upload must be rejected");
        assert_eq!(error.code, ErrorCode::Validation, "{remote_path}");
    }

    for remote_path in [format!("{root}/../secret.txt"), "/etc/passwd".to_string()] {
        let error = transfers
            .download(download_request(
                "traversal-download",
                &root,
                &remote_path,
                &tree._directory.path().join("traversal-download.txt"),
                false,
            ))
            .await
            .expect_err("traversal download must be rejected");
        assert_eq!(error.code, ErrorCode::Validation, "{remote_path}");
    }

    let error = transfers
        .download(download_request(
            "local-traversal",
            &root,
            &root_string(&tree.root.join("readme.txt")),
            Path::new("/tmp/../etc/serverdesk-traversal"),
            false,
        ))
        .await
        .expect_err("local parent segment must be rejected");
    assert_eq!(error.code, ErrorCode::Validation);

    let error = create_remote_folder(
        &connections,
        "fixture-server",
        &root,
        &format!("{root}/../outside"),
        "escape",
    )
    .await
    .expect_err("traversal parent must be rejected");
    assert_eq!(error.code, ErrorCode::Validation);

    let error = rename_remote_entry(
        &connections,
        "fixture-server",
        &root,
        "/etc/passwd",
        "passwd",
        true,
    )
    .await
    .expect_err("traversal rename must be rejected");
    assert_eq!(error.code, ErrorCode::Validation);

    let error = delete_remote_entry(
        &connections,
        "fixture-server",
        &root,
        "/etc/passwd",
        true,
        false,
    )
    .await
    .expect_err("traversal delete must be rejected");
    assert_eq!(error.code, ErrorCode::Validation);

    assert!(
        fixture.subsystems().is_empty(),
        "all traversal checks must run before opening an SFTP channel"
    );

    connections
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
    fixture.wait_until_idle().await;
}
