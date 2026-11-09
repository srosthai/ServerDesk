use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::{
    core::{
        AppError,
        ssh::{ConnectionManager, ConnectionSnapshot, validate_host_key_metadata},
    },
    storage::{Database, KnownHost, StorageError},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostProfile {
    pub id: String,
    pub server_id: String,
    pub key_type: String,
    pub fingerprint: String,
    pub first_seen_at: i64,
    pub verified_at: Option<i64>,
}

impl From<KnownHost> for KnownHostProfile {
    fn from(host: KnownHost) -> Self {
        Self {
            id: host.id,
            server_id: host.server_id,
            key_type: host.key_type,
            fingerprint: host.fingerprint,
            first_seen_at: host.first_seen_at,
            verified_at: host.verified_at,
        }
    }
}

#[tauri::command]
pub fn list_known_hosts(
    database: State<'_, Database>,
    id: String,
) -> Result<Vec<KnownHostProfile>, AppError> {
    ensure_server_exists(&database, &id)?;

    Ok(database
        .list_known_hosts(&id)?
        .into_iter()
        .map(KnownHostProfile::from)
        .collect())
}

#[tauri::command]
pub fn trust_host_key(
    manager: State<'_, Arc<ConnectionManager>>,
    database: State<'_, Database>,
    id: String,
    key_type: String,
    fingerprint: String,
) -> Result<KnownHostProfile, AppError> {
    validate_host_key_metadata(&key_type, &fingerprint)?;
    ensure_server_exists(&database, &id)?;

    let pending = manager
        .pending_host_key(&id)?
        .ok_or_else(|| AppError::host_key("There is no host key waiting for verification."))?;
    if pending.key_type != key_type || pending.fingerprint != fingerprint {
        return Err(AppError::host_key(
            "The host key changed while it was being verified.",
        )
        .with_details(
            "Reconnect and review the fingerprint from the new connection attempt before trusting it.",
        ));
    }

    let host = match database.insert_known_host(&id, &key_type, &fingerprint) {
        Ok(host) => host,
        Err(StorageError::Conflict(_)) => {
            return Err(AppError::host_key(
                "A different host key is already trusted for this server.",
            )
            .with_details(
                "The saved fingerprint was not changed. Remove the trusted key first if the server key was replaced intentionally.",
            ));
        }
        Err(error) => return Err(error.into()),
    };

    manager.approve_host_key(&id, &key_type, &fingerprint)?;
    Ok(KnownHostProfile::from(host))
}

#[tauri::command]
pub fn reject_host_key(
    manager: State<'_, Arc<ConnectionManager>>,
    id: String,
    key_type: String,
    fingerprint: String,
) -> Result<ConnectionSnapshot, AppError> {
    manager.reject_host_key(&id, &key_type, &fingerprint)
}

#[tauri::command]
pub fn delete_known_host(
    database: State<'_, Database>,
    id: String,
    key_type: String,
    confirmed: bool,
) -> Result<(), AppError> {
    if !confirmed {
        return Err(AppError::validation(
            "Removing a trusted host key requires explicit confirmation.",
        )
        .with_details(
            "The saved fingerprint for this server and key type is removed. The next connection will ask for verification again.",
        ));
    }
    if !database.delete_known_host(&id, &key_type)? {
        return Err(AppError::not_found(
            "That trusted host key was not found. It may already have been removed.",
        ));
    }

    Ok(())
}

fn ensure_server_exists(database: &Database, server_id: &str) -> Result<(), AppError> {
    if database.get_server(server_id)?.is_none() {
        return Err(AppError::not_found("Server profile not found."));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_known_hosts_for_the_frontend() {
        let profile = KnownHostProfile::from(KnownHost {
            id: "host-1".to_string(),
            server_id: "server-1".to_string(),
            key_type: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:trusted".to_string(),
            first_seen_at: 1,
            verified_at: Some(2),
        });
        let value = serde_json::to_value(profile).expect("serialized known host");

        assert_eq!(value["serverId"], "server-1");
        assert_eq!(value["keyType"], "ssh-ed25519");
        assert_eq!(value["fingerprint"], "SHA256:trusted");
        assert_eq!(value["verifiedAt"], 2);
    }
}
