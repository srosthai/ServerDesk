use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use keyring::{Entry, Error as KeyringError};
use uuid::Uuid;

use crate::core::AppError;

const KEYRING_SERVICE: &str = "com.serverdesk.app";

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
    if secret.len() > 4096 {
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
}
