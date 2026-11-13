//! Deterministic row-browser fixture tests.
//!
//! These tests use the in-memory adapter and pure engine SQL builders. They
//! cover empty tables, hard page bounds, filters, search, sorting, exact and
//! estimated counts, stable-identity detection, cancellation, and injection
//! attempts for both MySQL and PostgreSQL without a live database.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        ColumnIdentity, DatabaseAdapter, DatabaseConnectRequest, DatabaseErrorKind,
        DatabasePassword, FakeDatabaseAdapter, FakeRowTable, MAX_ROW_FILTERS, MAX_ROW_PAGE_LIMIT,
        MAX_ROW_SEARCH_CHARACTERS, MetadataValue, RowColumn, RowFilter, RowFilterOperator,
        RowFilterValue, RowIdentity, RowIdentityKind, RowPageRequest, RowQueryControl,
        RowQueryRegistry, RowReadOnlyReason, RowReadRequest, RowSearch, RowSort, RowSortDirection,
        RowTableMetadata, SqlValue, TableRef, mysql, postgres, row_query_timeout,
    },
    storage::DatabaseEngine,
};

const DATABASE: &str = "app";

fn identifier(value: &str) -> serverdesk_lib::database::Identifier {
    serverdesk_lib::database::Identifier::new(value).expect("identifier")
}

fn column(name: &str, data_type: &str, nullable: bool, ordinal: u32, primary: bool) -> RowColumn {
    RowColumn::new(
        identifier(name),
        data_type.to_string(),
        nullable,
        ordinal,
        primary,
        ColumnIdentity::None,
        false,
    )
}

fn users_table(engine: DatabaseEngine) -> FakeRowTable {
    let schema = match engine {
        DatabaseEngine::Mysql => DATABASE,
        DatabaseEngine::Postgresql => "public",
    };
    FakeRowTable::new(
        TableRef::new(Some(DATABASE), Some(schema), "users").expect("table"),
        vec![
            column("id", "bigint", false, 1, true),
            column("name", "character varying(120)", false, 2, false),
            column("score", "integer", false, 3, false),
            column("note", "text", true, 4, false),
            column("payload", "jsonb", true, 5, false),
        ],
        RowIdentity::primary_key(vec![identifier("id")]),
        MetadataValue::available(4),
        vec![
            vec![
                Some("1".to_string()),
                Some("ann".to_string()),
                Some("10".to_string()),
                Some("first".to_string()),
                Some("{\"a\":1}".to_string()),
            ],
            vec![
                Some("2".to_string()),
                Some("bob".to_string()),
                Some("30".to_string()),
                None,
                None,
            ],
            vec![
                Some("3".to_string()),
                Some("anna".to_string()),
                Some("20".to_string()),
                Some("third".to_string()),
                None,
            ],
            vec![
                Some("4".to_string()),
                Some("zoe".to_string()),
                Some("40".to_string()),
                Some("fourth".to_string()),
                None,
            ],
        ],
    )
}

fn page(limit: u32, offset: u64) -> RowPageRequest {
    RowPageRequest::try_new(limit, offset).expect("page")
}

fn base_request(engine: DatabaseEngine) -> RowReadRequest {
    let (database, schema) = match engine {
        DatabaseEngine::Mysql => (Some(DATABASE), Some(DATABASE)),
        DatabaseEngine::Postgresql => (Some(DATABASE), Some("public")),
    };
    RowReadRequest {
        table: TableRef::new(database, schema, "users").expect("table"),
        page: page(10, 0),
        visible_columns: Vec::new(),
        sort: Vec::new(),
        filters: Vec::new(),
        search: None,
        include_exact_count: false,
    }
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

async fn base_adapter(engine: DatabaseEngine, read_only: bool) -> FakeDatabaseAdapter {
    let mut adapter =
        FakeDatabaseAdapter::new(engine, Some("8.0.36")).with_row_table(users_table(engine));
    adapter
        .connect(connect_request(read_only))
        .await
        .expect("fake connect");
    adapter
}

fn metadata(engine: DatabaseEngine) -> RowTableMetadata {
    let schema = match engine {
        DatabaseEngine::Mysql => DATABASE,
        DatabaseEngine::Postgresql => "public",
    };
    RowTableMetadata {
        engine,
        database: Some(DATABASE.to_string()),
        schema: Some(schema.to_string()),
        table: identifier("users"),
        columns: vec![
            column("id", "bigint", false, 1, true),
            column("name", "character varying(120)", false, 2, false),
            column("score", "integer", false, 3, false),
        ],
        identity: RowIdentity::primary_key(vec![identifier("id")]),
        estimated_rows: MetadataValue::available(4),
    }
}

#[tokio::test]
async fn mysql_filters_sort_search_and_paginate_with_exact_and_estimated_counts() {
    let adapter = base_adapter(DatabaseEngine::Mysql, false).await;
    let mut request = base_request(DatabaseEngine::Mysql);
    request.page = page(1, 0);
    request.visible_columns = vec![identifier("name"), identifier("score")];
    request.sort.push(RowSort {
        column: identifier("score"),
        direction: RowSortDirection::Desc,
    });
    request.filters.push(RowFilter {
        column: identifier("name"),
        operator: RowFilterOperator::Contains,
        value: Some(RowFilterValue::Text("ANN".to_string())),
        values: Vec::new(),
    });
    request.search = Some(RowSearch {
        query: "ann".to_string(),
        columns: Vec::new(),
    });
    request.include_exact_count = true;

    let first = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect("first page");
    assert_eq!(first.rows.len(), 1);
    assert!(first.page.has_more);
    assert_eq!(first.page.returned, 1);
    assert_eq!(first.page.limit, 1);
    assert_eq!(
        first.counts.estimated_table_rows,
        MetadataValue::available(4)
    );
    assert_eq!(
        first.counts.exact_rows,
        Some(serverdesk_lib::database::RowExactCount {
            value: 2,
            filtered: true,
        })
    );
    assert_eq!(first.rows[0].cells[0].value.as_deref(), Some("anna"));
    assert_eq!(first.rows[0].cells[1].value.as_deref(), Some("20"));
    assert_eq!(first.rows[0].identity[0].column, identifier("id"));
    assert_eq!(first.rows[0].identity[0].value.as_deref(), Some("3"));
    assert_eq!(
        first.visible_columns,
        vec![identifier("name"), identifier("score")]
    );
    assert!(first.editable);

    request.page = page(1, 1);
    let second = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect("second page");
    assert_eq!(second.rows[0].cells[0].value.as_deref(), Some("ann"));
    assert!(!second.page.has_more);
}

#[tokio::test]
async fn postgres_empty_table_returns_an_empty_page_and_exact_zero() {
    let mut fixture = users_table(DatabaseEngine::Postgresql);
    fixture.rows.clear();
    fixture.estimated_rows = MetadataValue::available(0);
    let mut adapter =
        FakeDatabaseAdapter::new(DatabaseEngine::Postgresql, Some("16.2")).with_row_table(fixture);
    adapter
        .connect(connect_request(false))
        .await
        .expect("fake connect");

    let mut request = base_request(DatabaseEngine::Postgresql);
    request.include_exact_count = true;
    let result = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect("empty page");

    assert!(result.rows.is_empty());
    assert_eq!(result.page.returned, 0);
    assert!(!result.page.has_more);
    assert_eq!(
        result.counts.exact_rows,
        Some(serverdesk_lib::database::RowExactCount {
            value: 0,
            filtered: false,
        })
    );
    assert_eq!(
        result.counts.estimated_table_rows,
        MetadataValue::available(0)
    );
}

#[tokio::test]
async fn large_pages_are_hard_bounded_and_never_read_past_the_limit() {
    let engine = DatabaseEngine::Mysql;
    let mut fixture = users_table(engine);
    fixture.rows = (0..250)
        .map(|index| {
            vec![
                Some(index.to_string()),
                Some(format!("user-{index}")),
                Some(index.to_string()),
                None,
                None,
            ]
        })
        .collect();
    fixture.estimated_rows = MetadataValue::available(250);
    let mut adapter = FakeDatabaseAdapter::new(engine, Some("8.0.36")).with_row_table(fixture);
    adapter
        .connect(connect_request(false))
        .await
        .expect("fake connect");

    let mut request = base_request(engine);
    request.page = page(MAX_ROW_PAGE_LIMIT, 0);
    request.include_exact_count = true;
    let result = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect("bounded page");
    assert_eq!(result.rows.len(), MAX_ROW_PAGE_LIMIT as usize);
    assert!(result.page.has_more);
    assert_eq!(result.counts.exact_rows.expect("exact").value, 250);

    request.page = RowPageRequest {
        limit: MAX_ROW_PAGE_LIMIT + 1,
        offset: 0,
    };
    let error = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect_err("oversized page must be rejected");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);
}

#[tokio::test]
async fn tables_without_a_stable_identity_are_read_only_for_editing() {
    let engine = DatabaseEngine::Postgresql;
    let fixture = FakeRowTable::new(
        TableRef::new(Some(DATABASE), Some("public"), "events").expect("table"),
        vec![
            column("kind", "text", false, 1, false),
            column("payload", "jsonb", true, 2, false),
        ],
        RowIdentity::none(),
        MetadataValue::available(2),
        vec![
            vec![Some("login".to_string()), Some("{}".to_string())],
            vec![Some("logout".to_string()), None],
        ],
    );
    let mut adapter = FakeDatabaseAdapter::new(engine, Some("16.2")).with_row_table(fixture);
    adapter
        .connect(connect_request(false))
        .await
        .expect("fake connect");

    let mut request = base_request(engine);
    request.table = TableRef::new(Some(DATABASE), Some("public"), "events").expect("table");
    let result = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect("row page");

    assert!(!result.editable);
    assert_eq!(
        result.read_only_reasons,
        [RowReadOnlyReason::NoStableIdentity]
    );
    assert_eq!(result.identity.kind, RowIdentityKind::None);
    assert!(!result.identity.stable);
    assert!(result.rows.iter().all(|row| row.identity.is_empty()));
}

#[tokio::test]
async fn read_only_session_adds_a_second_editing_reason() {
    let engine = DatabaseEngine::Mysql;
    let adapter = base_adapter(engine, true).await;
    let result = adapter
        .read_rows(&base_request(engine), &RowQueryControl::default())
        .await
        .expect("row page");

    assert!(!result.editable);
    assert_eq!(
        result.read_only_reasons,
        [RowReadOnlyReason::SessionReadOnly]
    );
}

#[tokio::test]
async fn cancellation_before_and_during_a_read_returns_a_cancelled_error() {
    let engine = DatabaseEngine::Mysql;
    let adapter = base_adapter(engine, false).await;

    let registry = RowQueryRegistry::default();
    let guard = registry.begin("row-read-1").expect("begin");
    assert!(registry.cancel("row-read-1"));
    let control = RowQueryControl::new(Duration::from_secs(10), Some(guard.cancellation()));
    let error = adapter
        .read_rows(&base_request(engine), &control)
        .await
        .expect_err("cancelled read");
    assert_eq!(error.kind, DatabaseErrorKind::Cancelled);

    let registry = RowQueryRegistry::default();
    let guard = registry.begin("row-read-2").expect("begin");
    guard.cancellation().cancel();
    let control = RowQueryControl::new(Duration::from_secs(10), Some(guard.cancellation()));
    let error = adapter
        .read_rows(&base_request(engine), &control)
        .await
        .expect_err("cancelled read");
    assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
}

#[tokio::test]
async fn invalid_filters_search_and_error_propagation_are_typed() {
    let engine = DatabaseEngine::Postgresql;
    let adapter = base_adapter(engine, false).await;

    let mut request = base_request(engine);
    for _ in 0..=MAX_ROW_FILTERS {
        request.filters.push(RowFilter {
            column: identifier("id"),
            operator: RowFilterOperator::Equals,
            value: Some(RowFilterValue::Integer(1)),
            values: Vec::new(),
        });
    }
    let error = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect_err("too many filters");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);

    let mut request = base_request(engine);
    request.search = Some(RowSearch {
        query: "x".repeat(MAX_ROW_SEARCH_CHARACTERS + 1),
        columns: Vec::new(),
    });
    let error = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect_err("oversized search");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);

    let mut request = base_request(engine);
    request.filters.push(RowFilter {
        column: identifier("payload"),
        operator: RowFilterOperator::Equals,
        value: Some(RowFilterValue::Text("{}".to_string())),
        values: Vec::new(),
    });
    let error = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect_err("json equality");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);

    let adapter = base_adapter(engine, false).await;
    let mut request = base_request(engine);
    request.filters.push(RowFilter {
        column: identifier("payload"),
        operator: RowFilterOperator::IsNull,
        value: None,
        values: Vec::new(),
    });
    let result = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect("json null check");
    assert_eq!(result.rows.len(), 3);

    let adapter = FakeDatabaseAdapter::new(engine, Some("16.2"))
        .with_row_table(users_table(engine))
        .with_row_read_error(serverdesk_lib::database::DatabaseError::timeout(
            "fixture timeout",
        ));
    let mut adapter = adapter;
    adapter
        .connect(connect_request(false))
        .await
        .expect("fake connect");
    let error = adapter
        .read_rows(&request, &RowQueryControl::default())
        .await
        .expect_err("propagated timeout");
    assert_eq!(error.kind, DatabaseErrorKind::Timeout);
}

#[test]
fn sql_builders_for_both_engines_bind_values_and_quote_identifiers() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let metadata = metadata(engine);
        let mut request = base_request(engine);
        request.page = page(25, 50);
        request.visible_columns = vec![identifier("name")];
        request.sort.push(RowSort {
            column: identifier("score"),
            direction: RowSortDirection::Desc,
        });
        request.filters.push(RowFilter {
            column: identifier("name"),
            operator: RowFilterOperator::Equals,
            value: Some(RowFilterValue::Text(
                "x' OR 1=1; DROP TABLE audit; --".to_string(),
            )),
            values: Vec::new(),
        });
        request.search = Some(RowSearch {
            query: "'); DELETE FROM users; --".to_string(),
            columns: vec![identifier("name")],
        });
        request.include_exact_count = true;

        let (page_sql, page_values) = match engine {
            DatabaseEngine::Mysql => {
                let select = mysql::rows::build_page_query(&metadata, &request).expect("mysql");
                (
                    select.query.sql().to_string(),
                    select.query.values().to_vec(),
                )
            }
            DatabaseEngine::Postgresql => {
                let select = postgres::rows::build_page_query(&metadata, &request).expect("pg");
                (
                    select.query.sql().to_string(),
                    select.query.values().to_vec(),
                )
            }
        };

        assert!(!page_sql.contains("DROP TABLE"));
        assert!(!page_sql.contains("DELETE FROM"));
        assert!(page_values.iter().any(|value| matches!(
            value,
            SqlValue::Text(text) if text.contains("DROP TABLE")
        )));
        assert_eq!(page_values[page_values.len() - 2], SqlValue::Integer(26));
        assert_eq!(page_values[page_values.len() - 1], SqlValue::Integer(50));
        match engine {
            DatabaseEngine::Mysql => {
                assert!(page_sql.contains('`'));
                assert!(page_sql.contains("LIMIT ? OFFSET ?"));
                assert!(!page_sql.contains("$1"));
            }
            DatabaseEngine::Postgresql => {
                assert!(page_sql.contains('"'));
                assert!(page_sql.contains("LIMIT $"));
                assert!(!page_sql.contains('?'));
            }
        }

        let count_sql = match engine {
            DatabaseEngine::Mysql => mysql::rows::build_count_query(&metadata, &request)
                .expect("mysql count")
                .sql()
                .to_string(),
            DatabaseEngine::Postgresql => postgres::rows::build_count_query(&metadata, &request)
                .expect("pg count")
                .sql()
                .to_string(),
        };
        assert!(count_sql.starts_with("SELECT COUNT(*)"));
        assert!(!count_sql.contains("LIMIT"));
        assert!(!count_sql.contains("DROP TABLE"));
    }
}

#[test]
fn unknown_sort_and_filter_columns_and_injection_ids_are_rejected() {
    let metadata = metadata(DatabaseEngine::Mysql);
    let mut request = base_request(DatabaseEngine::Mysql);
    request.sort.push(RowSort {
        column: identifier("name` DESC, (SELECT 1); --"),
        direction: RowSortDirection::Asc,
    });
    assert_eq!(
        mysql::rows::build_page_query(&metadata, &request)
            .expect_err("unknown sort")
            .kind,
        DatabaseErrorKind::Validation
    );

    let mut request = base_request(DatabaseEngine::Mysql);
    request.filters.push(RowFilter {
        column: identifier("missing"),
        operator: RowFilterOperator::Equals,
        value: Some(RowFilterValue::Integer(1)),
        values: Vec::new(),
    });
    assert_eq!(
        postgres::rows::build_count_query(&metadata, &request)
            .expect_err("unknown filter")
            .kind,
        DatabaseErrorKind::Validation
    );

    let registry = RowQueryRegistry::default();
    for invalid in ["", "row read", "row;read", &"a".repeat(200)] {
        assert!(registry.begin(invalid).is_err());
    }
    assert!(!registry.cancel("row;read"));
}

#[test]
fn timeout_seconds_are_clamped_to_the_hard_bound() {
    assert_eq!(row_query_timeout(None), Duration::from_secs(10));
    assert_eq!(
        row_query_timeout(Some(u64::MAX)),
        serverdesk_lib::database::MAX_ROW_QUERY_TIMEOUT
    );
}
