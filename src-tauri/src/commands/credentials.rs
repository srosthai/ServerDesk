use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    credentials::{
        CredentialStore, encode_private_key_credential, new_reference, validate_reference,
    },
    ssh::validate_private_key,
};

#[tauri::command]
pub fn create_credential(
    credentials: State<'_, Arc<dyn CredentialStore>>,
    secret: String,
) -> Result<String, AppError> {
    let reference = new_reference();
    credentials.set_secret(&reference, &secret)?;
    Ok(reference)
}

#[tauri::command]
pub fn store_credential(
    credentials: State<'_, Arc<dyn CredentialStore>>,
    reference: String,
    secret: String,
) -> Result<(), AppError> {
    validate_reference(&reference)?;
    credentials.set_secret(&reference, &secret)
}

#[tauri::command]
pub fn delete_credential(
    credentials: State<'_, Arc<dyn CredentialStore>>,
    reference: String,
) -> Result<(), AppError> {
    validate_reference(&reference)?;
    credentials.delete_secret(&reference)
}

#[tauri::command]
pub fn create_private_key_credential(
    credentials: State<'_, Arc<dyn CredentialStore>>,
    key: String,
    passphrase: Option<String>,
) -> Result<String, AppError> {
    let secret = private_key_secret(&key, passphrase.as_deref())?;
    let reference = new_reference();
    credentials.set_secret(&reference, &secret)?;
    Ok(reference)
}

#[tauri::command]
pub fn store_private_key_credential(
    credentials: State<'_, Arc<dyn CredentialStore>>,
    reference: String,
    key: String,
    passphrase: Option<String>,
) -> Result<(), AppError> {
    validate_reference(&reference)?;
    let secret = private_key_secret(&key, passphrase.as_deref())?;
    credentials.set_secret(&reference, &secret)
}

fn private_key_secret(key: &str, passphrase: Option<&str>) -> Result<String, AppError> {
    validate_private_key(key, passphrase).map_err(|error| error.to_app_error())?;
    encode_private_key_credential(key, passphrase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::credentials::decode_private_key_credential;

    const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n-----END OPENSSH PRIVATE KEY-----";

    #[test]
    fn private_key_secrets_round_trip_through_the_envelope() {
        let secret = encode_private_key_credential(KEY, Some("passphrase")).expect("secret");
        let decoded = decode_private_key_credential(&secret).expect("credential");

        assert_eq!(decoded.key, KEY);
        assert_eq!(decoded.passphrase.as_deref(), Some("passphrase"));
    }

    #[test]
    fn private_key_secret_rejects_invalid_material() {
        assert!(private_key_secret("", None).is_err());
        assert!(private_key_secret("not a key", None).is_err());
    }
}
