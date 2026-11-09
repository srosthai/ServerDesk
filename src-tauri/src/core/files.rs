//! Safe remote file browsing, mutation, and transfer over the authenticated
//! SSH session.
//!
//! All operations use the SFTP subsystem on the existing SSH connection. No
//! separate file-transfer listener or public port is opened. Every request is
//! bounded to one selected root: the requested path must be lexically inside
//! that root, and its canonical SFTP path must still be inside the canonical
//! root. That second check blocks symlinks that point outside the selected
//! directory even when a crafted path bypasses the UI.
//!
//! Transfers write to a sibling partial file and rename it only after every
//! byte has been verified. A cancelled or interrupted transfer therefore never
//! replaces an existing complete destination file, and the partial path plus
//! byte count are reported so the state is never silently presented as success.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::core::{
    AppError, ErrorCode,
    projects::{RemotePath, normalize_remote_path},
    ssh::{
        ConnectionManager, ConnectionState, SftpBrowser, SftpEntryKind, SftpError, SftpMetadata,
        SftpOpenMode,
    },
};

/// Maximum number of directory entries returned by one browse request.
pub const MAX_FILE_ENTRIES: usize = 2_000;
/// Hard duration bound for one complete browse request.
pub const FILE_BROWSE_TIMEOUT: Duration = Duration::from_secs(15);

pub const FILE_TRANSFER_EVENT: &str = "file-transfer-event";
/// Bytes read or written per SFTP request.
pub const TRANSFER_CHUNK_BYTES: u32 = 32 * 1024;
pub const MAX_TRANSFER_ENTRIES: usize = 20_000;
pub const MAX_TRANSFER_DEPTH: usize = 32;
pub const MAX_PARTIAL_PATHS: usize = 20;
pub const MAX_LOCAL_PATH_CHARS: usize = 4_096;

const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);
const MAX_REMOTE_NAME_CHARS: usize = 255;
const MAX_SYMLINK_TARGET_CHARS: usize = 1_024;
const MAX_DETAIL_CHARS: usize = 300;

/// The type shown for one remote directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteEntryType {
    File,
    Directory,
    Symlink,
    Other,
}

/// One file or directory in a browsed remote directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileEntry {
    pub name: String,
    /// Normalized path built from the browsed path and this entry name.
    pub path: String,
    pub entry_type: RemoteEntryType,
    pub size: Option<u64>,
    /// Symbolic permissions in `ls -l` form, for example `-rw-r--r--`.
    pub permissions: Option<String>,
    /// Raw Unix mode bits, including the file-type bits.
    pub mode: Option<u32>,
    /// Modification time in seconds since the Unix epoch.
    pub modified_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
    /// `None` means the symlink target could not be verified safely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink_escapes_root: Option<bool>,
    pub broken_symlink: bool,
}

/// The result of browsing one directory inside a selected root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileListing {
    pub server_id: String,
    /// The selected root that bounds every navigation request.
    pub root: String,
    pub path: String,
    /// The canonical SFTP path of the browsed directory.
    pub resolved_path: String,
    pub entries: Vec<RemoteFileEntry>,
    pub truncated: bool,
    pub sampled_at: i64,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferKind {
    File,
    Directory,
}

/// Progress emitted while one upload or download is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTransferProgress {
    pub transfer_id: String,
    pub server_id: String,
    pub direction: FileTransferDirection,
    pub status: FileTransferStatus,
    pub kind: FileTransferKind,
    pub bytes_transferred: u64,
    pub total_bytes: Option<u64>,
    pub files_transferred: u64,
    pub files_total: Option<u64>,
    pub current_path: Option<String>,
    pub may_be_incomplete: bool,
    pub emitted_at: i64,
}

/// Final state of one upload or download.
///
/// A transfer that starts and then fails or is cancelled returns this record
/// rather than a bare error so the UI can keep the byte count, partial path,
/// and incomplete flag. Pre-flight failures such as a missing source or an
/// unconfirmed overwrite return a typed [`AppError`] before anything is written.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTransferResult {
    pub transfer_id: String,
    pub server_id: String,
    pub direction: FileTransferDirection,
    pub kind: FileTransferKind,
    pub status: FileTransferStatus,
    /// Local path for uploads, remote path for downloads.
    pub source_path: String,
    /// Remote path for uploads, local path for downloads.
    pub destination_path: String,
    pub bytes_transferred: u64,
    pub total_bytes: Option<u64>,
    pub files_transferred: u64,
    pub files_total: Option<u64>,
    pub directories_created: u64,
    pub skipped_entries: u64,
    pub warnings: Vec<String>,
    pub partial_paths: Vec<String>,
    pub may_be_incomplete: bool,
    pub overwrite: bool,
    pub started_at: i64,
    pub finished_at: i64,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
    pub connection_state: ConnectionState,
}

/// The action performed by one remote mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteMutationAction {
    CreateFolder,
    Rename,
    Delete,
}

/// Result of a create, rename, or delete operation after re-reading the path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMutationResult {
    pub server_id: String,
    pub action: RemoteMutationAction,
    pub root: String,
    /// The affected path before the operation.
    pub path: String,
    /// The path after the operation, when it changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    pub recursive: bool,
    pub deleted_entries: u64,
    /// True when a follow-up SFTP read confirmed the expected filesystem state.
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_detail: Option<String>,
    pub completed_at: i64,
    pub connection_state: ConnectionState,
}

pub trait FileTransferEventSink: Send + Sync {
    fn emit(&self, progress: &FileTransferProgress);
}

#[derive(Debug, Clone)]
pub struct UploadRequest {
    pub transfer_id: String,
    pub server_id: String,
    pub root: String,
    pub local_path: String,
    pub remote_path: String,
    pub overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub transfer_id: String,
    pub server_id: String,
    pub root: String,
    pub remote_path: String,
    pub local_path: String,
    pub overwrite: bool,
}

/// Tracks live transfers and routes progress to the UI event sink.
pub struct FileTransferManager {
    connections: Arc<ConnectionManager>,
    events: Arc<dyn FileTransferEventSink>,
    cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl FileTransferManager {
    pub fn new(
        connections: Arc<ConnectionManager>,
        events: Arc<dyn FileTransferEventSink>,
    ) -> Self {
        Self {
            connections,
            events,
            cancellations: Mutex::new(HashMap::new()),
        }
    }

    /// Uploads one local file or directory without ever opening a second port.
    pub async fn upload(&self, request: UploadRequest) -> Result<FileTransferResult, AppError> {
        validate_transfer_id(&request.transfer_id)?;
        if request.server_id.trim().is_empty() {
            return Err(AppError::validation("Server ID is required."));
        }
        let (root, remote_path) = validate_browse_paths(&request.root, &request.remote_path)?;
        let local_path = normalize_local_path(&request.local_path)?;
        let source_metadata = tokio::fs::symlink_metadata(&local_path)
            .await
            .map_err(|_| {
                AppError::not_found("The local upload source does not exist.")
                    .with_details("Choose an existing absolute local file or directory path.")
            })?;
        let kind = if source_metadata.file_type().is_file() {
            FileTransferKind::File
        } else if source_metadata.file_type().is_dir() {
            FileTransferKind::Directory
        } else {
            return Err(AppError::validation(
                "Only regular local files and directories can be uploaded.",
            )
            .with_details(
                "Symbolic links and special files are not followed. Choose a real file or \
                 directory instead.",
            ));
        };

        let cancel = self.register_transfer(&request.transfer_id)?;
        let transfer_id = request.transfer_id.clone();
        let result = self
            .run_upload(
                request,
                root,
                remote_path,
                local_path,
                kind,
                source_metadata.len(),
                cancel,
            )
            .await;
        self.finish_transfer(&transfer_id);
        result
    }

    /// Downloads one remote file or directory to a local destination.
    pub async fn download(&self, request: DownloadRequest) -> Result<FileTransferResult, AppError> {
        validate_transfer_id(&request.transfer_id)?;
        if request.server_id.trim().is_empty() {
            return Err(AppError::validation("Server ID is required."));
        }
        let (root, remote_path) = validate_browse_paths(&request.root, &request.remote_path)?;
        let local_path = normalize_local_path(&request.local_path)?;

        let cancel = self.register_transfer(&request.transfer_id)?;
        let transfer_id = request.transfer_id.clone();
        let result = self
            .run_download(request, root, remote_path, local_path, cancel)
            .await;
        self.finish_transfer(&transfer_id);
        result
    }

    /// Requests cancellation of one running transfer.
    ///
    /// Returns `false` when the transfer already finished or never existed.
    pub fn cancel(&self, transfer_id: &str) -> Result<bool, AppError> {
        let cancellations = self.lock_cancellations()?;
        if let Some(flag) = cancellations.get(transfer_id) {
            flag.store(true, Ordering::SeqCst);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Cancels every running transfer. Used when the application is closing.
    pub fn cancel_all(&self) {
        if let Ok(cancellations) = self.cancellations.lock() {
            for flag in cancellations.values() {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }

    fn register_transfer(&self, transfer_id: &str) -> Result<Arc<AtomicBool>, AppError> {
        let mut cancellations = self.lock_cancellations()?;
        if cancellations.contains_key(transfer_id) {
            return Err(AppError::conflict(
                "A transfer with this identifier is already running.",
            ));
        }
        let flag = Arc::new(AtomicBool::new(false));
        cancellations.insert(transfer_id.to_string(), Arc::clone(&flag));
        Ok(flag)
    }

    fn finish_transfer(&self, transfer_id: &str) {
        if let Ok(mut cancellations) = self.cancellations.lock() {
            cancellations.remove(transfer_id);
        }
    }

    fn lock_cancellations(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>, AppError> {
        self.cancellations
            .lock()
            .map_err(|_| AppError::internal("File transfer lock was poisoned."))
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_upload(
        &self,
        request: UploadRequest,
        root: RemotePath,
        remote_path: RemotePath,
        local_path: PathBuf,
        kind: FileTransferKind,
        source_size: u64,
        cancel: Arc<AtomicBool>,
    ) -> Result<FileTransferResult, AppError> {
        let started = Instant::now();
        let started_at = now_millis();
        let browser = self.connections.open_sftp(&request.server_id).await?;
        let mut context = TransferContext::new(
            self,
            &request.transfer_id,
            &request.server_id,
            FileTransferDirection::Upload,
            cancel,
            started,
            started_at,
        );

        let plan = match upload_preflight(
            browser.as_ref(),
            &root,
            &remote_path,
            &local_path,
            source_size,
            kind,
            request.overwrite,
            &mut context,
        )
        .await
        {
            Ok(plan) => plan,
            Err(error) if error.code == ErrorCode::Cancelled => {
                let _ = browser.close().await;
                return Ok(context.finish(FileTransferStatus::Cancelled, request, kind, None));
            }
            Err(error) => {
                let _ = browser.close().await;
                return Err(error);
            }
        };
        let (kind, total_bytes, files_total) = plan.summary();
        context.total_bytes = Some(total_bytes);
        context.files_total = Some(files_total);
        context.emit(false);

        let execution = execute_upload(browser.as_ref(), &plan, &mut context).await;
        let error = match execution {
            Ok(()) => None,
            Err(error) if error.code == ErrorCode::Cancelled => {
                let _ = browser.close().await;
                return Ok(context.finish(FileTransferStatus::Cancelled, request, kind, None));
            }
            Err(error) => Some(error),
        };

        let _ = browser.close().await;
        let status = if error.is_some() {
            FileTransferStatus::Failed
        } else {
            FileTransferStatus::Completed
        };
        Ok(context.finish(status, request, kind, error))
    }

    async fn run_download(
        &self,
        request: DownloadRequest,
        root: RemotePath,
        remote_path: RemotePath,
        local_path: PathBuf,
        cancel: Arc<AtomicBool>,
    ) -> Result<FileTransferResult, AppError> {
        let started = Instant::now();
        let started_at = now_millis();
        let browser = self.connections.open_sftp(&request.server_id).await?;
        let mut context = TransferContext::new(
            self,
            &request.transfer_id,
            &request.server_id,
            FileTransferDirection::Download,
            cancel,
            started,
            started_at,
        );

        let plan = match download_preflight(
            browser.as_ref(),
            &root,
            &remote_path,
            &local_path,
            request.overwrite,
            &mut context,
        )
        .await
        {
            Ok(plan) => plan,
            Err(error) if error.code == ErrorCode::Cancelled => {
                let _ = browser.close().await;
                return Ok(context.finish(
                    FileTransferStatus::Cancelled,
                    request,
                    context.kind,
                    None,
                ));
            }
            Err(error) => {
                let _ = browser.close().await;
                return Err(error);
            }
        };
        let (kind, total_bytes, files_total) = plan.summary();
        context.total_bytes = total_bytes;
        context.files_total = Some(files_total);
        context.emit(false);

        let execution = execute_download(browser.as_ref(), &plan, &mut context).await;
        let error = match execution {
            Ok(()) => None,
            Err(error) if error.code == ErrorCode::Cancelled => {
                let _ = browser.close().await;
                return Ok(context.finish(FileTransferStatus::Cancelled, request, kind, None));
            }
            Err(error) => Some(error),
        };

        let _ = browser.close().await;
        let status = if error.is_some() {
            FileTransferStatus::Failed
        } else {
            FileTransferStatus::Completed
        };
        Ok(context.finish(status, request, kind, error))
    }
}

struct TransferContext<'a> {
    manager: &'a FileTransferManager,
    transfer_id: String,
    server_id: String,
    direction: FileTransferDirection,
    cancel: Arc<AtomicBool>,
    started: Instant,
    started_at: i64,
    bytes_transferred: u64,
    total_bytes: Option<u64>,
    files_transferred: u64,
    files_total: Option<u64>,
    directories_created: u64,
    skipped_entries: u64,
    warnings: Vec<String>,
    partial_paths: Vec<String>,
    current_path: Option<String>,
    kind: FileTransferKind,
    last_emit: Instant,
}

impl<'a> TransferContext<'a> {
    fn new(
        manager: &'a FileTransferManager,
        transfer_id: &str,
        server_id: &str,
        direction: FileTransferDirection,
        cancel: Arc<AtomicBool>,
        started: Instant,
        started_at: i64,
    ) -> Self {
        Self {
            manager,
            transfer_id: transfer_id.to_string(),
            server_id: server_id.to_string(),
            direction,
            cancel,
            started,
            started_at,
            bytes_transferred: 0,
            total_bytes: None,
            files_transferred: 0,
            files_total: None,
            directories_created: 0,
            skipped_entries: 0,
            warnings: Vec::new(),
            partial_paths: Vec::new(),
            current_path: None,
            kind: FileTransferKind::File,
            last_emit: started,
        }
    }

    fn check_cancelled(&self) -> Result<(), AppError> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(
                AppError::cancelled("The transfer was cancelled.").with_details(
                    "Data already transferred is preserved. The partial file path is reported and \
                 the destination is only replaced after a verified complete transfer.",
                ),
            )
        } else {
            Ok(())
        }
    }

    fn warn(&mut self, warning: impl Into<String>) {
        if self.warnings.len() < MAX_PARTIAL_PATHS {
            self.warnings.push(warning.into());
        }
    }

    fn skip(&mut self, warning: impl Into<String>) {
        self.skipped_entries = self.skipped_entries.saturating_add(1);
        self.warn(warning);
    }

    fn record_partial(&mut self, path: impl Into<String>) {
        let path = path.into();
        if self.partial_paths.len() < MAX_PARTIAL_PATHS && !self.partial_paths.contains(&path) {
            self.partial_paths.push(path);
        }
    }

    fn emit(&mut self, force: bool) {
        let now = Instant::now();
        if !force && now.duration_since(self.last_emit) < PROGRESS_INTERVAL {
            return;
        }
        self.last_emit = now;
        self.manager
            .events
            .emit(&self.progress(FileTransferStatus::Running));
    }

    fn progress(&self, status: FileTransferStatus) -> FileTransferProgress {
        FileTransferProgress {
            transfer_id: self.transfer_id.clone(),
            server_id: self.server_id.clone(),
            direction: self.direction,
            status,
            kind: self.kind,
            bytes_transferred: self.bytes_transferred,
            total_bytes: self.total_bytes,
            files_transferred: self.files_transferred,
            files_total: self.files_total,
            current_path: self.current_path.clone(),
            may_be_incomplete: !self.partial_paths.is_empty(),
            emitted_at: now_millis(),
        }
    }

    fn finish(
        &mut self,
        status: FileTransferStatus,
        request: impl TransferRequestView,
        kind: FileTransferKind,
        error: Option<AppError>,
    ) -> FileTransferResult {
        self.kind = kind;
        let now = now_millis();
        let may_be_incomplete = !self.partial_paths.is_empty()
            || matches!(
                status,
                FileTransferStatus::Cancelled | FileTransferStatus::Failed
            );
        self.manager.events.emit(&FileTransferProgress {
            may_be_incomplete,
            ..self.progress(status)
        });

        FileTransferResult {
            transfer_id: self.transfer_id.clone(),
            server_id: self.server_id.clone(),
            direction: self.direction,
            kind: self.kind,
            status,
            source_path: request.source_path().to_string(),
            destination_path: request.destination_path().to_string(),
            bytes_transferred: self.bytes_transferred,
            total_bytes: self.total_bytes,
            files_transferred: self.files_transferred,
            files_total: self.files_total,
            directories_created: self.directories_created,
            skipped_entries: self.skipped_entries,
            warnings: self.warnings.clone(),
            partial_paths: self.partial_paths.clone(),
            may_be_incomplete,
            overwrite: request.overwrite(),
            started_at: self.started_at,
            finished_at: now,
            duration_ms: duration_millis(self.started),
            error,
            connection_state: self
                .manager
                .connections
                .connection_state(&self.server_id)
                .map(|snapshot| snapshot.state)
                .unwrap_or(ConnectionState::Disconnected),
        }
    }
}

trait TransferRequestView {
    fn source_path(&self) -> &str;
    fn destination_path(&self) -> &str;
    fn overwrite(&self) -> bool;
}

impl TransferRequestView for UploadRequest {
    fn source_path(&self) -> &str {
        &self.local_path
    }
    fn destination_path(&self) -> &str {
        &self.remote_path
    }
    fn overwrite(&self) -> bool {
        self.overwrite
    }
}

impl TransferRequestView for DownloadRequest {
    fn source_path(&self) -> &str {
        &self.remote_path
    }
    fn destination_path(&self) -> &str {
        &self.local_path
    }
    fn overwrite(&self) -> bool {
        self.overwrite
    }
}

enum UploadPlan {
    File(UploadFilePlan),
    Directory(UploadDirectoryPlan),
}

struct UploadFilePlan {
    local_path: PathBuf,
    remote_path: String,
    size: u64,
    overwrite: bool,
}

struct UploadDirectoryPlan {
    remote_root: String,
    directories: Vec<String>,
    files: Vec<UploadFilePlan>,
    overwrite: bool,
}

impl UploadPlan {
    fn summary(&self) -> (FileTransferKind, u64, u64) {
        match self {
            Self::File(file) => (FileTransferKind::File, file.size, 1),
            Self::Directory(directory) => (
                FileTransferKind::Directory,
                directory.files.iter().map(|file| file.size).sum(),
                directory.files.len() as u64,
            ),
        }
    }
}

struct DownloadPlan {
    kind: FileTransferKind,
    local_root: PathBuf,
    size: Option<u64>,
    directories: Vec<String>,
    files: Vec<DownloadFilePlan>,
    overwrite: bool,
}

struct DownloadFilePlan {
    remote_path: String,
    relative_path: String,
    size: Option<u64>,
}

impl DownloadPlan {
    fn summary(&self) -> (FileTransferKind, Option<u64>, u64) {
        let total = match self.kind {
            FileTransferKind::File => self.size,
            FileTransferKind::Directory => {
                if self.files.iter().all(|file| file.size.is_some()) {
                    Some(self.files.iter().filter_map(|file| file.size).sum())
                } else {
                    None
                }
            }
        };
        (self.kind, total, self.files.len() as u64)
    }
}

/// Normalizes a selected root and requested path, and rejects any path that is
/// not lexically inside the root.
pub fn validate_browse_paths(root: &str, path: &str) -> Result<(RemotePath, RemotePath), AppError> {
    let root = normalize_remote_path(root)?;
    let path = normalize_remote_path(path)?;

    if !is_within_remote_root(root.as_str(), path.as_str()) {
        return Err(outside_root_error());
    }

    Ok((root, path))
}

/// Returns true when `path` is the root or a descendant of it.
pub fn is_within_remote_root(root: &str, path: &str) -> bool {
    if root == "/" {
        return path.starts_with('/');
    }

    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

fn outside_root_error() -> AppError {
    AppError::validation("The requested path is outside the selected root directory.").with_details(
        "ServerDesk only browses the selected root and its descendants. Parent segments and \
         absolute paths outside the root are rejected before any SFTP request is sent.",
    )
}

fn resolved_outside_root_error() -> AppError {
    AppError::permission("The requested path resolves outside the selected root directory.")
        .with_details(
            "The path is a symlink that leaves the selected root. ServerDesk blocked the browse \
             instead of following it. Choose a directory inside the root.",
        )
}

/// Browses one directory inside `root` over the managed authenticated session.
pub async fn browse_remote_files(
    manager: &ConnectionManager,
    server_id: &str,
    root: &str,
    path: &str,
) -> Result<RemoteFileListing, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let (root, path) = validate_browse_paths(root, path)?;
    let started = Instant::now();
    let browser = manager.open_sftp(server_id).await?;

    let result = tokio::time::timeout(
        FILE_BROWSE_TIMEOUT,
        browse_with_browser(manager, server_id, &root, &path, browser.as_ref(), started),
    )
    .await;
    let _ = browser.close().await;

    match result {
        Err(_) => Err(AppError::timeout(format!(
            "Remote file browsing timed out after {} seconds.",
            FILE_BROWSE_TIMEOUT.as_secs()
        ))),
        Ok(result) => result,
    }
}

async fn browse_with_browser(
    manager: &ConnectionManager,
    server_id: &str,
    root: &RemotePath,
    path: &RemotePath,
    browser: &dyn SftpBrowser,
    started: Instant,
) -> Result<RemoteFileListing, AppError> {
    let root_real = browser
        .canonicalize(root.as_str())
        .await
        .map_err(sftp_error)?;
    let path_real = browser
        .canonicalize(path.as_str())
        .await
        .map_err(sftp_error)?;

    if !is_within_remote_root(&root_real, &path_real) {
        return Err(resolved_outside_root_error());
    }

    let metadata = browser.stat(&path_real).await.map_err(sftp_error)?;
    if metadata.kind != SftpEntryKind::Directory {
        return Err(AppError::validation(
            "The selected remote path is not a directory, so it cannot be browsed.",
        ));
    }

    let (raw_entries, truncated) = browser
        .read_dir(&path_real, MAX_FILE_ENTRIES)
        .await
        .map_err(sftp_error)?;

    let mut entries = Vec::with_capacity(raw_entries.len());
    for raw in raw_entries {
        let Some(entry) = describe_entry(browser, &root_real, path.as_str(), &raw).await else {
            continue;
        };
        entries.push(entry);
    }

    entries.sort_by(|left, right| {
        entry_sort_rank(left.entry_type)
            .cmp(&entry_sort_rank(right.entry_type))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });

    let connection_state = manager.connection_state(server_id)?.state;

    Ok(RemoteFileListing {
        server_id: server_id.to_string(),
        root: root.as_str().to_string(),
        path: path.as_str().to_string(),
        resolved_path: path_real,
        entries,
        truncated,
        sampled_at: now_millis(),
        duration_ms: duration_millis(started),
        connection_state,
    })
}

async fn describe_entry(
    browser: &dyn SftpBrowser,
    root_real: &str,
    parent_path: &str,
    raw: &crate::core::ssh::SftpDirEntry,
) -> Option<RemoteFileEntry> {
    let name = safe_entry_name(&raw.name)?;
    let entry_path = join_remote_path(parent_path, &name);
    let mut entry = RemoteFileEntry {
        name,
        path: entry_path,
        entry_type: entry_type(raw.metadata.kind),
        size: raw.metadata.size,
        permissions: raw.metadata.mode.map(format_permissions),
        mode: raw.metadata.mode,
        modified_at: raw.metadata.modified_at,
        symlink_target: None,
        resolved_path: None,
        symlink_escapes_root: None,
        broken_symlink: false,
    };

    if raw.metadata.kind == SftpEntryKind::Symlink {
        if let Ok(target) = browser.read_link(&entry.path).await {
            entry.symlink_target = safe_symlink_target(&target);
        }

        match browser.canonicalize(&entry.path).await {
            Ok(resolved) => {
                let escapes = !is_within_remote_root(root_real, &resolved);
                entry.symlink_escapes_root = Some(escapes);
                if !escapes {
                    entry.resolved_path = Some(resolved);
                }
            }
            Err(SftpError::NotFound) => entry.broken_symlink = true,
            Err(_) => {
                // Leaving the target unverified is safer than guessing. The UI
                // blocks navigation until the link can be resolved.
            }
        }
    }

    Some(entry)
}

fn entry_type(kind: SftpEntryKind) -> RemoteEntryType {
    match kind {
        SftpEntryKind::File => RemoteEntryType::File,
        SftpEntryKind::Directory => RemoteEntryType::Directory,
        SftpEntryKind::Symlink => RemoteEntryType::Symlink,
        SftpEntryKind::Other => RemoteEntryType::Other,
    }
}

fn entry_sort_rank(kind: RemoteEntryType) -> u8 {
    match kind {
        RemoteEntryType::Directory => 0,
        RemoteEntryType::Symlink => 1,
        RemoteEntryType::File => 2,
        RemoteEntryType::Other => 3,
    }
}

fn safe_entry_name(value: &str) -> Option<String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\0')
        || value.chars().count() > MAX_REMOTE_NAME_CHARS
        || value.chars().any(char::is_control)
    {
        return None;
    }

    Some(value.to_string())
}

fn safe_symlink_target(value: &str) -> Option<String> {
    let bounded = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_SYMLINK_TARGET_CHARS)
        .collect::<String>();

    if bounded.is_empty() {
        None
    } else {
        Some(bounded)
    }
}

fn join_remote_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// Formats Unix mode bits as an `ls -l` style permission string.
pub fn format_permissions(mode: u32) -> String {
    let mut result = String::with_capacity(10);
    result.push(match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '-',
    });

    let owner = (mode >> 6) & 0o7;
    let group = (mode >> 3) & 0o7;
    let other = mode & 0o7;
    push_permission_triplet(&mut result, owner, mode & 0o4000 != 0, 's');
    push_permission_triplet(&mut result, group, mode & 0o2000 != 0, 's');
    push_permission_triplet(&mut result, other, mode & 0o1000 != 0, 't');

    result
}

fn push_permission_triplet(result: &mut String, bits: u32, special: bool, special_char: char) {
    result.push(if bits & 0o4 != 0 { 'r' } else { '-' });
    result.push(if bits & 0o2 != 0 { 'w' } else { '-' });
    if special {
        result.push(if bits & 0o1 != 0 {
            special_char
        } else {
            special_char.to_ascii_uppercase()
        });
    } else {
        result.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
}

fn sftp_error(error: SftpError) -> AppError {
    let mut error = error.to_app_error();
    if let Some(details) = error.details.take() {
        let details = details
            .chars()
            .filter(|character| !character.is_control() || *character == '\t')
            .take(MAX_DETAIL_CHARS)
            .collect::<String>();
        if !details.trim().is_empty() {
            error.details = Some(details);
        }
    }
    error
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn duration_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn validate_transfer_id(value: &str) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(AppError::validation(
            "The transfer identifier must be a short ASCII identifier.",
        ));
    }
    Ok(())
}

/// Normalizes an absolute local path and rejects parent segments.
///
/// Local filesystem access is limited to explicit absolute paths supplied by
/// the user. `..` segments are rejected instead of being collapsed so a typo
/// can never redirect a download outside the chosen directory.
pub fn normalize_local_path(value: &str) -> Result<PathBuf, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_LOCAL_PATH_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return Err(AppError::validation(
            "Enter an absolute local path without control characters.",
        ));
    }

    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return Err(AppError::validation(
            "Local paths must be absolute, such as /home/user/file.zip.",
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AppError::validation(
            "Local paths must not contain parent (..) segments.",
        ));
    }

    Ok(path.to_path_buf())
}

fn remote_parent(path: &str) -> Option<&str> {
    if path == "/" {
        return None;
    }
    let index = path.rfind('/')?;
    Some(if index == 0 { "/" } else { &path[..index] })
}

fn remote_name(path: &str) -> Option<&str> {
    if path == "/" {
        return None;
    }
    path.rsplit('/').next().filter(|name| !name.is_empty())
}

fn join_relative_remote_path(base: &str, relative: &str) -> String {
    if relative.is_empty() {
        base.to_string()
    } else {
        join_remote_path(base, relative)
    }
}

fn transfer_part_path(destination: &str, transfer_id: &str) -> String {
    let suffix = transfer_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    let suffix = if suffix.is_empty() {
        "partial".to_string()
    } else {
        suffix
    };
    format!("{destination}.serverdesk-{suffix}.part")
}

fn local_part_path(destination: &Path, transfer_id: &str) -> PathBuf {
    let suffix = transfer_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    let suffix = if suffix.is_empty() {
        "partial".to_string()
    } else {
        suffix
    };
    let mut value = destination.as_os_str().to_os_string();
    value.push(format!(".serverdesk-{suffix}.part"));
    PathBuf::from(value)
}

async fn stat_optional(
    browser: &dyn SftpBrowser,
    path: &str,
) -> Result<Option<SftpMetadata>, SftpError> {
    match browser.lstat(path).await {
        Ok(metadata) => Ok(Some(metadata)),
        Err(SftpError::NotFound) => Ok(None),
        Err(error) => Err(error),
    }
}

fn resolved_outside_root_error_for_write() -> AppError {
    AppError::permission("The requested path resolves outside the selected root directory.")
        .with_details(
            "ServerDesk blocked the write instead of following a path or symbolic link outside \
             the selected root.",
        )
}

fn directory_conflict_error(paths: &[String]) -> AppError {
    let shown = paths.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    AppError::conflict(format!(
        "{} existing destination {} would be overwritten.",
        paths.len(),
        if paths.len() == 1 { "file" } else { "files" }
    ))
    .with_details(format!(
        "Overwrite was not confirmed. Existing paths: {shown}. Confirm overwrite explicitly to \
         replace them."
    ))
}

fn local_storage_error(action: &str, error: std::io::Error) -> AppError {
    AppError::new(
        ErrorCode::Storage,
        format!("The local destination could not be {action}."),
    )
    .with_details(error.to_string())
}

async fn resolve_write_target(
    browser: &dyn SftpBrowser,
    root_real: &str,
    remote_path: &str,
) -> Result<String, AppError> {
    let parent = remote_parent(remote_path).ok_or_else(|| {
        AppError::validation("The remote destination must name a file or directory.")
    })?;
    let name = remote_name(remote_path)
        .and_then(safe_entry_name)
        .ok_or_else(|| AppError::validation("The remote destination name is not valid."))?;
    let parent_real = browser.canonicalize(parent).await.map_err(sftp_error)?;
    if !is_within_remote_root(root_real, &parent_real) {
        return Err(resolved_outside_root_error_for_write());
    }
    let parent_metadata = browser.stat(&parent_real).await.map_err(sftp_error)?;
    if parent_metadata.kind != SftpEntryKind::Directory {
        return Err(AppError::validation(
            "The remote destination parent is not a directory.",
        ));
    }
    Ok(join_remote_path(&parent_real, &name))
}

struct ScannedUploadFile {
    local_path: PathBuf,
    relative_path: String,
    size: u64,
}

async fn scan_local_directory(
    local_root: &Path,
    directories: &mut Vec<String>,
    files: &mut Vec<ScannedUploadFile>,
    context: &mut TransferContext<'_>,
) -> Result<(), AppError> {
    let mut pending = vec![(local_root.to_path_buf(), String::new(), 0usize)];
    let mut scanned = 0usize;

    while let Some((directory, relative, depth)) = pending.pop() {
        if depth > MAX_TRANSFER_DEPTH {
            return Err(AppError::validation(format!(
                "The local upload directory is deeper than {MAX_TRANSFER_DEPTH} levels."
            )));
        }

        let mut entries = Vec::new();
        let mut reader = tokio::fs::read_dir(&directory)
            .await
            .map_err(|error| local_storage_error("read", error))?;
        while let Some(entry) = reader
            .next_entry()
            .await
            .map_err(|error| local_storage_error("read", error))?
        {
            entries.push(entry);
        }
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            scanned += 1;
            if scanned > MAX_TRANSFER_ENTRIES {
                return Err(AppError::validation(format!(
                    "The local upload directory contains more than {MAX_TRANSFER_ENTRIES} entries."
                )));
            }
            context.check_cancelled()?;

            let name = entry.file_name().to_string_lossy().to_string();
            let Some(name) = safe_entry_name(&name) else {
                context.skip(format!(
                    "Skipped an upload entry with an unsafe name: {name:?}."
                ));
                continue;
            };
            let relative_path = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let path = entry.path();
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|error| local_storage_error("read", error))?;

            if metadata.file_type().is_symlink() {
                context.skip(format!(
                    "Skipped local symbolic link {relative_path}; links are never followed."
                ));
            } else if metadata.is_dir() {
                directories.push(relative_path.clone());
                pending.push((path, relative_path, depth + 1));
            } else if metadata.is_file() {
                files.push(ScannedUploadFile {
                    local_path: path,
                    relative_path,
                    size: metadata.len(),
                });
            } else {
                context.skip(format!(
                    "Skipped special local file {relative_path}; only regular files are uploaded."
                ));
            }
        }
    }

    Ok(())
}

// Keep the generic scan helper independent from the transport type while
// retaining testable ordering and bounds.
#[allow(clippy::too_many_arguments)]
async fn upload_preflight(
    browser: &dyn SftpBrowser,
    root: &RemotePath,
    remote_path: &RemotePath,
    local_path: &Path,
    source_size: u64,
    kind: FileTransferKind,
    overwrite: bool,
    context: &mut TransferContext<'_>,
) -> Result<UploadPlan, AppError> {
    let root_real = browser
        .canonicalize(root.as_str())
        .await
        .map_err(sftp_error)?;
    context.check_cancelled()?;
    let target = resolve_write_target(browser, &root_real, remote_path.as_str()).await?;

    match kind {
        FileTransferKind::File => {
            if let Some(existing) = stat_optional(browser, &target).await.map_err(sftp_error)? {
                match existing.kind {
                    SftpEntryKind::File => {}
                    SftpEntryKind::Directory => {
                        return Err(AppError::validation(
                            "The remote destination is a directory, not a file.",
                        ));
                    }
                    _ => {
                        return Err(AppError::validation(
                            "The remote destination is a symbolic link or special file. \
                             ServerDesk will not overwrite it.",
                        ));
                    }
                }
                if !overwrite {
                    return Err(directory_conflict_error(&[target]));
                }
            }
            Ok(UploadPlan::File(UploadFilePlan {
                local_path: local_path.to_path_buf(),
                remote_path: target,
                size: source_size,
                overwrite,
            }))
        }
        FileTransferKind::Directory => {
            let remote_root = match stat_optional(browser, &target).await.map_err(sftp_error)? {
                Some(existing) => match existing.kind {
                    SftpEntryKind::Directory => {
                        let resolved = browser.canonicalize(&target).await.map_err(sftp_error)?;
                        if !is_within_remote_root(&root_real, &resolved) {
                            return Err(resolved_outside_root_error_for_write());
                        }
                        resolved
                    }
                    SftpEntryKind::File => {
                        return Err(AppError::validation(
                            "The remote destination is a file, not a directory.",
                        ));
                    }
                    _ => {
                        return Err(AppError::validation(
                            "The remote destination is a symbolic link or special file. \
                             ServerDesk will not merge into it.",
                        ));
                    }
                },
                None => target,
            };

            let mut directories = Vec::new();
            let mut scanned_files = Vec::new();
            scan_local_directory(local_path, &mut directories, &mut scanned_files, context).await?;
            context.check_cancelled()?;

            for relative in &directories {
                let destination = join_relative_remote_path(&remote_root, relative);
                if let Some(existing) = stat_optional(browser, &destination)
                    .await
                    .map_err(sftp_error)?
                    && existing.kind != SftpEntryKind::Directory
                {
                    return Err(AppError::validation(format!(
                        "The remote path {destination} exists but is not a directory."
                    )));
                }
                context.check_cancelled()?;
            }

            let mut conflicts = Vec::new();
            let mut files = Vec::with_capacity(scanned_files.len());
            for scanned in scanned_files {
                let destination = join_relative_remote_path(&remote_root, &scanned.relative_path);
                if let Some(existing) = stat_optional(browser, &destination)
                    .await
                    .map_err(sftp_error)?
                {
                    if existing.kind != SftpEntryKind::File {
                        return Err(AppError::validation(format!(
                            "The remote path {destination} exists but is not a regular file."
                        )));
                    }
                    if !overwrite {
                        conflicts.push(destination.clone());
                    }
                }
                files.push(UploadFilePlan {
                    local_path: scanned.local_path,
                    remote_path: destination,
                    size: scanned.size,
                    overwrite,
                });
                context.check_cancelled()?;
            }

            if !conflicts.is_empty() {
                return Err(directory_conflict_error(&conflicts));
            }

            Ok(UploadPlan::Directory(UploadDirectoryPlan {
                remote_root,
                directories,
                files,
                overwrite,
            }))
        }
    }
}

async fn ensure_remote_directory(
    browser: &dyn SftpBrowser,
    path: &str,
    context: &mut TransferContext<'_>,
) -> Result<(), AppError> {
    match stat_optional(browser, path).await.map_err(sftp_error)? {
        Some(metadata) if metadata.kind == SftpEntryKind::Directory => Ok(()),
        Some(_) => Err(AppError::validation(format!(
            "The remote path {path} exists but is not a directory."
        ))),
        None => {
            browser.mkdir(path).await.map_err(sftp_error)?;
            context.directories_created = context.directories_created.saturating_add(1);
            Ok(())
        }
    }
}

async fn execute_upload(
    browser: &dyn SftpBrowser,
    plan: &UploadPlan,
    context: &mut TransferContext<'_>,
) -> Result<(), AppError> {
    match plan {
        UploadPlan::File(file) => upload_one_file(browser, file, file.overwrite, context).await,
        UploadPlan::Directory(directory) => {
            ensure_remote_directory(browser, &directory.remote_root, context).await?;
            for relative in &directory.directories {
                context.check_cancelled()?;
                let destination = join_relative_remote_path(&directory.remote_root, relative);
                ensure_remote_directory(browser, &destination, context).await?;
            }
            for file in &directory.files {
                context.check_cancelled()?;
                upload_one_file(browser, file, directory.overwrite, context).await?;
            }
            Ok(())
        }
    }
}

async fn upload_one_file(
    browser: &dyn SftpBrowser,
    file: &UploadFilePlan,
    overwrite: bool,
    context: &mut TransferContext<'_>,
) -> Result<(), AppError> {
    context.check_cancelled()?;
    context.current_path = Some(file.remote_path.clone());
    let part_path = transfer_part_path(&file.remote_path, &context.transfer_id);
    let mut local = tokio::fs::File::open(&file.local_path)
        .await
        .map_err(|error| local_storage_error("read", error))?;
    let remote = browser
        .open_file(&part_path, SftpOpenMode::Write)
        .await
        .map_err(sftp_error)?;

    let mut offset = 0u64;
    let mut file_bytes = 0u64;
    let transfer = async {
        loop {
            context.check_cancelled()?;
            let mut buffer = vec![0u8; TRANSFER_CHUNK_BYTES as usize];
            let read = local
                .read(&mut buffer)
                .await
                .map_err(|error| local_storage_error("read", error))?;
            if read == 0 {
                break;
            }
            buffer.truncate(read);
            remote.write(offset, &buffer).await.map_err(sftp_error)?;
            offset = offset.saturating_add(read as u64);
            file_bytes = file_bytes.saturating_add(read as u64);
            context.bytes_transferred = context.bytes_transferred.saturating_add(read as u64);
            context.emit(false);
        }

        if offset != file.size {
            return Err(AppError::remote(
                "The local file changed size while it was being uploaded.",
            ));
        }
        context.check_cancelled()?;
        Ok(())
    }
    .await;

    if let Err(error) = transfer {
        let _ = remote.close().await;
        if file_bytes > 0 {
            context.record_partial(part_path);
        } else {
            let _ = browser.remove_file(&part_path).await;
        }
        return Err(error);
    }

    let _ = remote.close().await;
    if let Err(error) = context.check_cancelled() {
        context.record_partial(part_path);
        return Err(error);
    }

    if !overwrite
        && stat_optional(browser, &file.remote_path)
            .await
            .map_err(sftp_error)?
            .is_some()
    {
        context.record_partial(part_path);
        return Err(AppError::conflict(
            "The remote destination appeared while the upload was running.",
        )
        .with_details(
            "Overwrite was not confirmed. The complete partial file was kept and the existing \
             destination was not replaced.",
        ));
    }

    match browser.rename(&part_path, &file.remote_path).await {
        Ok(()) => {}
        Err(error) if overwrite => {
            // Move the existing destination aside first so a failed replace
            // can never destroy it before the complete part file is in place.
            let backup = transfer_part_path(&file.remote_path, "backup");
            let had_destination = stat_optional(browser, &file.remote_path)
                .await
                .map_err(sftp_error)?
                .is_some();
            if had_destination
                && let Err(backup_error) = browser.rename(&file.remote_path, &backup).await
            {
                context.record_partial(part_path);
                return Err(sftp_error(backup_error));
            }
            if let Err(second) = browser.rename(&part_path, &file.remote_path).await {
                if had_destination {
                    let _ = browser.rename(&backup, &file.remote_path).await;
                }
                context.record_partial(part_path);
                return Err(sftp_error(if matches!(second, SftpError::NotFound) {
                    error
                } else {
                    second
                }));
            }
            if had_destination {
                let _ = browser.remove_file(&backup).await;
            }
        }
        Err(error) => {
            context.record_partial(part_path);
            return Err(sftp_error(error));
        }
    }

    context.files_transferred = context.files_transferred.saturating_add(1);
    Ok(())
}

struct ScannedDownloadFile {
    remote_path: String,
    relative_path: String,
    size: Option<u64>,
}

async fn walk_remote_directory(
    browser: &dyn SftpBrowser,
    root_real: &str,
    source: &str,
    depth: usize,
    directories: &mut Vec<String>,
    files: &mut Vec<ScannedDownloadFile>,
    context: &mut TransferContext<'_>,
) -> Result<(), AppError> {
    let mut stack = vec![(source.to_string(), String::new(), depth)];
    let mut scanned = 0usize;

    while let Some((directory, relative, depth)) = stack.pop() {
        if depth > MAX_TRANSFER_DEPTH {
            return Err(AppError::validation(format!(
                "The remote directory is deeper than {MAX_TRANSFER_DEPTH} levels."
            )));
        }
        let remaining = MAX_TRANSFER_ENTRIES.saturating_sub(scanned).max(1);
        let (entries, truncated) = browser
            .read_dir(&directory, remaining)
            .await
            .map_err(sftp_error)?;
        if truncated {
            return Err(AppError::validation(format!(
                "The remote directory contains more than {MAX_TRANSFER_ENTRIES} entries."
            )));
        }
        scanned = scanned.saturating_add(entries.len());
        if scanned > MAX_TRANSFER_ENTRIES {
            return Err(AppError::validation(format!(
                "The remote directory contains more than {MAX_TRANSFER_ENTRIES} entries."
            )));
        }

        for entry in entries {
            context.check_cancelled()?;
            let Some(name) = safe_entry_name(&entry.name) else {
                context.skip(format!(
                    "Skipped a remote entry with an unsafe name: {:?}.",
                    entry.name
                ));
                continue;
            };
            let relative_path = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let child = join_remote_path(&directory, &name);

            match entry.metadata.kind {
                SftpEntryKind::Directory => {
                    let resolved = browser.canonicalize(&child).await.map_err(sftp_error)?;
                    if !is_within_remote_root(root_real, &resolved) {
                        return Err(resolved_outside_root_error_for_write());
                    }
                    directories.push(relative_path.clone());
                    stack.push((resolved, relative_path, depth + 1));
                }
                SftpEntryKind::File => {
                    let resolved = browser.canonicalize(&child).await.map_err(sftp_error)?;
                    if !is_within_remote_root(root_real, &resolved) {
                        return Err(resolved_outside_root_error_for_write());
                    }
                    files.push(ScannedDownloadFile {
                        remote_path: resolved,
                        relative_path,
                        size: entry.metadata.size,
                    });
                }
                SftpEntryKind::Symlink => {
                    context.skip(format!(
                        "Skipped remote symbolic link {relative_path}; links are never followed."
                    ));
                }
                SftpEntryKind::Other => {
                    context.skip(format!(
                        "Skipped special remote file {relative_path}; only regular files are downloaded."
                    ));
                }
            }
        }
    }

    Ok(())
}

async fn download_preflight(
    browser: &dyn SftpBrowser,
    root: &RemotePath,
    remote_path: &RemotePath,
    local_path: &Path,
    overwrite: bool,
    context: &mut TransferContext<'_>,
) -> Result<DownloadPlan, AppError> {
    let root_real = browser
        .canonicalize(root.as_str())
        .await
        .map_err(sftp_error)?;
    context.check_cancelled()?;

    let lexical = browser
        .lstat(remote_path.as_str())
        .await
        .map_err(sftp_error)?;
    if remote_path.as_str() != root.as_str() && lexical.kind == SftpEntryKind::Symlink {
        return Err(AppError::validation(
            "Downloading through a symbolic link is not supported. Choose the link target inside the root instead.",
        ));
    }
    let source_real = browser
        .canonicalize(remote_path.as_str())
        .await
        .map_err(sftp_error)?;
    if !is_within_remote_root(&root_real, &source_real) {
        return Err(resolved_outside_root_error_for_write());
    }

    match lexical.kind {
        SftpEntryKind::File => {
            context.kind = FileTransferKind::File;
            preflight_local_file_destination(local_path, overwrite)?;
            Ok(DownloadPlan {
                kind: FileTransferKind::File,
                local_root: local_path.to_path_buf(),
                size: lexical.size,
                directories: Vec::new(),
                files: vec![DownloadFilePlan {
                    remote_path: source_real,
                    relative_path: String::new(),
                    size: lexical.size,
                }],
                overwrite,
            })
        }
        SftpEntryKind::Directory => {
            context.kind = FileTransferKind::Directory;
            preflight_local_directory_destination(local_path)?;
            let mut directories = Vec::new();
            let mut scanned_files = Vec::new();
            walk_remote_directory(
                browser,
                &root_real,
                &source_real,
                0,
                &mut directories,
                &mut scanned_files,
                context,
            )
            .await?;
            context.check_cancelled()?;

            for relative in &directories {
                let destination = join_local_relative(local_path, relative);
                match tokio::fs::symlink_metadata(&destination).await {
                    Ok(metadata) => {
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(AppError::validation(format!(
                                "The local path {} exists but is not a real directory.",
                                destination.display()
                            )));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(local_storage_error("read", error)),
                }
            }

            let mut conflicts = Vec::new();
            let mut files = Vec::with_capacity(scanned_files.len());
            for scanned in scanned_files {
                let destination = join_local_relative(local_path, &scanned.relative_path);
                match tokio::fs::symlink_metadata(&destination).await {
                    Ok(metadata) => {
                        if metadata.file_type().is_symlink() || !metadata.is_file() {
                            return Err(AppError::validation(format!(
                                "The local path {} exists but is not a regular file.",
                                destination.display()
                            )));
                        }
                        if !overwrite {
                            conflicts.push(destination.display().to_string());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(local_storage_error("read", error)),
                }
                files.push(DownloadFilePlan {
                    remote_path: scanned.remote_path,
                    relative_path: scanned.relative_path,
                    size: scanned.size,
                });
                context.check_cancelled()?;
            }

            if !conflicts.is_empty() {
                return Err(directory_conflict_error(&conflicts));
            }

            Ok(DownloadPlan {
                kind: FileTransferKind::Directory,
                local_root: local_path.to_path_buf(),
                size: None,
                directories,
                files,
                overwrite,
            })
        }
        SftpEntryKind::Symlink | SftpEntryKind::Other => Err(AppError::validation(
            "Only regular remote files and directories can be downloaded.",
        )),
    }
}

fn join_local_relative(base: &Path, relative: &str) -> PathBuf {
    if relative.is_empty() {
        return base.to_path_buf();
    }
    let mut path = base.to_path_buf();
    for segment in relative.split('/').filter(|segment| !segment.is_empty()) {
        path.push(segment);
    }
    path
}

fn preflight_local_file_destination(destination: &Path, overwrite: bool) -> Result<(), AppError> {
    match std::fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(AppError::validation(
                    "The local destination is a symbolic link. Choose a real file path.",
                ));
            }
            if metadata.is_dir() {
                return Err(AppError::validation(
                    "The local destination is a directory, not a file.",
                ));
            }
            if !metadata.is_file() {
                return Err(AppError::validation(
                    "The local destination is not a regular file.",
                ));
            }
            if !overwrite {
                return Err(directory_conflict_error(&[destination
                    .display()
                    .to_string()]));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(local_storage_error("read", error)),
    }

    let parent = destination
        .parent()
        .ok_or_else(|| AppError::validation("Choose an absolute local destination file path."))?;
    match std::fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(AppError::validation(
            "The local destination parent is not a real directory.",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(AppError::validation(
            "The local destination parent directory does not exist.",
        )),
        Err(error) => Err(local_storage_error("read", error)),
    }
}

fn preflight_local_directory_destination(destination: &Path) -> Result<(), AppError> {
    if destination.exists() {
        let metadata = std::fs::symlink_metadata(destination)
            .map_err(|error| local_storage_error("read", error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AppError::validation(
                "The local destination exists but is not a real directory.",
            ));
        }
        return Ok(());
    }

    let parent = destination.parent().ok_or_else(|| {
        AppError::validation("Choose an absolute local destination directory path.")
    })?;
    match std::fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(AppError::validation(
            "The local destination parent is not a real directory.",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(AppError::validation(
            "The local destination parent directory does not exist.",
        )),
        Err(error) => Err(local_storage_error("read", error)),
    }
}

async fn ensure_local_directory(path: &Path, created: &mut u64) -> Result<(), AppError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(AppError::validation(format!(
            "The local path {} exists but is not a real directory.",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(|error| local_storage_error("created", error))?;
            *created = created.saturating_add(1);
            Ok(())
        }
        Err(error) => Err(local_storage_error("read", error)),
    }
}

async fn execute_download(
    browser: &dyn SftpBrowser,
    plan: &DownloadPlan,
    context: &mut TransferContext<'_>,
) -> Result<(), AppError> {
    match plan.kind {
        FileTransferKind::File => {
            let file = plan
                .files
                .first()
                .ok_or_else(|| AppError::internal("The download plan is missing its file."))?;
            download_one_file(browser, file, &plan.local_root, plan.overwrite, context).await
        }
        FileTransferKind::Directory => {
            ensure_local_directory(&plan.local_root, &mut context.directories_created).await?;
            for relative in &plan.directories {
                context.check_cancelled()?;
                let destination = join_local_relative(&plan.local_root, relative);
                ensure_local_directory(&destination, &mut context.directories_created).await?;
            }
            for file in &plan.files {
                context.check_cancelled()?;
                let destination = join_local_relative(&plan.local_root, &file.relative_path);
                download_one_file(browser, file, &destination, plan.overwrite, context).await?;
            }
            Ok(())
        }
    }
}

async fn download_one_file(
    browser: &dyn SftpBrowser,
    file: &DownloadFilePlan,
    destination: &Path,
    overwrite: bool,
    context: &mut TransferContext<'_>,
) -> Result<(), AppError> {
    context.check_cancelled()?;
    context.current_path = Some(file.remote_path.clone());
    let part_path = local_part_path(destination, &context.transfer_id);
    let remote = browser
        .open_file(&file.remote_path, SftpOpenMode::Read)
        .await
        .map_err(sftp_error)?;
    let mut local = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&part_path)
        .await
        .map_err(|error| local_storage_error("created", error))?;

    let mut offset = 0u64;
    let transfer = async {
        loop {
            context.check_cancelled()?;
            let data = remote
                .read(offset, TRANSFER_CHUNK_BYTES)
                .await
                .map_err(sftp_error)?;
            if data.is_empty() {
                break;
            }
            local
                .write_all(&data)
                .await
                .map_err(|error| local_storage_error("written", error))?;
            offset = offset.saturating_add(data.len() as u64);
            context.bytes_transferred = context.bytes_transferred.saturating_add(data.len() as u64);
            context.emit(false);
        }

        if let Some(expected) = file.size
            && offset != expected
        {
            return Err(AppError::remote(
                "The remote file changed size while it was being downloaded.",
            ));
        }
        local
            .flush()
            .await
            .map_err(|error| local_storage_error("written", error))?;
        context.check_cancelled()?;
        Ok(())
    }
    .await;

    if let Err(error) = transfer {
        let _ = remote.close().await;
        drop(local);
        if offset > 0 {
            context.record_partial(part_path.display().to_string());
        } else {
            let _ = tokio::fs::remove_file(&part_path).await;
        }
        return Err(error);
    }

    let _ = remote.close().await;
    drop(local);
    if let Err(error) = context.check_cancelled() {
        context.record_partial(part_path.display().to_string());
        return Err(error);
    }

    if !overwrite && destination.exists() {
        context.record_partial(part_path.display().to_string());
        return Err(AppError::conflict(
            "The local destination appeared while the download was running.",
        )
        .with_details(
            "Overwrite was not confirmed. The complete partial file was kept and the existing \
             destination was not replaced.",
        ));
    }

    if let Err(error) = tokio::fs::rename(&part_path, destination).await {
        if overwrite {
            // Keep the existing destination until the complete part file has
            // been moved into place, so a failed replace cannot destroy it.
            let mut backup = destination.as_os_str().to_os_string();
            backup.push(".serverdesk-replaced");
            let backup = PathBuf::from(backup);
            let had_destination = tokio::fs::symlink_metadata(destination).await.is_ok();
            if had_destination && tokio::fs::rename(destination, &backup).await.is_err() {
                context.record_partial(part_path.display().to_string());
                return Err(local_storage_error("written", error));
            }
            if tokio::fs::rename(&part_path, destination).await.is_err() {
                if had_destination {
                    let _ = tokio::fs::rename(&backup, destination).await;
                }
                context.record_partial(part_path.display().to_string());
                return Err(local_storage_error("written", error));
            }
            if had_destination {
                let _ = tokio::fs::remove_file(&backup).await;
            }
        } else {
            context.record_partial(part_path.display().to_string());
            return Err(local_storage_error("written", error));
        }
    }

    context.files_transferred = context.files_transferred.saturating_add(1);
    Ok(())
}

async fn resolve_mutation_target(
    browser: &dyn SftpBrowser,
    root: &RemotePath,
    path: &RemotePath,
) -> Result<(String, String, String, String, SftpMetadata), AppError> {
    if path.as_str() == root.as_str() {
        return Err(AppError::validation(
            "The selected root directory itself cannot be renamed or deleted.",
        ));
    }
    let root_real = browser
        .canonicalize(root.as_str())
        .await
        .map_err(sftp_error)?;
    let parent = remote_parent(path.as_str())
        .ok_or_else(|| AppError::validation("The remote path has no parent directory."))?;
    let parent_real = browser.canonicalize(parent).await.map_err(sftp_error)?;
    if !is_within_remote_root(&root_real, &parent_real) {
        return Err(resolved_outside_root_error_for_write());
    }
    let metadata = browser.stat(&parent_real).await.map_err(sftp_error)?;
    if metadata.kind != SftpEntryKind::Directory {
        return Err(AppError::validation(
            "The remote parent path is not a directory.",
        ));
    }
    let name = remote_name(path.as_str())
        .and_then(safe_entry_name)
        .ok_or_else(|| AppError::validation("The remote entry name is not valid."))?;
    let target = join_remote_path(&parent_real, &name);
    let target_metadata = browser.lstat(&target).await.map_err(sftp_error)?;
    Ok((root_real, parent_real, name, target, target_metadata))
}

#[allow(clippy::too_many_arguments)]
fn verified_result(
    server_id: &str,
    action: RemoteMutationAction,
    root: &str,
    path: &str,
    target_path: Option<String>,
    recursive: bool,
    deleted_entries: u64,
    verified: bool,
    detail: Option<String>,
    state: ConnectionState,
) -> RemoteMutationResult {
    RemoteMutationResult {
        server_id: server_id.to_string(),
        action,
        root: root.to_string(),
        path: path.to_string(),
        target_path,
        recursive,
        deleted_entries,
        verified,
        verification_detail: detail,
        completed_at: now_millis(),
        connection_state: state,
    }
}

/// Creates one directory inside the selected root.
pub async fn create_remote_folder(
    manager: &ConnectionManager,
    server_id: &str,
    root: &str,
    parent: &str,
    name: &str,
) -> Result<RemoteMutationResult, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }
    let (root, parent) = validate_browse_paths(root, parent)?;
    let name = safe_entry_name(name.trim())
        .ok_or_else(|| AppError::validation("The folder name is not valid."))?;
    let browser = manager.open_sftp(server_id).await?;

    let operation = async {
        let root_real = browser
            .canonicalize(root.as_str())
            .await
            .map_err(sftp_error)?;
        let parent_real = browser
            .canonicalize(parent.as_str())
            .await
            .map_err(sftp_error)?;
        if !is_within_remote_root(&root_real, &parent_real) {
            return Err(resolved_outside_root_error_for_write());
        }
        let metadata = browser.stat(&parent_real).await.map_err(sftp_error)?;
        if metadata.kind != SftpEntryKind::Directory {
            return Err(AppError::validation(
                "The remote parent path is not a directory.",
            ));
        }
        let target = join_remote_path(&parent_real, &name);
        if stat_optional(browser.as_ref(), &target)
            .await
            .map_err(sftp_error)?
            .is_some()
        {
            return Err(AppError::conflict(
                "A file or directory already exists at that path.",
            ));
        }
        browser.mkdir(&target).await.map_err(sftp_error)?;
        Ok::<String, AppError>(target)
    }
    .await;

    let target = match operation {
        Ok(target) => target,
        Err(error) => {
            let _ = browser.close().await;
            return Err(error);
        }
    };
    let verified = matches!(
        stat_optional(browser.as_ref(), &target).await,
        Ok(Some(SftpMetadata {
            kind: SftpEntryKind::Directory,
            ..
        }))
    );
    let state = manager
        .connection_state(server_id)
        .map(|snapshot| snapshot.state)
        .unwrap_or(ConnectionState::Disconnected);
    let _ = browser.close().await;
    Ok(verified_result(
        server_id,
        RemoteMutationAction::CreateFolder,
        root.as_str(),
        parent.as_str(),
        Some(target),
        false,
        0,
        verified,
        if verified {
            None
        } else {
            Some("The folder was created but could not be re-read.".to_string())
        },
        state,
    ))
}

/// Renames one entry inside the selected root. Confirmation is mandatory.
pub async fn rename_remote_entry(
    manager: &ConnectionManager,
    server_id: &str,
    root: &str,
    path: &str,
    new_name: &str,
    confirmed: bool,
) -> Result<RemoteMutationResult, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }
    if !confirmed {
        return Err(AppError::validation(
            "Renaming a remote entry requires explicit confirmation.",
        ));
    }
    let (root, path) = validate_browse_paths(root, path)?;
    let new_name = safe_entry_name(new_name.trim())
        .ok_or_else(|| AppError::validation("The new name is not valid."))?;
    let browser = manager.open_sftp(server_id).await?;

    let operation = async {
        let (_root_real, parent_real, old_name, source, source_metadata) =
            resolve_mutation_target(browser.as_ref(), &root, &path).await?;
        if old_name == new_name {
            return Err(AppError::validation(
                "The new name is the same as the current name.",
            ));
        }
        let target = join_remote_path(&parent_real, &new_name);
        if stat_optional(browser.as_ref(), &target)
            .await
            .map_err(sftp_error)?
            .is_some()
        {
            return Err(AppError::conflict(
                "A file or directory already exists at the new name.",
            ));
        }
        browser.rename(&source, &target).await.map_err(sftp_error)?;

        let source_missing = matches!(browser.lstat(&source).await, Err(SftpError::NotFound));
        let target_verified = matches!(
            browser.lstat(&target).await,
            Ok(metadata) if metadata.kind == source_metadata.kind
        );
        Ok::<(String, bool, bool), AppError>((target, source_missing, target_verified))
    }
    .await;

    let state = manager
        .connection_state(server_id)
        .map(|snapshot| snapshot.state)
        .unwrap_or(ConnectionState::Disconnected);
    let outcome = match operation {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = browser.close().await;
            return Err(error);
        }
    };
    let (target, source_missing, target_verified) = outcome;
    let _ = browser.close().await;
    let verified = source_missing && target_verified;
    Ok(verified_result(
        server_id,
        RemoteMutationAction::Rename,
        root.as_str(),
        path.as_str(),
        Some(target),
        false,
        0,
        verified,
        if verified {
            None
        } else {
            Some(
                "The rename completed but the expected state could not be fully verified."
                    .to_string(),
            )
        },
        state,
    ))
}

/// Deletes one entry inside the selected root. Confirmation is mandatory, and
/// directories additionally require `recursive`. Symbolic links are removed
/// as links and are never followed.
pub async fn delete_remote_entry(
    manager: &ConnectionManager,
    server_id: &str,
    root: &str,
    path: &str,
    confirmed: bool,
    recursive: bool,
) -> Result<RemoteMutationResult, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }
    if !confirmed {
        return Err(AppError::validation(
            "Deleting a remote entry requires explicit confirmation.",
        ));
    }
    let (root, path) = validate_browse_paths(root, path)?;
    let browser = manager.open_sftp(server_id).await?;

    let operation = async {
        let (_root_real, _parent_real, _name, target, metadata) =
            resolve_mutation_target(browser.as_ref(), &root, &path).await?;
        let mut deleted_entries = 1u64;
        if metadata.kind == SftpEntryKind::Directory {
            if !recursive {
                return Err(AppError::validation(
                    "Deleting a directory requires recursive confirmation.",
                ));
            }
            let mut entries: Vec<(String, bool)> = Vec::new();
            let mut stack = vec![(target.clone(), 0usize)];
            let mut scanned = 0usize;
            while let Some((directory, depth)) = stack.pop() {
                if depth > MAX_TRANSFER_DEPTH {
                    return Err(AppError::validation(format!(
                        "The remote directory is deeper than {MAX_TRANSFER_DEPTH} levels."
                    )));
                }
                let remaining = MAX_TRANSFER_ENTRIES.saturating_sub(scanned).max(1);
                let (children, truncated) = browser
                    .read_dir(&directory, remaining)
                    .await
                    .map_err(sftp_error)?;
                if truncated {
                    return Err(AppError::validation(format!(
                        "The remote directory contains more than {MAX_TRANSFER_ENTRIES} entries."
                    )));
                }
                scanned = scanned.saturating_add(children.len());
                if scanned > MAX_TRANSFER_ENTRIES {
                    return Err(AppError::validation(format!(
                        "The remote directory contains more than {MAX_TRANSFER_ENTRIES} entries."
                    )));
                }
                for child in children {
                    let name = safe_entry_name(&child.name).ok_or_else(|| {
                        AppError::validation(
                            "The remote directory contains an entry name ServerDesk cannot delete safely.",
                        )
                    })?;
                    let child_path = join_remote_path(&directory, &name);
                    let child_metadata =
                        browser.lstat(&child_path).await.map_err(sftp_error)?;
                    if child_metadata.kind == SftpEntryKind::Directory {
                        entries.push((child_path.clone(), true));
                        stack.push((child_path, depth + 1));
                    } else {
                        entries.push((child_path, false));
                    }
                }
            }
            for (entry, is_directory) in entries.into_iter().rev() {
                if is_directory {
                    browser.remove_dir(&entry).await.map_err(sftp_error)?;
                } else {
                    browser.remove_file(&entry).await.map_err(sftp_error)?;
                }
                deleted_entries = deleted_entries.saturating_add(1);
            }
            browser.remove_dir(&target).await.map_err(sftp_error)?;
        } else {
            browser.remove_file(&target).await.map_err(sftp_error)?;
        }
        Ok::<u64, AppError>(deleted_entries)
    }
    .await;

    let state = manager
        .connection_state(server_id)
        .map(|snapshot| snapshot.state)
        .unwrap_or(ConnectionState::Disconnected);
    let deleted_entries = match operation {
        Ok(deleted) => deleted,
        Err(error) => {
            let _ = browser.close().await;
            return Err(error);
        }
    };
    let target_gone = matches!(browser.lstat(path.as_str()).await, Err(SftpError::NotFound));
    let _ = browser.close().await;
    Ok(verified_result(
        server_id,
        RemoteMutationAction::Delete,
        root.as_str(),
        path.as_str(),
        None,
        recursive,
        deleted_entries,
        target_gone,
        if target_gone {
            None
        } else {
            Some("The delete completed but the path still exists.".to_string())
        },
        state,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths_and_rejects_traversal_and_paths_outside_the_root() {
        let (root, path) = validate_browse_paths(" /var//www/./app/ ", "/var/www/app/public")
            .expect("valid paths");
        assert_eq!(root.as_str(), "/var/www/app");
        assert_eq!(path.as_str(), "/var/www/app/public");

        for (root, path) in [
            ("/var/www/app", "/var/www/application"),
            ("/var/www/app", "/var/www"),
            ("/var/www/app", "/etc/passwd"),
            ("/var/www/app", "/var/www/app/../etc"),
            ("/var/www/app", "relative/path"),
            ("", "/var/www"),
            ("/var/www", ""),
        ] {
            assert!(
                validate_browse_paths(root, path).is_err(),
                "{root:?} {path:?} must be rejected"
            );
        }
    }

    #[test]
    fn root_containment_handles_the_filesystem_root_and_boundaries() {
        assert!(is_within_remote_root("/", "/etc/passwd"));
        assert!(is_within_remote_root("/var/www", "/var/www"));
        assert!(is_within_remote_root("/var/www", "/var/www/app"));
        assert!(!is_within_remote_root("/var/www", "/var/wwwroot"));
        assert!(!is_within_remote_root("/var/www", "/var"));
        assert!(!is_within_remote_root("/var/www/", "/var/www"));
    }

    #[test]
    fn formats_ls_style_permissions_and_special_bits() {
        assert_eq!(format_permissions(0o040755), "drwxr-xr-x");
        assert_eq!(format_permissions(0o100644), "-rw-r--r--");
        assert_eq!(format_permissions(0o120777), "lrwxrwxrwx");
        assert_eq!(format_permissions(0o104755), "-rwsr-xr-x");
        assert_eq!(format_permissions(0o102755), "-rwxr-sr-x");
        assert_eq!(format_permissions(0o041777), "drwxrwxrwt");
        assert_eq!(format_permissions(0o041770), "drwxrwx--T");
        assert_eq!(format_permissions(0o041776), "drwxrwxrwT");
    }

    #[test]
    fn rejects_unsafe_entry_names_and_bounds_symlink_targets() {
        for valid in ["file.txt", "dir with spaces", ".hidden", "a-b_c"] {
            assert_eq!(safe_entry_name(valid).as_deref(), Some(valid));
        }
        for invalid in ["", ".", "..", "a/b", "a\0b", "line\nbreak"] {
            assert!(safe_entry_name(invalid).is_none(), "{invalid:?}");
        }

        let long = "a".repeat(MAX_SYMLINK_TARGET_CHARS + 20);
        assert_eq!(
            safe_symlink_target(&long)
                .expect("bounded target")
                .chars()
                .count(),
            MAX_SYMLINK_TARGET_CHARS
        );
        assert!(safe_symlink_target("\n").is_none());
    }

    #[test]
    fn joins_entry_paths_without_double_slashes() {
        assert_eq!(join_remote_path("/", "etc"), "/etc");
        assert_eq!(join_remote_path("/var/www", "app"), "/var/www/app");
    }

    #[test]
    fn serializes_metadata_and_symlink_flags_for_the_frontend() {
        let listing = RemoteFileListing {
            server_id: "server-1".to_string(),
            root: "/var/www/app".to_string(),
            path: "/var/www/app".to_string(),
            resolved_path: "/var/www/app".to_string(),
            entries: vec![
                RemoteFileEntry {
                    name: "public".to_string(),
                    path: "/var/www/app/public".to_string(),
                    entry_type: RemoteEntryType::Directory,
                    size: Some(4_096),
                    permissions: Some("drwxr-xr-x".to_string()),
                    mode: Some(0o040755),
                    modified_at: Some(1_762_665_720),
                    symlink_target: None,
                    resolved_path: None,
                    symlink_escapes_root: None,
                    broken_symlink: false,
                },
                RemoteFileEntry {
                    name: "latest".to_string(),
                    path: "/var/www/app/latest".to_string(),
                    entry_type: RemoteEntryType::Symlink,
                    size: Some(12),
                    permissions: Some("lrwxrwxrwx".to_string()),
                    mode: Some(0o120777),
                    modified_at: Some(1_762_665_720),
                    symlink_target: Some("/etc".to_string()),
                    resolved_path: None,
                    symlink_escapes_root: Some(true),
                    broken_symlink: false,
                },
            ],
            truncated: false,
            sampled_at: 1_762_665_720_000,
            duration_ms: 12,
            connection_state: ConnectionState::Connected,
        };

        let value = serde_json::to_value(listing).expect("serialized listing");
        assert_eq!(value["root"], "/var/www/app");
        assert_eq!(value["entries"][0]["entryType"], "directory");
        assert_eq!(value["entries"][0]["permissions"], "drwxr-xr-x");
        assert_eq!(value["entries"][0]["modifiedAt"], 1_762_665_720_i64);
        assert_eq!(value["entries"][1]["entryType"], "symlink");
        assert_eq!(value["entries"][1]["symlinkEscapesRoot"], true);
        assert!(value["entries"][0].get("symlinkTarget").is_none());
    }

    #[test]
    fn sftp_errors_map_to_typed_application_errors() {
        assert_eq!(
            sftp_error(SftpError::PermissionDenied).code,
            crate::core::ErrorCode::Permission
        );
        assert_eq!(
            sftp_error(SftpError::NotFound).code,
            crate::core::ErrorCode::NotFound
        );
        assert_eq!(
            sftp_error(SftpError::NotDirectory).code,
            crate::core::ErrorCode::Validation
        );
        assert_eq!(
            sftp_error(SftpError::Timeout).code,
            crate::core::ErrorCode::Timeout
        );
        assert_eq!(
            sftp_error(SftpError::Disconnected).code,
            crate::core::ErrorCode::Connection
        );
    }
}
