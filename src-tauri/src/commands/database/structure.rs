//! Structure-change preview and execution commands.
//!
//! Execution is deliberately narrow in this slice: table create, rename,
//! duplicate, truncate, and drop. Every execution re-validates the request,
//! regenerates the DDL preview, refuses an unconfirmed destructive action,
//! rejects read-only profiles, checks blocking dependencies, and re-reads
//! metadata before reporting success. The full structure editor is #174.

use std::time::Instant;

use tauri::State;

use crate::{
    core::{
        AppError, ErrorCode,
        history::{now_unix_seconds, record_operation, summarize_error},
    },
    database::{
        AffectedObject, DatabaseConnectionRegistry, DatabaseStructureService,
        StructureActionRequest, StructureActionResult, StructureDdlPreview,
    },
    storage::{
        Database, DatabaseConnection, NewOperationRecord, OperationCategory, OperationResult,
    },
};

/// Generates engine-specific DDL for a table, column, index, or constraint
/// change without executing it.
#[tauri::command]
pub async fn preview_database_structure_action(
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: StructureActionRequest,
) -> Result<StructureDdlPreview, AppError> {
    DatabaseStructureService::new(connections.inner())
        .preview(&profile_id, request)
        .await
}

/// Executes one structure action after the same preview/capability path.
#[tauri::command]
pub async fn run_database_structure_action(
    database: State<'_, Database>,
    connections: State<'_, DatabaseConnectionRegistry>,
    profile_id: String,
    request: StructureActionRequest,
    confirmed: bool,
) -> Result<StructureActionResult, AppError> {
    let profile = load_profile(database.inner(), &profile_id)?;
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let outcome = DatabaseStructureService::new(connections.inner())
        .execute(&profile_id, request.clone(), confirmed)
        .await;

    record_structure_history(
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

fn table_label(object: &AffectedObject) -> String {
    let mut label = String::new();
    if let Some(database) = object.database.as_deref() {
        label.push_str(database);
        label.push('.');
    }
    if let Some(schema) = object.schema.as_deref()
        && Some(schema) != object.database.as_deref()
    {
        label.push_str(schema);
        label.push('.');
    }
    label.push_str(&object.table);
    label
}

fn record_structure_history(
    database: &Database,
    profile: &DatabaseConnection,
    request: &StructureActionRequest,
    outcome: &Result<StructureActionResult, AppError>,
    started_at: i64,
    duration_ms: i64,
) {
    let action = request.kind();
    let engine = engine_label(profile.engine);
    let object = request
        .validate()
        .map(|validated| validated.object())
        .unwrap_or_else(|_| AffectedObject {
            kind: crate::database::StructureObjectKind::Table,
            database: None,
            schema: None,
            table: request.table_name().unwrap_or("unknown").to_string(),
            name: None,
        });
    let label = table_label(&object);
    let (result, safe_summary, output_summary) = history_outcome(
        action.as_str(),
        engine,
        &label,
        &object,
        &profile.name,
        outcome,
    );

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
    object: &AffectedObject,
    profile_name: &str,
    outcome: &Result<StructureActionResult, AppError>,
) -> (OperationResult, String, Option<String>) {
    match outcome {
        Ok(outcome) if outcome.verified => (
            OperationResult::Success,
            verified_structure_summary(action, engine, label, object, profile_name, outcome),
            Some(outcome.verification.clone()),
        ),
        Ok(outcome) => (
            OperationResult::Unknown,
            format!(
                "The {action} action for table {label} on {engine} connection {profile_name} \
                 finished, but metadata verification did not confirm the result"
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
                "Cancelled the {action} action for table {label} on {engine} connection \
                 {profile_name} before execution"
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
                        "The {action} action for table {label} on {engine} connection \
                         {profile_name} has an unknown outcome; re-read metadata before retrying"
                    )
                } else {
                    format!(
                        "The {action} action for table {label} on {engine} connection \
                         {profile_name} did not complete"
                    )
                },
                summarize_error(error),
            )
        }
    }
}

fn verified_structure_summary(
    action: &str,
    engine: &str,
    label: &str,
    object: &AffectedObject,
    profile_name: &str,
    outcome: &StructureActionResult,
) -> String {
    match action {
        "create_table" => format!(
            "Created table {label} on {engine} connection {profile_name} and verified its metadata"
        ),
        "rename_table" => format!(
            "Renamed table {label} to {} on {engine} connection {profile_name} and verified both \
             names",
            outcome.target_table.as_deref().unwrap_or("the new name")
        ),
        "duplicate_table" => format!(
            "Duplicated table {label} as {} on {engine} connection {profile_name} and verified the \
             new structure",
            outcome
                .target_table
                .as_deref()
                .unwrap_or("the duplicate name")
        ),
        "truncate_table" => format!(
            "Truncated table {label} on {engine} connection {profile_name} and verified the table \
             metadata"
        ),
        "drop_table" => format!(
            "Dropped table {label} on {engine} connection {profile_name} and verified it is absent"
        ),
        _ if object.name.is_some() => format!(
            "Completed the {action} action for {} on {engine} connection {profile_name} and \
             verified its metadata",
            object.name.as_deref().unwrap_or(label)
        ),
        _ => format!(
            "Completed the {action} action for table {label} on {engine} connection {profile_name} \
             and verified its metadata"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database::{StructureActionKind, StructureObjectKind, TableSummary},
        storage::DatabaseEngine,
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

    fn object(table: &str) -> AffectedObject {
        AffectedObject {
            kind: StructureObjectKind::Table,
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: table.to_string(),
            name: None,
        }
    }

    fn result(action: StructureActionKind, verified: bool) -> StructureActionResult {
        StructureActionResult {
            action,
            engine: DatabaseEngine::Postgresql,
            object: object("accounts"),
            target_table: Some("accounts_copy".to_string()),
            statements: vec![
                "CREATE TABLE \"public\".\"accounts_copy\" (LIKE \"public\".\"accounts\")"
                    .to_string(),
            ],
            warnings: Vec::new(),
            verified,
            verification: "Metadata re-read confirms the table exists.".to_string(),
            verification_error: (!verified).then(|| "Metadata still lists the table.".to_string()),
            metadata: None::<TableSummary>,
            duration_ms: 4,
        }
    }

    #[test]
    fn success_history_summary_never_contains_generated_ddl_or_rows() {
        let outcome = result(StructureActionKind::DuplicateTable, true);
        let (status, summary, output) = history_outcome(
            "duplicate_table",
            "PostgreSQL",
            "app.public.accounts",
            &object("accounts"),
            &profile(DatabaseEngine::Postgresql).name,
            &Ok(outcome),
        );

        assert_eq!(status, OperationResult::Success);
        assert!(summary.contains("Duplicated table app.public.accounts as accounts_copy"));
        assert!(summary.contains("PostgreSQL"));
        assert!(!summary.contains("CREATE TABLE"));
        assert!(!summary.contains("INSERT"));
        assert!(!summary.contains("row"));
        assert!(output.is_some_and(|output| !output.contains("CREATE TABLE")));
    }

    #[test]
    fn verification_failure_is_an_unknown_history_outcome() {
        let (status, summary, _) = history_outcome(
            "create_table",
            "MySQL",
            "app.users",
            &object("users"),
            "Primary",
            &Ok(result(StructureActionKind::CreateTable, false)),
        );

        assert_eq!(status, OperationResult::Unknown);
        assert!(summary.contains("did not confirm the result"));
    }

    #[test]
    fn cancellation_and_permission_denial_have_distinct_history_results() {
        let cancelled = AppError::new(ErrorCode::Cancelled, "No statement was executed.");
        let (status, summary, _) = history_outcome(
            "drop_table",
            "PostgreSQL",
            "app.public.users",
            &object("users"),
            "Primary",
            &Err(cancelled),
        );
        assert_eq!(status, OperationResult::Cancelled);
        assert!(summary.contains("Cancelled the drop_table action"));

        let denied = AppError::new(ErrorCode::Permission, "The profile is read-only.");
        let (status, summary, _) = history_outcome(
            "truncate_table",
            "MySQL",
            "app.users",
            &object("users"),
            "Primary",
            &Err(denied),
        );
        assert_eq!(status, OperationResult::Failure);
        assert!(summary.contains("did not complete"));
    }
}
