use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use russh::{
    ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect,
    client::{self, Config, Handle, Msg},
    keys::{HashAlg, PrivateKeyWithHashAlg},
};
use russh_sftp::{
    client::{RawSftpSession, error::Error as SftpClientError},
    protocol::{FileAttributes, FileType as SftpFileType, OpenFlags, StatusCode},
};
use tokio::net::TcpStream;

use crate::core::terminal::{TerminalChannel, TerminalChannelEvent, TerminalOptions, TerminalSize};

use super::{
    Authentication, CommandOutput, ConnectionRequest, SftpBrowser, SftpDirEntry, SftpEntryKind,
    SftpError, SftpFile, SftpMetadata, SftpOpenMode, SshSession, SshTransport, TransportError,
    TrustedHostKey, TunnelStream, private_key::load_private_key,
};

const MAX_COMMAND_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_TERMINAL_HANDSHAKE_BUFFER_BYTES: usize = 1024 * 1024;
const TERMINAL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SFTP_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SFTP_REQUEST_TIMEOUT_SECONDS: u64 = 10;

#[derive(Debug, Clone)]
enum HostKeyObservation {
    Trusted,
    Unknown {
        key_type: String,
        fingerprint: String,
    },
    Changed {
        key_type: String,
        expected_fingerprint: String,
        actual_fingerprint: String,
    },
}

struct ClientHandler {
    trusted_host_keys: Vec<TrustedHostKey>,
    observation: Arc<Mutex<Option<HostKeyObservation>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = server_public_key.public_key();
        let key_type = key.algorithm().as_str().to_string();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();

        let observation = if self
            .trusted_host_keys
            .iter()
            .any(|trusted| trusted.key_type == key_type && trusted.fingerprint == fingerprint)
        {
            HostKeyObservation::Trusted
        } else if self.trusted_host_keys.is_empty() {
            HostKeyObservation::Unknown {
                key_type,
                fingerprint,
            }
        } else {
            HostKeyObservation::Changed {
                key_type,
                expected_fingerprint: self
                    .trusted_host_keys
                    .iter()
                    .map(|trusted| trusted.fingerprint.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
                actual_fingerprint: fingerprint,
            }
        };

        let trusted = matches!(observation, HostKeyObservation::Trusted);
        if let Ok(mut stored) = self.observation.lock() {
            *stored = Some(observation);
        }

        Ok(trusted)
    }
}

pub struct RusshTransport;

impl RusshTransport {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RusshTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SshTransport for RusshTransport {
    async fn connect(
        &self,
        request: &ConnectionRequest,
    ) -> Result<Arc<dyn SshSession>, TransportError> {
        let addresses = tokio::net::lookup_host((request.host.as_str(), request.port))
            .await
            .map_err(|_| TransportError::Dns)?
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(TransportError::Dns);
        }

        let mut stream = None;
        let mut last_error = None;
        for address in addresses {
            match TcpStream::connect(address).await {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let stream = stream.ok_or_else(|| {
            last_error
                .map(map_io_error)
                .unwrap_or(TransportError::Network)
        })?;

        let observation = Arc::new(Mutex::new(None));
        let handler = ClientHandler {
            trusted_host_keys: request.trusted_host_keys.clone(),
            observation: Arc::clone(&observation),
        };
        let config = Arc::new(Config {
            inactivity_timeout: Some(Duration::from_secs(120)),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            nodelay: true,
            ..Config::default()
        });

        let mut handle = match client::connect_stream(config, stream, handler).await {
            Ok(handle) => handle,
            Err(error) => {
                let observation = observation
                    .lock()
                    .ok()
                    .and_then(|mut observation| observation.take());
                return Err(match observation {
                    Some(HostKeyObservation::Unknown {
                        key_type,
                        fingerprint,
                    }) => TransportError::HostKeyUnknown {
                        key_type,
                        fingerprint,
                    },
                    Some(HostKeyObservation::Changed {
                        key_type,
                        expected_fingerprint,
                        actual_fingerprint,
                    }) => TransportError::HostKeyChanged {
                        key_type,
                        expected_fingerprint,
                        actual_fingerprint,
                    },
                    _ => map_russh_error(error),
                });
            }
        };

        let authentication = match &request.authentication {
            Authentication::Password(password) => handle
                .authenticate_password(request.username.clone(), password.clone())
                .await
                .map_err(map_russh_error)?,
            Authentication::PrivateKey { key, passphrase } => {
                let key = load_private_key(key, passphrase.as_deref())?;
                let hash_alg = handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(map_russh_error)?
                    .flatten();
                handle
                    .authenticate_publickey(
                        request.username.clone(),
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                    )
                    .await
                    .map_err(map_russh_error)?
            }
        };

        if !authentication.success() {
            return Err(TransportError::Authentication);
        }

        Ok(Arc::new(RusshSession {
            handle: tokio::sync::Mutex::new(handle),
            closed: AtomicBool::new(false),
        }))
    }
}

struct RusshSession {
    handle: tokio::sync::Mutex<Handle<ClientHandler>>,
    closed: AtomicBool,
}

impl RusshSession {
    async fn execute_with_limit(
        &self,
        command: &str,
        max_output_bytes: usize,
    ) -> Result<CommandOutput, TransportError> {
        let handle = self.handle.lock().await;
        if self.closed.load(Ordering::SeqCst) || handle.is_closed() {
            return Err(TransportError::Network);
        }

        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(map_russh_error)?;
        channel
            .exec(true, command.as_bytes().to_vec())
            .await
            .map_err(map_russh_error)?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code = None;
        let mut output_too_large = false;

        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => {
                    if stdout
                        .len()
                        .saturating_add(stderr.len())
                        .saturating_add(data.len())
                        > max_output_bytes
                    {
                        output_too_large = true;
                        break;
                    }
                    stdout.extend_from_slice(&data);
                }
                ChannelMsg::ExtendedData { data, .. } => {
                    if stdout
                        .len()
                        .saturating_add(stderr.len())
                        .saturating_add(data.len())
                        > max_output_bytes
                    {
                        output_too_large = true;
                        break;
                    }
                    stderr.extend_from_slice(&data);
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = Some(exit_status);
                }
                ChannelMsg::ExitSignal { .. } => {
                    exit_code = None;
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }

        let _ = channel.close().await;

        if output_too_large {
            return Err(TransportError::Remote(
                "The remote command produced more output than ServerDesk allows.".to_string(),
            ));
        }

        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            exit_code,
        })
    }
}

#[async_trait]
impl SshSession for RusshSession {
    async fn execute(&self, command: &str) -> Result<CommandOutput, TransportError> {
        self.execute_with_limit(command, MAX_COMMAND_OUTPUT_BYTES)
            .await
    }

    async fn execute_bounded(
        &self,
        command: &str,
        max_output_bytes: usize,
    ) -> Result<CommandOutput, TransportError> {
        self.execute_with_limit(command, max_output_bytes.min(MAX_COMMAND_OUTPUT_BYTES))
            .await
    }

    async fn open_terminal(
        &self,
        options: TerminalOptions,
    ) -> Result<Arc<dyn TerminalChannel>, TransportError> {
        if let Err(error) = options.validate() {
            return Err(TransportError::Remote(error.message));
        }

        let mut channel = {
            let handle = self.handle.lock().await;
            if self.closed.load(Ordering::SeqCst) || handle.is_closed() {
                return Err(TransportError::Network);
            }
            handle
                .channel_open_session()
                .await
                .map_err(map_russh_error)?
        };

        let size = options.size;
        let handshake = async {
            channel
                .request_pty(
                    true,
                    &options.term,
                    size.cols,
                    size.rows,
                    size.pixel_width,
                    size.pixel_height,
                    &[],
                )
                .await
                .map_err(map_russh_error)?;
            channel.request_shell(true).await.map_err(map_russh_error)?;

            // Wait for the server to accept both requests. Any output sent
            // before the replies (a prompt, for example) is buffered so it is
            // not lost before the reader task starts.
            let mut pending = VecDeque::new();
            let mut pending_bytes = 0usize;
            let mut successes = 0;
            let wait_for_replies = async {
                while successes < 2 {
                    match channel.wait().await {
                        Some(ChannelMsg::Success) => successes += 1,
                        Some(ChannelMsg::Failure) => {
                            return Err(TransportError::Remote(
                                "The SSH server rejected the interactive terminal request."
                                    .to_string(),
                            ));
                        }
                        Some(ChannelMsg::Data { data }) => {
                            pending_bytes = pending_bytes.saturating_add(data.len());
                            if pending_bytes > MAX_TERMINAL_HANDSHAKE_BUFFER_BYTES {
                                return Err(TransportError::Remote(
                                    "The remote terminal produced too much output before its shell was ready."
                                        .to_string(),
                                ));
                            }
                            pending.push_back(data.to_vec());
                        }
                        Some(ChannelMsg::ExtendedData { data, .. }) => {
                            pending_bytes = pending_bytes.saturating_add(data.len());
                            if pending_bytes > MAX_TERMINAL_HANDSHAKE_BUFFER_BYTES {
                                return Err(TransportError::Remote(
                                    "The remote terminal produced too much output before its shell was ready."
                                        .to_string(),
                                ));
                            }
                            pending.push_back(data.to_vec());
                        }
                        Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                            return Err(TransportError::Network);
                        }
                        _ => {}
                    }
                }
                Ok(pending)
            };

            tokio::time::timeout(TERMINAL_HANDSHAKE_TIMEOUT, wait_for_replies)
                .await
                .map_err(|_| TransportError::Timeout)?
        }
        .await;

        let pending = match handshake {
            Ok(pending) => pending,
            Err(error) => {
                let _ = channel.close().await;
                return Err(error);
            }
        };

        let (read, write) = channel.split();
        Ok(Arc::new(RusshTerminalChannel {
            read: tokio::sync::Mutex::new(read),
            write,
            pending: Mutex::new(pending),
            closed: AtomicBool::new(false),
        }))
    }

    async fn open_sftp(&self) -> Result<Arc<dyn SftpBrowser>, TransportError> {
        let channel = {
            let handle = self.handle.lock().await;
            if self.closed.load(Ordering::SeqCst) || handle.is_closed() {
                return Err(TransportError::Network);
            }
            handle
                .channel_open_session()
                .await
                .map_err(map_russh_error)?
        };

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(map_russh_error)?;

        // The subsystem runs over this SSH channel on the authenticated
        // connection. No separate SFTP listener or file-transfer port is used.
        let session = RawSftpSession::new_with_config(
            channel.into_stream(),
            russh_sftp::client::Config {
                request_timeout_secs: SFTP_REQUEST_TIMEOUT_SECONDS,
                ..Default::default()
            },
        );

        match tokio::time::timeout(SFTP_HANDSHAKE_TIMEOUT, session.init()).await {
            Err(_) => {
                let _ = session.close_session();
                Err(TransportError::Timeout)
            }
            Ok(Err(error)) => {
                let _ = session.close_session();
                Err(map_sftp_setup_error(error))
            }
            Ok(Ok(_)) => Ok(Arc::new(RusshSftpBrowser {
                session: Arc::new(session),
            })),
        }
    }

    async fn open_direct_tcpip(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn TunnelStream>, TransportError> {
        let channel = {
            let handle = self.handle.lock().await;
            if self.closed.load(Ordering::SeqCst) || handle.is_closed() {
                return Err(TransportError::Network);
            }
            handle
                .channel_open_direct_tcpip(host.to_string(), u32::from(port), "127.0.0.1", 0)
                .await
                .map_err(map_russh_error)?
        };

        // The tunnel manager moves bytes between the loopback listener and this
        // channel. Dropping the stream closes the SSH channel and never exposes
        // a second local or remote port.
        Ok(Box::new(channel.into_stream()))
    }

    async fn is_healthy(&self) -> bool {
        if self.closed.load(Ordering::SeqCst) {
            return false;
        }
        // Contention means another operation is using the handle, not that the
        // session is dead. Waiting for the lock here could turn a long command
        // into a false health failure.
        match self.handle.try_lock() {
            Ok(handle) => !handle.is_closed(),
            Err(_) => true,
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.closed.store(true, Ordering::SeqCst);
        let handle = self.handle.lock().await;
        if handle.is_closed() {
            return Ok(());
        }

        handle
            .disconnect(Disconnect::ByApplication, "ServerDesk disconnected", "en")
            .await
            .map_err(map_russh_error)
    }
}

struct RusshTerminalChannel {
    read: tokio::sync::Mutex<ChannelReadHalf>,
    write: ChannelWriteHalf<Msg>,
    pending: Mutex<VecDeque<Vec<u8>>>,
    closed: AtomicBool,
}

#[async_trait]
impl TerminalChannel for RusshTerminalChannel {
    async fn read(&self) -> Option<TerminalChannelEvent> {
        if let Ok(mut pending) = self.pending.lock()
            && let Some(data) = pending.pop_front()
        {
            return Some(TerminalChannelEvent::Data(data));
        }
        if self.closed.load(Ordering::SeqCst) {
            return None;
        }

        let mut read = self.read.lock().await;
        loop {
            match read.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    return Some(TerminalChannelEvent::Data(data.to_vec()));
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    return Some(TerminalChannelEvent::Data(data.to_vec()));
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return Some(TerminalChannelEvent::ExitStatus(exit_status));
                }
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    return Some(TerminalChannelEvent::ExitSignal(format!("{signal_name:?}")));
                }
                Some(ChannelMsg::Close) | None => {
                    self.closed.store(true, Ordering::SeqCst);
                    return Some(TerminalChannelEvent::Closed);
                }
                _ => {}
            }
        }
    }

    async fn write(&self, data: &[u8]) -> Result<(), TransportError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(TransportError::Network);
        }
        self.write
            .data_bytes(data.to_vec())
            .await
            .map_err(map_russh_error)
    }

    async fn resize(&self, size: TerminalSize) -> Result<(), TransportError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(TransportError::Network);
        }
        self.write
            .window_change(size.cols, size.rows, size.pixel_width, size.pixel_height)
            .await
            .map_err(map_russh_error)
    }

    async fn close(&self) -> Result<(), TransportError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.write.close().await.map_err(map_russh_error)
    }
}

struct RusshSftpBrowser {
    session: Arc<RawSftpSession>,
}

struct RusshSftpFile {
    session: Arc<RawSftpSession>,
    handle: String,
    closed: AtomicBool,
}

#[async_trait]
impl SftpFile for RusshSftpFile {
    async fn read(&self, offset: u64, length: u32) -> Result<Vec<u8>, SftpError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SftpError::Disconnected);
        }
        match self
            .session
            .read(self.handle.as_str(), offset, length)
            .await
        {
            Ok(data) => Ok(data.data),
            Err(SftpClientError::Status(status)) if status.status_code == StatusCode::Eof => {
                Ok(Vec::new())
            }
            Err(error) => Err(map_sftp_client_error(error)),
        }
    }

    async fn write(&self, offset: u64, data: &[u8]) -> Result<(), SftpError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SftpError::Disconnected);
        }
        self.session
            .write(self.handle.as_str(), offset, data.to_vec())
            .await
            .map_err(map_sftp_client_error)?;
        Ok(())
    }

    async fn close(&self) -> Result<(), SftpError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.session
            .close(self.handle.as_str())
            .await
            .map_err(map_sftp_client_error)?;
        Ok(())
    }
}

#[async_trait]
impl SftpBrowser for RusshSftpBrowser {
    async fn canonicalize(&self, path: &str) -> Result<String, SftpError> {
        let name = self
            .session
            .realpath(path)
            .await
            .map_err(map_sftp_client_error)?;
        name.files
            .into_iter()
            .next()
            .map(|file| file.filename)
            .ok_or_else(|| SftpError::Protocol("The SFTP server returned no resolved path.".into()))
    }

    async fn stat(&self, path: &str) -> Result<SftpMetadata, SftpError> {
        let attrs = self
            .session
            .stat(path)
            .await
            .map_err(map_sftp_client_error)?;
        Ok(metadata_from_attrs(&attrs.attrs))
    }

    async fn lstat(&self, path: &str) -> Result<SftpMetadata, SftpError> {
        let attrs = self
            .session
            .lstat(path)
            .await
            .map_err(map_sftp_client_error)?;
        Ok(metadata_from_attrs(&attrs.attrs))
    }

    async fn read_dir(
        &self,
        path: &str,
        max_entries: usize,
    ) -> Result<(Vec<SftpDirEntry>, bool), SftpError> {
        let handle = self
            .session
            .opendir(path)
            .await
            .map_err(map_sftp_client_error)?;
        let mut entries = Vec::new();
        let mut truncated = false;

        loop {
            match self.session.readdir(handle.handle.as_str()).await {
                Ok(name) => {
                    for file in name.files {
                        // OpenSSH and other SFTP servers include the synthetic
                        // `.` and `..` entries in directory listings. They are
                        // not real children, are never valid file names, and
                        // must not count against the entry bound or reach the
                        // recursive delete/transfer walkers.
                        if file.filename == "." || file.filename == ".." {
                            continue;
                        }
                        if entries.len() >= max_entries {
                            truncated = true;
                            break;
                        }
                        entries.push(SftpDirEntry {
                            name: file.filename,
                            metadata: metadata_from_attrs(&file.attrs),
                        });
                    }
                    if truncated {
                        break;
                    }
                }
                Err(SftpClientError::Status(status)) if status.status_code == StatusCode::Eof => {
                    break;
                }
                Err(error) => {
                    let _ = self.session.close(handle.handle).await;
                    return Err(map_sftp_client_error(error));
                }
            }
        }

        let _ = self.session.close(handle.handle).await;
        Ok((entries, truncated))
    }

    async fn read_link(&self, path: &str) -> Result<String, SftpError> {
        let name = self
            .session
            .readlink(path)
            .await
            .map_err(map_sftp_client_error)?;
        name.files
            .into_iter()
            .next()
            .map(|file| file.filename)
            .ok_or_else(|| SftpError::Protocol("The SFTP link has no target.".into()))
    }

    async fn open_file(
        &self,
        path: &str,
        mode: SftpOpenMode,
    ) -> Result<Box<dyn SftpFile>, SftpError> {
        let flags = match mode {
            SftpOpenMode::Read => OpenFlags::READ,
            SftpOpenMode::Write => OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
        };
        let handle = self
            .session
            .open(path, flags, FileAttributes::default())
            .await
            .map_err(map_sftp_client_error)?;
        Ok(Box::new(RusshSftpFile {
            session: Arc::clone(&self.session),
            handle: handle.handle,
            closed: AtomicBool::new(false),
        }))
    }

    async fn mkdir(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .mkdir(path, FileAttributes::default())
            .await
            .map_err(map_sftp_client_error)?;
        Ok(())
    }

    async fn remove_file(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .remove(path)
            .await
            .map_err(map_sftp_client_error)?;
        Ok(())
    }

    async fn remove_dir(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .rmdir(path)
            .await
            .map_err(map_sftp_client_error)?;
        Ok(())
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), SftpError> {
        self.session
            .rename(from, to)
            .await
            .map_err(map_sftp_client_error)?;
        Ok(())
    }

    async fn close(&self) -> Result<(), SftpError> {
        self.session.close_session().map_err(map_sftp_client_error)
    }
}

fn metadata_from_attrs(attrs: &FileAttributes) -> SftpMetadata {
    SftpMetadata {
        kind: match attrs.file_type() {
            SftpFileType::Dir => SftpEntryKind::Directory,
            SftpFileType::File => SftpEntryKind::File,
            SftpFileType::Symlink => SftpEntryKind::Symlink,
            SftpFileType::Other => SftpEntryKind::Other,
        },
        size: attrs.size,
        mode: attrs.permissions,
        modified_at: attrs.mtime.map(i64::from),
    }
}

fn map_sftp_client_error(error: SftpClientError) -> SftpError {
    match error {
        SftpClientError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => SftpError::NotFound,
            StatusCode::PermissionDenied => SftpError::PermissionDenied,
            StatusCode::NoConnection | StatusCode::ConnectionLost => SftpError::Disconnected,
            StatusCode::OpUnsupported => SftpError::Unsupported,
            _ => SftpError::Protocol(status.error_message),
        },
        SftpClientError::Timeout => SftpError::Timeout,
        SftpClientError::IO(detail)
            if detail.to_ascii_lowercase().contains("eof")
                || detail.to_ascii_lowercase().contains("closed") =>
        {
            SftpError::Disconnected
        }
        SftpClientError::UnexpectedBehavior(detail) if detail.contains("session closed") => {
            SftpError::Disconnected
        }
        other => SftpError::Protocol(other.to_string()),
    }
}

fn map_sftp_setup_error(error: SftpClientError) -> TransportError {
    match map_sftp_client_error(error) {
        SftpError::Timeout => TransportError::Timeout,
        SftpError::Disconnected => TransportError::Network,
        SftpError::Unsupported => TransportError::Remote(
            "The SSH server does not support the SFTP subsystem.".to_string(),
        ),
        other => TransportError::Remote(other.to_app_error().message),
    }
}

fn map_io_error(error: std::io::Error) -> TransportError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => TransportError::Timeout,
        std::io::ErrorKind::NotFound | std::io::ErrorKind::AddrNotAvailable => TransportError::Dns,
        _ => TransportError::Network,
    }
}

fn map_russh_error(error: russh::Error) -> TransportError {
    match error {
        russh::Error::ConnectionTimeout
        | russh::Error::KeepaliveTimeout
        | russh::Error::InactivityTimeout
        | russh::Error::Elapsed(_) => TransportError::Timeout,
        russh::Error::IO(error) => map_io_error(error),
        russh::Error::UnknownKey | russh::Error::WrongServerSig => TransportError::HostKeyChanged {
            key_type: "unknown".to_string(),
            expected_fingerprint: "a saved host key".to_string(),
            actual_fingerprint: "an untrusted key".to_string(),
        },
        russh::Error::NotAuthenticated | russh::Error::UnsupportedAuthMethod => {
            TransportError::Authentication
        }
        russh::Error::CouldNotReadKey => TransportError::Credential(
            "The private key could not be read. Check the key format and permissions.".to_string(),
        ),
        russh::Error::Keys(russh::keys::Error::KeyIsEncrypted) => TransportError::Credential(
            "The private key is encrypted and cannot be used yet.".to_string(),
        ),
        russh::Error::Keys(_) => TransportError::Protocol,
        russh::Error::HUP | russh::Error::Disconnect => TransportError::Network,
        russh::Error::ChannelOpenFailure(_)
        | russh::Error::RequestDenied
        | russh::Error::RecvError
        | russh::Error::SendError => {
            TransportError::Remote("The SSH server rejected the remote operation.".to_string())
        }
        _ => TransportError::Protocol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_internal_timeouts_to_timeout_errors() {
        assert_eq!(
            map_russh_error(russh::Error::ConnectionTimeout),
            TransportError::Timeout
        );
    }

    #[test]
    fn authentication_and_unreadable_key_errors_keep_distinct_categories() {
        for error in [
            russh::Error::NotAuthenticated,
            russh::Error::UnsupportedAuthMethod,
        ] {
            assert_eq!(map_russh_error(error), TransportError::Authentication);
        }
        assert!(matches!(
            map_russh_error(russh::Error::CouldNotReadKey),
            TransportError::Credential(_)
        ));
    }

    #[test]
    fn fingerprints_use_the_standard_openssh_sha256_format() {
        let mut rng = rand::rng();
        let key = russh::keys::PrivateKey::random(&mut rng, russh::keys::Algorithm::Ed25519)
            .expect("generate test key");
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();

        assert!(fingerprint.starts_with("SHA256:"));
        let encoded = fingerprint.trim_start_matches("SHA256:");
        assert_eq!(encoded.len(), 43);
        assert!(
            encoded.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '+' | '/')
            )
        );
        assert_eq!(key.public_key().algorithm().as_str(), "ssh-ed25519");
    }
}
