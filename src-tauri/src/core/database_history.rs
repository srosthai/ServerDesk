//! Sanitized database query history and reusable SQL snippets.
//!
//! History stores only bounded, already-sanitized SQL and a short error
//! summary. Result grids, parameters, credentials, connection passwords, and
//! environment files are never persisted. Snippets are sanitized the same way
//! before they reach storage, so a password literal cannot be reused from a
//! saved query later.
//!
//! The storage layer prunes history beyond [`MAX_DATABASE_QUERY_HISTORY_RECORDS`]
//! on every insert. The bounds here keep one entry or snippet small even when a
//! caller bypasses the UI and sends a very large SQL body.

use crate::{
    core::{AppError, redaction::sanitize_sql_bounded},
    storage::{
        Database, DatabaseConnection, DatabaseQueryHistory, DatabaseQueryHistoryFilter,
        DatabaseQueryResult, DatabaseSavedQuery, NewDatabaseQueryHistory, NewDatabaseSavedQuery,
        StorageError,
    },
};

pub use crate::storage::MAX_DATABASE_QUERY_HISTORY_RECORDS;

/// Maximum characters of SQL kept for one history entry.
pub const MAX_DATABASE_HISTORY_SQL_CHARS: usize = 8_192;
/// Maximum characters kept for one history error summary.
pub const MAX_DATABASE_HISTORY_ERROR_CHARS: usize = 512;
/// Maximum characters allowed in a saved-query name.
pub const MAX_DATABASE_SNIPPET_NAME_CHARS: usize = 120;
/// Maximum characters allowed in one saved-query SQL body.
pub const MAX_DATABASE_SNIPPET_SQL_CHARS: usize = 65_536;

/// One query outcome to record after an execution attempt.
pub struct DatabaseQueryHistoryDraft<'a> {
    pub profile: &'a DatabaseConnection,
    pub sql: &'a str,
    pub result: DatabaseQueryResult,
    pub duration_ms: i64,
    pub row_count: Option<i64>,
    pub error_summary: Option<&'a str>,
    pub started_at: i64,
}

/// Sanitizes and bounds SQL before it is stored in history.
///
/// Generic redaction covers assignments, bearer tokens, URL credentials,
/// private keys, dotenv lines, and common token shapes. SQL-specific redaction
/// additionally covers `IDENTIFIED BY`, `PASSWORD`, and `PASSWORD =` literals.
pub fn sanitize_query_history_sql(sql: &str) -> String {
    let sanitized = sanitize_sql_bounded(sql.trim(), MAX_DATABASE_HISTORY_SQL_CHARS);
    if sanitized.trim().is_empty() {
        "[redacted SQL]".to_string()
    } else {
        sanitized
    }
}

/// Sanitizes a short error summary for history storage.
pub fn sanitize_query_error_summary(summary: Option<&str>) -> Option<String> {
    summary
        .map(|summary| sanitize_sql_bounded(summary, MAX_DATABASE_HISTORY_ERROR_CHARS))
        .filter(|summary| !summary.trim().is_empty())
}

/// Records one sanitized query-history row.
pub fn record_query_history(
    database: &Database,
    draft: &DatabaseQueryHistoryDraft<'_>,
) -> Result<DatabaseQueryHistory, AppError> {
    if draft.sql.trim().is_empty() {
        return Err(AppError::validation("Cannot record history for empty SQL."));
    }

    let input = NewDatabaseQueryHistory {
        server_id: draft.profile.server_id.clone(),
        connection_id: draft.profile.id.clone(),
        engine: draft.profile.engine,
        database_name: draft.profile.database_name.clone(),
        sql_text: sanitize_query_history_sql(draft.sql),
        result: draft.result,
        duration_ms: draft.duration_ms.max(0),
        row_count: draft.row_count.map(|count| count.max(0)),
        error_summary: sanitize_query_error_summary(draft.error_summary),
        started_at: draft.started_at,
    };

    database
        .record_database_query(&input)
        .map_err(AppError::from)
}

pub fn list_query_history(
    database: &Database,
    filter: &DatabaseQueryHistoryFilter,
) -> Result<Vec<DatabaseQueryHistory>, AppError> {
    database
        .list_database_query_history(filter)
        .map_err(AppError::from)
}

/// Clears local query history only after the caller confirms the action.
pub fn clear_query_history(database: &Database, confirmed: bool) -> Result<(), AppError> {
    if !confirmed {
        return Err(AppError::validation(
            "Clearing query history requires explicit confirmation.",
        ));
    }

    database
        .clear_database_query_history()
        .map_err(AppError::from)
}

/// Sanitizes and validates a saved-query name.
pub fn sanitize_saved_query_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation("Enter a name for this snippet."));
    }
    if trimmed.chars().count() > MAX_DATABASE_SNIPPET_NAME_CHARS {
        return Err(AppError::validation(format!(
            "A snippet name cannot exceed {MAX_DATABASE_SNIPPET_NAME_CHARS} characters."
        )));
    }

    let sanitized = sanitize_sql_bounded(trimmed, MAX_DATABASE_SNIPPET_NAME_CHARS);
    if sanitized.trim().is_empty() {
        return Err(AppError::validation(
            "The snippet name contains only redacted secret material.",
        ));
    }
    Ok(sanitized)
}

/// Sanitizes and validates a saved-query SQL body.
pub fn sanitize_saved_query_sql(sql: &str) -> Result<String, AppError> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation("Enter SQL before saving a snippet."));
    }
    if trimmed.chars().count() > MAX_DATABASE_SNIPPET_SQL_CHARS {
        return Err(AppError::validation(format!(
            "A saved snippet cannot exceed {MAX_DATABASE_SNIPPET_SQL_CHARS} characters."
        )));
    }

    let sanitized = sanitize_sql_bounded(trimmed, MAX_DATABASE_SNIPPET_SQL_CHARS);
    if sanitized.trim().is_empty() {
        return Err(AppError::validation(
            "The snippet contains only redacted secret material.",
        ));
    }
    Ok(sanitized)
}

pub fn create_saved_query(
    database: &Database,
    draft: &NewDatabaseSavedQuery,
) -> Result<DatabaseSavedQuery, AppError> {
    let input = NewDatabaseSavedQuery {
        name: sanitize_saved_query_name(&draft.name)?,
        engine: draft.engine,
        sql_text: sanitize_saved_query_sql(&draft.sql_text)?,
    };

    database
        .create_database_saved_query(&input)
        .map_err(AppError::from)
}

pub fn update_saved_query(
    database: &Database,
    id: &str,
    draft: &NewDatabaseSavedQuery,
) -> Result<DatabaseSavedQuery, AppError> {
    let input = NewDatabaseSavedQuery {
        name: sanitize_saved_query_name(&draft.name)?,
        engine: draft.engine,
        sql_text: sanitize_saved_query_sql(&draft.sql_text)?,
    };

    database
        .update_database_saved_query(id, &input)
        .map_err(|error| match error {
            StorageError::NotFound => AppError::not_found("Saved query not found."),
            other => AppError::from(other),
        })
}

/// Deletes one snippet only after the caller confirms the action.
pub fn delete_saved_query(database: &Database, id: &str, confirmed: bool) -> Result<(), AppError> {
    if !confirmed {
        return Err(AppError::validation(
            "Deleting a saved query requires explicit confirmation.",
        ));
    }

    database
        .delete_database_saved_query(id)
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_sql_is_redacted_and_bounded() {
        let sanitized = sanitize_query_history_sql(
            "CREATE USER 'app'@'%' IDENTIFIED BY 'fixture-password';\n\
             -- Authorization: Bearer fixture-token\n\
             SELECT 1;",
        );

        assert!(!sanitized.contains("fixture-password"));
        assert!(!sanitized.contains("fixture-token"));
        assert!(sanitized.contains("IDENTIFIED BY '<redacted>'"));
        assert!(sanitized.contains("Bearer <redacted>"));
    }

    #[test]
    fn oversized_history_sql_is_summarized() {
        let sql = format!(
            "SELECT '{}'",
            "x".repeat(MAX_DATABASE_HISTORY_SQL_CHARS * 2)
        );
        let sanitized = sanitize_query_history_sql(&sql);

        assert!(sanitized.chars().count() <= MAX_DATABASE_HISTORY_SQL_CHARS);
        assert!(sanitized.ends_with('…'));
    }

    #[test]
    fn snippet_validation_rejects_empty_and_oversized_bodies() {
        assert!(sanitize_saved_query_name("  ").is_err());
        assert!(sanitize_saved_query_sql("  ").is_err());
        assert!(sanitize_saved_query_sql(&"x".repeat(MAX_DATABASE_SNIPPET_SQL_CHARS + 1)).is_err());
        assert!(
            sanitize_saved_query_name(&"x".repeat(MAX_DATABASE_SNIPPET_NAME_CHARS + 1)).is_err()
        );
    }
}
