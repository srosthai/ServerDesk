use std::time::Duration;

use serverdesk_lib::{
    database::{
        DatabaseAction, DatabaseAdapter, DatabaseConnectRequest, DatabaseErrorKind,
        DatabasePassword, FakeDatabaseAdapter, MetadataScope, PageRequest, TableRef,
    },
    storage::DatabaseEngine,
};

fn connect_request(database: &str) -> DatabaseConnectRequest {
    DatabaseConnectRequest {
        host: "127.0.0.1".to_string(),
        port: 5432,
        socket: None,
        database: Some(database.to_string()),
        username: "app".to_string(),
        password: DatabasePassword::new("fixture-password"),
        read_only: true,
        ssl_mode: Some("require".to_string()),
        connect_timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn fake_adapter_proves_the_shared_interface_without_a_database() {
    let mut adapter = FakeDatabaseAdapter::new(DatabaseEngine::Postgresql, Some("16.2"));

    assert_eq!(adapter.engine(), DatabaseEngine::Postgresql);
    assert_eq!(adapter.dialect().engine(), DatabaseEngine::Postgresql);
    assert!(!adapter.is_connected());

    let error = adapter.ping().await.expect_err("disconnected adapter");
    assert_eq!(error.kind, DatabaseErrorKind::Connection);

    let connection = adapter
        .connect(connect_request("app"))
        .await
        .expect("fake connect");
    assert_eq!(connection.engine, DatabaseEngine::Postgresql);
    assert_eq!(connection.version.raw, "16.2");
    assert!(connection.read_only);
    assert!(adapter.is_connected());
    assert_eq!(adapter.connect_count(), 1);

    let ping = adapter.ping().await.expect("fake ping");
    assert_eq!(ping.latency_ms, 1);
    assert_eq!(adapter.ping_count(), 1);

    let version = adapter.version().await.expect("fake version");
    assert_eq!(version.version.to_string(), "16.2.0");

    let databases = adapter
        .list_databases(PageRequest::new(10, 0))
        .await
        .expect("databases");
    assert_eq!(databases.items.len(), 2);
    assert_eq!(databases.items[0].name, "app");
    assert_eq!(databases.total, Some(2));

    let schemas = adapter
        .list_schemas(PageRequest::new(10, 0))
        .await
        .expect("schemas");
    assert_eq!(schemas.items[0].name, "public");

    let scope = MetadataScope::new(Some("app"), Some("public")).expect("scope");
    let tables = adapter
        .list_tables(&scope, PageRequest::new(1, 0))
        .await
        .expect("tables");
    assert_eq!(tables.items.len(), 1);
    assert_eq!(tables.items[0].name, "users");
    assert!(tables.has_more);

    let next_tables = adapter
        .list_tables(&scope, PageRequest::new(1, 1))
        .await
        .expect("next page");
    assert_eq!(next_tables.items[0].name, "orders");
    assert!(!next_tables.has_more);

    let table = TableRef::new(Some("app"), Some("public"), "users").expect("table ref");
    let columns = adapter
        .list_columns(&table, PageRequest::new(10, 0))
        .await
        .expect("columns");
    assert_eq!(columns.items.len(), 2);
    assert!(columns.items[0].primary_key);

    let indexes = adapter
        .list_indexes(&table, PageRequest::new(10, 0))
        .await
        .expect("indexes");
    assert_eq!(indexes.items[0].name, "users_pkey");

    let constraints = adapter
        .list_constraints(&table, PageRequest::new(10, 0))
        .await
        .expect("constraints");
    assert_eq!(constraints.items[0].name, "users_pkey");

    adapter.close().await.expect("fake close");
    assert!(!adapter.is_connected());
    assert_eq!(adapter.close_count(), 1);
}

#[tokio::test]
async fn shared_interface_is_object_safe_for_commands() {
    let mut adapter: Box<dyn DatabaseAdapter> = Box::new(FakeDatabaseAdapter::new(
        DatabaseEngine::Mysql,
        Some("8.0.36"),
    ));

    adapter
        .connect(connect_request("app"))
        .await
        .expect("connect through trait object");
    adapter.ping().await.expect("ping through trait object");
    let databases = adapter
        .list_databases(PageRequest::default())
        .await
        .expect("metadata through trait object");
    assert!(!databases.items.is_empty());
    adapter.close().await.expect("close through trait object");
}

#[tokio::test]
async fn capability_checks_reject_unsupported_actions() {
    let adapter = FakeDatabaseAdapter::new(DatabaseEngine::Mysql, Some("5.7.44"));
    let error = adapter
        .require_capability(DatabaseAction::CreateCheckConstraint)
        .expect_err("MySQL 5.7 does not enforce CHECK constraints");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert!(error.message.contains("create_check_constraint"));

    let old_postgres = FakeDatabaseAdapter::new(DatabaseEngine::Postgresql, Some("9.2"));
    let error = old_postgres
        .require_capability(DatabaseAction::RefreshMaterializedView)
        .expect_err("PostgreSQL 9.2 predates materialized views");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);

    let current_postgres = FakeDatabaseAdapter::new(DatabaseEngine::Postgresql, Some("16.2"));
    current_postgres
        .require_capability(DatabaseAction::RefreshMaterializedView)
        .expect("PostgreSQL 16 supports materialized views");
}

#[tokio::test]
async fn typed_connect_failures_use_the_shared_taxonomy() {
    let mut adapter = FakeDatabaseAdapter::new(DatabaseEngine::Mysql, Some("8.0.36"))
        .with_connect_error(serverdesk_lib::database::DatabaseError::database_auth(
            "Access denied for user 'app'",
        ));

    let error = adapter
        .connect(connect_request("app"))
        .await
        .expect_err("fake authentication failure");
    assert_eq!(error.kind, DatabaseErrorKind::DatabaseAuth);
    assert_eq!(adapter.connect_count(), 1);
    assert!(!adapter.is_connected());
}

#[test]
fn capability_snapshots_never_contain_engine_sql() {
    let adapter = FakeDatabaseAdapter::new(DatabaseEngine::Mysql, Some("8.0.36"));
    let snapshot = adapter.capabilities().snapshot();
    let json = serde_json::to_string(&snapshot).expect("capability json");

    assert!(json.contains("browse_tables"));
    assert!(json.contains("\"engine\":\"mysql\""));
    assert!(!json.contains("SELECT"));
    assert!(!json.contains("information_schema"));
    assert!(!json.contains("`"));
}
