//! Saved database profile commands.
//!
//! React sees only the camelCase profile and connection documents. Passwords
//! are read from the credential store inside the command, passed to the
//! adapter, and dropped. The opaque credential reference never leaves Rust.

use std::sync::Arc;

use tauri::State;

use crate::{
    core::{
        AppError,
        credential_index::CredentialIndex,
        credentials::CredentialStore,
        database_credentials::DatabaseCredentialService,
        database_profiles::{
            DatabaseConnectionSnapshot, DatabaseConnectionState, DatabaseProfile,
            DatabaseProfileInput, DatabaseProfileService, DatabaseTestResult,
        },
        ssh::TunnelManager,
    },
    database::{DatabaseConnectionRegistry, DatabasePassword},
    storage::{Database, DatabaseConnection},
};

#[tauri::command]
pub fn list_database_profiles(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    server_id: String,
) -> Result<Vec<DatabaseProfile>, AppError> {
    DatabaseProfileService::new(
        database.inner(),
        credentials.inner().as_ref(),
        credential_index.inner(),
    )
    .list(&server_id)
}

#[tauri::command]
pub fn get_database_profile(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    id: String,
) -> Result<Option<DatabaseProfile>, AppError> {
    DatabaseProfileService::new(
        database.inner(),
        credentials.inner().as_ref(),
        credential_index.inner(),
    )
    .get(&id)
}

#[tauri::command]
pub fn create_database_profile(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    input: DatabaseProfileInput,
) -> Result<DatabaseProfile, AppError> {
    DatabaseProfileService::new(
        database.inner(),
        credentials.inner().as_ref(),
        credential_index.inner(),
    )
    .create(input)
}

#[tauri::command]
pub async fn update_database_profile(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    tunnels: State<'_, Arc<TunnelManager>>,
    id: String,
    input: DatabaseProfileInput,
) -> Result<DatabaseProfile, AppError> {
    let existing = load_profile(database.inner(), &id)?;
    let profile = DatabaseProfileService::new(
        database.inner(),
        credentials.inner().as_ref(),
        credential_index.inner(),
    )
    .update(&id, input)?;

    let updated = load_profile(database.inner(), &id)?;
    if connection_details_changed(&existing, &updated) {
        connections.disconnect(&id).await?;
        tunnels.close(&id).await?;
    }

    Ok(profile)
}

fn connection_details_changed(existing: &DatabaseConnection, updated: &DatabaseConnection) -> bool {
    existing.server_id != updated.server_id
        || existing.engine != updated.engine
        || existing.host != updated.host
        || existing.port != updated.port
        || existing.socket != updated.socket
        || existing.database_name != updated.database_name
        || existing.username != updated.username
        || existing.read_only != updated.read_only
        || existing.ssl_mode != updated.ssl_mode
}

/// Removes the live session and tunnel before deleting the saved credential
/// and profile metadata.
#[tauri::command]
pub async fn delete_database_profile(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    tunnels: State<'_, Arc<TunnelManager>>,
    id: String,
) -> Result<(), AppError> {
    let profile = load_profile(database.inner(), &id)?;

    // Both calls are idempotent. Closing the registry session also closes its
    // tunnel, and the explicit close covers a tunnel that exists without a
    // live adapter.
    connections.disconnect(&profile.id).await?;
    tunnels.close(&profile.id).await?;

    DatabaseProfileService::new(
        database.inner(),
        credentials.inner().as_ref(),
        credential_index.inner(),
    )
    .delete(&id)
}

#[tauri::command]
pub async fn test_database_profile(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    id: String,
) -> Result<DatabaseTestResult, AppError> {
    let profile = load_profile(database.inner(), &id)?;
    let password = saved_password(
        credentials.inner().as_ref(),
        credential_index.inner(),
        &profile,
    )?;

    connections.test(&profile, password).await
}

/// Tests the values currently in the create/edit dialog without saving them.
///
/// The password comes from the request when it is being replaced, or from the
/// saved credential when an existing profile is tested with an empty password
/// field. The transient tunnel and adapter are closed before returning, so a
/// failed test leaves the saved profile untouched.
#[tauri::command]
pub async fn test_database_profile_draft(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    input: DatabaseProfileInput,
    profile_id: Option<String>,
) -> Result<DatabaseTestResult, AppError> {
    let validated = input.validate()?;
    if database.get_server(&validated.server_id)?.is_none() {
        return Err(AppError::not_found("Server profile not found."));
    }

    let password = match input.password.as_deref() {
        Some(password) if !password.is_empty() => DatabasePassword::new(password.to_string()),
        _ => match profile_id.as_deref() {
            Some(id) => {
                let profile = load_profile(database.inner(), id)?;
                saved_password(
                    credentials.inner().as_ref(),
                    credential_index.inner(),
                    &profile,
                )?
            }
            None => {
                return Err(AppError::credential_missing(
                    "Enter the database password before testing this profile.",
                ));
            }
        },
    };

    let id = profile_id.unwrap_or_else(|| "draft".to_string());
    let profile = DatabaseConnection {
        id,
        server_id: validated.server_id,
        name: validated.name,
        engine: validated.engine,
        host: validated.host,
        port: validated.port,
        socket: validated.socket,
        database_name: validated.database_name,
        username: validated.username,
        credential_ref: None,
        read_only: validated.read_only,
        ssl_mode: validated.ssl_mode.map(|mode| mode.as_str().to_string()),
        created_at: 0,
        updated_at: 0,
        last_used_at: None,
    };

    connections.test(&profile, password).await
}

#[tauri::command]
pub async fn connect_database_profile(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    tunnels: State<'_, Arc<TunnelManager>>,
    id: String,
) -> Result<DatabaseConnectionSnapshot, AppError> {
    connect_profile(
        database.inner(),
        credentials.inner().as_ref(),
        credential_index.inner(),
        connections.inner(),
        tunnels.inner().as_ref(),
        &id,
    )
    .await
}

#[tauri::command]
pub async fn reconnect_database_profile(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    credential_index: State<'_, CredentialIndex>,
    connections: State<'_, DatabaseConnectionRegistry>,
    tunnels: State<'_, Arc<TunnelManager>>,
    id: String,
) -> Result<DatabaseConnectionSnapshot, AppError> {
    // The registry closes the previous adapter and tunnel before every
    // connect, so reconnect uses the same deterministic path.
    connect_profile(
        database.inner(),
        credentials.inner().as_ref(),
        credential_index.inner(),
        connections.inner(),
        tunnels.inner().as_ref(),
        &id,
    )
    .await
}

#[tauri::command]
pub async fn disconnect_database_profile(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    tunnels: State<'_, Arc<TunnelManager>>,
    id: String,
) -> Result<DatabaseConnectionSnapshot, AppError> {
    let profile = load_profile(database.inner(), &id)?;

    connections.disconnect(&profile.id).await?;
    tunnels.close(&profile.id).await?;

    Ok(DatabaseConnectionSnapshot::disconnected(&profile))
}

#[tauri::command]
pub async fn get_database_connection_state(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    tunnels: State<'_, Arc<TunnelManager>>,
    id: String,
) -> Result<DatabaseConnectionSnapshot, AppError> {
    let profile = load_profile(database.inner(), &id)?;
    let tunnel = tunnels.snapshot(&profile.id)?;

    if connections.is_connected(&profile.id).await {
        return match connections.info(&profile.id).await {
            Ok(info) => Ok(DatabaseConnectionSnapshot::connected(
                &profile, info, tunnel,
            )),
            Err(error) => Ok(DatabaseConnectionSnapshot::failed(
                &profile,
                DatabaseConnectionState::Error,
                error,
            )
            .with_tunnel(tunnel)),
        };
    }

    Ok(DatabaseConnectionSnapshot::disconnected(&profile).with_tunnel(tunnel))
}

async fn connect_profile(
    database: &Database,
    credentials: &dyn CredentialStore,
    credential_index: &CredentialIndex,
    connections: &DatabaseConnectionRegistry,
    tunnels: &TunnelManager,
    id: &str,
) -> Result<DatabaseConnectionSnapshot, AppError> {
    let profile = load_profile(database, id)?;
    let password = saved_password(credentials, credential_index, &profile)?;
    let info = connections.connect(&profile, password).await?;

    // Last-used time is informational. A storage failure must not turn a live
    // database session into a command failure.
    let profile = database
        .mark_database_connection_used(&profile.id)
        .unwrap_or(profile);
    let tunnel = tunnels.snapshot(&profile.id)?;

    Ok(DatabaseConnectionSnapshot::connected(
        &profile, info, tunnel,
    ))
}

fn load_profile(database: &Database, id: &str) -> Result<DatabaseConnection, AppError> {
    database
        .get_database_connection(id)?
        .ok_or_else(|| AppError::not_found("Database profile not found."))
}

fn saved_password(
    credentials: &dyn CredentialStore,
    credential_index: &CredentialIndex,
    profile: &DatabaseConnection,
) -> Result<DatabasePassword, AppError> {
    let reference = profile.credential_ref.as_deref().ok_or_else(|| {
        AppError::credential_missing(
            "No database password is saved for this profile. Save the password again.",
        )
    })?;
    let secret = DatabaseCredentialService::new(credentials, credential_index).read(reference)?;

    Ok(DatabasePassword::new(secret))
}
