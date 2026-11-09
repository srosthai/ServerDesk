use std::sync::Arc;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::core::{
    AppError,
    files::{
        DownloadRequest, FILE_TRANSFER_EVENT, FileTransferEventSink, FileTransferManager,
        FileTransferProgress, FileTransferResult, RemoteFileListing, RemoteMutationResult,
        UploadRequest, browse_remote_files as browse, create_remote_folder as create_folder,
        delete_remote_entry as delete_entry, rename_remote_entry as rename_entry,
    },
    ssh::ConnectionManager,
};

pub struct TauriFileTransferEventSink {
    app: AppHandle,
}

impl TauriFileTransferEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl FileTransferEventSink for TauriFileTransferEventSink {
    fn emit(&self, progress: &FileTransferProgress) {
        let _ = self.app.emit(FILE_TRANSFER_EVENT, progress.clone());
    }
}

/// Browses one directory inside the selected root over the authenticated SFTP
/// subsystem. The root and path are normalized and checked again in Rust.
#[tauri::command]
pub async fn browse_remote_files(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    root: String,
    path: String,
) -> Result<RemoteFileListing, AppError> {
    browse(manager.inner().as_ref(), &id, &root, &path).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadFileInput {
    pub transfer_id: String,
    pub server_id: String,
    pub root: String,
    pub local_path: String,
    pub remote_path: String,
    #[serde(default)]
    pub overwrite: bool,
}

/// Uploads one local file or directory over the managed SSH session.
#[tauri::command]
pub async fn upload_remote_file(
    transfers: State<'_, Arc<FileTransferManager>>,
    input: UploadFileInput,
) -> Result<FileTransferResult, AppError> {
    transfers
        .upload(UploadRequest {
            transfer_id: input.transfer_id,
            server_id: input.server_id,
            root: input.root,
            local_path: input.local_path,
            remote_path: input.remote_path,
            overwrite: input.overwrite,
        })
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFileInput {
    pub transfer_id: String,
    pub server_id: String,
    pub root: String,
    pub remote_path: String,
    pub local_path: String,
    #[serde(default)]
    pub overwrite: bool,
}

/// Downloads one remote file or directory over the managed SSH session.
#[tauri::command]
pub async fn download_remote_file(
    transfers: State<'_, Arc<FileTransferManager>>,
    input: DownloadFileInput,
) -> Result<FileTransferResult, AppError> {
    transfers
        .download(DownloadRequest {
            transfer_id: input.transfer_id,
            server_id: input.server_id,
            root: input.root,
            remote_path: input.remote_path,
            local_path: input.local_path,
            overwrite: input.overwrite,
        })
        .await
}

#[tauri::command]
pub fn cancel_file_transfer(
    transfers: State<'_, Arc<FileTransferManager>>,
    transfer_id: String,
) -> Result<bool, AppError> {
    transfers.cancel(&transfer_id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderInput {
    pub server_id: String,
    pub root: String,
    pub parent: String,
    pub name: String,
}

#[tauri::command]
pub async fn create_remote_folder(
    manager: State<'_, Arc<ConnectionManager>>,
    input: CreateFolderInput,
) -> Result<RemoteMutationResult, AppError> {
    create_folder(
        manager.inner().as_ref(),
        &input.server_id,
        &input.root,
        &input.parent,
        &input.name,
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameEntryInput {
    pub server_id: String,
    pub root: String,
    pub path: String,
    pub new_name: String,
    #[serde(default)]
    pub confirmed: bool,
}

#[tauri::command]
pub async fn rename_remote_entry(
    manager: State<'_, Arc<ConnectionManager>>,
    input: RenameEntryInput,
) -> Result<RemoteMutationResult, AppError> {
    rename_entry(
        manager.inner().as_ref(),
        &input.server_id,
        &input.root,
        &input.path,
        &input.new_name,
        input.confirmed,
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEntryInput {
    pub server_id: String,
    pub root: String,
    pub path: String,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default)]
    pub recursive: bool,
}

#[tauri::command]
pub async fn delete_remote_entry(
    manager: State<'_, Arc<ConnectionManager>>,
    input: DeleteEntryInput,
) -> Result<RemoteMutationResult, AppError> {
    delete_entry(
        manager.inner().as_ref(),
        &input.server_id,
        &input.root,
        &input.path,
        input.confirmed,
        input.recursive,
    )
    .await
}
