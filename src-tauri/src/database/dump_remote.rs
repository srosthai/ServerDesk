//! SSH/SFTP implementation of the dump remote contract.
//!
//! The dump tools execute over the managed SSH session. One SFTP channel is
//! reused for the temporary credential file, progress samples, downloads, and
//! uploads. No database or file-transfer port is opened outside SSH.

use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::core::{
    AppError,
    command::shell_quote,
    ssh::{CommandOutput, ConnectionManager, SftpBrowser, SftpEntryKind, SftpError, SftpOpenMode},
};

use super::dump::{DumpRemote, DumpRemoteSession, RemoteCommandOutput, RemoteFileInfo};
use super::rows::RowQueryCancellation;

const TRANSFER_CHUNK_BYTES: u32 = 32 * 1024;

/// Opens dump sessions on the managed SSH connection.
pub struct SshDumpRemote {
    connections: Arc<ConnectionManager>,
}

impl SshDumpRemote {
    pub fn new(connections: Arc<ConnectionManager>) -> Self {
        Self { connections }
    }
}

#[async_trait]
impl DumpRemote for SshDumpRemote {
    async fn open(&self, server_id: &str) -> Result<Box<dyn DumpRemoteSession>, AppError> {
        let browser = self.connections.open_sftp(server_id).await?;
        Ok(Box::new(SshDumpRemoteSession {
            connections: Arc::clone(&self.connections),
            server_id: server_id.to_string(),
            browser,
        }))
    }
}

struct SshDumpRemoteSession {
    connections: Arc<ConnectionManager>,
    server_id: String,
    browser: Arc<dyn SftpBrowser>,
}

impl SshDumpRemoteSession {
    fn sftp_error(error: SftpError) -> AppError {
        error.to_app_error()
    }
}

#[async_trait]
impl DumpRemoteSession for SshDumpRemoteSession {
    async fn execute(&self, command: &str) -> Result<RemoteCommandOutput, AppError> {
        let output = self.connections.execute(&self.server_id, command).await?;
        Ok(remote_output(output))
    }

    async fn execute_long(
        &self,
        command: &str,
        timeout: Duration,
    ) -> Result<RemoteCommandOutput, AppError> {
        let output = self
            .connections
            .execute_with_timeout(&self.server_id, command, timeout)
            .await?;
        Ok(remote_output(output))
    }

    async fn write_private_file(&self, path: &str, bytes: &[u8]) -> Result<(), AppError> {
        let file = self
            .browser
            .open_file(path, SftpOpenMode::Write)
            .await
            .map_err(Self::sftp_error)?;
        let mut offset = 0u64;
        for chunk in bytes.chunks(TRANSFER_CHUNK_BYTES as usize) {
            if let Err(error) = file.write(offset, chunk).await {
                let _ = file.close().await;
                return Err(Self::sftp_error(error));
            }
            offset = offset.saturating_add(chunk.len() as u64);
        }
        file.close().await.map_err(Self::sftp_error)?;

        let output = self
            .connections
            .execute(
                &self.server_id,
                &format!("chmod 600 -- {}", shell_quote(path)),
            )
            .await?;
        if output.exit_code != Some(0) {
            return Err(AppError::remote(
                "The temporary credential file could not be protected on the remote server.",
            ));
        }
        Ok(())
    }

    async fn stat(&self, path: &str) -> Result<Option<RemoteFileInfo>, AppError> {
        match self.browser.stat(path).await {
            Ok(metadata) => Ok(Some(RemoteFileInfo::from_metadata(metadata))),
            Err(SftpError::NotFound) => Ok(None),
            Err(error) => Err(Self::sftp_error(error)),
        }
    }

    async fn read_bounded(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, AppError> {
        let file = match self.browser.open_file(path, SftpOpenMode::Read).await {
            Ok(file) => file,
            Err(SftpError::NotFound) => return Ok(None),
            Err(error) => return Err(Self::sftp_error(error)),
        };
        let mut bytes = Vec::new();
        while bytes.len() < max_bytes {
            let remaining = max_bytes - bytes.len();
            let length = u32::try_from(remaining.min(TRANSFER_CHUNK_BYTES as usize))
                .unwrap_or(TRANSFER_CHUNK_BYTES);
            match file.read(bytes.len() as u64, length).await {
                Ok(chunk) if chunk.is_empty() => break,
                Ok(chunk) => bytes.extend_from_slice(&chunk),
                Err(error) => {
                    let _ = file.close().await;
                    return Err(Self::sftp_error(error));
                }
            }
        }
        let _ = file.close().await;
        Ok(Some(bytes))
    }

    async fn download(
        &self,
        remote_path: &str,
        local_path: &Path,
        total_bytes: u64,
        cancel: &RowQueryCancellation,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<u64, AppError> {
        let remote = self
            .browser
            .open_file(remote_path, SftpOpenMode::Read)
            .await
            .map_err(Self::sftp_error)?;

        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut local = options
            .open(local_path)
            .await
            .map_err(|_| AppError::internal("The local partial dump file could not be created."))?;

        let outcome: Result<u64, AppError> = async {
            let mut offset = 0u64;
            loop {
                if cancel.is_cancelled() {
                    return Err(super::errors::DatabaseError::cancelled(
                        "The dump download was cancelled.",
                    )
                    .into());
                }
                let chunk = remote
                    .read(offset, TRANSFER_CHUNK_BYTES)
                    .await
                    .map_err(Self::sftp_error)?;
                if chunk.is_empty() {
                    break;
                }
                local.write_all(&chunk).await.map_err(|_| {
                    AppError::internal("The local partial dump file could not be written.")
                })?;
                offset = offset.saturating_add(chunk.len() as u64);
                progress(offset.min(total_bytes.max(offset)));
            }
            local.flush().await.map_err(|_| {
                AppError::internal("The local partial dump file could not be flushed.")
            })?;
            local.sync_all().await.map_err(|_| {
                AppError::internal("The local partial dump file could not be synced.")
            })?;
            Ok(offset)
        }
        .await;

        let _ = remote.close().await;
        drop(local);
        if outcome.is_err() {
            let _ = tokio::fs::remove_file(local_path).await;
        }
        outcome
    }

    async fn upload(
        &self,
        local_path: &Path,
        remote_path: &str,
        total_bytes: u64,
        cancel: &RowQueryCancellation,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<u64, AppError> {
        let mut local = tokio::fs::File::open(local_path)
            .await
            .map_err(|_| AppError::not_found("The local dump source could not be opened."))?;
        let remote = self
            .browser
            .open_file(remote_path, SftpOpenMode::Write)
            .await
            .map_err(Self::sftp_error)?;

        let outcome: Result<u64, AppError> = async {
            let mut buffer = vec![0u8; TRANSFER_CHUNK_BYTES as usize];
            let mut offset = 0u64;
            loop {
                if cancel.is_cancelled() {
                    return Err(super::errors::DatabaseError::cancelled(
                        "The dump upload was cancelled.",
                    )
                    .into());
                }
                let read = local
                    .read(&mut buffer)
                    .await
                    .map_err(|_| AppError::internal("The local dump source could not be read."))?;
                if read == 0 {
                    break;
                }
                remote
                    .write(offset, &buffer[..read])
                    .await
                    .map_err(Self::sftp_error)?;
                offset = offset.saturating_add(read as u64);
                progress(offset.min(total_bytes.max(offset)));
            }
            Ok(offset)
        }
        .await;

        let _ = remote.close().await;
        if outcome.is_err() {
            let _ = self.browser.remove_file(remote_path).await;
        }
        outcome
    }
}

fn remote_output(output: CommandOutput) -> RemoteCommandOutput {
    RemoteCommandOutput {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.exit_code,
    }
}

/// Returns true when SFTP metadata describes a regular file.
pub fn is_regular_file(metadata: &crate::core::ssh::SftpMetadata) -> bool {
    metadata.kind == SftpEntryKind::File
}
