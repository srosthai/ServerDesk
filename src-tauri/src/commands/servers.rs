use tauri::State;

use crate::{
    core::{AppError, servers::ServerProfile, servers::ServerProfileInput},
    storage::Database,
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
pub fn update_server(
    database: State<'_, Database>,
    id: String,
    input: ServerProfileInput,
) -> Result<ServerProfile, AppError> {
    let new_server = input.into_new_server()?;

    if database.server_name_exists(&new_server.name, Some(&id))? {
        return Err(AppError::validation(
            "A server with this name already exists.",
        ));
    }

    Ok(ServerProfile::from(
        database.update_server(&id, &new_server)?,
    ))
}

#[tauri::command]
pub fn delete_server(database: State<'_, Database>, id: String) -> Result<(), AppError> {
    database.delete_server(&id)?;
    Ok(())
}
