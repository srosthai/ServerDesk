use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::{
    core::{
        AppError,
        credentials::CredentialStore,
        ssh::{
            Authentication, CONNECTION_STATE_EVENT, ConnectionEventSink, ConnectionManager,
            ConnectionRequest, ConnectionSnapshot, TrustedHostKey,
        },
    },
    storage::{Database, Server},
};

pub struct TauriConnectionEventSink {
    app: AppHandle,
}

impl TauriConnectionEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ConnectionEventSink for TauriConnectionEventSink {
    fn emit(&self, snapshot: &ConnectionSnapshot) {
        let _ = self.app.emit(CONNECTION_STATE_EVENT, snapshot.clone());
    }
}

#[tauri::command]
pub async fn connect_server(
    manager: State<'_, Arc<ConnectionManager>>,
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    id: String,
) -> Result<ConnectionSnapshot, AppError> {
    let server = database
        .get_server(&id)?
        .ok_or_else(|| AppError::not_found("Server profile not found."))?;
    let trusted_host_keys = database
        .list_known_hosts(&id)?
        .into_iter()
        .map(|host| TrustedHostKey {
            key_type: host.key_type,
            fingerprint: host.fingerprint,
        })
        .collect();
    let credential_ref = server
        .credential_ref
        .as_deref()
        .ok_or_else(|| AppError::credential("No SSH credential is saved for this server."))?;
    let secret = credentials.get_secret(credential_ref)?;
    let authentication = match server.auth_type {
        crate::storage::AuthType::Password => Authentication::Password(secret),
        crate::storage::AuthType::PrivateKey => Authentication::PrivateKey(secret),
    };

    manager
        .connect(connection_request(
            server,
            authentication,
            trusted_host_keys,
        ))
        .await
}

#[tauri::command]
pub async fn disconnect_server(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
) -> Result<ConnectionSnapshot, AppError> {
    manager.disconnect(&id).await
}

#[tauri::command]
pub fn get_connection_state(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
) -> Result<ConnectionSnapshot, AppError> {
    manager.connection_state(&id)
}

fn connection_request(
    server: Server,
    authentication: Authentication,
    trusted_host_keys: Vec<TrustedHostKey>,
) -> ConnectionRequest {
    ConnectionRequest {
        server_id: server.id,
        host: server.host,
        port: server.port,
        username: server.username,
        authentication,
        trusted_host_keys,
    }
}
