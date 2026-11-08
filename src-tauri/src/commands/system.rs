use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{core::AppError, storage::Database};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub app_version: String,
    pub database_schema_version: i32,
}

#[tauri::command]
pub fn get_app_info(app: AppHandle, database: State<'_, Database>) -> Result<AppInfo, AppError> {
    let database_schema_version = database.schema_version()?;

    Ok(AppInfo {
        app_version: app.package_info().version.to_string(),
        database_schema_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_serializes_as_camel_case() {
        let value = serde_json::to_value(AppInfo {
            app_version: "0.1.0".to_string(),
            database_schema_version: 1,
        })
        .expect("serialized app info");

        assert_eq!(value["appVersion"], "0.1.0");
        assert_eq!(value["databaseSchemaVersion"], 1);
    }
}
