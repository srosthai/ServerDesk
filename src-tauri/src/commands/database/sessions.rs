//! Database session and running-query commands.
//!
//! Listing is a bounded metadata read. Cancelling a query or terminating a
//! session requires the exact typed confirmation phrase, is rejected on a
//! read-only profile, and reports success only after the session list is
//! re-read. History stores the action, session id, account, engine, and
//! verification text; query text and credentials never reach logs or history.

use std::time::Instant;

use tauri::State;

use crate::{
    core::{
        AppError, ErrorCode,
        history::{now_unix_seconds, record_operation, summarize_error},
    },
    database::{
        DatabaseConnectionRegistry, DatabaseSessionService, Page, PageRequest,
        SessionActionRequest, SessionActionResult, SessionSummary,
    },
    storage::{
        Database, DatabaseConnection, NewOperationRecord, OperationCategory, OperationResult,
    },
};

/// Lists active sessions visible to the connected database account.
#[tauri::command]
pub async fn list_database_sessions(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    limit: Option<u32>,
    offset: Option<u64>,
    running_only: Option<bool>,
) -> Result<Page<SessionSummary>, AppError> {
    DatabaseSessionService::new(connections.inner())
        .list(
            &profile_id,
            PageRequest::clamped(limit, offset),
            running_only.unwrap_or(false),
        )
        .await
}

/// Cancels a running query or terminates a session after exact confirmation.
#[tauri::command]
pub async fn run_database_session_action(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: SessionActionRequest,
) -> Result<SessionActionResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = DatabaseSessionService::new(connections.inner())
        .execute(&profile_id, request.clone())
        .await;

    record_session_history(
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

fn record_session_history(
    database: &Database,
    profile: &DatabaseConnection,
    request: &SessionActionRequest,
    outcome: &Result<SessionActionResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let action = request.action.as_str();
    let engine = engine_label(profile.engine);
    let user = request.expected_user.as_str();
    let session_id = request.session_id;
    let (result, safe_summary, output_summary) =
        history_outcome(action, engine, user, session_id, &profile.name, outcome);

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(profile.server_id.clone()),
            project_id: None,
            category: OperationCategory::Database,
            operation: format!("database-{action}"),
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

fn history_outcome(
    action: &str,
    engine: &str,
    user: &str,
    session_id: i64,
    profile_name: &str,
    outcome: &Result<SessionActionResult, AppError>,
) -> (OperationResult, String, Option<String>) {
    match outcome {
        Ok(result) if result.verified => (
            OperationResult::Success,
            format!(
                "Completed the {action} action for session {session_id} (account {user}) on \
                 {engine} connection {profile_name} and verified the outcome"
            ),
            Some(result.verification.clone()),
        ),
        Ok(result) => (
            OperationResult::Unknown,
            format!(
                "The {action} action for session {session_id} (account {user}) on {engine} \
                 connection {profile_name} finished, but verification did not confirm the outcome"
            ),
            Some(
                result
                    .verification_error
                    .clone()
                    .unwrap_or_else(|| result.verification.clone()),
            ),
        ),
        Err(error) if error.code == ErrorCode::Cancelled => (
            OperationResult::Cancelled,
            format!(
                "Cancelled the {action} action for session {session_id} (account {user}) on \
                 {engine} connection {profile_name} before any request was sent"
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
                        "The {action} action for session {session_id} (account {user}) on {engine} \
                         connection {profile_name} has an unknown outcome; refresh the session \
                         list before retrying"
                    )
                } else {
                    format!(
                        "The {action} action for session {session_id} (account {user}) on {engine} \
                         connection {profile_name} did not complete"
                    )
                },
                summarize_error(error),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database::{MetadataValue, SessionActionKind, SessionActionResult},
        storage::{AuthType, DatabaseEngine, NewServer},
    };

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

    fn request() -> SessionActionRequest {
        SessionActionRequest {
            session_id: 501,
            action: SessionActionKind::TerminateSession,
            expected_user: "reporting".to_string(),
            expected_database: Some("app".to_string()),
            confirmation: "terminate session 501".to_string(),
        }
    }

    fn result(verified: bool) -> SessionActionResult {
        SessionActionResult {
            action: SessionActionKind::TerminateSession,
            engine: DatabaseEngine::Postgresql,
            session_id: 501,
            user: "reporting".to_string(),
            database: MetadataValue::available("app".to_string()),
            request_accepted: true,
            verified,
            verification: "Session 501 is no longer present. Termination verified.".to_string(),
            verification_error: (!verified).then(|| "Session 501 was still present.".to_string()),
            duration_ms: 4,
        }
    }

    #[test]
    fn history_never_contains_query_text_or_secrets() {
        let (status, summary, output) = history_outcome(
            "terminate_session",
            "PostgreSQL",
            "reporting",
            501,
            "Primary",
            &Ok(result(true)),
        );

        assert_eq!(status, OperationResult::Success);
        assert!(summary.contains("session 501"));
        assert!(summary.contains("reporting"));
        assert!(!summary.contains("pg_sleep"));
        assert!(!summary.contains("PASSWORD"));
        assert!(
            output
                .as_deref()
                .is_some_and(|output| !output.contains("SELECT") && !output.contains("secret"))
        );
    }

    #[test]
    fn unverified_and_permission_denied_outcomes_are_distinct() {
        let (status, summary, _) = history_outcome(
            "terminate_session",
            "PostgreSQL",
            "reporting",
            501,
            "Primary",
            &Ok(result(false)),
        );
        assert_eq!(status, OperationResult::Unknown);
        assert!(summary.contains("verification did not confirm"));

        let denied = AppError::new(ErrorCode::Permission, "The profile is read-only.");
        let (status, _, _) = history_outcome(
            "terminate_session",
            "PostgreSQL",
            "reporting",
            501,
            "Primary",
            &Err(denied),
        );
        assert_eq!(status, OperationResult::Failure);
    }

    #[test]
    fn history_is_persisted_without_query_text() {
        let database = Database::in_memory().expect("in-memory storage");
        let server = database
            .create_server(&NewServer {
                name: "Fixture".to_string(),
                host: "127.0.0.1".to_string(),
                port: 22,
                username: "deploy".to_string(),
                auth_type: AuthType::Password,
                credential_ref: None,
                group_name: None,
                tags: Vec::new(),
                notes: None,
            })
            .expect("server");
        let mut profile = profile(DatabaseEngine::Postgresql);
        profile.server_id = server.id;

        record_session_history(&database, &profile, &request(), &Ok(result(true)), 1, 4);

        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("session operation recorded");
        assert_eq!(row.operation, "database-terminate_session");
        assert!(row.safe_summary.contains("session 501"));
        assert!(!row.safe_summary.contains("SELECT"));
        assert!(
            row.output_summary
                .as_deref()
                .is_some_and(|output| !output.contains("pg_sleep"))
        );
    }
}
