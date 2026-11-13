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

use serverdesk_lib::{
    database::{
        ColumnDefaultRequest, ColumnDefinitionRequest, ColumnIdentity, ColumnPositionRequest,
        DatabaseAction, DatabaseActionRequest, DatabaseAdapter, DatabaseConnectRequest,
        DatabaseErrorKind, DatabasePassword, Identifier, MetadataScope, MetadataValue,
        MysqlDatabaseAdapter, PageRequest, StructureActionRequest, TableOptionsRequest,
        UniqueKeyRequest, execute_structure_with_adapter, execute_with_adapter,
        preview_structure_with_adapter, preview_with_adapter,
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
