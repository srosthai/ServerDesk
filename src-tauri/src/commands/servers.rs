use std::sync::Arc;

use tauri::State;

use crate::{
    core::{
        AppError,
        credential_index::CredentialIndex,
        credentials::CredentialStore,
        servers::{ServerProfile, ServerProfileInput},
        ssh::ConnectionManager,
    },
    storage::{Database, NewServer, Server},
};

#[tauri::command]
pub fn list_servers(database: State<'_, Database>) -> Result<Vec<ServerProfile>, AppError> {
    let servers = database
        .list_servers()?
        .into_iter()
        .map(ServerProfile::from)
        .collect();

    Ok(servers)
}

#[tauri::command]
pub fn get_server(
    database: State<'_, Database>,
    id: String,
) -> Result<Option<ServerProfile>, AppError> {
    Ok(database.get_server(&id)?.map(ServerProfile::from))
}

#[tauri::command]
pub fn create_server(
    database: State<'_, Database>,
    input: ServerProfileInput,
) -> Result<ServerProfile, AppError> {
    let new_server = input.into_new_server()?;

    if database.server_name_exists(&new_server.name, None)? {
        return Err(AppError::validation(
            "A server with this name already exists.",
        ));
    }

    Ok(ServerProfile::from(database.create_server(&new_server)?))
}

#[tauri::command]
pub async fn update_server(
    database: State<'_, Database>,
    connection_manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    input: ServerProfileInput,
) -> Result<ServerProfile, AppError> {
    let new_server = input.into_new_server()?;

    if database.server_name_exists(&new_server.name, Some(&id))? {
        return Err(AppError::validation(
            "A server with this name already exists.",
        ));
    }

    if let Some(existing) = database.get_server(&id)?
        && connection_details_changed(&existing, &new_server)
    {
        connection_manager.disconnect(&id).await?;
    }

    Ok(ServerProfile::from(
        database.update_server(&id, &new_server)?,
    ))
}

fn connection_details_changed(existing: &Server, updated: &NewServer) -> bool {
    existing.host != updated.host
        || existing.port != updated.port
        || existing.username != updated.username
        || existing.auth_type != updated.auth_type
        || existing.credential_ref != updated.credential_ref
}

#[tauri::command]
pub async fn delete_server(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connection_manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    confirmed: bool,
) -> Result<(), AppError> {
    if !confirmed {
        return Err(AppError::validation(
            "Deleting a server profile requires explicit confirmation.",
        )
        .with_details(
            "The server, its saved credential reference, and its local records are removed. No remote command is sent.",
        ));
    }

    connection_manager.disconnect(&id).await?;

    if let Some(reference) = database
        .get_server(&id)?
        .and_then(|server| server.credential_ref)
    {
        credentials.delete_secret(&reference)?;
        let _ = credential_index.remove(&reference);
    }

    database.delete_server(&id)?;
    Ok(())
}
