//! MySQL adapter integration tests.
//!
//! The live tests are opt-in and target a loopback-only container fixture.
//! They are skipped when the fixture environment is absent, so the default
//! `cargo test` run never needs a database port. See
//! `docs/database-architecture.md` for the container command.

use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener},
    time::Duration,
};

use sqlx::{Connection, MySqlConnection, mysql::MySqlConnectOptions};

use serverdesk_lib::{
    database::{
        ColumnDefaultRequest, ColumnDefinitionRequest, ColumnIdentity, ColumnPositionRequest,
        ConstraintKind, ConstraintRequest, DatabaseAction, DatabaseActionRequest, DatabaseAdapter,
        DatabaseConnectRequest, DatabaseErrorKind, DatabasePassword, Identifier, MetadataScope,
        MetadataValue, MysqlDatabaseAdapter, PageRequest, ReferentialAction, RowFilter,
        RowFilterOperator, RowFilterValue, RowIdentityKind, RowPageRequest, RowQueryControl,
        RowQueryRegistry, RowReadOnlyReason, RowReadRequest, RowSearch, RowSort, RowSortDirection,
        StructureActionRequest, TableOptionsRequest, TableRef, UniqueKeyRequest,
        execute_structure_with_adapter, execute_with_adapter, preview_structure_with_adapter,
        preview_with_adapter,
    },
    storage::DatabaseEngine,
};

struct LiveMysql {
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
}

impl LiveMysql {
    fn from_env() -> Option<Self> {
        Some(Self {
            host: std::env::var("SERVERDESK_TEST_MYSQL_HOST").ok()?,
            port: std::env::var("SERVERDESK_TEST_MYSQL_PORT")
                .ok()?
                .parse()
                .ok()?,
            database: std::env::var("SERVERDESK_TEST_MYSQL_DATABASE").ok()?,
            username: std::env::var("SERVERDESK_TEST_MYSQL_USER").ok()?,
            password: std::env::var("SERVERDESK_TEST_MYSQL_PASSWORD").ok()?,
        })
    }

    fn request(&self, read_only: bool) -> DatabaseConnectRequest {
        DatabaseConnectRequest {
            host: self.host.clone(),
            port: self.port,
            socket: None,
            database: Some(self.database.clone()),
            username: self.username.clone(),
            password: DatabasePassword::new(self.password.clone()),
            read_only,
            ssl_mode: Some("preferred".to_string()),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

fn closed_loopback_port() -> u16 {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .expect("temporary loopback listener");
    let port = listener.local_addr().expect("listener address").port();
    drop(listener);
    port
}

#[tokio::test]
async fn live_mysql_connects_pings_reports_version_and_closes() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL integration test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    assert!(!adapter.is_connected());

    let info = adapter
        .connect(fixture.request(true))
        .await
        .expect("connect to fixture");
    assert_eq!(info.engine, DatabaseEngine::Mysql);
    assert!(info.read_only);
    assert!(!info.version.raw.is_empty());
    assert!(info.capabilities.version.is_some());
    assert!(adapter.is_connected());
    assert_eq!(adapter.engine(), DatabaseEngine::Mysql);
    assert_eq!(adapter.dialect().engine(), DatabaseEngine::Mysql);
    assert!(
        adapter
            .capabilities()
            .supports(DatabaseAction::BrowseDatabases)
    );

    let ping = adapter.ping().await.expect("ping fixture");
    assert!(ping.latency_ms < 10_000);

    let version = adapter.version().await.expect("version fixture");
    assert_eq!(version, info.version);

    let databases = adapter
        .list_databases(PageRequest::new(50, 0))
        .await
        .expect("list databases");
    assert!(
        databases
            .items
            .iter()
            .any(|database| database.name == fixture.database)
    );
    let database = databases
        .items
        .iter()
        .find(|database| database.name == fixture.database)
        .expect("fixture database");
    assert!(database.metadata.encoding.is_available());
    assert!(database.metadata.collation.is_available());
    assert!(!database.metadata.owner.is_available());

    let scope = MetadataScope::new(Some(&fixture.database), None).expect("scope");
    let views = adapter
        .list_views(&scope, PageRequest::new(50, 0))
        .await
        .expect("list views");
    assert!(views.limit > 0);
    let counts = adapter.object_counts(&scope).await.expect("object counts");
    assert!(counts.tables.is_available());
    assert!(counts.views.is_available());
    assert_eq!(counts.sequences, MetadataValue::Unavailable);

    adapter.close().await.expect("close fixture");
    assert!(!adapter.is_connected());
    assert_eq!(
        adapter.ping().await.expect_err("closed adapter").kind,
        DatabaseErrorKind::Connection
    );
}

#[tokio::test]
async fn live_mysql_wrong_credentials_are_distinct_from_an_unavailable_host() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL authentication test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    let mut request = fixture.request(true);
    let secret = "serverdesk-wrong-password-token".to_string();
    request.password = DatabasePassword::new(secret.clone());

    let error = adapter
        .connect(request)
        .await
        .expect_err("wrong credentials must fail");
    let debug = format!("{error:?}");
    let serialized = serde_json::to_string(&error).expect("serialized error");

    assert_eq!(error.kind, DatabaseErrorKind::DatabaseAuth);
    assert!(!debug.contains(&secret));
    assert!(!serialized.contains(&secret));
    assert!(!adapter.is_connected());
}

#[tokio::test]
async fn unavailable_hosts_produce_a_connection_error_not_an_auth_error() {
    let mut adapter = MysqlDatabaseAdapter::new();
    let secret = "serverdesk-unavailable-password-token".to_string();
    let error = adapter
        .connect(DatabaseConnectRequest {
            host: "127.0.0.1".to_string(),
            port: closed_loopback_port(),
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            password: DatabasePassword::new(secret.clone()),
            read_only: true,
            ssl_mode: Some("disabled".to_string()),
            connect_timeout: Duration::from_secs(2),
        })
        .await
        .expect_err("closed loopback port must fail");

    assert_eq!(error.kind, DatabaseErrorKind::Connection);
    assert_ne!(error.kind, DatabaseErrorKind::DatabaseAuth);
    assert!(!format!("{error:?}").contains(&secret));
    assert!(!adapter.is_connected());
}

#[tokio::test]
async fn non_loopback_hosts_are_rejected_before_a_driver_connect() {
    let mut adapter = MysqlDatabaseAdapter::new();
    let error = adapter
        .connect(DatabaseConnectRequest {
            host: "db.example.test".to_string(),
            port: 3306,
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            password: DatabasePassword::new("fixture-password"),
            read_only: true,
            ssl_mode: Some("preferred".to_string()),
            connect_timeout: Duration::from_secs(2),
        })
        .await
        .expect_err("non-loopback target is rejected");

    assert_eq!(error.kind, DatabaseErrorKind::Validation);
    assert!(error.message.contains("loopback"));
    assert!(!adapter.is_connected());
}

#[tokio::test]
async fn live_mysql_create_set_active_and_drop_are_verified() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL database action test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    adapter
        .connect(fixture.request(false))
        .await
        .expect("connect to fixture");

    let target = "serverdesk_action_test";
    let _ = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Drop {
            name: target.to_string(),
        },
        true,
    )
    .await;

    let request = DatabaseActionRequest::Create {
        name: target.to_string(),
        encoding: Some("utf8mb4".to_string()),
        collation: Some("utf8mb4_unicode_ci".to_string()),
        owner: None,
        template: None,
    };

    let preview = preview_with_adapter(&adapter, &request)
        .await
        .expect("create preview");
    assert!(
        preview
            .ddl
            .contains("CREATE DATABASE `serverdesk_action_test`")
    );
    assert_eq!(
        adapter
            .database_summary(&Identifier::new(target).expect("target"))
            .await
            .expect("summary before create"),
        None
    );

    let created = execute_with_adapter(&mut adapter, request, false)
        .await
        .expect("create database");
    assert!(created.verified, "{}", created.verification);
    let metadata = created.metadata.expect("created metadata");
    assert_eq!(metadata.name, target);
    assert_eq!(
        metadata.metadata.encoding,
        MetadataValue::available("utf8mb4".to_string())
    );
    assert_eq!(
        metadata.metadata.collation,
        MetadataValue::available("utf8mb4_unicode_ci".to_string())
    );

    let active = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::SetActive {
            name: target.to_string(),
        },
        false,
    )
    .await
    .expect("set active database");
    assert!(active.verified, "{}", active.verification);
    assert_eq!(active.active_database.as_deref(), Some(target));
    assert_eq!(adapter.current_database(), Some(target));

    let restore_request = DatabaseActionRequest::SetActive {
        name: fixture.database.clone(),
    };
    execute_with_adapter(&mut adapter, restore_request, false)
        .await
        .expect("restore active database");
    assert_eq!(adapter.current_database(), Some(fixture.database.as_str()));

    let dropped = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Drop {
            name: target.to_string(),
        },
        true,
    )
    .await
    .expect("drop database");
    assert!(dropped.verified, "{}", dropped.verification);
    assert_eq!(
        adapter
            .database_summary(&Identifier::new(target).expect("target"))
            .await
            .expect("summary after drop"),
        None
    );

    let rename = preview_with_adapter(
        &adapter,
        &DatabaseActionRequest::Rename {
            name: fixture.database.clone(),
            new_name: "serverdesk_renamed".to_string(),
        },
    )
    .await
    .expect_err("MySQL rename is unsupported");
    assert_eq!(rename.kind, DatabaseErrorKind::Capability);
    assert!(rename.message.contains("does not support renaming"));

    adapter.close().await.expect("close fixture");
}

#[tokio::test]
async fn live_mysql_read_only_profile_rejects_create_with_a_permission_error() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL read-only action test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    adapter
        .connect(fixture.request(true))
        .await
        .expect("connect read-only to fixture");

    let error = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Create {
            name: "serverdesk_read_only_created".to_string(),
            encoding: None,
            collation: None,
            owner: None,
            template: None,
        },
        false,
    )
    .await
    .expect_err("read-only profile must reject create");
    assert_eq!(error.kind, DatabaseErrorKind::Permission);
    assert!(error.message.contains("read-only"));

    adapter.close().await.expect("close fixture");
}

#[tokio::test]
async fn live_mysql_table_lifecycle_is_verified() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL table lifecycle test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    adapter
        .connect(fixture.request(false))
        .await
        .expect("connect to fixture");

    let table = "serverdesk_table_ops";
    let copy = "serverdesk_table_ops_copy";
    let renamed = "serverdesk_table_ops_renamed";
    for leftover in [table, copy, renamed] {
        let _ = execute_structure_with_adapter(
            &mut adapter,
            StructureActionRequest::DropTable {
                database: Some(fixture.database.clone()),
                schema: None,
                name: leftover.to_string(),
            },
            true,
        )
        .await;
    }

    let mut id = ColumnDefinitionRequest {
        name: "id".to_string(),
        data_type: "bigint unsigned".to_string(),
        nullable: false,
        default: None,
        comment: None,
        generated: None,
        identity: Some(ColumnIdentity::AutoIncrement),
    };
    id.identity = Some(ColumnIdentity::AutoIncrement);
    let create = StructureActionRequest::CreateTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: table.to_string(),
        columns: vec![
            id,
            ColumnDefinitionRequest {
                name: "email".to_string(),
                data_type: "varchar(320)".to_string(),
                nullable: false,
                default: None,
                comment: Some("Login address".to_string()),
                generated: None,
                identity: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        unique_keys: vec![UniqueKeyRequest {
            name: "serverdesk_table_ops_email_key".to_string(),
            columns: vec!["email".to_string()],
        }],
        options: Some(TableOptionsRequest {
            engine: Some("InnoDB".to_string()),
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_unicode_ci".to_string()),
            tablespace: None,
            unlogged: false,
            comment: Some("ServerDesk table lifecycle fixture".to_string()),
        }),
    };

    let preview = preview_structure_with_adapter(&adapter, &create)
        .await
        .expect("create table preview");
    assert!(preview.ddl.contains("CREATE TABLE"));
    assert!(preview.ddl.contains("AUTO_INCREMENT"));
    assert!(preview.ddl.contains("UNIQUE KEY"));
    assert!(preview.ddl.contains("ENGINE=InnoDB"));

    let created = execute_structure_with_adapter(&mut adapter, create, false)
        .await
        .expect("create table");
    assert!(created.verified, "{}", created.verification);
    assert_eq!(
        created
            .metadata
            .as_ref()
            .map(|metadata| metadata.name.as_str()),
        Some(table)
    );

    let duplicate = StructureActionRequest::DuplicateTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: table.to_string(),
        new_name: copy.to_string(),
    };
    let duplicated = execute_structure_with_adapter(&mut adapter, duplicate, false)
        .await
        .expect("duplicate table");
    assert!(duplicated.verified, "{}", duplicated.verification);
    assert_eq!(duplicated.target_table.as_deref(), Some(copy));

    let rename = StructureActionRequest::RenameTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: copy.to_string(),
        new_name: renamed.to_string(),
    };
    let renamed_result = execute_structure_with_adapter(&mut adapter, rename, false)
        .await
        .expect("rename table");
    assert!(renamed_result.verified, "{}", renamed_result.verification);

    let truncate = StructureActionRequest::TruncateTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: renamed.to_string(),
    };
    let truncated = execute_structure_with_adapter(&mut adapter, truncate, true)
        .await
        .expect("truncate table");
    assert!(truncated.verified, "{}", truncated.verification);

    let drop = StructureActionRequest::DropTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: renamed.to_string(),
    };
    let dropped = execute_structure_with_adapter(&mut adapter, drop, true)
        .await
        .expect("drop table");
    assert!(dropped.verified, "{}", dropped.verification);

    let _ = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::DropTable {
            database: Some(fixture.database.clone()),
            schema: None,
            name: table.to_string(),
        },
        true,
    )
    .await;

    adapter.close().await.expect("close fixture");
}

#[tokio::test]
async fn live_mysql_column_lifecycle_is_verified() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL column lifecycle test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    adapter
        .connect(fixture.request(false))
        .await
        .expect("connect to fixture");

    let table = "serverdesk_column_ops";
    let _ = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::DropTable {
            database: Some(fixture.database.clone()),
            schema: None,
            name: table.to_string(),
        },
        true,
    )
    .await;

    let create = StructureActionRequest::CreateTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: table.to_string(),
        columns: vec![
            ColumnDefinitionRequest {
                name: "id".to_string(),
                data_type: "bigint unsigned".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: Some(ColumnIdentity::AutoIncrement),
            },
            ColumnDefinitionRequest {
                name: "email".to_string(),
                data_type: "varchar(320)".to_string(),
                nullable: false,
                default: None,
                comment: Some("Login address".to_string()),
                generated: None,
                identity: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        unique_keys: Vec::new(),
        options: None,
    };
    let created = execute_structure_with_adapter(&mut adapter, create, false)
        .await
        .expect("create column fixture table");
    assert!(created.verified, "{}", created.verification);

    let mut display = ColumnDefinitionRequest {
        name: "Display Name".to_string(),
        data_type: "varchar(120)".to_string(),
        nullable: true,
        default: Some(ColumnDefaultRequest::String {
            value: "anonymous".to_string(),
        }),
        comment: Some("Shown in lists".to_string()),
        generated: None,
        identity: None,
    };
    let add = StructureActionRequest::AddColumn {
        database: Some(fixture.database.clone()),
        schema: None,
        table: table.to_string(),
        column: display.clone(),
        position: Some(ColumnPositionRequest::After {
            column: "email".to_string(),
        }),
    };
    let preview = preview_structure_with_adapter(&adapter, &add)
        .await
        .expect("add column preview");
    assert!(preview.ddl.contains("ADD COLUMN `Display Name`"));
    assert!(preview.ddl.contains("AFTER `email`"));
    let added = execute_structure_with_adapter(&mut adapter, add, false)
        .await
        .expect("add column");
    assert!(added.verified, "{}", added.verification);

    let alter = StructureActionRequest::AlterColumn {
        database: Some(fixture.database.clone()),
        schema: None,
        table: table.to_string(),
        name: "email".to_string(),
        data_type: Some("varchar(500)".to_string()),
        nullable: Some(false),
        default: Some(ColumnDefaultRequest::String {
            value: "n/a".to_string(),
        }),
        drop_default: false,
        comment: Some("Primary address".to_string()),
        drop_comment: false,
    };
    let alter_preview = preview_structure_with_adapter(&adapter, &alter)
        .await
        .expect("alter column preview");
    assert!(alter_preview.destructive);
    assert!(
        alter_preview
            .warnings
            .iter()
            .any(|warning| warning.to_ascii_lowercase().contains("data loss"))
    );
    let altered = execute_structure_with_adapter(&mut adapter, alter, true)
        .await
        .expect("alter column");
    assert!(altered.verified, "{}", altered.verification);

    let rename = StructureActionRequest::RenameColumn {
        database: Some(fixture.database.clone()),
        schema: None,
        table: table.to_string(),
        name: "Display Name".to_string(),
        new_name: "Nickname".to_string(),
    };
    let renamed = execute_structure_with_adapter(&mut adapter, rename, false)
        .await
        .expect("rename column");
    assert!(renamed.verified, "{}", renamed.verification);

    display.name = "Nickname".to_string();
    let reorder = StructureActionRequest::ReorderColumn {
        database: Some(fixture.database.clone()),
        schema: None,
        table: table.to_string(),
        column: display,
        position: ColumnPositionRequest::First,
    };
    let reordered = execute_structure_with_adapter(&mut adapter, reorder, false)
        .await
        .expect("reorder column");
    assert!(reordered.verified, "{}", reordered.verification);

    let drop = StructureActionRequest::DropColumn {
        database: Some(fixture.database.clone()),
        schema: None,
        table: table.to_string(),
        name: "Nickname".to_string(),
    };
    let drop_preview = preview_structure_with_adapter(&adapter, &drop)
        .await
        .expect("drop column preview");
    assert!(drop_preview.destructive);
    assert!(
        drop_preview
            .warnings
            .iter()
            .any(|warning| warning.to_ascii_lowercase().contains("data loss"))
    );
    let dropped = execute_structure_with_adapter(&mut adapter, drop, true)
        .await
        .expect("drop column");
    assert!(dropped.verified, "{}", dropped.verification);

    let _ = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::DropTable {
            database: Some(fixture.database.clone()),
            schema: None,
            name: table.to_string(),
        },
        true,
    )
    .await;

    adapter.close().await.expect("close fixture");
}

#[tokio::test]
async fn live_mysql_index_and_constraint_lifecycle_is_verified() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL index/constraint test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    adapter
        .connect(fixture.request(false))
        .await
        .expect("connect to fixture");

    let parent = "serverdesk_schema_parent";
    let child = "serverdesk_schema_child";
    for leftover in [child, parent] {
        let _ = execute_structure_with_adapter(
            &mut adapter,
            StructureActionRequest::DropTable {
                database: Some(fixture.database.clone()),
                schema: None,
                name: leftover.to_string(),
            },
            true,
        )
        .await;
    }

    let create_parent = StructureActionRequest::CreateTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: parent.to_string(),
        columns: vec![
            ColumnDefinitionRequest {
                name: "id".to_string(),
                data_type: "bigint unsigned".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: Some(ColumnIdentity::AutoIncrement),
            },
            ColumnDefinitionRequest {
                name: "email".to_string(),
                data_type: "varchar(320)".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        unique_keys: Vec::new(),
        options: None,
    };
    execute_structure_with_adapter(&mut adapter, create_parent, false)
        .await
        .expect("create parent fixture table");

    let create_child = StructureActionRequest::CreateTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: child.to_string(),
        columns: vec![
            ColumnDefinitionRequest {
                name: "id".to_string(),
                data_type: "bigint unsigned".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: Some(ColumnIdentity::AutoIncrement),
            },
            ColumnDefinitionRequest {
                name: "parent_id".to_string(),
                data_type: "bigint unsigned".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            },
            ColumnDefinitionRequest {
                name: "code".to_string(),
                data_type: "varchar(64)".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        unique_keys: Vec::new(),
        options: None,
    };
    execute_structure_with_adapter(&mut adapter, create_child, false)
        .await
        .expect("create child fixture table");

    let index = "serverdesk_schema_child_code_idx";
    let renamed_index = "serverdesk_schema_child_code_lookup";
    let created = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::CreateIndex {
            database: Some(fixture.database.clone()),
            schema: None,
            table: child.to_string(),
            name: index.to_string(),
            columns: vec!["code".to_string()],
            unique: true,
            method: Some("btree".to_string()),
            predicate: None,
        },
        false,
    )
    .await
    .expect("create index");
    assert!(created.verified, "{}", created.verification);

    let renamed = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::RenameIndex {
            database: Some(fixture.database.clone()),
            schema: None,
            table: child.to_string(),
            name: index.to_string(),
            new_name: renamed_index.to_string(),
        },
        false,
    )
    .await
    .expect("rename index");
    assert!(renamed.verified, "{}", renamed.verification);

    let dropped = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::DropIndex {
            database: Some(fixture.database.clone()),
            schema: None,
            table: child.to_string(),
            name: renamed_index.to_string(),
        },
        true,
    )
    .await
    .expect("drop index");
    assert!(dropped.verified, "{}", dropped.verification);

    let unique = "serverdesk_schema_parent_email_key";
    let added_unique = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::AddConstraint {
            database: Some(fixture.database.clone()),
            schema: None,
            table: parent.to_string(),
            constraint: ConstraintRequest::Unique {
                name: unique.to_string(),
                columns: vec!["email".to_string()],
            },
        },
        false,
    )
    .await
    .expect("add unique constraint");
    assert!(added_unique.verified, "{}", added_unique.verification);
    let dropped_unique = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::DropConstraint {
            database: Some(fixture.database.clone()),
            schema: None,
            table: parent.to_string(),
            name: unique.to_string(),
            kind: ConstraintKind::Unique,
        },
        true,
    )
    .await
    .expect("drop unique constraint");
    assert!(dropped_unique.verified, "{}", dropped_unique.verification);

    let foreign_key = "serverdesk_schema_child_parent_fk";
    let added_fk = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::AddConstraint {
            database: Some(fixture.database.clone()),
            schema: None,
            table: child.to_string(),
            constraint: ConstraintRequest::ForeignKey {
                name: foreign_key.to_string(),
                columns: vec!["parent_id".to_string()],
                referenced_schema: Some(fixture.database.clone()),
                referenced_table: parent.to_string(),
                referenced_columns: vec!["id".to_string()],
                on_delete: Some(ReferentialAction::Cascade),
                on_update: Some(ReferentialAction::Restrict),
            },
        },
        false,
    )
    .await
    .expect("add foreign key");
    assert!(added_fk.verified, "{}", added_fk.verification);
    let table = TableRef::new(Some(&fixture.database), None, child).expect("table ref");
    let constraints = adapter
        .list_constraints(&table, PageRequest::new(500, 0))
        .await
        .expect("list constraints");
    let constraint = constraints
        .items
        .iter()
        .find(|constraint| constraint.name == foreign_key)
        .expect("foreign key metadata");
    assert_eq!(
        constraint
            .referenced_table
            .value()
            .and_then(|value| value.as_deref()),
        Some(parent)
    );
    assert_eq!(
        constraint.on_delete,
        MetadataValue::available(Some("CASCADE".to_string()))
    );
    assert_eq!(
        constraint.on_update,
        MetadataValue::available(Some("RESTRICT".to_string()))
    );
    let dropped_fk = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::DropConstraint {
            database: Some(fixture.database.clone()),
            schema: None,
            table: child.to_string(),
            name: foreign_key.to_string(),
            kind: ConstraintKind::ForeignKey,
        },
        true,
    )
    .await
    .expect("drop foreign key");
    assert!(dropped_fk.verified, "{}", dropped_fk.verification);

    if adapter
        .capabilities()
        .supports(DatabaseAction::CreateCheckConstraint)
    {
        let check = "serverdesk_schema_child_parent_check";
        let added_check = execute_structure_with_adapter(
            &mut adapter,
            StructureActionRequest::AddConstraint {
                database: Some(fixture.database.clone()),
                schema: None,
                table: child.to_string(),
                constraint: ConstraintRequest::Check {
                    name: check.to_string(),
                    expression: "parent_id > 0".to_string(),
                },
            },
            false,
        )
        .await
        .expect("add check constraint");
        assert!(added_check.verified, "{}", added_check.verification);
        let dropped_check = execute_structure_with_adapter(
            &mut adapter,
            StructureActionRequest::DropConstraint {
                database: Some(fixture.database.clone()),
                schema: None,
                table: child.to_string(),
                name: check.to_string(),
                kind: ConstraintKind::Check,
            },
            true,
        )
        .await
        .expect("drop check constraint");
        assert!(dropped_check.verified, "{}", dropped_check.verification);
    }

    for leftover in [child, parent] {
        let _ = execute_structure_with_adapter(
            &mut adapter,
            StructureActionRequest::DropTable {
                database: Some(fixture.database.clone()),
                schema: None,
                name: leftover.to_string(),
            },
            true,
        )
        .await;
    }

    adapter.close().await.expect("close fixture");
}

#[tokio::test]
async fn live_mysql_read_only_profile_rejects_table_changes() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL read-only table test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    adapter
        .connect(fixture.request(true))
        .await
        .expect("connect read-only to fixture");

    let error = execute_structure_with_adapter(
        &mut adapter,
        StructureActionRequest::CreateTable {
            database: Some(fixture.database.clone()),
            schema: None,
            name: "serverdesk_read_only_table".to_string(),
            columns: vec![ColumnDefinitionRequest {
                name: "id".to_string(),
                data_type: "bigint".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            }],
            primary_key: Vec::new(),
            unique_keys: Vec::new(),
            options: None,
        },
        false,
    )
    .await
    .expect_err("read-only profile must reject table changes");
    assert_eq!(error.kind, DatabaseErrorKind::Permission);
    assert!(error.message.contains("read-only"));

    adapter.close().await.expect("close fixture");
}

#[tokio::test]
async fn live_mysql_row_browser_filters_sorts_counts_cancels_and_flags_missing_identity() {
    let Some(fixture) = LiveMysql::from_env() else {
        eprintln!("skipping live MySQL row browser test: fixture environment is not set");
        return;
    };

    let mut adapter = MysqlDatabaseAdapter::new();
    adapter
        .connect(fixture.request(false))
        .await
        .expect("connect to fixture");

    let table = "serverdesk_row_ops";
    let no_identity = "serverdesk_row_ops_no_identity";
    for leftover in [table, no_identity] {
        let _ = execute_structure_with_adapter(
            &mut adapter,
            StructureActionRequest::DropTable {
                database: Some(fixture.database.clone()),
                schema: None,
                name: leftover.to_string(),
            },
            true,
        )
        .await;
    }

    let create = StructureActionRequest::CreateTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: table.to_string(),
        columns: vec![
            ColumnDefinitionRequest {
                name: "id".to_string(),
                data_type: "bigint unsigned".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: Some(ColumnIdentity::AutoIncrement),
            },
            ColumnDefinitionRequest {
                name: "name".to_string(),
                data_type: "varchar(120)".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            },
            ColumnDefinitionRequest {
                name: "score".to_string(),
                data_type: "int".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            },
            ColumnDefinitionRequest {
                name: "note".to_string(),
                data_type: "text".to_string(),
                nullable: true,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        unique_keys: Vec::new(),
        options: None,
    };
    let created = execute_structure_with_adapter(&mut adapter, create, false)
        .await
        .expect("create row fixture table");
    assert!(created.verified, "{}", created.verification);

    let options = MySqlConnectOptions::new()
        .host(&fixture.host)
        .port(fixture.port)
        .username(&fixture.username)
        .password(&fixture.password)
        .database(&fixture.database);
    let mut raw = MySqlConnection::connect_with(&options)
        .await
        .expect("raw fixture connection");
    sqlx::query(
        "INSERT INTO `serverdesk_row_ops` (`name`, `score`, `note`) VALUES \
         (?, ?, ?), (?, ?, ?), (?, ?, ?)",
    )
    .bind("ann")
    .bind(10_i32)
    .bind(Some("first"))
    .bind("bob")
    .bind(30_i32)
    .bind(None::<String>)
    .bind("anna")
    .bind(20_i32)
    .bind(Some("third"))
    .execute(&mut raw)
    .await
    .expect("insert row fixture values");
    raw.close().await.expect("close raw fixture connection");

    let mut request = RowReadRequest {
        table: TableRef::new(Some(&fixture.database), None, table).expect("table ref"),
        page: RowPageRequest::try_new(1, 0).expect("page"),
        visible_columns: vec![
            Identifier::new("name").expect("name"),
            Identifier::new("score").expect("score"),
        ],
        sort: vec![RowSort {
            column: Identifier::new("score").expect("score"),
            direction: RowSortDirection::Desc,
        }],
        filters: vec![RowFilter {
            column: Identifier::new("name").expect("name"),
            operator: RowFilterOperator::Contains,
            value: Some(RowFilterValue::Text("ANN".to_string())),
            values: Vec::new(),
        }],
        search: None,
        include_exact_count: true,
    };

    let first = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect("first live row page");
    assert_eq!(first.rows.len(), 1);
    assert!(first.page.has_more);
    assert_eq!(first.rows[0].cells[0].value.as_deref(), Some("anna"));
    assert_eq!(first.rows[0].cells[1].value.as_deref(), Some("20"));
    assert_eq!(first.rows[0].identity[0].value.as_deref(), Some("3"));
    assert_eq!(first.identity.kind, RowIdentityKind::PrimaryKey);
    assert!(first.counts.estimated_table_rows.is_available());
    assert_eq!(first.counts.exact_rows.expect("exact count").value, 2);
    assert!(first.editable);
    assert!(
        first
            .visible_columns
            .iter()
            .any(|name| name.as_str() == "name")
    );

    request.page = RowPageRequest::try_new(1, 1).expect("page");
    let second = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect("second live row page");
    assert_eq!(second.rows[0].cells[0].value.as_deref(), Some("ann"));
    assert!(!second.page.has_more);

    let mut injection = request.clone();
    injection.filters = vec![RowFilter {
        column: Identifier::new("name").expect("name"),
        operator: RowFilterOperator::Equals,
        value: Some(RowFilterValue::Text(
            "x' OR 1=1; DROP TABLE serverdesk_row_ops; --".to_string(),
        )),
        values: Vec::new(),
    }];
    injection.search = None;
    injection.include_exact_count = false;
    let result = adapter
        .read_rows(&injection, &RowQueryControl::default())
        .await
        .expect("injection attempts remain bound values");
    assert!(result.rows.is_empty());
    assert_eq!(
        adapter
            .table_summary(&TableRef::new(Some(&fixture.database), None, table).expect("table"))
            .await
            .expect("table still exists")
            .map(|summary| summary.name),
        Some(table.to_string())
    );

    let registry = RowQueryRegistry::default();
    let guard = registry.begin("live-mysql-row-read").expect("begin");
    guard.cancellation().cancel();
    let control = RowQueryControl::new(Duration::from_secs(5), Some(guard.cancellation()));
    let cancelled = adapter
        .read_rows(&request, &control)
        .await
        .expect_err("cancelled read");
    assert_eq!(cancelled.kind, DatabaseErrorKind::Cancelled);

    let create_no_identity = StructureActionRequest::CreateTable {
        database: Some(fixture.database.clone()),
        schema: None,
        name: no_identity.to_string(),
        columns: vec![
            ColumnDefinitionRequest {
                name: "code".to_string(),
                data_type: "varchar(64)".to_string(),
                nullable: false,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            },
            ColumnDefinitionRequest {
                name: "note".to_string(),
                data_type: "text".to_string(),
                nullable: true,
                default: None,
                comment: None,
                generated: None,
                identity: None,
            },
        ],
        primary_key: Vec::new(),
        unique_keys: Vec::new(),
        options: None,
    };
    execute_structure_with_adapter(&mut adapter, create_no_identity, false)
        .await
        .expect("create no-identity table");
    let no_identity_result = adapter
        .read_rows(
            &RowReadRequest {
                table: TableRef::new(Some(&fixture.database), None, no_identity).expect("table"),
                page: RowPageRequest::try_new(10, 0).expect("page"),
                visible_columns: Vec::new(),
                sort: Vec::new(),
                filters: Vec::new(),
                search: Some(RowSearch {
                    query: "anything".to_string(),
                    columns: Vec::new(),
                }),
                include_exact_count: true,
            },
            &RowQueryControl::default(),
        )
        .await
        .expect("no-identity row page");
    assert!(!no_identity_result.editable);
    assert_eq!(
        no_identity_result.read_only_reasons,
        [RowReadOnlyReason::NoStableIdentity]
    );
    assert_eq!(no_identity_result.identity.kind, RowIdentityKind::None);
    assert_eq!(
        no_identity_result.counts.exact_rows.expect("exact").value,
        0
    );

    for leftover in [table, no_identity] {
        let _ = execute_structure_with_adapter(
            &mut adapter,
            StructureActionRequest::DropTable {
                database: Some(fixture.database.clone()),
                schema: None,
                name: leftover.to_string(),
            },
            true,
        )
        .await;
    }

    adapter.close().await.expect("close fixture");
}
