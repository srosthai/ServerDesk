use rusqlite::{Connection, params};
use serverdesk_lib::storage::{
    AuthType, Database, DatabaseEngine, DatabaseQueryHistoryFilter, DatabaseQueryResult,
    NewDatabaseConnection, NewDatabaseQueryHistory, NewDatabaseSavedQuery, NewServer, StorageError,
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
    let (port, socket, ssl_mode) = match engine {
        DatabaseEngine::Mysql => (
            3306,
            Some("/var/run/mysqld/mysqld.sock".to_string()),
            Some("preferred".to_string()),
        ),
        DatabaseEngine::Postgresql => (5432, None, Some("require".to_string())),
    };

    NewDatabaseConnection {
        server_id: server_id.to_string(),
        name: name.to_string(),
        engine,
        host: "127.0.0.1".to_string(),
        port,
        socket,
        database_name: Some("app".to_string()),
        username: "app".to_string(),
        credential_ref: Some("credential:database-profile".to_string()),
        read_only: true,
        ssl_mode,
    }
}

fn query_input(
    server_id: &str,
    connection_id: &str,
    sql_text: &str,
    result: DatabaseQueryResult,
    started_at: i64,
) -> NewDatabaseQueryHistory {
    NewDatabaseQueryHistory {
        server_id: server_id.to_string(),
        connection_id: connection_id.to_string(),
        engine: DatabaseEngine::Postgresql,
        database_name: Some("app".to_string()),
        sql_text: sql_text.to_string(),
        result,
        duration_ms: 42,
        row_count: Some(3),
        error_summary: (result == DatabaseQueryResult::Failure)
            .then(|| "syntax error at or near \"form\"".to_string()),
        started_at,
    }
}

#[test]
fn database_connection_crud_round_trip() {
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");

    let mysql = database
        .create_database_connection(&connection_input(
            &server.id,
            "Primary MySQL",
            DatabaseEngine::Mysql,
        ))
        .expect("create mysql connection");

    assert_eq!(mysql.server_id, server.id);
    assert_eq!(mysql.engine, DatabaseEngine::Mysql);
    assert_eq!(mysql.port, 3306);
    assert_eq!(mysql.socket.as_deref(), Some("/var/run/mysqld/mysqld.sock"));
    assert_eq!(mysql.database_name.as_deref(), Some("app"));
    assert!(mysql.read_only);
    assert_eq!(mysql.ssl_mode.as_deref(), Some("preferred"));
    assert_eq!(
        mysql.credential_ref.as_deref(),
        Some("credential:database-profile")
    );
    assert_eq!(mysql.last_used_at, None);
    assert_eq!(
        database
            .get_database_connection(&mysql.id)
            .expect("get connection"),
        Some(mysql.clone())
    );

    let postgres = database
        .create_database_connection(&connection_input(
            &server.id,
            "Analytics",
            DatabaseEngine::Postgresql,
        ))
        .expect("create postgres connection");
    let connections = database
        .list_database_connections(&server.id)
        .expect("list connections");
    assert_eq!(connections.len(), 2);
    assert_eq!(connections[0].id, postgres.id);
    assert_eq!(connections[1].id, mysql.id);

    let mut update = connection_input(&server.id, "Primary MySQL", DatabaseEngine::Mysql);
    update.name = "Primary MySQL (read-write)".to_string();
    update.host = "db.internal".to_string();
    update.port = 3307;
    update.socket = None;
    update.database_name = None;
    update.username = "writer".to_string();
    update.credential_ref = Some("credential:database-writer".to_string());
    update.read_only = false;
    update.ssl_mode = None;

    let updated = database
        .update_database_connection(&mysql.id, &update)
        .expect("update connection");
    assert_eq!(updated.name, "Primary MySQL (read-write)");
    assert_eq!(updated.host, "db.internal");
    assert_eq!(updated.port, 3307);
    assert_eq!(updated.socket, None);
    assert_eq!(updated.database_name, None);
    assert!(!updated.read_only);
    assert_eq!(updated.created_at, mysql.created_at);
    assert!(updated.updated_at >= mysql.updated_at);
    assert_eq!(updated.last_used_at, None);

    let used = database
        .mark_database_connection_used(&mysql.id)
        .expect("mark connection used");
    assert!(used.last_used_at.is_some());

    database
        .delete_database_connection(&postgres.id)
        .expect("delete connection");
    assert!(
        database
            .get_database_connection(&postgres.id)
            .expect("get deleted connection")
            .is_none()
    );
    assert_eq!(
        database
            .list_database_connections(&server.id)
            .expect("list after delete")
            .len(),
        1
    );
}

#[test]
fn duplicate_connection_names_are_rejected_case_insensitively_per_server() {
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");
    let first = database
        .create_database_connection(&connection_input(
            &server.id,
            "Production",
            DatabaseEngine::Mysql,
        ))
        .expect("create connection");

    let error = database
        .create_database_connection(&connection_input(
            &server.id,
            "production",
            DatabaseEngine::Postgresql,
        ))
        .expect_err("duplicate name must fail");
    assert!(matches!(error, StorageError::Conflict(_)));
    assert!(
        database
            .database_connection_name_exists(&server.id, "PRODUCTION", None)
            .expect("name lookup")
    );
    assert!(
        !database
            .database_connection_name_exists(&server.id, "PRODUCTION", Some(&first.id))
            .expect("name lookup excluding self")
    );
    assert_eq!(
        database
            .list_database_connections(&server.id)
            .expect("list after rejected duplicate")
            .len(),
        1
    );

    let replica = database
        .create_database_connection(&connection_input(
            &server.id,
            "Replica",
            DatabaseEngine::Mysql,
        ))
        .expect("create replica");
    let duplicate = connection_input(&server.id, "PRODUCTION", DatabaseEngine::Mysql);
    let error = database
        .update_database_connection(&replica.id, &duplicate)
        .expect_err("updating to a duplicate name must fail");
    assert!(matches!(error, StorageError::Conflict(_)));

    // Keeping the same name on the same profile is allowed.
    let unchanged = database
        .update_database_connection(
            &replica.id,
            &connection_input(&server.id, "Replica", DatabaseEngine::Mysql),
        )
        .expect("update with unchanged name");
    assert_eq!(unchanged.id, replica.id);

    let other_server = database
        .create_server(&server_input("Staging"))
        .expect("create other server");
    database
        .create_database_connection(&connection_input(
            &other_server.id,
            "Production",
            DatabaseEngine::Postgresql,
        ))
        .expect("same name on another server is allowed");
}

#[test]
fn deleting_a_server_cascades_connections_and_history_but_keeps_snippets() {
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");
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

    database
        .record_database_query(&query_input(
            &server.id,
            &first.id,
            "SELECT 1",
            DatabaseQueryResult::Success,
            100,
        ))
        .expect("record first query");
    database
        .record_database_query(&query_input(
            &server.id,
            &second.id,
            "SELECT * FROM orders",
            DatabaseQueryResult::Success,
            200,
        ))
        .expect("record second query");

    let snippet = database
        .create_database_saved_query(&NewDatabaseSavedQuery {
            name: "Recent orders".to_string(),
            engine: DatabaseEngine::Mysql,
            sql_text: "SELECT * FROM orders ORDER BY created_at DESC".to_string(),
        })
        .expect("create snippet");

    database
        .delete_database_connection(&second.id)
        .expect("delete one connection");
    assert!(
        database
            .list_database_query_history(&DatabaseQueryHistoryFilter {
                connection_id: Some(second.id.clone()),
                ..DatabaseQueryHistoryFilter::default()
            })
            .expect("history for deleted connection")
            .is_empty()
    );
    assert_eq!(
        database
            .list_database_query_history(&DatabaseQueryHistoryFilter::default())
            .expect("history for remaining connection")
            .len(),
        1
    );

    database.delete_server(&server.id).expect("delete server");
    assert!(
        database
            .list_database_connections(&server.id)
            .expect("connections after cascade")
            .is_empty()
    );
    assert!(
        database
            .list_database_query_history(&DatabaseQueryHistoryFilter::default())
            .expect("history after cascade")
            .is_empty()
    );
    assert_eq!(
        database
            .get_database_saved_query(&snippet.id)
            .expect("snippet survives server deletion"),
        Some(snippet)
    );
}

#[test]
fn query_history_and_snippets_survive_a_restart_with_filters() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("serverdesk.sqlite3");
    let server_id;
    let connection_id;
    let snippet_id;

    {
        let database = Database::open(&path).expect("open database");
        let server = database
            .create_server(&server_input("Production"))
            .expect("create server");
        server_id = server.id.clone();
        let connection = database
            .create_database_connection(&connection_input(
                &server.id,
                "Reporting",
                DatabaseEngine::Postgresql,
            ))
            .expect("create connection");
        connection_id = connection.id.clone();

        database
            .record_database_query(&query_input(
                &server.id,
                &connection.id,
                "SELECT 1",
                DatabaseQueryResult::Success,
                100,
            ))
            .expect("record success");
        database
            .record_database_query(&query_input(
                &server.id,
                &connection.id,
                "SELECT * FORM users",
                DatabaseQueryResult::Failure,
                200,
            ))
            .expect("record failure");
        database
            .record_database_query(&query_input(
                &server.id,
                &connection.id,
                "SELECT pg_sleep(10)",
                DatabaseQueryResult::Cancelled,
                300,
            ))
            .expect("record cancellation");

        let all = database
            .list_database_query_history(&DatabaseQueryHistoryFilter {
                connection_id: Some(connection.id.clone()),
                ..DatabaseQueryHistoryFilter::default()
            })
            .expect("list history");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].started_at, 300);
        assert_eq!(all[2].started_at, 100);

        let failures = database
            .list_database_query_history(&DatabaseQueryHistoryFilter {
                result: Some(DatabaseQueryResult::Failure),
                ..DatabaseQueryHistoryFilter::default()
            })
            .expect("filter by result");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].sql_text, "SELECT * FORM users");
        assert_eq!(
            failures[0].error_summary.as_deref(),
            Some("syntax error at or near \"form\"")
        );

        let recent = database
            .list_database_query_history(&DatabaseQueryHistoryFilter {
                started_after: Some(150),
                ..DatabaseQueryHistoryFilter::default()
            })
            .expect("filter after timestamp");
        assert_eq!(recent.len(), 2);

        let older = database
            .list_database_query_history(&DatabaseQueryHistoryFilter {
                server_id: Some(server.id.clone()),
                started_before: Some(150),
                ..DatabaseQueryHistoryFilter::default()
            })
            .expect("filter before timestamp");
        assert_eq!(older.len(), 1);

        snippet_id = database
            .create_database_saved_query(&NewDatabaseSavedQuery {
                name: "Active users".to_string(),
                engine: DatabaseEngine::Postgresql,
                sql_text: "SELECT * FROM users WHERE active = true".to_string(),
            })
            .expect("create snippet")
            .id;
    }

    {
        let database = Database::open(&path).expect("reopen database");
        assert_eq!(database.schema_version().expect("schema version"), 6);

        let connection = database
            .get_database_connection(&connection_id)
            .expect("get connection after restart")
            .expect("connection exists");
        assert_eq!(connection.server_id, server_id);

        let history = database
            .list_database_query_history(&DatabaseQueryHistoryFilter::default())
            .expect("history after restart");
        assert_eq!(history.len(), 3);
        assert_eq!(history[1].result, DatabaseQueryResult::Failure);

        let snippet = database
            .get_database_saved_query(&snippet_id)
            .expect("get snippet after restart")
            .expect("snippet exists");
        let updated = database
            .update_database_saved_query(
                &snippet.id,
                &NewDatabaseSavedQuery {
                    name: "Active users and teams".to_string(),
                    engine: DatabaseEngine::Postgresql,
                    sql_text: "SELECT * FROM users WHERE active = true AND team_id IS NOT NULL"
                        .to_string(),
                },
            )
            .expect("update snippet");
        assert_eq!(updated.name, "Active users and teams");
        assert!(updated.updated_at >= snippet.updated_at);
        assert_eq!(
            database
                .list_database_saved_queries()
                .expect("list snippets")
                .len(),
            1
        );

        database
            .delete_database_saved_query(&snippet_id)
            .expect("delete snippet");
        assert!(
            database
                .list_database_saved_queries()
                .expect("list after delete")
                .is_empty()
        );

        database
            .clear_database_query_history()
            .expect("clear history");
        assert!(
            database
                .list_database_query_history(&DatabaseQueryHistoryFilter::default())
                .expect("history after clear")
                .is_empty()
        );
    }
}

#[test]
fn database_tables_store_no_secret_columns() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("serverdesk.sqlite3");
    drop(Database::open(&path).expect("open database"));

    let connection = Connection::open(&path).expect("open raw database");
    for table in [
        "database_connections",
        "database_query_history",
        "database_saved_queries",
    ] {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare table info");
        let columns: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("read columns")
            .collect::<Result<_, _>>()
            .expect("collect columns");
        assert!(!columns.is_empty(), "{table} must exist");

        for column in columns {
            let normalized = column.to_ascii_lowercase();
            for forbidden in ["password", "passphrase", "private_key", "secret", "token"] {
                assert!(
                    !normalized.contains(forbidden),
                    "{table}.{column} must not store secret material"
                );
            }
        }
    }
}

#[test]
fn database_engine_check_rejects_other_engines() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("serverdesk.sqlite3");
    drop(Database::open(&path).expect("open database"));

    let connection = Connection::open(&path).expect("open raw database");
    connection
        .execute(
            "INSERT INTO servers (
                id, name, host, port, username, auth_type, tags, created_at, updated_at
            ) VALUES ('server-raw', 'Raw', '127.0.0.1', 22, 'deploy', 'password', '[]', 1, 1)",
            [],
        )
        .expect("insert raw server");

    let error = connection
        .execute(
            "INSERT INTO database_connections (
                id, server_id, name, engine, host, port, username, created_at, updated_at
            ) VALUES ('raw-connection', 'server-raw', 'Raw', 'sqlite', '127.0.0.1', 3306, 'app', 1, 1)",
            params![],
        )
        .expect_err("unknown engine must be rejected");
    assert!(
        error.to_string().contains("CHECK constraint"),
        "unexpected error: {error}"
    );
}
