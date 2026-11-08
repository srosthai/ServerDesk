use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::{
    core::{
        AppError,
        credentials::{CredentialStore, decode_private_key_credential},
        ssh::{
            Authentication, CONNECTION_STATE_EVENT, ConnectionEventSink, ConnectionManager,
            ConnectionRequest, ConnectionSnapshot, TrustedHostKey,
        },
    },
    storage::{AuthType, Database, Server},
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
        AuthType::Password => Authentication::Password(secret),
        AuthType::PrivateKey => {
            let credential = decode_private_key_credential(&secret)?;
            Authentication::PrivateKey {
                key: credential.key,
                passphrase: credential.passphrase,
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_request_keeps_custom_port_and_username() {
        let server = Server {
            id: "server-1".to_string(),
            name: "Fixture".to_string(),
            host: "127.0.0.1".to_string(),
            port: 2222,
            username: "deploy".to_string(),
            auth_type: AuthType::PrivateKey,
            credential_ref: Some("serverdesk:ssh:test".to_string()),
            group_name: None,
            tags: Vec::new(),
            notes: None,
            created_at: 1,
            updated_at: 1,
        };
        let request = connection_request(
            server,
            Authentication::PrivateKey {
                key: "key-material".to_string(),
                passphrase: Some("secret-passphrase".to_string()),
            },
            Vec::new(),
        );

        assert_eq!(request.port, 2222);
        assert_eq!(request.username, "deploy");
        let debug = format!("{request:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-passphrase"));
        assert!(!debug.contains("key-material"));
    }
}
