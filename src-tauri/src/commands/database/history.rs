//! Query history and saved-query commands for the SQL workspace.
//!
//! Every SQL body is sanitized in [`crate::core::database_history`] before it
//! reaches storage. Result grids and raw engine output never cross this
//! boundary. Clear-history and delete-snippet calls require an explicit
//! confirmation flag so a UI mistake cannot erase local context silently.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    core::{
        AppError,
        database_history::{
            MAX_DATABASE_HISTORY_ERROR_CHARS, MAX_DATABASE_HISTORY_SQL_CHARS,
            MAX_DATABASE_QUERY_HISTORY_RECORDS, MAX_DATABASE_SNIPPET_NAME_CHARS,
            MAX_DATABASE_SNIPPET_SQL_CHARS, clear_query_history, create_saved_query,
            delete_saved_query, list_query_history, update_saved_query,
        },
    },
    storage::{
        Database, DatabaseEngine, DatabaseQueryHistory, DatabaseQueryHistoryFilter,
        DatabaseSavedQuery, NewDatabaseSavedQuery,
    },
};

/// Local retention and size policy shown in the SQL workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseQueryHistoryPolicy {
    /// Newest rows kept before the oldest are pruned automatically.
    pub max_entries: u32,
    /// Hard character bound for one stored SQL body.
    pub max_sql_chars: u32,
    /// Hard character bound for one stored error summary.
    pub error_summary_max_chars: u32,
    /// Maximum characters in a snippet name.
    pub snippet_name_max_chars: u32,
    /// Maximum characters in a saved snippet SQL body.
    pub snippet_sql_max_chars: u32,
    /// Short explanation suitable for direct display.
    pub retention: String,
}

/// User-supplied snippet fields. Storage adds ids and timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSavedQueryInput {
    pub name: String,
    pub engine: DatabaseEngine,
    pub sql: String,
}

#[tauri::command]
pub fn get_database_query_history_policy() -> DatabaseQueryHistoryPolicy {
    DatabaseQueryHistoryPolicy {
        max_entries: MAX_DATABASE_QUERY_HISTORY_RECORDS,
        max_sql_chars: u32::try_from(MAX_DATABASE_HISTORY_SQL_CHARS).unwrap_or(u32::MAX),
        error_summary_max_chars: u32::try_from(MAX_DATABASE_HISTORY_ERROR_CHARS)
            .unwrap_or(u32::MAX),
        snippet_name_max_chars: u32::try_from(MAX_DATABASE_SNIPPET_NAME_CHARS).unwrap_or(u32::MAX),
        snippet_sql_max_chars: u32::try_from(MAX_DATABASE_SNIPPET_SQL_CHARS).unwrap_or(u32::MAX),
        retention: format!(
            "ServerDesk keeps the newest {MAX_DATABASE_QUERY_HISTORY_RECORDS} queries and \
             automatically removes older entries. Stored SQL is limited to \
             {MAX_DATABASE_HISTORY_SQL_CHARS} characters and is sanitized before it is saved."
        ),
    }
}

#[tauri::command]
pub fn list_database_query_history(
    database: State<'_, Database>,
    filter: Option<DatabaseQueryHistoryFilter>,
) -> Result<Vec<DatabaseQueryHistory>, AppError> {
    list_query_history(database.inner(), &filter.unwrap_or_default())
}

/// Clears local query history after the UI confirms the action.
#[tauri::command]
pub fn clear_database_query_history(
    database: State<'_, Database>,
    confirmed: bool,
) -> Result<(), AppError> {
    clear_query_history(database.inner(), confirmed)
}

#[tauri::command]
pub fn list_database_saved_queries(
    database: State<'_, Database>,
) -> Result<Vec<DatabaseSavedQuery>, AppError> {
    database
        .list_database_saved_queries()
        .map_err(AppError::from)
}

#[tauri::command]
pub fn create_database_saved_query(
    database: State<'_, Database>,
    input: DatabaseSavedQueryInput,
) -> Result<DatabaseSavedQuery, AppError> {
    create_saved_query(database.inner(), &new_saved_query(input))
}

#[tauri::command]
pub fn update_database_saved_query(
    database: State<'_, Database>,
    id: String,
    input: DatabaseSavedQueryInput,
) -> Result<DatabaseSavedQuery, AppError> {
    update_saved_query(database.inner(), &id, &new_saved_query(input))
}

/// Deletes one snippet after the UI confirms the action.
#[tauri::command]
pub fn delete_database_saved_query(
    database: State<'_, Database>,
    id: String,
    confirmed: bool,
) -> Result<(), AppError> {
    delete_saved_query(database.inner(), &id, confirmed)
}

fn new_saved_query(input: DatabaseSavedQueryInput) -> NewDatabaseSavedQuery {
    NewDatabaseSavedQuery {
        name: input.name,
        engine: input.engine,
        sql_text: input.sql,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_serializes_with_camel_case_fields_and_bounds() {
        let policy = get_database_query_history_policy();
        let value = serde_json::to_value(policy).expect("serialized policy");

        assert_eq!(value["maxEntries"], 1_000);
        assert_eq!(value["maxSqlChars"], 8_192);
        assert_eq!(value["errorSummaryMaxChars"], 512);
        assert_eq!(value["snippetNameMaxChars"], 120);
        assert_eq!(value["snippetSqlMaxChars"], 65_536);
        assert!(
            value["retention"]
                .as_str()
                .is_some_and(|text| { text.contains("sanitized") && text.contains("newest 1000") })
        );
    }

    #[test]
    fn saved_query_input_maps_to_storage_fields() {
        let input = DatabaseSavedQueryInput {
            name: "Active users".to_string(),
            engine: DatabaseEngine::Postgresql,
            sql: "SELECT 1".to_string(),
        };
        let mapped = new_saved_query(input);

        assert_eq!(mapped.name, "Active users");
        assert_eq!(mapped.engine, DatabaseEngine::Postgresql);
        assert_eq!(mapped.sql_text, "SELECT 1");
    }
}
