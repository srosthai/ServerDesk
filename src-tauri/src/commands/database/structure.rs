//! Structure-change preview and execution commands.
//!
//! Execution covers table lifecycle, column add/edit/rename/drop/reorder, index
//! create/rename/drop, and primary-key, unique, foreign-key, and check
//! constraint add/drop. Every execution re-validates the request, regenerates
//! the DDL preview, refuses an unconfirmed destructive action, rejects
//! read-only profiles, checks blocking dependencies, and re-reads metadata
//! before reporting success. The full structure editor is #174.

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
        "add_column" => format!(
            "Added column {} to table {label} on {engine} connection {profile_name} and verified \
             its metadata",
            object.name.as_deref().unwrap_or("the requested column")
        ),
        "alter_column" => format!(
            "Edited column {} on table {label} on {engine} connection {profile_name} and verified \
             its metadata",
            object.name.as_deref().unwrap_or("the requested column")
        ),
        "rename_column" => format!(
            "Renamed column {} on table {label} on {engine} connection {profile_name} and verified \
             the new name",
            object.name.as_deref().unwrap_or("the requested column")
        ),
        "drop_column" => format!(
            "Dropped column {} from table {label} on {engine} connection {profile_name} and \
             verified it is absent",
            object.name.as_deref().unwrap_or("the requested column")
        ),
        "reorder_column" => format!(
            "Reordered column {} on table {label} on {engine} connection {profile_name} and \
             verified its position",
            object.name.as_deref().unwrap_or("the requested column")
        ),
        "create_index" => format!(
            "Created index {} on table {label} on {engine} connection {profile_name} and verified \
             its metadata",
            object.name.as_deref().unwrap_or("the requested index")
        ),
        "rename_index" => format!(
            "Renamed index {} on table {label} on {engine} connection {profile_name} and verified \
             the new name",
            object.name.as_deref().unwrap_or("the requested index")
        ),
        "drop_index" => format!(
            "Dropped index {} from table {label} on {engine} connection {profile_name} and \
             verified it is absent",
            object.name.as_deref().unwrap_or("the requested index")
        ),
        "add_constraint" => format!(
            "Added constraint {} to table {label} on {engine} connection {profile_name} and \
             verified its metadata",
            object.name.as_deref().unwrap_or("the requested constraint")
        ),
        "drop_constraint" => format!(
            "Dropped constraint {} from table {label} on {engine} connection {profile_name} and \
             verified it is absent",
            object.name.as_deref().unwrap_or("the requested constraint")
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
        database::{
            ColumnDefaultRequest, ColumnDefinitionRequest, StructureActionKind,
            StructureObjectKind, TableSummary,
        },
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

    fn object(table: &str) -> AffectedObject {
        AffectedObject {
            kind: StructureObjectKind::Table,
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: table.to_string(),
            name: None,
        }
    }

    fn column_object(table: &str, name: &str) -> AffectedObject {
        AffectedObject {
            kind: StructureObjectKind::Column,
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: table.to_string(),
            name: Some(name.to_string()),
        }
    }

    fn index_object(table: &str, name: &str) -> AffectedObject {
        AffectedObject {
            kind: StructureObjectKind::Index,
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: table.to_string(),
            name: Some(name.to_string()),
        }
    }

    fn constraint_object(table: &str, name: &str) -> AffectedObject {
        AffectedObject {
            kind: StructureObjectKind::Constraint,
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: table.to_string(),
            name: Some(name.to_string()),
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

    #[test]
    fn column_history_summaries_name_the_action_without_ddl_or_values() {
        let mut outcome = result(StructureActionKind::AddColumn, true);
        outcome.object = column_object("accounts", "Display Name");
        let object = outcome.object.clone();
        let (status, summary, output) = history_outcome(
            "add_column",
            "PostgreSQL",
            "app.public.accounts",
            &object,
            "Primary",
            &Ok(outcome),
        );

        assert_eq!(status, OperationResult::Success);
        assert!(summary.contains("Added column Display Name to table app.public.accounts"));
        assert!(!summary.contains("ALTER TABLE"));
        assert!(!summary.contains("varchar"));
        assert!(output.is_some_and(|output| !output.contains("ALTER TABLE")));
    }

    #[test]
    fn column_operation_history_is_persisted_without_generated_ddl_or_defaults() {
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

        let request = StructureActionRequest::AddColumn {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "accounts".to_string(),
            column: ColumnDefinitionRequest {
                name: "Display Name".to_string(),
                data_type: "varchar(120)".to_string(),
                nullable: false,
                default: Some(ColumnDefaultRequest::String {
                    value: "super-secret-default".to_string(),
                }),
                comment: Some("Shown in lists".to_string()),
                generated: None,
                identity: None,
            },
            position: None,
        };
        let mut outcome = result(StructureActionKind::AddColumn, true);
        outcome.object = column_object("accounts", "Display Name");
        outcome.statements = vec![
            "ALTER TABLE \"public\".\"accounts\" ADD COLUMN \"Display Name\" varchar(120) NOT \
             NULL DEFAULT 'super-secret-default'"
                .to_string(),
        ];
        outcome.verification =
            "Metadata re-read confirms column Display Name was added.".to_string();

        record_structure_history(&database, &profile, &request, &Ok(outcome), 1, 7);

        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("column operation recorded");
        assert_eq!(row.operation, "database-add_column");
        assert_eq!(row.category, OperationCategory::Database);
        assert!(row.safe_summary.contains("Added column Display Name"));
        assert!(!row.safe_summary.contains("ALTER TABLE"));
        assert!(!row.safe_summary.contains("super-secret-default"));
        assert!(
            row.output_summary
                .as_deref()
                .is_some_and(|output| !output.contains("super-secret-default"))
        );
    }

    #[test]
    fn index_and_constraint_history_summaries_never_include_ddl() {
        let mut index_outcome = result(StructureActionKind::CreateIndex, true);
        index_outcome.object = index_object("accounts", "accounts_email_idx");
        index_outcome.statements = vec![
            "CREATE UNIQUE INDEX \"accounts_email_idx\" ON \"public\".\"accounts\" (\"email\")"
                .to_string(),
        ];
        let object = index_outcome.object.clone();
        let (status, summary, output) = history_outcome(
            "create_index",
            "PostgreSQL",
            "app.public.accounts",
            &object,
            "Primary",
            &Ok(index_outcome),
        );
        assert_eq!(status, OperationResult::Success);
        assert!(summary.contains("Created index accounts_email_idx"));
        assert!(!summary.contains("CREATE"));
        assert!(output.is_some_and(|output| !output.contains("CREATE")));

        let mut constraint_outcome = result(StructureActionKind::DropConstraint, true);
        constraint_outcome.object = constraint_object("orders", "orders_user_fk");
        constraint_outcome.statements = vec![
            "ALTER TABLE \"public\".\"orders\" DROP CONSTRAINT \"orders_user_fk\"".to_string(),
        ];
        let object = constraint_outcome.object.clone();
        let (status, summary, output) = history_outcome(
            "drop_constraint",
            "PostgreSQL",
            "app.public.orders",
            &object,
            "Primary",
            &Ok(constraint_outcome),
        );
        assert_eq!(status, OperationResult::Success);
        assert!(summary.contains("Dropped constraint orders_user_fk"));
        assert!(!summary.contains("ALTER TABLE"));
        assert!(output.is_some_and(|output| !output.contains("ALTER TABLE")));
    }

    #[test]
    fn index_history_is_persisted_without_generated_ddl() {
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

        let request = StructureActionRequest::CreateIndex {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "accounts".to_string(),
            name: "accounts_email_idx".to_string(),
            columns: vec!["email".to_string()],
            unique: true,
            method: None,
            predicate: Some("email IS NOT NULL".to_string()),
        };
        let mut outcome = result(StructureActionKind::CreateIndex, true);
        outcome.object = index_object("accounts", "accounts_email_idx");
        outcome.statements = vec![
            "CREATE UNIQUE INDEX \"accounts_email_idx\" ON \"public\".\"accounts\" (\"email\") \
             WHERE (email IS NOT NULL)"
                .to_string(),
        ];

        record_structure_history(&database, &profile, &request, &Ok(outcome), 1, 3);

        let rows = database.list_operations(10).expect("history");
        let row = rows.first().expect("index operation recorded");
        assert_eq!(row.operation, "database-create_index");
        assert!(
            row.safe_summary
                .contains("Created index accounts_email_idx")
        );
        assert!(!row.safe_summary.contains("CREATE INDEX"));
        assert!(
            row.output_summary
                .as_deref()
                .is_some_and(|output| !output.contains("CREATE INDEX"))
        );
    }
}
