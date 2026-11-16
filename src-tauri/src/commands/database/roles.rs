//! Database user, role, and grant commands.
//!
//! Listing and inspection are bounded metadata reads. Every mutation
//! re-validates the request, regenerates the exact preview, refuses an
//! unconfirmed change, rejects read-only profiles, and re-reads role metadata
//! and effective privileges before reporting success. History records only the
//! action, account label, and safe verification text; generated DDL, password
//! material, and privilege internals never reach logs.

use std::time::Instant;

use tauri::State;

use crate::{
    core::{
        AppError, ErrorCode,
        history::{now_unix_seconds, record_operation, summarize_error},
    },
    database::{
        DatabaseConnectionRegistry, DatabaseRoleService, Page, PageRequest, RoleActionRequest,
        RoleActionResult, RoleDdlPreview, RolePrivilegeSnapshot, RoleRefRequest, RoleSummary,
    },
    storage::{
        Database, DatabaseConnection, NewOperationRecord, OperationCategory, OperationResult,
    },
};

/// Lists database users and roles without exposing authentication columns.
#[tauri::command]
pub async fn list_database_roles(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    limit: Option<u32>,
    offset: Option<u64>,
) -> Result<Page<RoleSummary>, AppError> {
    DatabaseRoleService::new(connections.inner())
        .list(&profile_id, PageRequest::clamped(limit, offset))
        .await
}

/// Reads one role's direct grants, memberships, and effective privileges.
#[tauri::command]
pub async fn get_database_role_privileges(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    role: RoleRefRequest,
) -> Result<RolePrivilegeSnapshot, AppError> {
    DatabaseRoleService::new(connections.inner())
        .inspect(&profile_id, role)
        .await
}

/// Generates exact grant, revoke, create, or alter DDL with a privilege diff.
#[tauri::command]
pub async fn preview_database_role_action(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: RoleActionRequest,
) -> Result<RoleDdlPreview, AppError> {
    DatabaseRoleService::new(connections.inner())
        .preview(&profile_id, request)
        .await
}

/// Executes one role or privilege action after the same preview path.
#[tauri::command]
pub async fn run_database_role_action(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: RoleActionRequest,
    confirmed: bool,
) -> Result<RoleActionResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = DatabaseRoleService::new(connections.inner())
        .execute(&profile_id, request.clone(), confirmed)
        .await;

    record_role_history(
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

fn record_role_history(
    database: &Database,
    profile: &DatabaseConnection,
    request: &RoleActionRequest,
    outcome: &Result<RoleActionResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let action = request.kind();
    let engine = engine_label(profile.engine);
    let label = request.role_label();
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
    outcome: &Result<RoleActionResult, AppError>,
) -> (OperationResult, String, Option<String>) {
    match outcome {
        Ok(outcome) if outcome.verified => (
            OperationResult::Success,
            format!(
                "Completed the {action} action for {label} on {engine} connection {profile_name} \
                 and verified role metadata and effective privileges"
            ),
            Some(outcome.verification.clone()),
        ),
        Ok(outcome) => (
            OperationResult::Unknown,
            format!(
                "The {action} action for {label} on {engine} connection {profile_name} finished, \
                 but role verification did not confirm the result"
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
                        "The {action} action for {label} on {engine} connection {profile_name} \
                         has an unknown outcome; re-read role metadata before retrying"
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
        database::{Privilege, PrivilegeTargetKind, PrivilegeTargetRequest, RoleActionKind},
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

    fn request() -> RoleActionRequest {
        RoleActionRequest::GrantPrivileges {
            grantee: RoleRefRequest {
                name: "reporting".to_string(),
                host: None,
            },
            privileges: vec![Privilege::Select],
            target: PrivilegeTargetRequest {
                kind: PrivilegeTargetKind::Table,
                database: None,
                schema: Some("public".to_string()),
                object: Some("orders".to_string()),
            },
            with_grant_option: false,
        }
    }

    fn result(verified: bool) -> RoleActionResult {
        RoleActionResult {
            action: RoleActionKind::GrantPrivileges,
            engine: DatabaseEngine::Postgresql,
            role: crate::database::RoleRef {
                name: "reporting".to_string(),
                host: None,
            },
            target: None,
            statements: vec![
                "GRANT SELECT ON TABLE \"public\".\"orders\" TO \"reporting\"".to_string(),
            ],
            warnings: Vec::new(),
            verified,
            verification: "Privilege metadata re-read confirms the effective privileges."
                .to_string(),
            verification_error: (!verified).then(|| "Privileges did not match.".to_string()),
            privilege_diff: crate::database::PrivilegeDiff::default(),
            role_changes: Vec::new(),
            duration_ms: 3,
        }
    }

    #[test]
    fn history_summary_never_contains_generated_sql_or_secrets() {
        let (status, summary, output) = history_outcome(
            "grant_privileges",
            "PostgreSQL",
            "reporting",
            "Primary",
            &Ok(result(true)),
        );

        assert_eq!(status, OperationResult::Success);
        assert!(summary.contains("grant_privileges"));
        assert!(summary.contains("reporting"));
        assert!(!summary.contains("GRANT"));
        assert!(!summary.contains("public"));
        assert!(!summary.contains("password"));
        assert!(output.is_some_and(|output| !output.contains("GRANT")));
    }

    #[test]
    fn cancelled_and_permission_denied_actions_have_distinct_results() {
        let cancelled = AppError::new(ErrorCode::Cancelled, "No statement was executed.");
        let (status, summary, _) = history_outcome(
            "revoke_privileges",
            "PostgreSQL",
            "reporting",
            "Primary",
            &Err(cancelled),
        );
        assert_eq!(status, OperationResult::Cancelled);
        assert!(summary.contains("Cancelled the revoke_privileges action"));

        let denied = AppError::new(ErrorCode::Permission, "The profile is read-only.");
        let (status, _, _) = history_outcome(
            "revoke_privileges",
            "PostgreSQL",
            "reporting",
            "Primary",
            &Err(denied),
        );
        assert_eq!(status, OperationResult::Failure);
    }

    #[test]
    fn history_is_persisted_without_statements() {
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

        record_role_history(&database, &profile, &request(), &Ok(result(true)), 1, 4);

        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("role operation recorded");
        assert_eq!(row.operation, "database-grant_privileges");
        assert!(row.safe_summary.contains("reporting"));
        assert!(!row.safe_summary.contains("GRANT"));
        assert!(
            row.output_summary
                .as_deref()
                .is_some_and(|output| !output.contains("GRANT"))
        );
    }
}
