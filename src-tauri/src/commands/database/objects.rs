//! View, routine, trigger, sequence, and materialized-view commands.
//!
//! Execution re-validates the request, regenerates the same preview, refuses an
//! unconfirmed drop/replace, rejects read-only profiles, and re-reads metadata
//! before reporting success. History records only the action and object name;
//! routine bodies, view queries, and trigger statements never reach logs.

use std::time::Instant;

use tauri::State;

use crate::{
    core::{
        AppError, ErrorCode,
        history::{now_unix_seconds, record_operation, summarize_error},
    },
    database::{
        DatabaseConnectionRegistry, DatabaseObjectService, ObjectActionRequest, ObjectActionResult,
        ObjectDdlPreview, ObjectDefinition, ObjectRef, ObjectRefRequest,
    },
    storage::{
        Database, DatabaseConnection, NewOperationRecord, OperationCategory, OperationResult,
    },
};

/// Reads one bounded view, routine, trigger, or sequence definition.
#[tauri::command]
pub async fn inspect_database_object(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    object: ObjectRefRequest,
) -> Result<ObjectDefinition, AppError> {
    DatabaseObjectService::new(connections.inner())
        .inspect(&profile_id, object)
        .await
}

/// Generates engine-specific DDL for one object action without executing it.
#[tauri::command]
pub async fn preview_database_object_action(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: ObjectActionRequest,
) -> Result<ObjectDdlPreview, AppError> {
    DatabaseObjectService::new(connections.inner())
        .preview(&profile_id, request)
        .await
}

/// Executes one object action after the same preview/capability path.
#[tauri::command]
pub async fn run_database_object_action(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: ObjectActionRequest,
    confirmed: bool,
) -> Result<ObjectActionResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = DatabaseObjectService::new(connections.inner())
        .execute(&profile_id, request.clone(), confirmed)
        .await;

    record_object_history(
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

fn object_label(object: &ObjectRef) -> String {
    let mut label = String::new();
    if let Some(database) = object.database.as_ref() {
        label.push_str(database.as_str());
        label.push('.');
    }
    if let Some(schema) = object.schema.as_ref()
        && Some(schema.as_str()) != object.database.as_ref().map(|value| value.as_str())
    {
        label.push_str(schema.as_str());
        label.push('.');
    }
    label.push_str(object.name.as_str());
    label
}

fn record_object_history(
    database: &Database,
    profile: &DatabaseConnection,
    request: &ObjectActionRequest,
    outcome: &Result<ObjectActionResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let action = request
        .validate()
        .map(|validated| validated.kind())
        .unwrap_or(crate::database::ObjectActionKind::CreateView);
    let engine = engine_label(profile.engine);
    let label = request
        .validate()
        .map(|validated| object_label(validated.object()))
        .unwrap_or_else(|_| "unknown object".to_string());
    let (result, safe_summary, output_summary) =
        history_outcome(action.as_str(), engine, &label, &profile.name, outcome);

    let _ = record_operation(
        database,
        &NewOperationRecord {
            server_id: Some(profile.server_id.clone()),
            project_id: None,
            category: OperationCategory::Database,
            operation: format!("database-{}", action.as_str()),
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
    label: &str,
    profile_name: &str,
    outcome: &Result<ObjectActionResult, AppError>,
) -> (OperationResult, String, Option<String>) {
    match outcome {
        Ok(outcome) if outcome.verified => (
            OperationResult::Success,
            format!(
                "Completed the {action} action for {label} on {engine} connection {profile_name} \
                 and verified its metadata"
            ),
            Some(outcome.verification.clone()),
        ),
        Ok(outcome) => (
            OperationResult::Unknown,
            format!(
                "The {action} action for {label} on {engine} connection {profile_name} finished, \
                 but metadata verification did not confirm the result"
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
                "Cancelled the {action} action for {label} on {engine} connection {profile_name} \
                 before execution"
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
                        "The {action} action for {label} on {engine} connection {profile_name} has \
                         an unknown outcome; re-read metadata before retrying"
                    )
                } else {
                    format!(
                        "The {action} action for {label} on {engine} connection {profile_name} did \
                         not complete"
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
        database::{DatabaseObjectKind, ObjectActionKind, ObjectRef, RoutineKind},
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

    fn object() -> ObjectRef {
        ObjectRef {
            kind: DatabaseObjectKind::Routine,
            database: Some(crate::database::Identifier::new("app").expect("database")),
            schema: Some(crate::database::Identifier::new("public").expect("schema")),
            name: crate::database::Identifier::new("user_email").expect("name"),
            routine_kind: Some(RoutineKind::Function),
            arguments: Some("user_id bigint".to_string()),
            table: None,
        }
    }

    fn result(verified: bool) -> ObjectActionResult {
        ObjectActionResult {
            action: ObjectActionKind::ReplaceRoutine,
            engine: DatabaseEngine::Postgresql,
            object: object(),
            statements: vec![
                "CREATE OR REPLACE FUNCTION \"public\".\"user_email\"(user_id bigint) RETURNS text \
                 LANGUAGE sql AS 'secret body'"
                    .to_string(),
            ],
            warnings: Vec::new(),
            verified,
            verification: "Metadata re-read confirms the routine exists.".to_string(),
            verification_error: (!verified)
                .then(|| "Metadata still lists the routine.".to_string()),
            definition: None,
            duration_ms: 3,
        }
    }

    #[test]
    fn history_summary_never_contains_definition_text() {
        let (status, summary, output) = history_outcome(
            "replace_routine",
            "PostgreSQL",
            "app.public.user_email",
            "Primary",
            &Ok(result(true)),
        );

        assert_eq!(status, OperationResult::Success);
        assert!(summary.contains("replace_routine"));
        assert!(summary.contains("app.public.user_email"));
        assert!(!summary.contains("CREATE OR REPLACE"));
        assert!(!summary.contains("secret body"));
        assert!(output.is_some_and(|output| !output.contains("secret body")));
    }

    #[test]
    fn cancelled_and_permission_denied_actions_have_distinct_results() {
        let cancelled = AppError::new(ErrorCode::Cancelled, "No statement was executed.");
        let (status, summary, _) = history_outcome(
            "drop_view",
            "PostgreSQL",
            "app.public.active_users",
            "Primary",
            &Err(cancelled),
        );
        assert_eq!(status, OperationResult::Cancelled);
        assert!(summary.contains("Cancelled the drop_view action"));

        let denied = AppError::new(ErrorCode::Permission, "The profile is read-only.");
        let (status, _, _) = history_outcome(
            "drop_view",
            "PostgreSQL",
            "app.public.active_users",
            "Primary",
            &Err(denied),
        );
        assert_eq!(status, OperationResult::Failure);
    }

    #[test]
    fn history_is_persisted_without_definitions() {
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

        let request = ObjectActionRequest::CreateRoutine {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "user_email".to_string(),
            routine_kind: RoutineKind::Function,
            arguments: Some("user_id bigint".to_string()),
            return_type: Some("text".to_string()),
            language: Some("sql".to_string()),
            body: "SELECT 'super-secret'".to_string(),
            or_replace: true,
        };

        record_object_history(&database, &profile, &request, &Ok(result(true)), 1, 4);

        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("object operation recorded");
        assert_eq!(row.operation, "database-replace_routine");
        assert!(row.safe_summary.contains("app.public.user_email"));
        assert!(!row.safe_summary.contains("super-secret"));
        assert!(
            row.output_summary
                .as_deref()
                .is_some_and(|output| !output.contains("super-secret"))
        );
    }
}
