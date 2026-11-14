//! Deterministic row-write fixture tests.
//!
//! These tests use the in-memory adapter and pure engine SQL builders. They
//! cover insert/update/delete, explicit NULL/empty/default modes, stable
//! identity enforcement, optimistic conflict detection, rollback, permission
//! denial, scoped bulk delete, and rollback-context reporting for both engines
//! without a live database.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        ColumnIdentity, DatabaseAdapter, DatabaseConnectRequest, DatabaseErrorKind,
        DatabasePassword, FakeDatabaseAdapter, FakeRowTable, MetadataValue, RowColumn,
        RowDeleteRequest, RowIdentity, RowIdentityValue, RowInsertRequest, RowQueryControl,
        RowTransactionState, RowUpdateRequest, RowValueMode, RowWriteOperation, RowWriteValue,
        TableRef, mysql, postgres,
    },
    storage::DatabaseEngine,
};

const DATABASE: &str = "app";

fn identifier(value: &str) -> serverdesk_lib::database::Identifier {
    serverdesk_lib::database::Identifier::new(value).expect("identifier")
}

fn column(
    name: &str,
    data_type: &str,
    nullable: bool,
    ordinal: u32,
    primary: bool,
    identity: ColumnIdentity,
) -> RowColumn {
    RowColumn::new(
        identifier(name),
        data_type.to_string(),
        nullable,
        ordinal,
        primary,
        identity,
        false,
    )
}

fn schema(engine: DatabaseEngine) -> &'static str {
    match engine {
        DatabaseEngine::Mysql => DATABASE,
        DatabaseEngine::Postgresql => "public",
    }
}

fn users_table(engine: DatabaseEngine) -> FakeRowTable {
    FakeRowTable::new(
        TableRef::new(Some(DATABASE), Some(schema(engine)), "users").expect("table"),
        vec![
            column(
                "id",
                "bigint",
                false,
                1,
                true,
                ColumnIdentity::AutoIncrement,
            ),
            column(
                "name",
                "character varying(120)",
                false,
                2,
                false,
                ColumnIdentity::None,
            ),
            column("nickname", "text", true, 3, false, ColumnIdentity::None),
            column("score", "integer", false, 4, false, ColumnIdentity::None),
            column(
                "created_at",
                "timestamp with time zone",
                false,
                5,
                false,
                ColumnIdentity::None,
            ),
        ],
        RowIdentity::primary_key(vec![identifier("id")]),
        MetadataValue::available(4),
        vec![
            vec![
                Some("1".to_string()),
                Some("ann".to_string()),
                Some("first".to_string()),
                Some("10".to_string()),
                Some("2026-01-01T00:00:00Z".to_string()),
            ],
            vec![
                Some("2".to_string()),
                Some("bob".to_string()),
                None,
                Some("30".to_string()),
                Some("2026-01-02T00:00:00Z".to_string()),
            ],
            vec![
                Some("3".to_string()),
                Some("anna".to_string()),
                Some("third".to_string()),
                Some("20".to_string()),
                Some("2026-01-03T00:00:00Z".to_string()),
            ],
            vec![
                Some("4".to_string()),
                Some("zoe".to_string()),
                Some("fourth".to_string()),
                Some("40".to_string()),
                Some("2026-01-04T00:00:00Z".to_string()),
            ],
        ],
    )
    .with_defaults(vec![
        None,
        None,
        None,
        Some("0".to_string()),
        Some("2026-01-01T00:00:00Z".to_string()),
    ])
}

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
    let mut adapter =
        FakeDatabaseAdapter::new(engine, Some("16.2")).with_row_table(users_table(engine));
    adapter
        .connect(connect_request(read_only))
        .await
        .expect("fake connect");
    adapter
}

fn table(engine: DatabaseEngine) -> TableRef {
    TableRef::new(Some(DATABASE), Some(schema(engine)), "users").expect("table")
}

fn write(column: &str, mode: RowValueMode, value: Option<&str>) -> RowWriteValue {
    RowWriteValue {
        column: identifier(column),
        mode,
        value: value.map(ToString::to_string),
    }
}

fn identity(id: &str) -> Vec<RowIdentityValue> {
    vec![RowIdentityValue {
        column: identifier("id"),
        value: Some(id.to_string()),
    }]
}

fn insert_request(engine: DatabaseEngine) -> RowInsertRequest {
    RowInsertRequest {
        table: table(engine),
        values: vec![
            write("name", RowValueMode::Value, Some("")),
            write("nickname", RowValueMode::Null, None),
            write("score", RowValueMode::Value, Some("7")),
            write("created_at", RowValueMode::Default, None),
        ],
    }
}

fn update_request(engine: DatabaseEngine) -> RowUpdateRequest {
    RowUpdateRequest {
        table: table(engine),
        identity: identity("1"),
        values: vec![write("name", RowValueMode::Value, Some("ann updated"))],
        expected: vec![RowIdentityValue {
            column: identifier("name"),
            value: Some("ann".to_string()),
        }],
    }
}

fn delete_request(engine: DatabaseEngine, ids: &[&str]) -> RowDeleteRequest {
    RowDeleteRequest {
        table: table(engine),
        targets: ids.iter().map(|id| identity(id)).collect(),
    }
}

#[tokio::test]
async fn insert_distinguishes_empty_null_and_default_on_both_engines() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = adapter(engine, false).await;
        let result = adapter
            .insert_row(&insert_request(engine), &RowQueryControl::default())
            .await
            .expect("insert");

        assert_eq!(result.operation, RowWriteOperation::Insert);
        assert_eq!(result.transaction, RowTransactionState::Committed);
        assert!(result.verified);
        assert_eq!(result.affected_rows, 1);

        let row = result.row.expect("re-read row");
        assert_eq!(
            row.cells[1].value.as_deref(),
            Some(""),
            "empty string stays distinct"
        );
        assert!(row.cells[2].value.is_none(), "explicit NULL stays distinct");
        assert_eq!(row.cells[3].value.as_deref(), Some("7"));
        assert_eq!(
            row.cells[4].value.as_deref(),
            Some("2026-01-01T00:00:00Z"),
            "default mode applies the column default"
        );
        assert_eq!(row.cells[0].value.as_deref(), Some("5"));
    }
}

#[tokio::test]
async fn update_uses_the_stable_identity_and_re_reads_the_row() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = adapter(engine, false).await;
        let result = adapter
            .update_row(&update_request(engine), &RowQueryControl::default())
            .await
            .expect("update");

        assert_eq!(result.operation, RowWriteOperation::Update);
        assert_eq!(result.transaction, RowTransactionState::Committed);
        assert_eq!(result.affected_rows, 1);
        assert_eq!(
            result.row.expect("row").cells[1].value.as_deref(),
            Some("ann updated")
        );

        let read = adapter
            .read_rows(
                &serverdesk_lib::database::RowReadRequest {
                    table: table(engine),
                    page: serverdesk_lib::database::RowPageRequest::try_new(10, 0).expect("page"),
                    visible_columns: Vec::new(),
                    sort: Vec::new(),
                    filters: Vec::new(),
                    search: None,
                    include_exact_count: false,
                },
                &RowQueryControl::default(),
            )
            .await
            .expect("read after update");
        assert_eq!(read.rows[0].cells[1].value.as_deref(), Some("ann updated"));
    }
}

#[tokio::test]
async fn null_and_empty_string_update_modes_are_preserved() {
    let engine = DatabaseEngine::Postgresql;
    let mut adapter = adapter(engine, false).await;
    let request = RowUpdateRequest {
        table: table(engine),
        identity: identity("1"),
        values: vec![
            write("name", RowValueMode::Value, Some("")),
            write("nickname", RowValueMode::Null, None),
        ],
        expected: vec![
            RowIdentityValue {
                column: identifier("name"),
                value: Some("ann".to_string()),
            },
            RowIdentityValue {
                column: identifier("nickname"),
                value: Some("first".to_string()),
            },
        ],
    };
    let result = adapter
        .update_row(&request, &RowQueryControl::default())
        .await
        .expect("update");
    let row = result.row.expect("row");
    assert_eq!(row.cells[1].value.as_deref(), Some(""));
    assert!(row.cells[2].value.is_none());
}

#[tokio::test]
async fn a_stale_concurrent_update_rolls_back_without_overwriting_newer_data() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = adapter(engine, false).await;
        adapter
            .update_row(&update_request(engine), &RowQueryControl::default())
            .await
            .expect("first update");

        let error = adapter
            .update_row(&update_request(engine), &RowQueryControl::default())
            .await
            .expect_err("stale update must conflict");
        assert_eq!(error.kind, DatabaseErrorKind::Dependency);
        assert!(
            error
                .details
                .as_deref()
                .is_some_and(|details| details.contains("super-secret") || details.contains("No"))
                || error.message.contains("changed")
        );
        assert_eq!(adapter.row_rollback_count(), 1);

        let read = adapter
            .read_rows(
                &serverdesk_lib::database::RowReadRequest {
                    table: table(engine),
                    page: serverdesk_lib::database::RowPageRequest::try_new(10, 0).expect("page"),
                    visible_columns: Vec::new(),
                    sort: Vec::new(),
                    filters: Vec::new(),
                    search: None,
                    include_exact_count: false,
                },
                &RowQueryControl::default(),
            )
            .await
            .expect("read");
        assert_eq!(read.rows[0].cells[1].value.as_deref(), Some("ann updated"));
    }
}

#[tokio::test]
async fn failed_writes_roll_back_and_leave_every_row_unchanged() {
    let engine = DatabaseEngine::Postgresql;
    let mut adapter = adapter(engine, false).await;
    adapter = adapter.with_row_write_error(serverdesk_lib::database::DatabaseError::query(
        "fixture write failure",
    ));
    let error = adapter
        .insert_row(&insert_request(engine), &RowQueryControl::default())
        .await
        .expect_err("fixture failure");
    assert_eq!(error.kind, DatabaseErrorKind::Query);
    assert!(
        error
            .details
            .as_deref()
            .is_some_and(|details| details.contains("rolled back"))
    );
    assert_eq!(adapter.row_rollback_count(), 1);

    let read = adapter
        .read_rows(
            &serverdesk_lib::database::RowReadRequest {
                table: table(engine),
                page: serverdesk_lib::database::RowPageRequest::try_new(10, 0).expect("page"),
                visible_columns: Vec::new(),
                sort: Vec::new(),
                filters: Vec::new(),
                search: None,
                include_exact_count: true,
            },
            &RowQueryControl::default(),
        )
        .await
        .expect("read");
    assert_eq!(read.rows.len(), 4);
    assert_eq!(read.counts.exact_rows.expect("count").value, 4);
}

#[tokio::test]
async fn permission_denial_and_read_only_sessions_are_typed() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = adapter(engine, true).await;
        let error = adapter
            .insert_row(&insert_request(engine), &RowQueryControl::default())
            .await
            .expect_err("read-only insert");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert!(error.message.contains("read-only"));

        let error = adapter
            .update_row(&update_request(engine), &RowQueryControl::default())
            .await
            .expect_err("read-only update");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);

        let error = adapter
            .delete_rows(&delete_request(engine, &["1"]), &RowQueryControl::default())
            .await
            .expect_err("read-only delete");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
    }
}

#[tokio::test]
async fn tables_without_stable_identity_cannot_be_edited() {
    let engine = DatabaseEngine::Postgresql;
    let fixture = FakeRowTable::new(
        TableRef::new(Some(DATABASE), Some("public"), "events").expect("table"),
        vec![
            column("kind", "text", false, 1, false, ColumnIdentity::None),
            column("payload", "jsonb", true, 2, false, ColumnIdentity::None),
        ],
        RowIdentity::none(),
        MetadataValue::available(2),
        vec![vec![Some("login".to_string()), None]],
    );
    let mut adapter = FakeDatabaseAdapter::new(engine, Some("16.2")).with_row_table(fixture);
    adapter
        .connect(connect_request(false))
        .await
        .expect("connect");

    let table = TableRef::new(Some(DATABASE), Some("public"), "events").expect("table");
    let insert = RowInsertRequest {
        table: table.clone(),
        values: vec![write("kind", RowValueMode::Value, Some("logout"))],
    };
    let error = adapter
        .insert_row(&insert, &RowQueryControl::default())
        .await
        .expect_err("no identity insert");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);
    assert!(error.message.contains("stable primary key"));

    let update = RowUpdateRequest {
        table: table.clone(),
        identity: Vec::new(),
        values: vec![write("kind", RowValueMode::Value, Some("logout"))],
        expected: Vec::new(),
    };
    assert!(
        adapter
            .update_row(&update, &RowQueryControl::default())
            .await
            .is_err()
    );

    let delete = RowDeleteRequest {
        table,
        targets: vec![Vec::new()],
    };
    assert!(
        adapter
            .delete_rows(&delete, &RowQueryControl::default())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn single_and_scoped_bulk_delete_report_the_affected_rows() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = adapter(engine, false).await;
        let single = adapter
            .delete_rows(&delete_request(engine, &["1"]), &RowQueryControl::default())
            .await
            .expect("single delete");
        assert_eq!(single.affected_rows, 1);
        assert_eq!(single.transaction, RowTransactionState::Committed);
        assert!(single.verification.contains("Deleted 1 row"));

        let bulk = adapter
            .delete_rows(
                &delete_request(engine, &["2", "3"]),
                &RowQueryControl::default(),
            )
            .await
            .expect("bulk delete");
        assert_eq!(bulk.affected_rows, 2);
        assert!(bulk.verification.contains("Deleted 2 rows"));

        let read = adapter
            .read_rows(
                &serverdesk_lib::database::RowReadRequest {
                    table: table(engine),
                    page: serverdesk_lib::database::RowPageRequest::try_new(10, 0).expect("page"),
                    visible_columns: Vec::new(),
                    sort: Vec::new(),
                    filters: Vec::new(),
                    search: None,
                    include_exact_count: true,
                },
                &RowQueryControl::default(),
            )
            .await
            .expect("read");
        assert_eq!(read.rows.len(), 1);
        assert_eq!(read.rows[0].cells[0].value.as_deref(), Some("4"));
    }
}

#[tokio::test]
async fn a_missing_delete_target_rolls_back_the_whole_batch() {
    let engine = DatabaseEngine::Postgresql;
    let mut adapter = adapter(engine, false).await;
    let error = adapter
        .delete_rows(
            &delete_request(engine, &["1", "999"]),
            &RowQueryControl::default(),
        )
        .await
        .expect_err("missing target");
    assert_eq!(error.kind, DatabaseErrorKind::Dependency);
    assert_eq!(adapter.row_rollback_count(), 1);

    let read = adapter
        .read_rows(
            &serverdesk_lib::database::RowReadRequest {
                table: table(engine),
                page: serverdesk_lib::database::RowPageRequest::try_new(10, 0).expect("page"),
                visible_columns: Vec::new(),
                sort: Vec::new(),
                filters: Vec::new(),
                search: None,
                include_exact_count: true,
            },
            &RowQueryControl::default(),
        )
        .await
        .expect("read");
    assert_eq!(read.rows.len(), 4);
}

#[test]
fn write_builders_bind_every_value_and_keep_identifiers_quoted() {
    let engine = DatabaseEngine::Postgresql;
    let fixture = users_table(engine);
    let metadata = serverdesk_lib::database::RowTableMetadata {
        engine,
        database: Some(DATABASE.to_string()),
        schema: Some("public".to_string()),
        table: identifier("users"),
        columns: fixture.columns.clone(),
        identity: fixture.identity.clone(),
        estimated_rows: MetadataValue::available(4),
    };
    let request = insert_request(engine);
    let plan = serverdesk_lib::database::row_writes::resolve_insert(&request, &metadata)
        .expect("resolved insert");
    let insert = postgres::rows::build_insert_query(&metadata, &plan).expect("insert sql");
    assert!(
        insert
            .query
            .sql()
            .starts_with("INSERT INTO \"public\".\"users\"")
    );
    assert!(insert.query.sql().contains("DEFAULT"));
    assert!(!insert.query.sql().contains("ann"));
    assert!(insert.query.values().iter().any(|value| {
        matches!(value, serverdesk_lib::database::SqlValue::Text(text) if text.is_empty())
    }));

    let update = update_request(engine);
    let plan = serverdesk_lib::database::row_writes::resolve_update(&update, &metadata)
        .expect("resolved update");
    let query = postgres::rows::build_update_query(&metadata, &plan).expect("update sql");
    assert!(query.sql().contains("\"id\" = $2::bigint"));
    assert!(query.sql().contains("IS NOT DISTINCT FROM"));
    assert!(!query.sql().contains("ann updated"));

    let delete = delete_request(engine, &["1", "2"]);
    let plan = serverdesk_lib::database::row_writes::resolve_delete(&delete, &metadata)
        .expect("resolved delete");
    let query = postgres::rows::build_delete_query(&metadata, &plan).expect("delete sql");
    assert!(
        query
            .sql()
            .starts_with("DELETE FROM \"public\".\"users\" WHERE")
    );
    assert!(query.sql().contains(" OR "));

    let mysql_metadata = serverdesk_lib::database::RowTableMetadata {
        engine: DatabaseEngine::Mysql,
        schema: Some(DATABASE.to_string()),
        ..metadata.clone()
    };
    let mysql_plan =
        serverdesk_lib::database::row_writes::resolve_insert(&request, &mysql_metadata)
            .expect("mysql insert plan");
    let mysql_insert =
        mysql::rows::build_insert_query(&mysql_metadata, &mysql_plan).expect("mysql insert sql");
    assert!(mysql_insert.query.sql().contains('`'));
    assert!(!mysql_insert.query.sql().contains('$'));
}
