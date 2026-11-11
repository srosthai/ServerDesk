use std::sync::Arc;

use tauri::State;

use crate::{
    core::{
        AppError,
        credential_index::CredentialIndex,
        credentials::CredentialStore,
        database_credentials::DatabaseCredentialService,
        ssh::{TunnelManager, TunnelRequest, TunnelSnapshot},
    },
    storage::Database,
};

#[tauri::command]
pub async fn open_database_tunnel(
    database: State<'_, Database>,
    tunnels: State<'_, Arc<TunnelManager>>,
    profile_id: String,
) -> Result<TunnelSnapshot, AppError> {
    let profile = database
        .get_database_connection(&profile_id)?
        .ok_or_else(|| AppError::not_found("Database profile not found."))?;

    tunnels
        .open(TunnelRequest {
            profile_id: profile.id,
            server_id: profile.server_id,
            remote_host: profile.host,
            remote_port: profile.port,
            socket: profile.socket,
        })
        .await
}

#[tauri::command]
pub async fn close_database_tunnel(
    tunnels: State<'_, Arc<TunnelManager>>,
    profile_id: String,
) -> Result<(), AppError> {
    tunnels.close(&profile_id).await
}

#[tauri::command]
pub fn get_database_tunnel_state(
    tunnels: State<'_, Arc<TunnelManager>>,
    profile_id: String,
) -> Result<Option<TunnelSnapshot>, AppError> {
    tunnels.snapshot(&profile_id)
}

#[tauri::command]
pub fn list_database_tunnels(
    tunnels: State<'_, Arc<TunnelManager>>,
) -> Result<Vec<TunnelSnapshot>, AppError> {
    tunnels.list()
}

/// Deletes a database profile after closing its tunnel and saved password.
#[tauri::command]
pub async fn delete_database_connection(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    tunnels: State<'_, Arc<TunnelManager>>,
    id: String,
) -> Result<(), AppError> {
    tunnels.close(&id).await?;
    let service =
        DatabaseCredentialService::new(credentials.inner().as_ref(), credential_index.inner());
    service.delete_profile(database.inner(), &id)
}
