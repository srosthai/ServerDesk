use tauri::State;

use crate::{core::AppError, core::preferences::UserPreferences, storage::Database};

#[tauri::command]
pub fn get_preferences(database: State<'_, Database>) -> Result<UserPreferences, AppError> {
    UserPreferences::load(&database)
}

#[tauri::command]
pub fn save_preferences(
    database: State<'_, Database>,
    preferences: UserPreferences,
) -> Result<UserPreferences, AppError> {
    preferences.save(&database)
}
