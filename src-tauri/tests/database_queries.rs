//! Deterministic SQL execution tests for both engines.
//!
//! The fake adapter exercises the shared safety gates, streaming result
//! collector, limits, read-only rejection, destructive confirmation, timeout,
//! cancellation, and structured errors without a live database. Live engine
//! tests in `database_mysql.rs` and `database_postgres.rs` cover the real SQLx
//! paths when their fixture environment is configured.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        DatabaseAdapter, DatabaseConnectRequest, DatabaseError, DatabaseErrorKind,
        DatabasePassword, FakeDatabaseAdapter, QueryControl, QueryRegistry, QueryRequest,
        SqlWarningCode, execute_query_with_adapter,
    },
    storage::DatabaseEngine,
};

const DATABASE: &str = "app";

fn connect_request(read_only: bool) -> DatabaseConnectRequest {
    DatabaseConnectRequest {
        host: "127.0.0.1".to_string(),
        port: 5432,
        socket: None,
        database: Some(DATABASE.to_string()),
        username: "app".to_string(),
        password: DatabasePassword::new("fixture-password"),
        read_only,
        ssl_mode: Some("preferred".to_string()),
        connect_timeout: Duration::from_secs(5),
    }
}

async fn adapter(engine: DatabaseEngine, read_only: bool) -> FakeDatabaseAdapter {
    let mut adapter = FakeDatabaseAdapter::new(engine, Some("16.2"));
    adapter
        .connect(connect_request(read_only))
        .await
        .expect("fake connect");
    adapter
}

fn query_control(request: &QueryRequest) -> QueryControl {
    QueryControl::new(Duration::from_secs(10), None, request.max_rows)
}

#[tokio::test]
async fn both_engines_execute_selects_multiple_result_sets_and_writes() {
    for engine in DatabaseEngine::ALL {
        let adapter = adapter(engine, false).await;

        let request = QueryRequest::try_new("SELECT 1; SELECT 2", None).expect("request");
        let result =
            execute_query_with_adapter(&adapter, &request, false, &query_control(&request))
                .await
                .expect("multi statement select");
        assert_eq!(result.engine, engine);
        assert!(!result.read_only);
        assert_eq!(result.results.len(), 2);
        assert_eq!(result.results[0].rows[0][0].value.as_deref(), Some("1"));
        assert_eq!(result.results[1].rows[0][0].value.as_deref(), Some("2"));
        assert!(!result.truncated);

        let request =
            QueryRequest::try_new("INSERT INTO users (name) VALUES ('ann') RETURNING id", None)
                .expect("write request");
        let result =
            execute_query_with_adapter(&adapter, &request, false, &query_control(&request))
                .await
                .expect("write statement");
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].affected_rows, Some(1));
        assert_eq!(result.results[0].last_insert_id, Some(7));
    }
}

#[tokio::test]
async fn result_limits_are_enforced_and_disclosed_for_both_engines() {
    for engine in DatabaseEngine::ALL {
        let adapter = adapter(engine, false).await;
        let request = QueryRequest::try_new("SELECT * FROM many", Some(2)).expect("request");
        let mut control = query_control(&request);
        control.max_cell_characters = 2;
        let result = execute_query_with_adapter(&adapter, &request, false, &control)
            .await
            .expect("bounded select");

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].columns.len(), 1);
        assert_eq!(result.results[0].returned_rows, 2);
        assert!(result.results[0].truncated);
        assert!(result.truncated);
        assert_eq!(result.max_rows, 2);
    }
}

#[tokio::test]
async fn read_only_mode_rejects_writes_before_any_execution() {
    for engine in DatabaseEngine::ALL {
        let adapter = adapter(engine, true).await;
        let request = QueryRequest::try_new("UPDATE users SET active = 0", None).expect("request");
        let error = execute_query_with_adapter(&adapter, &request, true, &query_control(&request))
            .await
            .expect_err("read-only write");

        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert!(error.message.contains("read-only"));
        assert_eq!(adapter.query_execution_count(), 0);

        let request = QueryRequest::try_new("SELECT 1", None).expect("read request");
        execute_query_with_adapter(&adapter, &request, false, &query_control(&request))
            .await
            .expect("read-only select");
        assert_eq!(adapter.query_execution_count(), 1);
    }
}

#[tokio::test]
async fn destructive_sql_requires_confirmation_and_returns_warnings() {
    for engine in DatabaseEngine::ALL {
        let adapter = adapter(engine, false).await;
        let request =
            QueryRequest::try_new("DROP TABLE users; DELETE FROM users;", None).expect("request");

        let error = execute_query_with_adapter(&adapter, &request, false, &query_control(&request))
            .await
            .expect_err("unconfirmed destructive SQL");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
        assert!(
            error
                .details
                .as_deref()
                .is_some_and(|details| details.contains("No statement was executed"))
        );
        assert_eq!(adapter.query_execution_count(), 0);

        let result = execute_query_with_adapter(&adapter, &request, true, &query_control(&request))
            .await
            .expect("confirmed destructive SQL");
        assert!(result.destructive);
        assert!(result.requires_confirmation);
        assert_eq!(result.warnings.len(), 2);
        assert_eq!(result.warnings[0].code, SqlWarningCode::Drop);
        assert_eq!(result.warnings[1].code, SqlWarningCode::DeleteWithoutWhere);
        assert_eq!(adapter.query_execution_count(), 1);
    }
}

#[tokio::test]
async fn timeout_and_cancellation_are_distinct_outcomes_for_both_engines() {
    for engine in DatabaseEngine::ALL {
        let adapter = adapter(engine, false).await;
        let request = QueryRequest::try_new("SELECT 1", None).expect("request");

        let registry = QueryRegistry::default();
        let guard = registry.begin("query-cancel-1").expect("begin");
        guard.cancellation().cancel();
        let control = QueryControl::new(
            Duration::from_secs(10),
            Some(guard.cancellation()),
            request.max_rows,
        );
        let error = execute_query_with_adapter(&adapter, &request, false, &control)
            .await
            .expect_err("cancelled query");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);

        let mut adapter = FakeDatabaseAdapter::new(engine, Some("16.2"))
            .with_query_error(DatabaseError::timeout("fixture timeout"));
        adapter
            .connect(connect_request(false))
            .await
            .expect("connect timeout fixture");
        let error = execute_query_with_adapter(&adapter, &request, false, &query_control(&request))
            .await
            .expect_err("timed out query");
        assert_eq!(error.kind, DatabaseErrorKind::Timeout);
    }
}

#[tokio::test]
async fn structured_errors_keep_code_position_and_duration_without_secrets() {
    let error = DatabaseError::query("The SQL statement was rejected.")
        .with_database_code("42601")
        .with_position(8)
        .with_duration_ms(12)
        .with_details("DB_PASSWORD=hunter2");
    let adapter =
        FakeDatabaseAdapter::new(DatabaseEngine::Postgresql, Some("16.2")).with_query_error(error);
    let mut adapter = adapter;
    adapter
        .connect(connect_request(false))
        .await
        .expect("connect error fixture");
    let request = QueryRequest::try_new("SELEC 1", None).expect("request");

    let error = execute_query_with_adapter(&adapter, &request, false, &query_control(&request))
        .await
        .expect_err("rejected query");
    assert_eq!(error.kind, DatabaseErrorKind::Query);
    assert_eq!(error.database_code(), Some("42601"));
    assert_eq!(error.position(), Some(8));
    assert_eq!(error.duration_ms(), Some(12));
    let serialized = serde_json::to_string(&error).expect("serialized error");
    assert!(!serialized.contains("hunter2"));
    assert!(serialized.contains("databaseCode"));
    assert!(serialized.contains("position"));
    assert!(serialized.contains("durationMs"));
}

#[tokio::test]
async fn empty_or_oversized_sql_is_rejected_before_execution() {
    let adapter = adapter(DatabaseEngine::Postgresql, false).await;
    assert!(QueryRequest::try_new("   ", None).is_err());
    assert!(QueryRequest::try_new("SELECT 1", Some(0)).is_err());
    assert_eq!(adapter.query_execution_count(), 0);
}
