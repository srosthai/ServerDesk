//! Table lifecycle execution tests.
//!
//! These run the complete preview, confirmation, dependency, execution, and
//! verification path through the deterministic fake adapter. Live MySQL and
//! PostgreSQL success paths are exercised by the opt-in fixture tests in
//! `database_mysql.rs` and `database_postgres.rs`.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        ColumnDefaultRequest, ColumnDefinitionRequest, ColumnIdentity, ColumnPositionRequest,
        DatabaseAdapter, DatabaseConnectRequest, DatabaseErrorKind, DatabasePassword,
        FakeDatabaseAdapter, StructureActionRequest, TableOptionsRequest, TableRef,
        UniqueKeyRequest, execute_structure_with_adapter, preview_structure_with_adapter,
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

fn add_column_request(
    engine: DatabaseEngine,
    table: &str,
    column: ColumnDefinitionRequest,
    position: Option<ColumnPositionRequest>,
) -> StructureActionRequest {
    StructureActionRequest::AddColumn {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        table: table.to_string(),
        column,
        position,
    }
}

fn alter_column_request(
    engine: DatabaseEngine,
    table: &str,
    name: &str,
    data_type: Option<&str>,
    nullable: Option<bool>,
    default: Option<ColumnDefaultRequest>,
    comment: Option<&str>,
) -> StructureActionRequest {
    StructureActionRequest::AlterColumn {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        table: table.to_string(),
        name: name.to_string(),
        data_type: data_type.map(str::to_string),
        nullable,
        default,
        drop_default: false,
        comment: comment.map(str::to_string),
        drop_comment: false,
    }
}

fn rename_column_request(
    engine: DatabaseEngine,
    table: &str,
    name: &str,
    new_name: &str,
) -> StructureActionRequest {
    StructureActionRequest::RenameColumn {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        table: table.to_string(),
        name: name.to_string(),
        new_name: new_name.to_string(),
    }
}

fn drop_column_request(engine: DatabaseEngine, table: &str, name: &str) -> StructureActionRequest {
    StructureActionRequest::DropColumn {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        table: table.to_string(),
        name: name.to_string(),
    }
}

fn reorder_column_request(
    engine: DatabaseEngine,
    table: &str,
    column: ColumnDefinitionRequest,
    position: ColumnPositionRequest,
) -> StructureActionRequest {
    StructureActionRequest::ReorderColumn {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        table: table.to_string(),
        column,
        position,
    }
}

fn table_ref(engine: DatabaseEngine, table: &str) -> TableRef {
    TableRef::new(
        Some("app"),
        (engine == DatabaseEngine::Postgresql).then_some("public"),
        table,
    )
    .expect("table ref")
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

    let error = execute_structure_with_adapter(
        &mut adapter,
        drop_column_request(DatabaseEngine::Postgresql, "users", "id"),
        true,
    )
    .await
    .expect_err("the id column is referenced by an incoming foreign key");
    assert_eq!(error.kind, DatabaseErrorKind::Dependency);
    assert!(error.message.contains("drop column id"));
    assert!(
        error
            .details
            .as_deref()
            .is_some_and(|details| details.contains("orders_user_fk"))
    );
}

#[tokio::test]
async fn column_lifecycle_is_verified_for_mysql_and_postgres() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, false).await;
        let table = format!("serverdesk_column_{}", engine.as_str());
        execute_structure_with_adapter(&mut adapter, create_request(engine, &table), false)
            .await
            .expect("create table");

        let mut display = column("Display Name", "varchar(120)", true);
        display.default = Some(ColumnDefaultRequest::String {
            value: "anonymous".to_string(),
        });
        display.comment = Some("Shown in lists".to_string());
        let position = (engine == DatabaseEngine::Mysql).then(|| ColumnPositionRequest::After {
            column: "email".to_string(),
        });
        let added = execute_structure_with_adapter(
            &mut adapter,
            add_column_request(engine, &table, display.clone(), position),
            false,
        )
        .await
        .expect("add column");
        assert!(added.verified, "{}", added.verification);
        assert!(added.verification.contains("Display Name"));
        assert!(
            added
                .statements
                .iter()
                .any(|statement| statement.contains("ADD COLUMN"))
        );

        let altered = execute_structure_with_adapter(
            &mut adapter,
            alter_column_request(
                engine,
                &table,
                "email",
                Some("varchar(500)"),
                Some(false),
                Some(ColumnDefaultRequest::String {
                    value: "n/a".to_string(),
                }),
                Some("Primary address"),
            ),
            true,
        )
        .await
        .expect("alter column");
        assert!(altered.verified, "{}", altered.verification);
        assert!(altered.statements.iter().any(|statement| {
            statement.contains("varchar(500)") || statement.contains("character varying(500)")
        }));

        if engine == DatabaseEngine::Mysql {
            let reordered = execute_structure_with_adapter(
                &mut adapter,
                reorder_column_request(
                    engine,
                    &table,
                    display.clone(),
                    ColumnPositionRequest::First,
                ),
                false,
            )
            .await
            .expect("reorder column");
            assert!(reordered.verified, "{}", reordered.verification);
            assert!(reordered.verification.contains("ordinal position 1"));
        }

        let renamed = execute_structure_with_adapter(
            &mut adapter,
            rename_column_request(engine, &table, "Display Name", "Nickname"),
            false,
        )
        .await
        .expect("rename column");
        assert!(renamed.verified, "{}", renamed.verification);
        assert!(renamed.verification.contains("renamed to Nickname"));

        let dropped = execute_structure_with_adapter(
            &mut adapter,
            drop_column_request(engine, &table, "Nickname"),
            true,
        )
        .await
        .expect("drop column");
        assert!(dropped.verified, "{}", dropped.verification);
        assert!(dropped.verification.contains("no longer present"));

        let _ =
            execute_structure_with_adapter(&mut adapter, drop_request(engine, &table), true).await;
    }
}

#[tokio::test]
async fn postgres_supports_partial_column_edits() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;
    let table = "serverdesk_partial_pg";
    execute_structure_with_adapter(
        &mut adapter,
        create_request(DatabaseEngine::Postgresql, table),
        false,
    )
    .await
    .expect("create table");

    let comment_only = alter_column_request(
        DatabaseEngine::Postgresql,
        table,
        "email",
        None,
        None,
        None,
        Some("Updated comment"),
    );
    let result = execute_structure_with_adapter(&mut adapter, comment_only, false)
        .await
        .expect("comment-only edit");
    assert!(result.verified, "{}", result.verification);
    assert_eq!(
        result.statements,
        ["COMMENT ON COLUMN \"public\".\"serverdesk_partial_pg\".\"email\" IS 'Updated comment'"]
    );

    let set_default = alter_column_request(
        DatabaseEngine::Postgresql,
        table,
        "email",
        None,
        None,
        Some(ColumnDefaultRequest::String {
            value: "n/a".to_string(),
        }),
        None,
    );
    let result = execute_structure_with_adapter(&mut adapter, set_default, false)
        .await
        .expect("set default");
    assert!(result.verified, "{}", result.verification);

    let drop_default = StructureActionRequest::AlterColumn {
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
        table: table.to_string(),
        name: "email".to_string(),
        data_type: None,
        nullable: None,
        default: None,
        drop_default: true,
        comment: None,
        drop_comment: false,
    };
    let result = execute_structure_with_adapter(&mut adapter, drop_default, false)
        .await
        .expect("drop default");
    assert!(result.verified, "{}", result.verification);

    let _ = execute_structure_with_adapter(
        &mut adapter,
        drop_request(DatabaseEngine::Postgresql, table),
        true,
    )
    .await;
}

#[tokio::test]
async fn destructive_column_changes_warn_about_data_loss_and_dependencies() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let adapter = connected(engine, false).await;

        let drop =
            preview_structure_with_adapter(&adapter, &drop_column_request(engine, "users", "id"))
                .await
                .expect("drop column preview");
        assert!(drop.destructive);
        assert!(drop.requires_confirmation);
        assert!(
            drop.warnings
                .iter()
                .any(|warning| warning.to_ascii_lowercase().contains("data loss"))
        );
        assert!(
            drop.warnings
                .iter()
                .any(|warning| warning.contains("orders_user_fk")),
            "dependency warning missing: {:?}",
            drop.warnings
        );

        let alter = preview_structure_with_adapter(
            &adapter,
            &alter_column_request(
                engine,
                "users",
                "email",
                Some("varchar(40)"),
                Some(false),
                None,
                None,
            ),
        )
        .await
        .expect("alter column preview");
        assert!(alter.destructive);
        assert!(alter.requires_confirmation);
        assert!(
            alter
                .warnings
                .iter()
                .any(|warning| warning.to_ascii_lowercase().contains("data loss"))
        );
    }
}

#[tokio::test]
async fn unconfirmed_column_drop_is_cancelled_and_leaves_metadata_untouched() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, false).await;
        let before = adapter
            .list_columns(
                &table_ref(engine, "users"),
                serverdesk_lib::database::PageRequest::new(50, 0),
            )
            .await
            .expect("columns before");

        let error = execute_structure_with_adapter(
            &mut adapter,
            drop_column_request(engine, "users", "email"),
            false,
        )
        .await
        .expect_err("unconfirmed drop column must be cancelled");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
        assert!(error.message.contains("not confirmed"));
        assert_eq!(adapter.structure_execution_count(), 0);

        let after = adapter
            .list_columns(
                &table_ref(engine, "users"),
                serverdesk_lib::database::PageRequest::new(50, 0),
            )
            .await
            .expect("columns after");
        assert_eq!(before.items.len(), after.items.len());
    }
}

#[tokio::test]
async fn capability_rejections_happen_before_column_execution() {
    let mut mysql = FakeDatabaseAdapter::new(DatabaseEngine::Mysql, Some("5.7.44"));
    mysql
        .connect(connect_request(DatabaseEngine::Mysql, false))
        .await
        .expect("connect MySQL 5.7");
    let error = execute_structure_with_adapter(
        &mut mysql,
        rename_column_request(DatabaseEngine::Mysql, "users", "email", "email_address"),
        false,
    )
    .await
    .expect_err("MySQL 5.7 cannot rename a column");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert!(error.message.contains("rename_column"));
    assert_eq!(mysql.structure_execution_count(), 0);

    let mut postgres = connected(DatabaseEngine::Postgresql, false).await;
    let error = execute_structure_with_adapter(
        &mut postgres,
        reorder_column_request(
            DatabaseEngine::Postgresql,
            "users",
            column("email", "text", false),
            ColumnPositionRequest::First,
        ),
        false,
    )
    .await
    .expect_err("PostgreSQL cannot reorder stored columns");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert!(error.message.contains("reorder_columns"));
    assert_eq!(postgres.structure_execution_count(), 0);
}

#[tokio::test]
async fn mysql_partial_column_edits_are_rejected_with_a_capability_explanation() {
    let mut adapter = connected(DatabaseEngine::Mysql, false).await;
    let error = execute_structure_with_adapter(
        &mut adapter,
        alter_column_request(
            DatabaseEngine::Mysql,
            "users",
            "email",
            None,
            Some(false),
            None,
            Some("Only a comment"),
        ),
        false,
    )
    .await
    .expect_err("MySQL MODIFY needs the full definition");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert!(error.message.contains("full column type"));
    assert_eq!(adapter.structure_execution_count(), 0);
}

#[tokio::test]
async fn read_only_profiles_reject_column_changes_with_permission_errors() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, true).await;
        let error = execute_structure_with_adapter(
            &mut adapter,
            add_column_request(
                engine,
                "users",
                column("nickname", "varchar(80)", true),
                None,
            ),
            false,
        )
        .await
        .expect_err("read-only profile must reject column changes");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert!(error.message.contains("read-only"));
        assert_eq!(adapter.structure_execution_count(), 0);
    }
}

#[tokio::test]
async fn verification_detects_a_column_change_that_did_not_apply() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, false).await.without_structure_execution();
        let result = execute_structure_with_adapter(
            &mut adapter,
            add_column_request(
                engine,
                "users",
                column("unapplied_column", "varchar(80)", true),
                None,
            ),
            false,
        )
        .await
        .expect("execution itself reports success");
        assert!(!result.verified);
        assert!(
            result.verification.contains("does not list column"),
            "{}",
            result.verification
        );
        assert!(result.verification_error.is_some());
    }
}

#[tokio::test]
async fn quoted_column_names_round_trip_through_execution() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, false).await;
        let table = format!("serverdesk_quoted_{}", engine.as_str());
        execute_structure_with_adapter(&mut adapter, create_request(engine, &table), false)
            .await
            .expect("create table");

        let name = match engine {
            DatabaseEngine::Mysql => "Quoted `Name`",
            DatabaseEngine::Postgresql => "Quoted \"Name\"",
        };
        let mut quoted = column(name, "varchar(40)", true);
        quoted.comment = Some("Quoted identifier".to_string());
        let added = execute_structure_with_adapter(
            &mut adapter,
            add_column_request(engine, &table, quoted.clone(), None),
            false,
        )
        .await
        .expect("add quoted column");
        assert!(added.verified, "{}", added.verification);
        assert_eq!(added.object.name.as_deref(), Some(name));

        let renamed_name = match engine {
            DatabaseEngine::Mysql => "Renamed `Name`",
            DatabaseEngine::Postgresql => "Renamed \"Name\"",
        };
        let renamed = execute_structure_with_adapter(
            &mut adapter,
            rename_column_request(engine, &table, name, renamed_name),
            false,
        )
        .await
        .expect("rename quoted column");
        assert!(renamed.verified, "{}", renamed.verification);

        let dropped = execute_structure_with_adapter(
            &mut adapter,
            drop_column_request(engine, &table, renamed_name),
            true,
        )
        .await
        .expect("drop quoted column");
        assert!(dropped.verified, "{}", dropped.verification);

        let _ =
            execute_structure_with_adapter(&mut adapter, drop_request(engine, &table), true).await;
    }
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
