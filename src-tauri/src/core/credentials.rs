use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::AppError;

const KEYRING_SERVICE: &str = "com.serverdesk.app";
const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_PASSPHRASE_BYTES: usize = 4096;
const PRIVATE_KEY_MARKER: &str = "serverdesk-credential:v1:";

#[derive(Clone, PartialEq, Eq)]
pub struct PrivateKeyCredential {
    pub key: String,
    pub passphrase: Option<String>,
}

impl std::fmt::Debug for PrivateKeyCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrivateKeyCredential")
            .field("key", &"<redacted>")
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateKeyCredentialPayload {
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passphrase: Option<String>,
}

pub trait CredentialStore: Send + Sync {
    fn set_secret(&self, reference: &str, secret: &str) -> Result<(), AppError>;
    fn get_secret(&self, reference: &str) -> Result<String, AppError>;
    fn delete_secret(&self, reference: &str) -> Result<(), AppError>;
}

pub struct OsCredentialStore;

impl CredentialStore for OsCredentialStore {
    fn set_secret(&self, reference: &str, secret: &str) -> Result<(), AppError> {
        validate_reference(reference)?;
        validate_secret(secret)?;

        entry(reference)?
            .set_password(secret)
            .map_err(map_keyring_error)
    }

    fn get_secret(&self, reference: &str) -> Result<String, AppError> {
        validate_reference(reference)?;

        entry(reference)?.get_password().map_err(map_keyring_error)
    }

    fn delete_secret(&self, reference: &str) -> Result<(), AppError> {
        validate_reference(reference)?;

        match entry(reference)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(map_keyring_error(error)),
        }
    }
}

pub fn new_reference() -> String {
    format!("serverdesk:ssh:{}", Uuid::new_v4())
}

pub fn encode_private_key_credential(
    key: &str,
    passphrase: Option<&str>,
) -> Result<String, AppError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(AppError::validation("Private key material is required."));
    }

    let passphrase = passphrase
        .filter(|passphrase| !passphrase.is_empty())
        .map(str::to_string);
    if passphrase
        .as_ref()
        .is_some_and(|passphrase| passphrase.len() > MAX_PASSPHRASE_BYTES)
    {
        return Err(AppError::validation(
            "The private key passphrase is too long.",
        ));
    }

    let payload = PrivateKeyCredentialPayload {
        key: key.to_string(),
        passphrase,
    };
    let encoded = serde_json::to_string(&payload)
        .map_err(|_| AppError::internal("Could not encode the private key credential."))?;
    let secret = format!("{PRIVATE_KEY_MARKER}{encoded}");
    validate_secret(&secret)?;

    Ok(secret)
}

pub fn decode_private_key_credential(secret: &str) -> Result<PrivateKeyCredential, AppError> {
    let Some(encoded) = secret.strip_prefix(PRIVATE_KEY_MARKER) else {
        // Credentials saved by earlier releases stored the key material directly.
        return Ok(PrivateKeyCredential {
            key: secret.to_string(),
            passphrase: None,
        });
    };

    let payload: PrivateKeyCredentialPayload = serde_json::from_str(encoded)
        .map_err(|_| AppError::credential("The saved private key credential is invalid."))?;
    if payload.key.trim().is_empty() {
        return Err(AppError::credential(
            "The saved private key credential is invalid.",
        ));
    }
    if payload
        .passphrase
        .as_ref()
        .is_some_and(|passphrase| passphrase.len() > MAX_PASSPHRASE_BYTES)
    {
        return Err(AppError::credential(
            "The saved private key passphrase is invalid.",
        ));
    }

    Ok(PrivateKeyCredential {
        key: payload.key,
        passphrase: payload
            .passphrase
            .filter(|passphrase| !passphrase.is_empty()),
    })
}

pub fn validate_reference(reference: &str) -> Result<(), AppError> {
    if reference.is_empty() || reference.len() > 256 {
        return Err(AppError::validation("Invalid credential reference."));
    }
    if !reference.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, ':' | '-' | '_' | '.')
    }) {
        return Err(AppError::validation("Invalid credential reference."));
    }

    Ok(())
}

fn validate_secret(secret: &str) -> Result<(), AppError> {
    if secret.is_empty() {
        return Err(AppError::validation("Secret cannot be empty."));
    }
    if secret.len() > MAX_SECRET_BYTES {
        return Err(AppError::validation("Secret is too long."));
    }

    Ok(())
}

fn entry(reference: &str) -> Result<Entry, AppError> {
    Entry::new(KEYRING_SERVICE, reference).map_err(map_keyring_error)
}

fn map_keyring_error(error: KeyringError) -> AppError {
    match error {
        KeyringError::NoEntry => AppError::not_found("Credential not found."),
        other => AppError::credential("The OS credential store is unavailable.")
            .with_details(other.to_string()),
    }
}

#[derive(Default)]
pub struct MemoryCredentialStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl CredentialStore for MemoryCredentialStore {
    fn set_secret(&self, reference: &str, secret: &str) -> Result<(), AppError> {
        validate_reference(reference)?;
        validate_secret(secret)?;

        self.secrets
            .lock()
            .map_err(|_| AppError::internal("Credential store lock poisoned."))?
            .insert(reference.to_string(), secret.to_string());

        Ok(())
    }

    fn get_secret(&self, reference: &str) -> Result<String, AppError> {
        self.secrets
            .lock()
            .map_err(|_| AppError::internal("Credential store lock poisoned."))?
            .get(reference)
            .cloned()
            .ok_or_else(|| AppError::not_found("Credential not found."))
    }

    fn delete_secret(&self, reference: &str) -> Result<(), AppError> {
        self.secrets
            .lock()
            .map_err(|_| AppError::internal("Credential store lock poisoned."))?
            .remove(reference);

        Ok(())
    }
}

pub type SharedCredentialStore = Arc<dyn CredentialStore>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let store = MemoryCredentialStore::default();
        store
            .set_secret("serverdesk:ssh:test", "hunter2")
            .expect("store secret");

        assert_eq!(
            store
                .get_secret("serverdesk:ssh:test")
                .expect("read secret"),
            "hunter2"
        );

        store
            .delete_secret("serverdesk:ssh:test")
            .expect("delete secret");
        assert_eq!(
            store.get_secret("serverdesk:ssh:test").unwrap_err().code,
            crate::core::ErrorCode::NotFound
        );
    }

    #[test]
    fn rejects_empty_secret_and_bad_reference() {
        let store = MemoryCredentialStore::default();

        assert!(store.set_secret("serverdesk:ssh:test", "").is_err());
        assert!(store.set_secret("", "secret").is_err());
        assert!(store.set_secret("serverdesk ssh test", "secret").is_err());
    }

    #[test]
    #[ignore = "requires an unlocked OS credential store"]
    fn os_credential_store_round_trip() {
        let store = OsCredentialStore;
        let reference = new_reference();

        store
            .set_secret(&reference, "serverdesk-test-secret")
            .expect("store OS secret");
        assert_eq!(
            store.get_secret(&reference).expect("read OS secret"),
            "serverdesk-test-secret"
        );
        store.delete_secret(&reference).expect("delete OS secret");
    }

    #[test]
    fn generated_references_are_unique_and_prefixed() {
        let first = new_reference();
        let second = new_reference();

        assert!(first.starts_with("serverdesk:ssh:"));
        assert_ne!(first, second);
        assert!(validate_reference(&first).is_ok());
    }

    #[test]
    fn round_trips_private_key_credentials() {
        let encoded = encode_private_key_credential(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n-----END OPENSSH PRIVATE KEY-----",
            Some("fixture-passphrase"),
        )
        .expect("encoded credential");
        let decoded = decode_private_key_credential(&encoded).expect("decoded credential");

        assert_eq!(
            decoded.key,
            "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n-----END OPENSSH PRIVATE KEY-----"
        );
        assert_eq!(decoded.passphrase.as_deref(), Some("fixture-passphrase"));
        assert!(!format!("{decoded:?}").contains("fixture-passphrase"));
    }

    #[test]
    fn accepts_legacy_raw_private_keys() {
        let decoded = decode_private_key_credential("legacy-key-material").expect("legacy key");

        assert_eq!(decoded.key, "legacy-key-material");
        assert!(decoded.passphrase.is_none());
    }

    #[test]
    fn treats_empty_passphrases_as_absent() {
        let encoded =
            encode_private_key_credential("key-material", Some("")).expect("encoded credential");
        let decoded = decode_private_key_credential(&encoded).expect("decoded credential");

        assert!(decoded.passphrase.is_none());
    }

    #[test]
    fn rejects_invalid_private_key_credential_payloads() {
        assert!(encode_private_key_credential("   ", None).is_err());
        assert!(decode_private_key_credential("serverdesk-credential:v1:not-json").is_err());
        assert!(
            decode_private_key_credential("serverdesk-credential:v1:{\"key\":\"  \"}").is_err()
        );
    }

    #[test]
    fn caps_large_secret_payloads() {
        let store = MemoryCredentialStore::default();
        let oversized = "x".repeat(MAX_SECRET_BYTES + 1);

        assert!(store.set_secret("serverdesk:ssh:test", &oversized).is_err());
    }
}
