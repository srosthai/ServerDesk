//! Session and running-query monitoring tests.
//!
//! The fake adapter exercises the same validation, confirmation, permission,
//! execution, verification, redaction, and bounded-pagination path as the live
//! adapters without a database container.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        DatabaseAdapter, DatabaseConnectRequest, DatabaseError, DatabaseErrorKind,
        DatabasePassword, FakeDatabaseAdapter, PageRequest, SessionActionKind,
        SessionActionRequest, SessionSummary, execute_session_action_with_adapter,
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

fn request(action: SessionActionKind, id: i64) -> SessionActionRequest {
    SessionActionRequest {
        session_id: id,
        action,
        expected_user: "app".to_string(),
        expected_database: Some("app".to_string()),
        confirmation: action.confirmation_phrase(id),
    }
}

#[tokio::test]
async fn mysql_sessions_match_fixture_state_and_bounded_pages() {
    let adapter = connected(DatabaseEngine::Mysql, false).await;
    let page = adapter
        .list_sessions(PageRequest::clamped(Some(2), Some(0)), false)
        .await
        .expect("sessions");

    assert_eq!(page.limit, 2);
    assert_eq!(page.items.len(), 2);
    assert!(page.has_more);
    assert_eq!(page.items[0].id, 101);
    assert_eq!(page.items[0].user, "app");
    assert_eq!(page.items[0].database_label(), Some("app"));
    assert_eq!(
        page.items[0].command.value().map(String::as_str),
        Some("Query")
    );
    assert!(page.items[0].is_running());
    assert_eq!(page.items[0].query_elapsed_ms(), Some(2_500));
}

#[tokio::test]
async fn postgres_sessions_expose_running_queries_and_elapsed_time() {
    let adapter = connected(DatabaseEngine::Postgresql, false).await;
    let page = adapter
        .list_sessions(PageRequest::clamped(Some(100), Some(0)), false)
        .await
        .expect("sessions");

    let running = page
        .items
        .iter()
        .find(|session| session.id == 501)
        .expect("running session");
    assert_eq!(running.user, "app");
    assert_eq!(running.state.value().map(String::as_str), Some("active"));
    assert_eq!(running.query_elapsed_ms(), Some(30_000));
    let query = running.query.value().expect("query text");
    assert_eq!(query.text, "SELECT pg_sleep(30)");
    assert_eq!(
        query.started_at.value().map(String::as_str),
        Some("2026-11-17T08:05:00Z")
    );
    assert!(!query.redacted);

    let idle = page
        .items
        .iter()
        .find(|session| session.id == 502)
        .expect("idle session");
    assert!(!idle.is_running());
    assert_eq!(idle.state.value().map(String::as_str), Some("idle"));
}

#[tokio::test]
async fn running_only_excludes_idle_sessions() {
    let adapter = connected(DatabaseEngine::Postgresql, false).await;
    let page = adapter
        .list_sessions(PageRequest::clamped(Some(100), Some(0)), true)
        .await
        .expect("sessions");

    assert!(!page.items.is_empty());
    assert!(page.items.iter().all(SessionSummary::is_running));
    assert!(!page.items.iter().any(|session| session.id == 502));
}

#[tokio::test]
async fn unsupported_fields_are_explicitly_unavailable() {
    let mysql = connected(DatabaseEngine::Mysql, false).await;
    let mysql_page = mysql
        .list_sessions(PageRequest::clamped(Some(100), Some(0)), false)
        .await
        .expect("mysql sessions");
    let mysql_session = mysql_page
        .items
        .iter()
        .find(|session| session.id == 101)
        .expect("mysql session");
    assert!(!mysql_session.application_name.is_available());
    assert!(!mysql_session.wait_event.is_available());
    assert!(!mysql_session.backend_type.is_available());
    assert!(!mysql_session.connected_at.is_available());

    let postgres = connected(DatabaseEngine::Postgresql, false).await;
    let postgres_page = postgres
        .list_sessions(PageRequest::clamped(Some(100), Some(0)), false)
        .await
        .expect("postgres sessions");
    let postgres_session = postgres_page
        .items
        .iter()
        .find(|session| session.id == 501)
        .expect("postgres session");
    assert!(!postgres_session.command.is_available());
    assert!(postgres_session.application_name.is_available());
    assert!(postgres_session.connected_at.is_available());
}

#[tokio::test]
async fn query_text_with_a_secret_is_redacted_before_serialization() {
    for (engine, session_id, secret) in [
        (DatabaseEngine::Mysql, 104, "fixture-session-secret"),
        (
            DatabaseEngine::Postgresql,
            504,
            "fixture-role-session-secret",
        ),
    ] {
        let adapter = connected(engine, false).await;
        let page = adapter
            .list_sessions(PageRequest::clamped(Some(100), Some(0)), false)
            .await
            .expect("sessions");
        let session = page
            .items
            .iter()
            .find(|session| session.id == session_id)
            .expect("secret fixture");
        let query = session.query.value().expect("query text");
        let json = serde_json::to_string(session).expect("session json");

        assert!(query.redacted);
        assert!(!query.text.contains(secret));
        assert!(!json.contains(secret));
        assert!(json.contains("<redacted>"));
    }
}

#[tokio::test]
async fn permission_denial_from_the_server_is_typed() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;
    adapter = adapter.with_session_execution_error(DatabaseError::permission(
        "The database account does not have permission for this operation.",
    ));

    let error = execute_session_action_with_adapter(
        &mut adapter,
        request(SessionActionKind::TerminateSession, 501),
    )
    .await
    .expect_err("permission denied");

    assert_eq!(error.kind, DatabaseErrorKind::Permission);
    assert_eq!(adapter.session_execution_count(), 1);
}

#[tokio::test]
async fn read_only_profiles_deny_session_actions_before_execution() {
    let mut adapter = connected(DatabaseEngine::Mysql, true).await;
    let error = execute_session_action_with_adapter(
        &mut adapter,
        request(SessionActionKind::TerminateSession, 101),
    )
    .await
    .expect_err("read-only profile");

    assert_eq!(error.kind, DatabaseErrorKind::Permission);
    assert_eq!(adapter.session_execution_count(), 0);
}

#[tokio::test]
async fn a_mismatched_confirmation_does_not_execute() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;
    let mut request = request(SessionActionKind::TerminateSession, 501);
    request.confirmation = "terminate session 999".to_string();

    let error = execute_session_action_with_adapter(&mut adapter, request)
        .await
        .expect_err("confirmation mismatch");

    assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
    assert_eq!(adapter.session_execution_count(), 0);
    assert!(
        adapter
            .session_summary(501)
            .await
            .expect("summary")
            .is_some()
    );
}

#[tokio::test]
async fn termination_is_reported_only_after_verification() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;
    let result = execute_session_action_with_adapter(
        &mut adapter,
        request(SessionActionKind::TerminateSession, 501),
    )
    .await
    .expect("termination");

    assert!(result.request_accepted);
    assert!(result.verified);
    assert!(result.verification.contains("Termination verified"));
    assert!(
        adapter
            .session_summary(501)
            .await
            .expect("summary")
            .is_none()
    );
}

#[tokio::test]
async fn an_unverified_termination_is_not_reported_as_success() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;
    adapter = adapter.with_skipped_session_execution();

    let result = execute_session_action_with_adapter(
        &mut adapter,
        request(SessionActionKind::TerminateSession, 501),
    )
    .await
    .expect("termination attempt");

    assert!(!result.verified);
    assert!(result.verification_error.is_some());
    assert!(result.verification.contains("still present"));
    assert!(
        adapter
            .session_summary(501)
            .await
            .expect("summary")
            .is_some()
    );
}

#[tokio::test]
async fn cancel_verification_accepts_a_session_with_no_running_query() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;
    let result = execute_session_action_with_adapter(
        &mut adapter,
        request(SessionActionKind::CancelQuery, 501),
    )
    .await
    .expect("cancel");

    assert!(result.verified);
    assert!(result.verification.contains("Cancellation verified"));
    let session = adapter
        .session_summary(501)
        .await
        .expect("summary")
        .expect("session remains");
    assert!(!session.is_running());
}

#[tokio::test]
async fn a_current_connection_cannot_be_terminated() {
    let mut adapter = connected(DatabaseEngine::Mysql, false).await;
    let request = request(SessionActionKind::TerminateSession, 103);

    let error = execute_session_action_with_adapter(&mut adapter, request)
        .await
        .expect_err("current session");

    assert_eq!(error.kind, DatabaseErrorKind::Validation);
    assert!(error.message.contains("own database connection"));
    assert_eq!(adapter.session_execution_count(), 0);
}

#[tokio::test]
async fn session_listing_errors_keep_the_typed_permission_kind() {
    let adapter = connected(DatabaseEngine::Postgresql, false)
        .await
        .with_session_list_error(DatabaseError::permission(
            "The database account does not have permission for this operation.",
        ));

    let error = adapter
        .list_sessions(PageRequest::clamped(Some(100), Some(0)), false)
        .await
        .expect_err("permission denied");

    assert_eq!(error.kind, DatabaseErrorKind::Permission);
}
