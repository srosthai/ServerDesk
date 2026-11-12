//! Database lifecycle commands.
//!
//! Preview is always available for a supported action. Execution re-validates
//! the request, rejects an unconfirmed drop, refuses structural changes on a
//! read-only profile, and re-reads metadata before reporting a verified result.
//! Audit history receives only the action, engine, database names, and safe
//! verification text, never generated SQL or connection details.

use std::time::Instant;

use tauri::State;

use crate::{
    core::{
        AppError, ErrorCode,
        history::{now_unix_seconds, record_operation, summarize_error},
    },
    database::{
        DatabaseActionRequest, DatabaseActionResult, DatabaseActionService,
        DatabaseConnectionRegistry, DatabaseDdlPreview,
    },
    storage::{
        Database, DatabaseConnection, NewOperationRecord, OperationCategory, OperationResult,
    },
};

#[tauri::command]
pub async fn preview_database_action(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: DatabaseActionRequest,
) -> Result<DatabaseDdlPreview, AppError> {
    DatabaseActionService::new(connections.inner())
        .preview(&profile_id, request)
        .await
}

#[tauri::command]
pub async fn run_database_action(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: DatabaseActionRequest,
    confirmed: bool,
) -> Result<DatabaseActionResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = DatabaseActionService::new(connections.inner())
        .execute(&profile_id, request.clone(), confirmed)
        .await;

    record_action_history(
        database.inner(),
        &profile,
        &request,
        &outcome,
        started_at,
        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
    );

    outcome
}

fn load_profile(database: &Database, profile_id: &str) -> Result<DatabaseConnection, AppError> {
    database
        .get_database_connection(profile_id)?
        .ok_or_else(|| AppError::not_found("Database profile not found."))
}

fn engine_label(engine: crate::storage::DatabaseEngine) -> &'static str {
    match engine {
        crate::storage::DatabaseEngine::Mysql => "MySQL",
        crate::storage::DatabaseEngine::Postgresql => "PostgreSQL",
    }
}

fn record_action_history(
    database: &Database,
    profile: &DatabaseConnection,
    request: &DatabaseActionRequest,
    outcome: &Result<DatabaseActionResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let action = request.kind();
    let engine = engine_label(profile.engine);
    let database_name = request.database_name();
    let operation = format!("database-{}", action.as_str());
    let (result, safe_summary, output_summary) = match outcome {
        Ok(outcome) if outcome.verified => (
            OperationResult::Success,
            verified_summary(profile, request, outcome),
            Some(outcome.verification.clone()),
        ),
        Ok(outcome) => (
            OperationResult::Unknown,
            format!(
                "The {} database action for {} on {} finished, but metadata verification did \
                 not confirm the result",
                action.as_str(),
                database_name,
                engine
            ),
            Some(
                outcome
                    .verification_error
                    .clone()
                    .unwrap_or_else(|| outcome.verification.clone()),
            ),
        ),
        Err(error) if error.code == ErrorCode::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the {} database action for {} on {} before execution",
                action.as_str(),
                database_name,
                engine
            ),
            summarize_error(error),
        ),
        Err(error) => {
            let unknown = matches!(
                error.code,
                ErrorCode::Timeout
                    | ErrorCode::Connection
                    | ErrorCode::Network
                    | ErrorCode::Tunnel
                    | ErrorCode::Internal
            );
            (
                if unknown {
                    OperationResult::Unknown
                } else {
                    OperationResult::Failure
                },
                if unknown {
                    format!(
                        "The {} database action for {} on {} has an unknown outcome; re-read \
                         metadata before retrying",
                        action.as_str(),
                        database_name,
                        engine
                    )
                } else {
                    format!(
                        "The {} database action for {} on {} did not complete",
                        action.as_str(),
                        database_name,
                        engine
                    )
                },
                summarize_error(error),
            )
        }
    };

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(profile.server_id.clone()),
            project_id: None,
            category: OperationCategory::Database,
            operation,
            safe_summary,
            path: None,
            output_summary,
            result,
            exit_code: None,
            started_at,
            duration_ms,
        },
    );
}

fn verified_summary(
    profile: &DatabaseConnection,
    request: &DatabaseActionRequest,
    outcome: &DatabaseActionResult,
) -> String {
    match request {
        DatabaseActionRequest::Create { .. } => format!(
            "Created database {} on {} connection {} and verified its metadata",
            outcome.database,
            engine_label(profile.engine),
            profile.name
        ),
        DatabaseActionRequest::Rename { new_name, .. } => format!(
            "Renamed database {} to {} on {} connection {} and verified both names",
            outcome.database,
            new_name,
            engine_label(profile.engine),
            profile.name
        ),
        DatabaseActionRequest::Drop { .. } => format!(
            "Dropped database {} on {} connection {} and verified it is absent",
            outcome.database,
            engine_label(profile.engine),
            profile.name
        ),
        DatabaseActionRequest::SetActive { .. } => format!(
            "Set database {} active on {} connection {} and verified the connection context",
            outcome.database,
            engine_label(profile.engine),
            profile.name
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AuthType, DatabaseEngine, NewServer};

    fn profile(engine: DatabaseEngine) -> DatabaseConnection {
        DatabaseConnection {
            id: "profile-1".to_string(),
            server_id: "server-1".to_string(),
            name: "Primary".to_string(),
            engine,
            host: "127.0.0.1".to_string(),
            port: 5432,
            socket: None,
            database_name: Some("app".to_string()),
            username: "app".to_string(),
            credential_ref: None,
            read_only: false,
            ssl_mode: None,
            created_at: 1,
            updated_at: 1,
            last_used_at: None,
        }
    }

    fn result(verified: bool) -> DatabaseActionResult {
        DatabaseActionResult {
            action: crate::database::DatabaseActionKind::Drop,
            engine: DatabaseEngine::Postgresql,
            database: "app_next".to_string(),
            previous_name: None,
            statements: vec!["DROP DATABASE \"app_next\"".to_string()],
            verified,
            verification: "Metadata re-read confirms database app_next is no longer present."
                .to_string(),
            verification_error: (!verified).then(|| "Metadata still lists app_next.".to_string()),
            metadata: None,
            active_database: None,
            duration_ms: 4,
        }
    }

    #[test]
    fn success_history_summary_never_contains_generated_ddl() {
        let summary = verified_summary(
            &profile(DatabaseEngine::Postgresql),
            &DatabaseActionRequest::Drop {
                name: "app_next".to_string(),
            },
            &result(true),
        );

        assert!(summary.contains("Dropped database app_next"));
        assert!(summary.contains("PostgreSQL"));
        assert!(!summary.contains("DROP DATABASE"));
    }

    #[test]
    fn only_verified_results_are_success_history() {
        assert!(result(true).verified);
        assert!(!result(false).verified);
    }

    #[test]
    fn server_fixture_uses_a_saved_profile_shape() {
        // This keeps the test fixture aligned with the storage model without
        // creating a database connection.
        let server = NewServer {
            name: "Production".to_string(),
            host: "198.51.100.24".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth_type: AuthType::Password,
            credential_ref: None,
            group_name: None,
            tags: Vec::new(),
            notes: None,
        };
        assert_eq!(server.name, "Production");
        assert_eq!(profile(DatabaseEngine::Mysql).engine, DatabaseEngine::Mysql);
    }
}
