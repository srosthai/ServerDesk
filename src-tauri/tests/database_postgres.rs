//! PostgreSQL adapter integration tests.
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
        DatabaseAction, DatabaseActionRequest, DatabaseAdapter, DatabaseConnectRequest,
        DatabaseErrorKind, DatabasePassword, MetadataScope, MetadataValue, PageRequest,
        PostgresDatabaseAdapter, execute_with_adapter, preview_with_adapter,
    },
    storage::DatabaseEngine,
};

struct LivePostgres {
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
}

impl LivePostgres {
    fn from_env() -> Option<Self> {
        Some(Self {
            host: std::env::var("SERVERDESK_TEST_POSTGRES_HOST").ok()?,
            port: std::env::var("SERVERDESK_TEST_POSTGRES_PORT")
                .ok()?
                .parse()
                .ok()?,
            database: std::env::var("SERVERDESK_TEST_POSTGRES_DATABASE").ok()?,
            username: std::env::var("SERVERDESK_TEST_POSTGRES_USER").ok()?,
            password: std::env::var("SERVERDESK_TEST_POSTGRES_PASSWORD").ok()?,
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
async fn live_postgres_connects_pings_reports_version_and_closes() {
    let Some(fixture) = LivePostgres::from_env() else {
        eprintln!("skipping live PostgreSQL integration test: fixture environment is not set");
        return;
    };

    let mut adapter = PostgresDatabaseAdapter::new();
    assert!(!adapter.is_connected());

    let info = adapter
        .connect(fixture.request(true))
        .await
        .expect("connect to fixture");
    assert_eq!(info.engine, DatabaseEngine::Postgresql);
    assert!(info.read_only);
    assert_eq!(info.database.as_deref(), Some(fixture.database.as_str()));
    assert!(!info.version.raw.is_empty());
    assert!(info.capabilities.version.is_some());
    assert!(adapter.is_connected());
    assert_eq!(adapter.engine(), DatabaseEngine::Postgresql);
    assert_eq!(adapter.dialect().engine(), DatabaseEngine::Postgresql);
    assert!(
        adapter
            .capabilities()
            .supports(DatabaseAction::BrowseDatabases)
    );
    assert!(
        adapter
            .capabilities()
            .supports(DatabaseAction::BrowseSchemas)
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

    let schemas = adapter
        .list_schemas(PageRequest::new(50, 0))
        .await
        .expect("list schemas");
    assert!(schemas.items.iter().any(|schema| schema.name == "public"));
    assert!(schemas.items.iter().any(|schema| schema.system));

    let scope = MetadataScope::new(Some(&fixture.database), Some("public")).expect("scope");
    let tables = adapter
        .list_tables(&scope, PageRequest::new(50, 0))
        .await
        .expect("list tables");
    assert!(
        tables
            .items
            .iter()
            .all(|table| table.database.as_deref() == Some(fixture.database.as_str()))
    );
    assert!(
        tables
            .items
            .iter()
            .all(|table| table.schema.as_deref() == Some("public"))
    );

    let views = adapter
        .list_views(&scope, PageRequest::new(50, 0))
        .await
        .expect("list views");
    assert!(views.limit > 0);
    let sequences = adapter
        .list_sequences(&scope, PageRequest::new(50, 0))
        .await
        .expect("list sequences");
    assert!(sequences.limit > 0);
    let counts = adapter.object_counts(&scope).await.expect("object counts");
    assert!(counts.tables.is_available());
    assert!(counts.views.is_available());
    assert!(counts.sequences.is_available());
    assert!(counts.materialized_views.is_available());

    let other =
        MetadataScope::new(Some("serverdesk_other_database"), Some("public")).expect("scope");
    let error = adapter
        .list_tables(&other, PageRequest::new(50, 0))
        .await
        .expect_err("cross-database browsing is rejected");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);

    adapter.close().await.expect("close fixture");
    assert!(!adapter.is_connected());
    assert!(adapter.capabilities().version().is_none());
    assert_eq!(
        adapter.ping().await.expect_err("closed adapter").kind,
        DatabaseErrorKind::Connection
    );
}

#[tokio::test]
async fn live_postgres_wrong_credentials_are_distinct_from_an_unavailable_host() {
    let Some(fixture) = LivePostgres::from_env() else {
        eprintln!("skipping live PostgreSQL authentication test: fixture environment is not set");
        return;
    };

    let mut adapter = PostgresDatabaseAdapter::new();
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
async fn unavailable_postgres_hosts_produce_a_connection_error_not_an_auth_error() {
    let mut adapter = PostgresDatabaseAdapter::new();
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
async fn non_loopback_postgres_hosts_are_rejected_before_a_driver_connect() {
    let mut adapter = PostgresDatabaseAdapter::new();
    let error = adapter
        .connect(DatabaseConnectRequest {
            host: "db.example.test".to_string(),
            port: 5432,
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
async fn live_postgres_create_rename_set_active_and_drop_are_verified() {
    let Some(fixture) = LivePostgres::from_env() else {
        eprintln!("skipping live PostgreSQL database action test: fixture environment is not set");
        return;
    };

    let mut adapter = PostgresDatabaseAdapter::new();
    adapter
        .connect(fixture.request(false))
        .await
        .expect("connect to fixture");

    let target = "serverdesk_action_test";
    let renamed = "serverdesk_action_test_renamed";
    for leftover in [target, renamed, "serverdesk_action_test_other"] {
        let _ = execute_with_adapter(
            &mut adapter,
            DatabaseActionRequest::Drop {
                name: leftover.to_string(),
            },
            true,
        )
        .await;
    }

    let create = DatabaseActionRequest::Create {
        name: target.to_string(),
        encoding: Some("UTF8".to_string()),
        collation: Some("C".to_string()),
        owner: Some(fixture.username.clone()),
        template: Some("template0".to_string()),
    };

    let preview = preview_with_adapter(&adapter, &create)
        .await
        .expect("create preview");
    assert!(
        preview
            .ddl
            .contains("CREATE DATABASE \"serverdesk_action_test\"")
    );
    assert!(preview.ddl.contains("TEMPLATE = \"template0\""));

    let created = execute_with_adapter(&mut adapter, create, false)
        .await
        .expect("create database");
    assert!(created.verified, "{}", created.verification);
    let metadata = created.metadata.expect("created metadata");
    assert_eq!(metadata.name, target);
    assert_eq!(
        metadata.metadata.owner,
        MetadataValue::available(fixture.username.clone())
    );

    let renamed_result = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Rename {
            name: target.to_string(),
            new_name: renamed.to_string(),
        },
        false,
    )
    .await
    .expect("rename database");
    assert!(renamed_result.verified, "{}", renamed_result.verification);
    assert_eq!(renamed_result.database, target);
    assert_eq!(renamed_result.previous_name.as_deref(), Some(target));

    let active = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::SetActive {
            name: renamed.to_string(),
        },
        false,
    )
    .await
    .expect("set active database");
    assert!(active.verified, "{}", active.verification);
    assert_eq!(active.active_database.as_deref(), Some(renamed));
    assert_eq!(adapter.current_database(), Some(renamed));

    let rename_active = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Rename {
            name: renamed.to_string(),
            new_name: "serverdesk_action_test_other".to_string(),
        },
        false,
    )
    .await
    .expect_err("active catalog cannot be renamed");
    assert_eq!(rename_active.kind, DatabaseErrorKind::Validation);

    execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::SetActive {
            name: fixture.database.clone(),
        },
        false,
    )
    .await
    .expect("restore active database");

    let dropped = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Drop {
            name: renamed.to_string(),
        },
        true,
    )
    .await
    .expect("drop database");
    assert!(dropped.verified, "{}", dropped.verification);

    adapter.close().await.expect("close fixture");
}

#[tokio::test]
async fn live_postgres_read_only_profile_rejects_create_with_a_permission_error() {
    let Some(fixture) = LivePostgres::from_env() else {
        eprintln!("skipping live PostgreSQL read-only action test: fixture environment is not set");
        return;
    };

    let mut adapter = PostgresDatabaseAdapter::new();
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
