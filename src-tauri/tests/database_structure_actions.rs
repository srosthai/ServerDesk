//! Table lifecycle execution tests.
//!
//! These run the complete preview, confirmation, dependency, execution, and
//! verification path through the deterministic fake adapter. Live MySQL and
//! PostgreSQL success paths are exercised by the opt-in fixture tests in
//! `database_mysql.rs` and `database_postgres.rs`.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        ColumnDefinitionRequest, ColumnIdentity, DatabaseAdapter, DatabaseConnectRequest,
        DatabaseErrorKind, DatabasePassword, FakeDatabaseAdapter, StructureActionRequest,
        TableOptionsRequest, UniqueKeyRequest, execute_structure_with_adapter,
        preview_structure_with_adapter,
    },
    storage::DatabaseEngine,
};

fn connect_request(engine: DatabaseEngine, read_only: bool) -> DatabaseConnectRequest {
    DatabaseConnectRequest {
        host: "127.0.0.1".to_string(),
        port: if engine == DatabaseEngine::Mysql {
            3306
        } else {
            5432
        },
        socket: None,
        database: Some("app".to_string()),
        username: "app".to_string(),
        password: DatabasePassword::new("fixture-password"),
        read_only,
        ssl_mode: None,
        connect_timeout: Duration::from_secs(5),
    }
}

async fn connected(engine: DatabaseEngine, read_only: bool) -> FakeDatabaseAdapter {
    let mut adapter = FakeDatabaseAdapter::new(
        engine,
        Some(if engine == DatabaseEngine::Mysql {
            "8.0.36"
        } else {
            "16.2"
        }),
    );
    adapter
        .connect(connect_request(engine, read_only))
        .await
        .expect("fake connect");
    adapter
}

fn column(name: &str, data_type: &str, nullable: bool) -> ColumnDefinitionRequest {
    ColumnDefinitionRequest {
        name: name.to_string(),
        data_type: data_type.to_string(),
        nullable,
        default: None,
        comment: None,
        generated: None,
        identity: None,
    }
}

fn create_request(engine: DatabaseEngine, table: &str) -> StructureActionRequest {
    let mut id = column("id", "bigint", false);
    id.identity = Some(match engine {
        DatabaseEngine::Mysql => ColumnIdentity::AutoIncrement,
        DatabaseEngine::Postgresql => ColumnIdentity::ByDefault,
    });
    let mut email = column("email", "varchar(320)", false);
    email.comment = Some("Login address".to_string());

    let options = match engine {
        DatabaseEngine::Mysql => Some(TableOptionsRequest {
            engine: Some("InnoDB".to_string()),
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_unicode_ci".to_string()),
            tablespace: None,
            unlogged: false,
            comment: Some("ServerDesk operations fixture".to_string()),
        }),
        DatabaseEngine::Postgresql => Some(TableOptionsRequest {
            engine: None,
            charset: None,
            collation: None,
            tablespace: None,
            unlogged: true,
            comment: Some("ServerDesk operations fixture".to_string()),
        }),
    };

    StructureActionRequest::CreateTable {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: table.to_string(),
        columns: vec![id, email],
        primary_key: vec!["id".to_string()],
        unique_keys: vec![UniqueKeyRequest {
            name: format!("{table}_email_key"),
            columns: vec!["email".to_string()],
        }],
        options,
    }
}

fn rename_request(engine: DatabaseEngine, table: &str, new_name: &str) -> StructureActionRequest {
    StructureActionRequest::RenameTable {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: table.to_string(),
        new_name: new_name.to_string(),
    }
}

fn duplicate_request(
    engine: DatabaseEngine,
    table: &str,
    new_name: &str,
) -> StructureActionRequest {
    StructureActionRequest::DuplicateTable {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: table.to_string(),
        new_name: new_name.to_string(),
    }
}

fn truncate_request(engine: DatabaseEngine, table: &str) -> StructureActionRequest {
    StructureActionRequest::TruncateTable {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: table.to_string(),
    }
}

fn drop_request(engine: DatabaseEngine, table: &str) -> StructureActionRequest {
    StructureActionRequest::DropTable {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: table.to_string(),
    }
}

async fn lifecycle_round_trip(engine: DatabaseEngine, table: &str) {
    let mut adapter = connected(engine, false).await;
    let duplicate = format!("{table}_copy");
    let renamed = format!("{table}_renamed");

    let preview = preview_structure_with_adapter(&adapter, &create_request(engine, table))
        .await
        .expect("create preview");
    assert!(preview.ddl.contains(table));
    assert!(preview.ddl.contains("UNIQUE"));
    match engine {
        DatabaseEngine::Mysql => {
            assert!(preview.ddl.contains("ENGINE=InnoDB"));
            assert!(preview.ddl.contains("DEFAULT CHARACTER SET utf8mb4"));
            assert!(preview.ddl.contains("COMMENT="));
            assert!(preview.ddl.contains("AUTO_INCREMENT"));
        }
        DatabaseEngine::Postgresql => {
            assert!(preview.ddl.contains("CREATE UNLOGGED TABLE"));
            assert!(preview.ddl.contains("GENERATED BY DEFAULT AS IDENTITY"));
            assert!(preview.ddl.contains("COMMENT ON TABLE"));
        }
    }

    let created =
        execute_structure_with_adapter(&mut adapter, create_request(engine, table), false)
            .await
            .expect("create table");
    assert!(created.verified, "{}", created.verification);
    assert_eq!(created.action.as_str(), "create_table");
    assert_eq!(
        created
            .metadata
            .as_ref()
            .map(|metadata| metadata.name.as_str()),
        Some(table)
    );

    let duplicated = execute_structure_with_adapter(
        &mut adapter,
        duplicate_request(engine, table, &duplicate),
        false,
    )
    .await
    .expect("duplicate table");
    assert!(duplicated.verified, "{}", duplicated.verification);
    assert_eq!(duplicated.target_table.as_deref(), Some(duplicate.as_str()));
    assert!(duplicated.statements[0].contains("LIKE"));

    let renamed_result = execute_structure_with_adapter(
        &mut adapter,
        rename_request(engine, &duplicate, &renamed),
        false,
    )
    .await
    .expect("rename table");
    assert!(renamed_result.verified, "{}", renamed_result.verification);
    assert_eq!(
        renamed_result.target_table.as_deref(),
        Some(renamed.as_str())
    );

    let truncated =
        execute_structure_with_adapter(&mut adapter, truncate_request(engine, &renamed), true)
            .await
            .expect("truncate table");
    assert!(truncated.verified, "{}", truncated.verification);
    assert_eq!(
        truncated
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.estimated_rows.value().copied()),
        Some(0)
    );

    let dropped =
        execute_structure_with_adapter(&mut adapter, drop_request(engine, &renamed), true)
            .await
            .expect("drop table");
    assert!(dropped.verified, "{}", dropped.verification);
    assert_eq!(
        adapter
            .table_summary(
                &serverdesk_lib::database::TableRef::new(
                    Some("app"),
                    (engine == DatabaseEngine::Postgresql).then_some("public"),
                    &renamed,
                )
                .expect("table ref")
            )
            .await
            .expect("summary after drop"),
        None
    );

    let _ = execute_structure_with_adapter(&mut adapter, drop_request(engine, table), true).await;
}

#[tokio::test]
async fn mysql_table_lifecycle_is_verified() {
    lifecycle_round_trip(DatabaseEngine::Mysql, "serverdesk_ops_mysql").await;
}

#[tokio::test]
async fn postgres_table_lifecycle_is_verified() {
    lifecycle_round_trip(DatabaseEngine::Postgresql, "serverdesk_ops_pg").await;
}

#[tokio::test]
async fn destructive_table_actions_require_explicit_confirmation() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, false).await;
        let table = format!("serverdesk_cancel_{}", engine.as_str());
        execute_structure_with_adapter(&mut adapter, create_request(engine, &table), false)
            .await
            .expect("create table");

        for request in [
            truncate_request(engine, &table),
            drop_request(engine, &table),
        ] {
            let error = execute_structure_with_adapter(&mut adapter, request, false)
                .await
                .expect_err("destructive action must be cancelled");
            assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
            assert!(error.message.contains("not confirmed"));
        }

        let summary = adapter
            .table_summary(
                &serverdesk_lib::database::TableRef::new(
                    Some("app"),
                    (engine == DatabaseEngine::Postgresql).then_some("public"),
                    &table,
                )
                .expect("table ref"),
            )
            .await
            .expect("summary");
        assert!(summary.is_some(), "cancelled actions must not run");

        let _ =
            execute_structure_with_adapter(&mut adapter, drop_request(engine, &table), true).await;
    }
}

#[tokio::test]
async fn read_only_profiles_reject_table_changes_with_permission_errors() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, true).await;
        let table = format!("serverdesk_readonly_{}", engine.as_str());

        let error =
            execute_structure_with_adapter(&mut adapter, create_request(engine, &table), false)
                .await
                .expect_err("read-only profile must reject create table");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert!(error.message.contains("read-only"));
        assert_eq!(adapter.structure_execution_count(), 0);
    }
}

#[tokio::test]
async fn unsupported_table_options_fail_before_execution() {
    let mut mysql = connected(DatabaseEngine::Mysql, false).await;
    let mut request = create_request(DatabaseEngine::Mysql, "serverdesk_options_mysql");
    if let StructureActionRequest::CreateTable { options, .. } = &mut request
        && let Some(options) = options
    {
        options.tablespace = Some("fast".to_string());
    }
    let error = execute_structure_with_adapter(&mut mysql, request, false)
        .await
        .expect_err("MySQL has no PostgreSQL tablespace option");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert_eq!(mysql.structure_execution_count(), 0);

    let mut postgres = connected(DatabaseEngine::Postgresql, false).await;
    let mut request = create_request(DatabaseEngine::Postgresql, "serverdesk_options_pg");
    if let StructureActionRequest::CreateTable { options, .. } = &mut request
        && let Some(options) = options
    {
        options.engine = Some("InnoDB".to_string());
    }
    let error = execute_structure_with_adapter(&mut postgres, request, false)
        .await
        .expect_err("PostgreSQL has no MySQL storage engine option");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert_eq!(postgres.structure_execution_count(), 0);
}

#[tokio::test]
async fn verification_detects_an_execution_that_did_not_apply() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false)
        .await
        .without_structure_execution();

    let result = execute_structure_with_adapter(
        &mut adapter,
        create_request(DatabaseEngine::Postgresql, "serverdesk_unapplied"),
        false,
    )
    .await
    .expect("execution itself reports success");

    assert!(!result.verified);
    assert!(
        result.verification.contains("does not list table"),
        "{}",
        result.verification
    );
    assert!(result.verification_error.is_some());
}

#[tokio::test]
async fn dependency_failures_name_the_blocking_objects() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;

    for request in [
        drop_request(DatabaseEngine::Postgresql, "users"),
        truncate_request(DatabaseEngine::Postgresql, "users"),
    ] {
        let error = execute_structure_with_adapter(&mut adapter, request, true)
            .await
            .expect_err("users has an incoming foreign key in the fixture");
        assert_eq!(error.kind, DatabaseErrorKind::Dependency);
        assert!(error.message.contains("depend"));
        assert!(
            error
                .details
                .as_deref()
                .is_some_and(|details| details.contains("orders_user_fk"))
        );
    }
}

#[tokio::test]
async fn non_table_actions_are_preview_only_until_the_structure_editor() {
    let mut adapter = connected(DatabaseEngine::Mysql, false).await;
    let request = StructureActionRequest::AddColumn {
        database: Some("app".to_string()),
        schema: None,
        table: "users".to_string(),
        column: column("nickname", "varchar(80)", true),
        position: None,
    };

    let error = execute_structure_with_adapter(&mut adapter, request, false)
        .await
        .expect_err("column execution is not part of this slice");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert_eq!(adapter.structure_execution_count(), 0);
}

#[tokio::test]
async fn capability_snapshot_exposes_duplicate_table() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let adapter = connected(engine, false).await;
        let snapshot = adapter.capabilities().snapshot();
        assert!(
            snapshot
                .actions
                .contains(&serverdesk_lib::database::DatabaseAction::DuplicateTable)
        );
        assert!(snapshot.flags.table_duplicate);
    }
}
