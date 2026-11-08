use std::sync::Arc;

use tauri::State;

use crate::core::{
    AppError,
    credentials::{CredentialStore, new_reference, validate_reference},
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
