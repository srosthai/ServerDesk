//! Database lifecycle action tests.
//!
//! These use the deterministic fake adapter so cancellation, permission,
//! capability, preview, and verification behavior run in every CI job. The
//! live MySQL and PostgreSQL fixture tests exercise the same adapter methods
//! against a container when their environment variables are present.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        DatabaseActionRequest, DatabaseAdapter, DatabaseConnectRequest, DatabaseErrorKind,
        DatabasePassword, FakeDatabaseAdapter, execute_with_adapter, preview_with_adapter,
    },
    storage::DatabaseEngine,
};

fn request() -> DatabaseConnectRequest {
    DatabaseConnectRequest {
        host: "127.0.0.1".to_string(),
        port: 5432,
        socket: None,
        database: Some("app".to_string()),
        username: "app".to_string(),
        password: DatabasePassword::new("fixture-password"),
        read_only: false,
        ssl_mode: Some("preferred".to_string()),
        connect_timeout: Duration::from_secs(5),
    }
}

fn create(name: &str) -> DatabaseActionRequest {
    DatabaseActionRequest::Create {
        name: name.to_string(),
        encoding: Some("UTF8".to_string()),
        collation: Some("C.UTF-8".to_string()),
        owner: Some("app".to_string()),
        template: Some("template0".to_string()),
    }
}

async fn connected_fake(engine: DatabaseEngine, read_only: bool) -> FakeDatabaseAdapter {
    let mut adapter = FakeDatabaseAdapter::new(engine, Some("16.2"));
    let mut request = request();
    request.read_only = read_only;
    request.port = if engine == DatabaseEngine::Mysql {
        3306
    } else {
        5432
    };
    adapter.connect(request).await.expect("fake connect");
    adapter
}

#[tokio::test]
async fn postgresql_create_set_active_rename_and_drop_are_verified() {
    let mut adapter = connected_fake(DatabaseEngine::Postgresql, false).await;

    let preview = preview_with_adapter(&adapter, &create("app_next"))
        .await
        .expect("create preview");
    assert!(preview.ddl.contains("CREATE DATABASE \"app_next\""));
    assert!(preview.ddl.contains("TEMPLATE = \"template0\""));
    assert_eq!(
        adapter
            .database_summary(&serverdesk_lib::database::Identifier::new("app_next").expect("name"))
            .await
            .expect("summary before create"),
        None
    );

    let created = execute_with_adapter(&mut adapter, create("app_next"), false)
        .await
        .expect("create database");
    assert!(created.verified, "{}", created.verification);
    assert_eq!(created.action.as_str(), "create");
    assert_eq!(
        created
            .metadata
            .as_ref()
            .map(|metadata| metadata.name.as_str()),
        Some("app_next")
    );

    let active = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::SetActive {
            name: "app_next".to_string(),
        },
        false,
    )
    .await
    .expect("set active");
    assert!(active.verified, "{}", active.verification);
    assert_eq!(active.active_database.as_deref(), Some("app_next"));
    assert_eq!(adapter.active_database(), Some("app_next"));

    let renamed = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Rename {
            name: "app".to_string(),
            new_name: "app_renamed".to_string(),
        },
        false,
    )
    .await
    .expect("rename database");
    assert!(renamed.verified, "{}", renamed.verification);
    assert_eq!(renamed.previous_name.as_deref(), Some("app"));

    let dropped = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Drop {
            name: "app_renamed".to_string(),
        },
        true,
    )
    .await
    .expect("drop database");
    assert!(dropped.verified, "{}", dropped.verification);
    assert!(dropped.verification.contains("no longer present"));
}

#[tokio::test]
async fn unconfirmed_drop_is_cancelled_without_touching_metadata() {
    let mut adapter = connected_fake(DatabaseEngine::Postgresql, false).await;

    let error = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Drop {
            name: "postgres".to_string(),
        },
        false,
    )
    .await
    .expect_err("drop without confirmation must be cancelled");

    assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
    let summary = adapter
        .database_summary(&serverdesk_lib::database::Identifier::new("postgres").expect("name"))
        .await
        .expect("summary");
    assert_eq!(
        summary.map(|summary| summary.name),
        Some("postgres".to_string())
    );
}

#[tokio::test]
async fn read_only_profiles_reject_structural_actions_with_permission_errors() {
    let mut adapter = connected_fake(DatabaseEngine::Postgresql, true).await;

    let error = execute_with_adapter(&mut adapter, create("app_next"), false)
        .await
        .expect_err("read-only profile must reject create");
    assert_eq!(error.kind, DatabaseErrorKind::Permission);
    assert!(error.message.contains("read-only"));
}

#[tokio::test]
async fn mysql_create_set_active_and_drop_use_mysql_metadata() {
    let mut adapter = connected_fake(DatabaseEngine::Mysql, false).await;

    let preview = preview_with_adapter(
        &adapter,
        &DatabaseActionRequest::Create {
            name: "app_next".to_string(),
            encoding: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_unicode_ci".to_string()),
            owner: None,
            template: None,
        },
    )
    .await
    .expect("create preview");
    assert!(preview.ddl.contains("CREATE DATABASE `app_next`"));

    let created = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Create {
            name: "app_next".to_string(),
            encoding: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_unicode_ci".to_string()),
            owner: None,
            template: None,
        },
        false,
    )
    .await
    .expect("create database");
    assert!(created.verified, "{}", created.verification);

    let active = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::SetActive {
            name: "app_next".to_string(),
        },
        false,
    )
    .await
    .expect("set active");
    assert_eq!(active.active_database.as_deref(), Some("app_next"));

    execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::SetActive {
            name: "app".to_string(),
        },
        false,
    )
    .await
    .expect("restore active database");

    let dropped = execute_with_adapter(
        &mut adapter,
        DatabaseActionRequest::Drop {
            name: "app_next".to_string(),
        },
        true,
    )
    .await
    .expect("drop database");
    assert!(dropped.verified, "{}", dropped.verification);
}

#[tokio::test]
async fn mysql_rename_is_a_capability_error_with_an_explanation() {
    let adapter = connected_fake(DatabaseEngine::Mysql, false).await;

    let error = preview_with_adapter(
        &adapter,
        &DatabaseActionRequest::Rename {
            name: "app".to_string(),
            new_name: "app_next".to_string(),
        },
    )
    .await
    .expect_err("MySQL rename must be unsupported");

    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert!(error.message.contains("cannot") || error.message.contains("does not support"));
}
