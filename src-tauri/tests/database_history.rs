//! Query-history and saved-snippet integration tests.
//!
//! These tests exercise the same service functions the Tauri commands call.
//! They verify sanitization before storage, bounded retention, connection /
//! result / date filters, restart persistence, snippet reuse and deletion, and
//! explicit clear-history confirmation.

use serverdesk_lib::{
    core::{
        database_history::{
            DatabaseQueryHistoryDraft, MAX_DATABASE_HISTORY_ERROR_CHARS,
            MAX_DATABASE_HISTORY_SQL_CHARS, MAX_DATABASE_QUERY_HISTORY_RECORDS,
            clear_query_history, create_saved_query, delete_saved_query, list_query_history,
            record_query_history, sanitize_query_history_sql, update_saved_query,
        },
        redaction::sanitize_sql_bounded,
    },
    storage::{
        AuthType, Database, DatabaseConnection, DatabaseEngine, DatabaseQueryHistoryFilter,
        DatabaseQueryResult, MAX_DATABASE_QUERY_HISTORY_LIMIT, NewDatabaseConnection,
        NewDatabaseSavedQuery, NewServer,
    },
};

fn server_input(name: &str) -> NewServer {
    NewServer {
        name: name.to_string(),
        host: "198.51.100.24".to_string(),
        port: 22,
        username: "deploy".to_string(),
        auth_type: AuthType::PrivateKey,
        credential_ref: Some("credential:test-server".to_string()),
        group_name: None,
        tags: vec![],
        notes: None,
    }
}

fn connection_input(server_id: &str, name: &str, engine: DatabaseEngine) -> NewDatabaseConnection {
    NewDatabaseConnection {
        server_id: server_id.to_string(),
        name: name.to_string(),
        engine,
        host: "127.0.0.1".to_string(),
        port: if engine == DatabaseEngine::Mysql {
            3306
        } else {
            5432
        },
        socket: None,
        database_name: Some("app".to_string()),
        username: "app".to_string(),
        credential_ref: Some("credential:database-profile".to_string()),
        read_only: true,
        ssl_mode: None,
    }
}

fn fixture() -> (Database, DatabaseConnection) {
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");
    let connection = database
        .create_database_connection(&connection_input(
            &server.id,
            "Primary",
            DatabaseEngine::Postgresql,
        ))
        .expect("create connection");
    (database, connection)
}

fn record(
    database: &Database,
    connection: &DatabaseConnection,
    sql: &str,
    result: DatabaseQueryResult,
    started_at: i64,
) {
    record_query_history(
        database,
        &DatabaseQueryHistoryDraft {
            profile: connection,
            sql,
            result,
            duration_ms: 12,
            row_count: (result == DatabaseQueryResult::Success).then_some(3),
            error_summary: (result == DatabaseQueryResult::Failure)
                .then_some("syntax error at or near \"form\""),
            started_at,
        },
    )
    .expect("record query history");
}

#[test]
fn history_redacts_sql_and_error_summaries_before_storage() {
    let (database, connection) = fixture();
    let sql = "CREATE USER 'app'@'%' IDENTIFIED BY 'fixture-password';\n\
               -- DB_PASSWORD=fixture-dotenv-password\n\
               -- Authorization: Bearer fixture-bearer-token\n\
               -- DATABASE_URL=mysql://deploy:fixture-url-password@127.0.0.1/app\n\
               SELECT 1;";
    record_query_history(
        &database,
        &DatabaseQueryHistoryDraft {
            profile: &connection,
            sql,
            result: DatabaseQueryResult::Failure,
            duration_ms: 42,
            row_count: None,
            error_summary: Some(
                "authentication failed for DB_PASSWORD=fixture-error-password with \
                 Authorization: Bearer fixture-error-token",
            ),
            started_at: 1_700_000_000,
        },
    )
    .expect("record redacted history");

    let history = list_query_history(&database, &DatabaseQueryHistoryFilter::default())
        .expect("list history");
    assert_eq!(history.len(), 1);
    let entry = &history[0];
    assert_eq!(entry.result, DatabaseQueryResult::Failure);
    assert_eq!(entry.duration_ms, 42);
    assert_eq!(entry.started_at, 1_700_000_000);
    assert_eq!(entry.engine, DatabaseEngine::Postgresql);
    assert_eq!(entry.connection_id, connection.id);

    let stored = format!(
        "{} {}",
        entry.sql_text,
        entry.error_summary.as_deref().unwrap_or("")
    );
    for secret in [
        "fixture-password",
        "fixture-dotenv-password",
        "fixture-bearer-token",
        "fixture-url-password",
        "fixture-error-password",
        "fixture-error-token",
    ] {
        assert!(
            !stored.contains(secret),
            "stored history still contains {secret:?}: {stored}"
        );
    }
    assert!(entry.sql_text.contains("IDENTIFIED BY '<redacted>'"));
    assert!(entry.sql_text.contains("DB_PASSWORD=<redacted>"));
    assert!(entry.sql_text.contains("Bearer <redacted>"));
    assert!(entry.sql_text.contains(":<redacted>"));
}

#[test]
fn oversized_history_sql_is_bounded_and_error_summary_is_short() {
    let (database, connection) = fixture();
    let sql = format!(
        "SELECT '{}'",
        "x".repeat(MAX_DATABASE_HISTORY_SQL_CHARS * 3)
    );
    let error = "y".repeat(MAX_DATABASE_HISTORY_ERROR_CHARS * 3);

    record_query_history(
        &database,
        &DatabaseQueryHistoryDraft {
            profile: &connection,
            sql: &sql,
            result: DatabaseQueryResult::Failure,
            duration_ms: 1,
            row_count: None,
            error_summary: Some(&error),
            started_at: 10,
        },
    )
    .expect("record bounded history");

    let entry = &list_query_history(&database, &DatabaseQueryHistoryFilter::default())
        .expect("list history")[0];
    assert!(entry.sql_text.chars().count() <= MAX_DATABASE_HISTORY_SQL_CHARS);
    assert!(entry.sql_text.ends_with('…'));
    assert!(
        entry
            .error_summary
            .as_deref()
            .is_some_and(|summary| summary.chars().count() <= MAX_DATABASE_HISTORY_ERROR_CHARS)
    );
}

#[test]
fn retention_keeps_the_newest_bounded_rows() {
    let (database, connection) = fixture();
    let total = MAX_DATABASE_QUERY_HISTORY_RECORDS + 5;

    for started_at in 1..=i64::from(total) {
        record(
            &database,
            &connection,
            &format!("SELECT {started_at}"),
            DatabaseQueryResult::Success,
            started_at,
        );
    }

    let history = list_query_history(
        &database,
        &DatabaseQueryHistoryFilter {
            limit: Some(MAX_DATABASE_QUERY_HISTORY_LIMIT),
            ..DatabaseQueryHistoryFilter::default()
        },
    )
    .expect("list bounded history");
    assert_eq!(
        history.len(),
        usize::try_from(MAX_DATABASE_QUERY_HISTORY_RECORDS).unwrap()
    );
    assert_eq!(history[0].started_at, i64::from(total));
    assert_eq!(history.last().map(|entry| entry.started_at), Some(6));
}

#[test]
fn filters_and_restart_persistence_keep_history_queryable() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("serverdesk.sqlite3");
    let server_id;
    let first_id;
    let snippet_id;

    {
        let database = Database::open(&path).expect("open database");
        let server = database
            .create_server(&server_input("Production"))
            .expect("create server");
        server_id = server.id.clone();
        let first = database
            .create_database_connection(&connection_input(
                &server.id,
                "Primary",
                DatabaseEngine::Postgresql,
            ))
            .expect("create first connection");
        let second = database
            .create_database_connection(&connection_input(
                &server.id,
                "Reporting",
                DatabaseEngine::Mysql,
            ))
            .expect("create second connection");
        first_id = first.id.clone();

        record(
            &database,
            &first,
            "SELECT 1",
            DatabaseQueryResult::Success,
            100,
        );
        record(
            &database,
            &first,
            "SELECT * FORM users",
            DatabaseQueryResult::Failure,
            200,
        );
        record(
            &database,
            &second,
            "SELECT * FROM orders",
            DatabaseQueryResult::Success,
            300,
        );

        snippet_id = create_saved_query(
            &database,
            &NewDatabaseSavedQuery {
                name: "Recent orders".to_string(),
                engine: DatabaseEngine::Mysql,
                sql_text: "SELECT * FROM orders ORDER BY created_at DESC".to_string(),
            },
        )
        .expect("create snippet")
        .id;
    }

    {
        let database = Database::open(&path).expect("reopen database");
        assert_eq!(database.schema_version().expect("schema version"), 6);

        let all = list_query_history(&database, &DatabaseQueryHistoryFilter::default())
            .expect("history after restart");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].started_at, 300);
        assert_eq!(all[2].started_at, 100);

        let first_connection = list_query_history(
            &database,
            &DatabaseQueryHistoryFilter {
                connection_id: Some(first_id.clone()),
                ..DatabaseQueryHistoryFilter::default()
            },
        )
        .expect("filter by connection");
        assert_eq!(first_connection.len(), 2);

        let failures = list_query_history(
            &database,
            &DatabaseQueryHistoryFilter {
                result: Some(DatabaseQueryResult::Failure),
                ..DatabaseQueryHistoryFilter::default()
            },
        )
        .expect("filter by result");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].sql_text, "SELECT * FORM users");

        let recent = list_query_history(
            &database,
            &DatabaseQueryHistoryFilter {
                started_after: Some(150),
                ..DatabaseQueryHistoryFilter::default()
            },
        )
        .expect("filter after date");
        assert_eq!(recent.len(), 2);

        let older = list_query_history(
            &database,
            &DatabaseQueryHistoryFilter {
                server_id: Some(server_id),
                started_before: Some(150),
                ..DatabaseQueryHistoryFilter::default()
            },
        )
        .expect("filter before date");
        assert_eq!(older.len(), 1);

        let snippets = database
            .list_database_saved_queries()
            .expect("snippets after restart");
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].id, snippet_id);
        assert_eq!(snippets[0].name, "Recent orders");
    }
}

#[test]
fn snippets_are_sanitized_reused_updated_and_deleted_with_confirmation() {
    let (database, connection) = fixture();
    record(
        &database,
        &connection,
        "SELECT 1",
        DatabaseQueryResult::Success,
        10,
    );

    let created = create_saved_query(
        &database,
        &NewDatabaseSavedQuery {
            name: "  Active users  ".to_string(),
            engine: DatabaseEngine::Postgresql,
            sql_text: "SELECT * FROM users WHERE password = 'fixture-password' AND active = true"
                .to_string(),
        },
    )
    .expect("create snippet");

    assert_eq!(created.name, "Active users");
    assert!(!created.sql_text.contains("fixture-password"));
    assert!(created.sql_text.contains("<redacted>"));

    let listed = database
        .list_database_saved_queries()
        .expect("list snippets");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sql_text, created.sql_text);

    let updated = update_saved_query(
        &database,
        &created.id,
        &NewDatabaseSavedQuery {
            name: "Active users and teams".to_string(),
            engine: DatabaseEngine::Postgresql,
            sql_text: "SELECT * FROM users WHERE active = true AND team_id IS NOT NULL".to_string(),
        },
    )
    .expect("update snippet");
    assert_eq!(updated.name, "Active users and teams");
    assert!(updated.updated_at >= created.updated_at);

    assert!(delete_saved_query(&database, &created.id, false).is_err());
    assert_eq!(
        database
            .list_database_saved_queries()
            .expect("snippet survives unconfirmed delete")
            .len(),
        1
    );

    delete_saved_query(&database, &created.id, true).expect("confirmed delete");
    assert!(
        database
            .list_database_saved_queries()
            .expect("list after delete")
            .is_empty()
    );

    // Clearing history is a separate destructive action and also needs
    // explicit confirmation.
    assert!(clear_query_history(&database, false).is_err());
    assert_eq!(
        list_query_history(&database, &DatabaseQueryHistoryFilter::default())
            .expect("history survives unconfirmed clear")
            .len(),
        1
    );
    clear_query_history(&database, true).expect("confirmed clear");
    assert!(
        list_query_history(&database, &DatabaseQueryHistoryFilter::default())
            .expect("history after clear")
            .is_empty()
    );
}

#[test]
fn sql_redaction_keeps_columns_and_covers_password_syntax() {
    let sanitized = sanitize_query_history_sql(
        "CREATE USER 'app'@'%' IDENTIFIED WITH caching_sha2_password BY 'fixture-password';\n\
         ALTER ROLE app WITH PASSWORD 'fixture-role-password';\n\
         SELECT password, token FROM users;",
    );

    assert!(!sanitized.contains("fixture-password"));
    assert!(!sanitized.contains("fixture-role-password"));
    assert!(sanitized.contains("SELECT password, token FROM users"));
    assert_eq!(
        sanitize_sql_bounded("SELECT 1", MAX_DATABASE_HISTORY_SQL_CHARS),
        "SELECT 1"
    );
}
