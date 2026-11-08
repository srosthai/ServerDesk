use std::sync::Arc;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::{
    core::{
        AppError,
        credentials::{CredentialStore, decode_private_key_credential},
        servers::validate_connection_fields,
        ssh::{
            Authentication, CONNECTION_STATE_EVENT, ConnectionEventSink, ConnectionManager,
            ConnectionRequest, ConnectionSnapshot, ConnectionTestOutcome, ConnectionTimeouts,
            RusshTransport, TrustedHostKey, test_connection, validate_host_key_metadata,
            validate_private_key,
        },
    },
    storage::{AuthType, Database, Server, StorageError},
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestInput {
    pub server_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub credential_ref: Option<String>,
    pub secret: Option<String>,
    pub passphrase: Option<String>,
    pub accepted_host_key: Option<TrustedHostKey>,
}

#[tauri::command]
pub async fn test_server_connection(
    database: State<'_, Database>,
    credentials: State<'_, Arc<dyn CredentialStore>>,
    input: ConnectionTestInput,
) -> Result<ConnectionTestOutcome, AppError> {
    let host = input.host.trim().to_string();
    let username = input.username.trim().to_string();
    validate_connection_fields(&host, input.port, &username)?;

    let saved_server = saved_server(&database, input.server_id.as_deref())?;
    let stored_host_keys = trusted_host_keys(&database, saved_server.as_ref())?;
    let trusted_host_keys =
        resolve_test_host_keys(&stored_host_keys, input.accepted_host_key.as_ref())?;
    let authentication = authentication_for_test(&input, credentials.inner().as_ref())?;
    // Only persist an accepted key when the test used the saved connection
    // details. A test against edited fields must not change trust for the
    // profile that is still on disk if the user cancels the dialog.
    let persist_accepted_host_key = saved_server.as_ref().is_some_and(|server| {
        server.host == host && server.port == input.port && server.username == username
    }) && stored_host_keys.is_empty()
        && input.accepted_host_key.is_some();

    let request = ConnectionRequest {
        server_id: input
            .server_id
            .clone()
            .unwrap_or_else(|| "connection-test".to_string()),
        host,
        port: input.port,
        username,
        authentication,
        trusted_host_keys,
    };

    let transport = RusshTransport::new();
    let timeouts = ConnectionTimeouts::default();
    let outcome = test_connection(&transport, &request, &timeouts).await?;

    if persist_accepted_host_key
        && let (Some(server_id), Some(host_key)) = (&input.server_id, &input.accepted_host_key)
    {
        match database.insert_known_host(server_id, &host_key.key_type, &host_key.fingerprint) {
            Ok(_) => {}
            Err(StorageError::Conflict(_)) => {
                return Err(AppError::host_key(
                    "A different host key is already trusted for this server.",
                )
                .with_details(
                    "The saved fingerprint was not changed. Remove the trusted key first if the server key was replaced intentionally.",
                ));
            }
            Err(error) => return Err(error.into()),
        }
    }

    Ok(outcome)
}

fn saved_server(database: &Database, server_id: Option<&str>) -> Result<Option<Server>, AppError> {
    let Some(server_id) = server_id else {
        return Ok(None);
    };

    database
        .get_server(server_id)?
        .map(Some)
        .ok_or_else(|| AppError::not_found("Server profile not found."))
}

fn trusted_host_keys(
    database: &Database,
    server: Option<&Server>,
) -> Result<Vec<TrustedHostKey>, AppError> {
    let Some(server) = server else {
        return Ok(Vec::new());
    };

    Ok(database
        .list_known_hosts(&server.id)?
        .into_iter()
        .map(|host| TrustedHostKey {
            key_type: host.key_type,
            fingerprint: host.fingerprint,
        })
        .collect())
}

fn resolve_test_host_keys(
    stored: &[TrustedHostKey],
    accepted: Option<&TrustedHostKey>,
) -> Result<Vec<TrustedHostKey>, AppError> {
    let Some(accepted) = accepted else {
        return Ok(stored.to_vec());
    };
    validate_host_key_metadata(&accepted.key_type, &accepted.fingerprint)?;

    if stored.is_empty() {
        return Ok(vec![accepted.clone()]);
    }
    if stored.iter().any(|host_key| {
        host_key.key_type == accepted.key_type && host_key.fingerprint == accepted.fingerprint
    }) {
        return Ok(stored.to_vec());
    }

    Err(
        AppError::host_key("The accepted host key does not match the trusted key for this server.")
            .with_details(
                "The connection test did not change the saved fingerprint. Review the server host key before trusting it.",
            ),
    )
}

fn authentication_for_test(
    input: &ConnectionTestInput,
    credentials: &dyn CredentialStore,
) -> Result<Authentication, AppError> {
    let secret = input.secret.as_deref().filter(|secret| !secret.is_empty());

    match input.auth_type {
        AuthType::Password => match secret {
            Some(secret) => Ok(Authentication::Password(secret.to_string())),
            None => {
                let reference = input.credential_ref.as_deref().ok_or_else(|| {
                    AppError::credential("Enter the SSH password before testing the connection.")
                })?;
                Ok(Authentication::Password(credentials.get_secret(reference)?))
            }
        },
        AuthType::PrivateKey => match secret {
            Some(key) => {
                let passphrase = input
                    .passphrase
                    .as_deref()
                    .filter(|passphrase| !passphrase.is_empty())
                    .map(str::to_string);
                validate_private_key(key, passphrase.as_deref())
                    .map_err(|error| error.to_app_error())?;
                Ok(Authentication::PrivateKey {
                    key: key.to_string(),
                    passphrase,
                })
            }
            None => {
                if input
                    .passphrase
                    .as_deref()
                    .is_some_and(|passphrase| !passphrase.is_empty())
                {
                    return Err(AppError::validation(
                        "Enter the private key again to change its passphrase.",
                    ));
                }
                let reference = input.credential_ref.as_deref().ok_or_else(|| {
                    AppError::credential("Enter a private key before testing the connection.")
                })?;
                let credential =
                    decode_private_key_credential(&credentials.get_secret(reference)?)?;
                Ok(Authentication::PrivateKey {
                    key: credential.key,
                    passphrase: credential.passphrase,
                })
            }
        },
    }
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
    use crate::core::credentials::MemoryCredentialStore;

    use super::*;

    fn test_input(auth_type: AuthType) -> ConnectionTestInput {
        ConnectionTestInput {
            server_id: None,
            host: "198.51.100.24".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth_type,
            credential_ref: None,
            secret: None,
            passphrase: None,
            accepted_host_key: None,
        }
    }

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

    #[test]
    fn test_authentication_uses_typed_secrets_without_persisting_them() {
        let credentials = MemoryCredentialStore::default();
        let mut input = test_input(AuthType::Password);
        input.secret = Some("one-off-password".to_string());

        let authentication = authentication_for_test(&input, &credentials).expect("authentication");
        match authentication {
            Authentication::Password(secret) => assert_eq!(secret, "one-off-password"),
            _ => panic!("expected password authentication"),
        }

        input.auth_type = AuthType::PrivateKey;
        input.secret = Some(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n-----END OPENSSH PRIVATE KEY-----"
                .to_string(),
        );
        let error = authentication_for_test(&input, &credentials).expect_err("invalid key");
        assert_eq!(error.code, crate::core::ErrorCode::Credential);
        assert!(!format!("{error:?}").contains("one-off-password"));
    }

    #[test]
    fn test_authentication_falls_back_to_the_saved_credential_reference() {
        let credentials = MemoryCredentialStore::default();
        credentials
            .set_secret("serverdesk:ssh:existing", "saved-password")
            .expect("saved secret");
        let mut input = test_input(AuthType::Password);
        input.credential_ref = Some("serverdesk:ssh:existing".to_string());

        let authentication = authentication_for_test(&input, &credentials).expect("authentication");
        match authentication {
            Authentication::Password(secret) => assert_eq!(secret, "saved-password"),
            _ => panic!("expected password authentication"),
        }
    }

    #[test]
    fn test_authentication_requires_a_secret_when_none_is_saved() {
        let credentials = MemoryCredentialStore::default();
        let input = test_input(AuthType::Password);

        let error = authentication_for_test(&input, &credentials).expect_err("missing secret");
        assert_eq!(error.code, crate::core::ErrorCode::Credential);
        assert!(error.message.contains("password"));
        assert!(!format!("{error:?}").contains("serverdesk:ssh"));
    }

    #[test]
    fn accepted_host_keys_are_only_used_for_an_empty_trust_store() {
        let accepted = TrustedHostKey {
            key_type: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
        };

        let keys = resolve_test_host_keys(&[], Some(&accepted)).expect("first key");
        assert_eq!(keys, vec![accepted.clone()]);

        let error = resolve_test_host_keys(
            &[TrustedHostKey {
                key_type: "ssh-ed25519".to_string(),
                fingerprint: "SHA256:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".to_string(),
            }],
            Some(&accepted),
        )
        .expect_err("accepted key must not replace a saved key");
        assert_eq!(error.code, crate::core::ErrorCode::HostKey);
    }
}
