//! Database Manager security regression suite.
//!
//! These tests attack the shared query builders and every mutating service
//! path with hostile identifiers, values, account names, sort keys, import
//! mappings, dump names, and confirmation bypasses. They use the deterministic
//! fake adapter and pure engine builders, so the suite runs in CI without a
//! live MySQL or PostgreSQL server.
//!
//! The suite also scans the application SQLite database for leaked plaintext
//! secrets and verifies the read-only and confirmation gates at the service
//! boundary rather than relying on the UI.

use std::time::Duration;

use serverdesk_lib::{
    core::database_history::{
        DatabaseQueryHistoryDraft, create_saved_query, record_query_history,
        sanitize_query_history_sql, sanitize_saved_query_sql,
    },
    database::{
        ColumnDefinitionRequest, ColumnIdentity, CsvColumnMapping, CsvImportObserver,
        CsvImportOptions, CsvImportRequest, CsvImportSourceFile, CsvWarning, DatabaseActionRequest,
        DatabaseAdapter, DatabaseConnectRequest, DatabaseError, DatabaseErrorKind,
        DatabasePassword, DumpConnection, DumpExportRequest, DumpFormat, DumpRestoreConfirmation,
        DumpRestoreRequest, DumpScope, DumpServerContext, DumpSource, FakeDatabaseAdapter,
        FakeRowTable, Identifier, IdentifierQuoter, MetadataValue, ObjectActionRequest, Privilege,
        PrivilegeTargetKind, PrivilegeTargetRequest, QueryControl, QueryRequest, RoleActionRequest,
        RoleKind, RoleRefRequest, RowColumn, RowFilter, RowFilterOperator, RowFilterValue,
        RowIdentity, RowInsertRequest, RowPageRequest, RowQueryControl, RowReadRequest, RowSearch,
        RowSort, RowSortDirection, SessionActionKind, SessionActionRequest, SqlDialect,
        StructureActionRequest, TableOptionsRequest, TableRef, UniqueKeyRequest, analyze_sql,
        execute_object_with_adapter, execute_query_with_adapter, execute_role_with_adapter,
        execute_session_action_with_adapter, execute_structure_with_adapter, execute_with_adapter,
        mysql, postgres, restore_replaced_data, validate_export_request, validate_restore_request,
    },
    storage::{
        AuthType, Database, DatabaseEngine, DatabaseQueryResult, NewDatabaseConnection,
        NewDatabaseSavedQuery, NewServer,
    },
};

const DATABASE: &str = "app";
const TABLE_SCOPE_INJECTION: &str = "users\"; DROP TABLE audit; --";
const MYSQL_TABLE_INJECTION: &str = "users`; DROP TABLE audit; --";
const VALUE_INJECTION: &str = "ann' OR '1'='1";
/// Built from pieces so secret scanners do not treat this test fixture as a
/// committed credential. It is never a real password.
fn password_secret() -> &'static str {
    concat!("fixture", "-db-", "password", "-never", "-persist")
}

fn auth_hash() -> &'static str {
    concat!("fixture-auth-", "hash")
}

fn identifier(value: &str) -> Identifier {
    Identifier::new(value).expect("identifier")
}

fn connect_request(engine: DatabaseEngine, read_only: bool) -> DatabaseConnectRequest {
    DatabaseConnectRequest {
        host: "127.0.0.1".to_string(),
        port: match engine {
            DatabaseEngine::Mysql => 3306,
            DatabaseEngine::Postgresql => 5432,
        },
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
    let mut adapter = FakeDatabaseAdapter::new(
        engine,
        Some(match engine {
            DatabaseEngine::Mysql => "8.0.36",
            DatabaseEngine::Postgresql => "16.2",
        }),
    );
    adapter
        .connect(connect_request(engine, read_only))
        .await
        .expect("fake connect");
    adapter
}

fn row_control() -> RowQueryControl {
    RowQueryControl {
        timeout: Duration::from_secs(5),
        cancellation: None,
    }
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

fn metadata(engine: DatabaseEngine) -> serverdesk_lib::database::RowTableMetadata {
    let (database, schema) = match engine {
        DatabaseEngine::Mysql => (Some(DATABASE), Some(DATABASE)),
        DatabaseEngine::Postgresql => (Some(DATABASE), Some("public")),
    };
    let table = match engine {
        DatabaseEngine::Mysql => "users",
        DatabaseEngine::Postgresql => "users",
    };
    serverdesk_lib::database::RowTableMetadata {
        engine,
        database: database.map(str::to_string),
        schema: schema.map(str::to_string),
        table: identifier(table),
        columns: vec![
            column("id", "bigint", false, 1, true),
            column("name", "character varying(120)", false, 2, false),
            column("note", "text", true, 3, false),
            column("payload", "jsonb", true, 4, false),
        ],
        identity: RowIdentity::primary_key(vec![identifier("id")]),
        estimated_rows: MetadataValue::available(4),
    }
}

fn row_request(engine: DatabaseEngine) -> RowReadRequest {
    let (database, schema) = match engine {
        DatabaseEngine::Mysql => (Some(DATABASE), Some(DATABASE)),
        DatabaseEngine::Postgresql => (Some(DATABASE), Some("public")),
    };
    RowReadRequest {
        table: TableRef::new(database, schema, "users").expect("table"),
        page: RowPageRequest::try_new(10, 0).expect("page"),
        visible_columns: Vec::new(),
        sort: Vec::new(),
        filters: Vec::new(),
        search: None,
        include_exact_count: false,
    }
}

fn built_page_query(
    engine: DatabaseEngine,
    request: &RowReadRequest,
) -> Result<serverdesk_lib::database::rows::RowSelectQuery, DatabaseError> {
    match engine {
        DatabaseEngine::Mysql => mysql::rows::build_page_query(&metadata(engine), request),
        DatabaseEngine::Postgresql => postgres::rows::build_page_query(&metadata(engine), request),
    }
}

#[test]
fn hostile_filter_and_search_values_are_bound_not_interpolated() {
    for engine in DatabaseEngine::ALL {
        let mut request = row_request(engine);
        request.filters = vec![RowFilter {
            column: identifier("name"),
            operator: RowFilterOperator::Equals,
            value: Some(RowFilterValue::Text(VALUE_INJECTION.to_string())),
            values: Vec::new(),
        }];
        request.search = Some(RowSearch {
            query: VALUE_INJECTION.to_string(),
            columns: Vec::new(),
        });

        let built = built_page_query(engine, &request).expect("page query");
        assert!(
            !built.query.sql().contains(VALUE_INJECTION),
            "filter value leaked into SQL for {engine:?}: {}",
            built.query.sql()
        );
        assert!(
            built.query.values().iter().any(|value| value
                == &serverdesk_lib::database::SqlValue::Text(VALUE_INJECTION.to_string())),
            "filter value must be bound as a parameter for {engine:?}"
        );
        let parameter_count = built.query.values().len();
        assert!(parameter_count >= 2, "filter and search bind separately");
    }
}

#[test]
fn hostile_sort_visible_and_filter_identifiers_fail_closed() {
    for engine in DatabaseEngine::ALL {
        let hostile = identifier(TABLE_SCOPE_INJECTION);

        let mut sort_request = row_request(engine);
        sort_request.sort = vec![RowSort {
            column: hostile.clone(),
            direction: RowSortDirection::Asc,
        }];
        let error = built_page_query(engine, &sort_request).expect_err("unknown sort column");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);

        let mut visible_request = row_request(engine);
        visible_request.visible_columns = vec![hostile.clone()];
        let error = built_page_query(engine, &visible_request).expect_err("unknown visible column");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);

        let mut filter_request = row_request(engine);
        filter_request.filters = vec![RowFilter {
            column: hostile,
            operator: RowFilterOperator::IsNull,
            value: None,
            values: Vec::new(),
        }];
        let error = built_page_query(engine, &filter_request).expect_err("unknown filter column");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);

        let mut json_sort = row_request(engine);
        json_sort.sort = vec![RowSort {
            column: identifier("payload"),
            direction: RowSortDirection::Desc,
        }];
        let error = built_page_query(engine, &json_sort).expect_err("json is not sortable");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);
    }
}

#[test]
fn hostile_table_names_stay_one_quoted_identifier_on_both_engines() {
    for engine in DatabaseEngine::ALL {
        let payload = match engine {
            DatabaseEngine::Mysql => MYSQL_TABLE_INJECTION,
            DatabaseEngine::Postgresql => TABLE_SCOPE_INJECTION,
        };
        let mut metadata = metadata(engine);
        metadata.table = identifier(payload);

        let built = match engine {
            DatabaseEngine::Mysql => mysql::rows::build_page_query(&metadata, &row_request(engine)),
            DatabaseEngine::Postgresql => {
                postgres::rows::build_page_query(&metadata, &row_request(engine))
            }
        }
        .expect("page query");

        let dialect: &dyn SqlDialect = match engine {
            DatabaseEngine::Mysql => &mysql::MYSQL_DIALECT,
            DatabaseEngine::Postgresql => &postgres::POSTGRES_DIALECT,
        };
        let quoted = dialect.quote_identifier(&identifier(payload));
        let namespace = match engine {
            DatabaseEngine::Mysql => DATABASE,
            DatabaseEngine::Postgresql => "public",
        };
        let qualified = format!(
            "{}.{}",
            dialect.quote_identifier(&identifier(namespace)),
            quoted
        );
        assert!(
            built.query.sql().contains(&format!(" FROM {qualified}")),
            "table must be one quoted identifier for {engine:?}: {}",
            built.query.sql()
        );
        // The payload only appears inside the quoted identifier, never as a
        // separate statement fragment.
        assert_eq!(
            built.query.sql().matches(quoted.as_str()).count(),
            1,
            "quoted table identifier must appear exactly once"
        );
    }
}

fn create_table_request(
    engine: DatabaseEngine,
    name: &str,
    options: Option<TableOptionsRequest>,
) -> StructureActionRequest {
    let (database, schema) = match engine {
        DatabaseEngine::Mysql => (Some(DATABASE.to_string()), None),
        DatabaseEngine::Postgresql => (None, Some("public".to_string())),
    };
    StructureActionRequest::CreateTable {
        database,
        schema,
        name: name.to_string(),
        columns: vec![ColumnDefinitionRequest {
            name: "id".to_string(),
            data_type: "bigint".to_string(),
            nullable: false,
            default: None,
            comment: None,
            generated: None,
            identity: None,
        }],
        primary_key: vec!["id".to_string()],
        unique_keys: vec![UniqueKeyRequest {
            name: "users_id_key".to_string(),
            columns: vec!["id".to_string()],
        }],
        options,
    }
}

#[test]
fn ddl_identifiers_comments_and_string_literals_cannot_break_out() {
    let comment = "x'); DROP TABLE audit; --";

    let mysql_request = create_table_request(
        DatabaseEngine::Mysql,
        MYSQL_TABLE_INJECTION,
        Some(TableOptionsRequest {
            comment: Some(comment.to_string()),
            ..TableOptionsRequest::default()
        }),
    );
    let validated = mysql_request.validate().expect("mysql create table");
    let preview = mysql::ddl::preview(&validated).expect("mysql ddl");
    let statement = &preview.statements[0];
    let quoted_table = format!(
        "{}.{}",
        mysql::MYSQL_DIALECT.quote_identifier(&identifier(DATABASE)),
        mysql::MYSQL_DIALECT.quote_identifier(&identifier(MYSQL_TABLE_INJECTION))
    );
    assert!(statement.contains(&format!("CREATE TABLE {quoted_table} (")));
    assert!(
        statement.contains("COMMENT='x''); DROP TABLE audit; --'"),
        "the comment must stay one escaped literal: {statement}"
    );

    let postgres_request = create_table_request(
        DatabaseEngine::Postgresql,
        TABLE_SCOPE_INJECTION,
        Some(TableOptionsRequest {
            comment: Some(comment.to_string()),
            ..TableOptionsRequest::default()
        }),
    );
    let validated = postgres_request.validate().expect("postgres create table");
    let preview = postgres::ddl::preview(&validated).expect("postgres ddl");
    let quoted_table = format!(
        "{}.{}",
        postgres::POSTGRES_DIALECT.quote_identifier(&identifier("public")),
        postgres::POSTGRES_DIALECT.quote_identifier(&identifier(TABLE_SCOPE_INJECTION))
    );
    assert!(preview.statements[0].contains(&format!("CREATE TABLE {quoted_table} (")));
    assert!(
        preview
            .statements
            .iter()
            .any(|statement| statement.contains("IS 'x''); DROP TABLE audit; --'")),
        "table comment must be an escaped literal: {:?}",
        preview.statements
    );
}

#[test]
fn ddl_types_and_table_options_reject_statement_breakouts() {
    let mut request = create_table_request(DatabaseEngine::Mysql, "users", None);
    if let StructureActionRequest::CreateTable { columns, .. } = &mut request {
        columns[0].data_type = "bigint; DROP TABLE audit".to_string();
    }
    assert!(request.validate().is_err(), "type breakout must fail");

    let request = create_table_request(
        DatabaseEngine::Mysql,
        "users",
        Some(TableOptionsRequest {
            engine: Some("InnoDB; DROP TABLE audit".to_string()),
            ..TableOptionsRequest::default()
        }),
    );
    assert!(request.validate().is_err(), "engine breakout must fail");

    let request = StructureActionRequest::CreateIndex {
        database: Some(DATABASE.to_string()),
        schema: None,
        table: "users".to_string(),
        name: "users_name_idx".to_string(),
        columns: vec!["name".to_string()],
        unique: false,
        method: Some("btree; DROP TABLE audit".to_string()),
        predicate: None,
    };
    let error = request
        .validate()
        .expect_err("hostile index method rejected");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);
}

#[test]
fn object_and_database_names_are_quoted_on_drop_previews() {
    let mysql_view = ObjectActionRequest::DropView {
        database: Some(DATABASE.to_string()),
        schema: None,
        name: MYSQL_TABLE_INJECTION.to_string(),
        materialized: false,
    }
    .validate()
    .expect("mysql view action");
    let preview = mysql::objects::preview(&mysql_view).expect("mysql drop view");
    let expected = format!(
        "DROP VIEW {}.{}",
        mysql::MYSQL_DIALECT.quote_identifier(&identifier(DATABASE)),
        mysql::MYSQL_DIALECT.quote_identifier(&identifier(MYSQL_TABLE_INJECTION))
    );
    assert_eq!(preview.statements, vec![expected]);

    let postgres_view = ObjectActionRequest::DropView {
        database: None,
        schema: Some("public".to_string()),
        name: TABLE_SCOPE_INJECTION.to_string(),
        materialized: false,
    }
    .validate()
    .expect("postgres view action");
    let preview = postgres::objects::preview(&postgres_view).expect("postgres drop view");
    let expected = format!(
        "DROP VIEW {}.{}",
        postgres::POSTGRES_DIALECT.quote_identifier(&identifier("public")),
        postgres::POSTGRES_DIALECT.quote_identifier(&identifier(TABLE_SCOPE_INJECTION))
    );
    assert_eq!(preview.statements, vec![expected]);

    let database = DatabaseActionRequest::Drop {
        name: MYSQL_TABLE_INJECTION.to_string(),
    }
    .validate()
    .expect("database drop action");
    let preview = mysql::actions::preview(&database).expect("mysql drop database");
    assert_eq!(
        preview.statements,
        vec![format!(
            "DROP DATABASE {}",
            mysql::MYSQL_DIALECT.quote_identifier(&identifier(MYSQL_TABLE_INJECTION))
        )]
    );
}

#[test]
fn role_and_account_injection_fails_closed_and_defaults_stay_least_privilege() {
    for engine in DatabaseEngine::ALL {
        for name in [
            "app'; DROP ROLE admin; --",
            "app\" DROP ROLE admin; --",
            "app\nadmin",
        ] {
            let request = RoleActionRequest::CreateRole {
                name: name.to_string(),
                host: (engine == DatabaseEngine::Mysql).then(|| "%".to_string()),
                role_kind: RoleKind::User,
                can_login: None,
                superuser: None,
                create_database: None,
                create_role: None,
                inherit: None,
            };
            assert!(
                request.validate(engine).is_err(),
                "role name {name:?} must fail closed on {engine:?}"
            );
        }
    }

    let hostile_host = RoleActionRequest::GrantPrivileges {
        grantee: RoleRefRequest {
            name: "reporting".to_string(),
            host: Some("%'; DROP USER app; --".to_string()),
        },
        privileges: vec![Privilege::Select],
        target: PrivilegeTargetRequest {
            kind: PrivilegeTargetKind::Table,
            database: Some(DATABASE.to_string()),
            schema: None,
            object: Some("users".to_string()),
        },
        with_grant_option: false,
    };
    assert!(
        hostile_host.validate(DatabaseEngine::Mysql).is_err(),
        "account host breakout must fail closed"
    );

    let mysql_create = RoleActionRequest::CreateRole {
        name: "reporting".to_string(),
        host: Some("%".to_string()),
        role_kind: RoleKind::User,
        can_login: None,
        superuser: None,
        create_database: None,
        create_role: None,
        inherit: None,
    }
    .validate(DatabaseEngine::Mysql)
    .expect("mysql role");
    let preview = mysql::roles::preview(&mysql_create).expect("mysql role preview");
    let statement = &preview.statements[0];
    assert!(statement.contains("ACCOUNT LOCK"));
    assert!(
        !statement.contains("IDENTIFIED BY") && !statement.contains("PASSWORD"),
        "ServerDesk must never invent a password: {statement}"
    );

    let postgres_create = RoleActionRequest::CreateRole {
        name: "reporting".to_string(),
        host: None,
        role_kind: RoleKind::Role,
        can_login: None,
        superuser: None,
        create_database: None,
        create_role: None,
        inherit: None,
    }
    .validate(DatabaseEngine::Postgresql)
    .expect("postgres role");
    let preview = postgres::roles::preview(&postgres_create).expect("postgres role preview");
    let statement = &preview.statements[0];
    assert!(statement.contains("NOLOGIN"));
    assert!(!statement.contains("SUPERUSER"));
    assert!(!statement.contains("CREATEDB"));
    assert!(!statement.contains("CREATEROLE"));

    let grant = RoleActionRequest::GrantPrivileges {
        grantee: RoleRefRequest {
            name: "reporting".to_string(),
            host: Some("%".to_string()),
        },
        privileges: vec![Privilege::Select],
        target: PrivilegeTargetRequest {
            kind: PrivilegeTargetKind::Table,
            database: Some(DATABASE.to_string()),
            schema: None,
            object: Some("users".to_string()),
        },
        with_grant_option: false,
    }
    .validate(DatabaseEngine::Mysql)
    .expect("mysql grant");
    let preview = mysql::roles::preview(&grant).expect("mysql grant preview");
    assert_eq!(
        preview.statements,
        vec!["GRANT SELECT ON `app`.`users` TO 'reporting'@'%'".to_string()]
    );
}

#[test]
fn session_actions_require_an_exact_phrase_and_reject_hostile_identity() {
    let mut request = SessionActionRequest {
        session_id: 42,
        action: SessionActionKind::TerminateSession,
        expected_user: "app".to_string(),
        expected_database: Some(DATABASE.to_string()),
        confirmation: "terminate session 41".to_string(),
    };
    let error = request.validate().expect_err("wrong session id phrase");
    assert_eq!(error.kind, DatabaseErrorKind::Cancelled);

    request.confirmation = "terminate session 42".to_string();
    request.expected_user = "app\nroot".to_string();
    let error = request
        .validate()
        .expect_err("control character in account");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);

    request.expected_user = "app".to_string();
    request
        .validate()
        .expect("the exact phrase with a valid identity is accepted");
}

#[tokio::test]
async fn session_actions_are_rejected_on_read_only_profiles() {
    let mut adapter = adapter(DatabaseEngine::Postgresql, true).await;
    let request = SessionActionRequest {
        session_id: 42,
        action: SessionActionKind::TerminateSession,
        expected_user: "app".to_string(),
        expected_database: Some(DATABASE.to_string()),
        confirmation: "terminate session 42".to_string(),
    };
    let error = execute_session_action_with_adapter(&mut adapter, request)
        .await
        .expect_err("read-only session action");
    assert_eq!(error.kind, DatabaseErrorKind::Permission);
    assert_eq!(adapter.session_execution_count(), 0);
}

fn drop_table_request(engine: DatabaseEngine) -> StructureActionRequest {
    let (database, schema) = match engine {
        DatabaseEngine::Mysql => (Some(DATABASE.to_string()), Some(DATABASE.to_string())),
        DatabaseEngine::Postgresql => (Some(DATABASE.to_string()), Some("public".to_string())),
    };
    StructureActionRequest::DropTable {
        database,
        schema,
        name: "users".to_string(),
    }
}

fn drop_view_request(engine: DatabaseEngine) -> ObjectActionRequest {
    match engine {
        DatabaseEngine::Mysql => ObjectActionRequest::DropView {
            database: Some(DATABASE.to_string()),
            schema: None,
            name: "active_users".to_string(),
            materialized: false,
        },
        DatabaseEngine::Postgresql => ObjectActionRequest::DropView {
            database: None,
            schema: Some("public".to_string()),
            name: "active_users".to_string(),
            materialized: false,
        },
    }
}

fn create_role_request(engine: DatabaseEngine) -> RoleActionRequest {
    RoleActionRequest::CreateRole {
        name: "new_reporting".to_string(),
        host: (engine == DatabaseEngine::Mysql).then(|| "%".to_string()),
        role_kind: RoleKind::Role,
        can_login: Some(false),
        superuser: None,
        create_database: None,
        create_role: None,
        inherit: None,
    }
}

#[tokio::test]
async fn destructive_service_actions_are_rejected_before_execution_without_confirmation() {
    for engine in DatabaseEngine::ALL {
        let mut structure_adapter = adapter(engine, false).await;
        let error = execute_structure_with_adapter(
            &mut structure_adapter,
            drop_table_request(engine),
            false,
        )
        .await
        .expect_err("unconfirmed drop table");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
        assert_eq!(structure_adapter.structure_execution_count(), 0);

        let mut object_adapter = adapter(engine, false).await;
        let error =
            execute_object_with_adapter(&mut object_adapter, drop_view_request(engine), false)
                .await
                .expect_err("unconfirmed drop view");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
        assert_eq!(object_adapter.object_execution_count(), 0);

        let mut role_adapter = adapter(engine, false).await;
        let error =
            execute_role_with_adapter(&mut role_adapter, create_role_request(engine), false)
                .await
                .expect_err("unconfirmed role change");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
        assert_eq!(role_adapter.role_execution_count(), 0);

        let mut database_adapter = adapter(engine, false).await;
        let error = execute_with_adapter(
            &mut database_adapter,
            DatabaseActionRequest::Drop {
                name: "obsolete".to_string(),
            },
            false,
        )
        .await
        .expect_err("unconfirmed database drop");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
    }
}

#[tokio::test]
async fn destructive_sql_requires_backend_confirmation_including_alter_drop() {
    for engine in DatabaseEngine::ALL {
        let adapter = adapter(engine, false).await;
        for sql in [
            "DROP TABLE users",
            "TRUNCATE TABLE users",
            "DELETE FROM users",
            "UPDATE users SET name = 'x'",
            "ALTER TABLE users DROP COLUMN note",
        ] {
            let request = QueryRequest::try_new(sql, None).expect("query");
            let control = QueryControl::new(Duration::from_secs(5), None, request.max_rows);
            let error = execute_query_with_adapter(&adapter, &request, false, &control)
                .await
                .expect_err("unconfirmed destructive SQL");
            assert_eq!(error.kind, DatabaseErrorKind::Cancelled, "{sql}");
        }
        assert_eq!(adapter.query_execution_count(), 0);
    }
}

#[tokio::test]
async fn read_only_gates_cover_queries_structure_objects_roles_and_rows() {
    for engine in DatabaseEngine::ALL {
        let adapter_read_only = adapter(engine, true).await;

        let request = QueryRequest::try_new(
            "SELECT set_config('default_transaction_read_only', 'off', false)",
            None,
        )
        .expect("query");
        let control = QueryControl::new(Duration::from_secs(5), None, request.max_rows);
        let error = execute_query_with_adapter(&adapter_read_only, &request, false, &control)
            .await
            .expect_err("read-only session config change");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(adapter_read_only.query_execution_count(), 0);

        let mut structure_adapter = adapter(engine, true).await;
        let error = execute_structure_with_adapter(
            &mut structure_adapter,
            drop_table_request(engine),
            true,
        )
        .await
        .expect_err("read-only structure change");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(structure_adapter.structure_execution_count(), 0);

        let mut object_adapter = adapter(engine, true).await;
        let error =
            execute_object_with_adapter(&mut object_adapter, drop_view_request(engine), true)
                .await
                .expect_err("read-only object change");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(object_adapter.object_execution_count(), 0);

        let mut role_adapter = adapter(engine, true).await;
        let error = execute_role_with_adapter(&mut role_adapter, create_role_request(engine), true)
            .await
            .expect_err("read-only role change");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(role_adapter.role_execution_count(), 0);

        let mut row_adapter = adapter(engine, true).await;
        let error = row_adapter
            .insert_row(
                &RowInsertRequest {
                    table: row_request(engine).table,
                    values: Vec::new(),
                },
                &row_control(),
            )
            .await
            .expect_err("read-only row insert");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);

        let mut database_adapter = adapter(engine, true).await;
        let error = execute_with_adapter(
            &mut database_adapter,
            DatabaseActionRequest::Create {
                name: "new_database".to_string(),
                encoding: None,
                collation: None,
                owner: None,
                template: None,
            },
            false,
        )
        .await
        .expect_err("read-only database create");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
    }
}

#[tokio::test]
async fn read_only_profiles_refuse_csv_imports_before_reading_the_file() {
    for engine in DatabaseEngine::ALL {
        let mut adapter = adapter(engine, true).await;
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("import.csv");
        std::fs::write(&path, "id,name\n1,Ann\n").expect("fixture");
        let options = CsvImportOptions {
            delimiter: serverdesk_lib::database::CsvDelimiter::Comma,
            encoding: serverdesk_lib::database::CsvEncoding::Utf8,
            has_header: true,
            null_mode: serverdesk_lib::database::CsvNullMode::UnquotedEmpty,
        };
        let mut source = CsvImportSourceFile::open(&path, &options).expect("source");
        let request = CsvImportRequest {
            table: row_request(engine).table,
            source_path: path.to_string_lossy().to_string(),
            options,
            mappings: vec![CsvColumnMapping {
                source_index: 0,
                target_column: identifier("id"),
            }],
        };
        let error = adapter
            .import_csv(&request, &mut source, &mut NoopObserver, &row_control())
            .await
            .expect_err("read-only CSV import");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
    }
}

struct NoopObserver;

impl CsvImportObserver for NoopObserver {
    fn row_imported(&mut self, _row_number: u64) {}
    fn row_skipped(&mut self, _warning: &CsvWarning) {}
}

fn csv_fixture(engine: DatabaseEngine) -> FakeRowTable {
    let (database, schema) = match engine {
        DatabaseEngine::Mysql => (DATABASE, DATABASE),
        DatabaseEngine::Postgresql => (DATABASE, "public"),
    };
    FakeRowTable::new(
        TableRef::new(Some(database), Some(schema), "users").expect("table"),
        vec![
            column("id", "bigint", false, 1, true),
            column("name", "character varying(120)", false, 2, false),
        ],
        RowIdentity::primary_key(vec![identifier("id")]),
        MetadataValue::available(1),
        vec![vec![Some("9".to_string()), Some("existing".to_string())]],
    )
}

#[tokio::test]
async fn csv_import_mappings_cannot_target_unknown_or_injected_columns() {
    for engine in DatabaseEngine::ALL {
        let mut adapter =
            FakeDatabaseAdapter::new(engine, Some("16.2")).with_row_table(csv_fixture(engine));
        adapter
            .connect(connect_request(engine, false))
            .await
            .expect("connect");
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("import.csv");
        std::fs::write(&path, "id,name\n1,Ann\n").expect("fixture");
        let options = CsvImportOptions {
            delimiter: serverdesk_lib::database::CsvDelimiter::Comma,
            encoding: serverdesk_lib::database::CsvEncoding::Utf8,
            has_header: true,
            null_mode: serverdesk_lib::database::CsvNullMode::UnquotedEmpty,
        };
        let mut source = CsvImportSourceFile::open(&path, &options).expect("source");
        let target = match engine {
            DatabaseEngine::Mysql => identifier(MYSQL_TABLE_INJECTION),
            DatabaseEngine::Postgresql => identifier(TABLE_SCOPE_INJECTION),
        };
        let request = CsvImportRequest {
            table: csv_fixture(engine).table,
            source_path: path.to_string_lossy().to_string(),
            options,
            mappings: vec![CsvColumnMapping {
                source_index: 0,
                target_column: target,
            }],
        };
        let error = adapter
            .import_csv(&request, &mut source, &mut NoopObserver, &row_control())
            .await
            .expect_err("injected target column");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);
        assert!(error.message.contains("does not exist"));
        assert_eq!(
            adapter
                .row_table_rows(&csv_fixture(engine).table)
                .expect("rows"),
            &[vec![Some("9".to_string()), Some("existing".to_string())]]
        );
    }
}

fn dump_connection(engine: DatabaseEngine) -> DumpConnection {
    DumpConnection {
        engine,
        host: "127.0.0.1".to_string(),
        port: match engine {
            DatabaseEngine::Mysql => 3306,
            DatabaseEngine::Postgresql => 5432,
        },
        socket: None,
        database: Some(DATABASE.to_string()),
        username: "app".to_string(),
        ssl_mode: None,
        read_only: false,
    }
}

fn dump_context(engine: DatabaseEngine) -> DumpServerContext {
    DumpServerContext {
        engine,
        version: serverdesk_lib::database::DatabaseVersion::parse("16.2").expect("version"),
    }
}

fn dump_export_request(scope: DumpScope) -> DumpExportRequest {
    DumpExportRequest {
        scope,
        format: DumpFormat::PlainSql,
        tool: None,
        destination: serverdesk_lib::database::DumpDestination::Remote {
            path: "/home/deploy/backup.sql".to_string(),
            overwrite: false,
        },
        no_owner: false,
    }
}

#[test]
fn routine_language_cannot_break_out_of_the_language_clause() {
    for engine in DatabaseEngine::ALL {
        let request = ObjectActionRequest::CreateRoutine {
            database: (engine == DatabaseEngine::Mysql).then(|| DATABASE.to_string()),
            schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
            name: "user_email".to_string(),
            routine_kind: serverdesk_lib::database::RoutineKind::Function,
            arguments: Some("user_id bigint".to_string()),
            return_type: Some("text".to_string()),
            language: Some("plpgsql; DROP TABLE audit; --".to_string()),
            body: "BEGIN RETURN 'x'; END".to_string(),
            or_replace: false,
        };
        let error = request
            .validate()
            .expect_err("hostile routine language must fail closed");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);
    }

    let request = ObjectActionRequest::CreateRoutine {
        database: None,
        schema: Some("public".to_string()),
        name: "user_email".to_string(),
        routine_kind: serverdesk_lib::database::RoutineKind::Function,
        arguments: Some("user_id bigint".to_string()),
        return_type: Some("text".to_string()),
        language: Some("plpgsql".to_string()),
        body: "BEGIN RETURN 'x'; END".to_string(),
        or_replace: false,
    };
    let validated = request.validate().expect("valid language");
    let preview = postgres::objects::preview(&validated).expect("routine preview");
    assert!(
        preview
            .statements
            .iter()
            .any(|statement| statement.contains("LANGUAGE \"plpgsql\"")),
        "language must be one quoted identifier: {:?}",
        preview.statements
    );
}

#[test]
fn dump_database_names_cannot_smuggle_a_pg_connection_string() {
    let hostile = "host=attacker port=5432 dbname=postgres";
    let connection = DumpConnection {
        database: Some(hostile.to_string()),
        ..dump_connection(DatabaseEngine::Postgresql)
    };
    let context = dump_context(DatabaseEngine::Postgresql);

    let export = dump_export_request(DumpScope::Database {
        database: Some(identifier(hostile)),
    });
    let error = validate_export_request(&context, &connection, &export)
        .expect_err("conninfo export target");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);
    assert!(error.message.contains("connection string"));

    let cluster = dump_export_request(DumpScope::Cluster);
    assert!(
        validate_export_request(&context, &connection, &cluster).is_err(),
        "cluster export also uses the profile database"
    );

    let mut restore = DumpRestoreRequest {
        source: DumpSource::Remote {
            path: "/home/deploy/backup.sql".to_string(),
        },
        format: DumpFormat::PlainSql,
        tool: None,
        target_database: identifier(hostile),
        clean: false,
        no_owner: false,
        confirmed: true,
        confirmation: Some(DumpRestoreConfirmation {
            server_id: "server-1".to_string(),
            server_name: "Fixture".to_string(),
            target_database: hostile.to_string(),
            replaced_data: restore_replaced_data(hostile, false),
            acknowledged: true,
        }),
    };
    let error = validate_restore_request(&context, &restore).expect_err("conninfo restore target");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);

    // A normal identifier remains valid.
    restore.target_database = identifier("app");
    restore
        .confirmation
        .as_mut()
        .expect("confirmation")
        .target_database = "app".to_string();
    restore
        .confirmation
        .as_mut()
        .expect("confirmation")
        .replaced_data = restore_replaced_data("app", false);
    validate_restore_request(&context, &restore).expect("normal target");
}

#[test]
fn dump_tool_positional_names_cannot_start_with_an_option_dash() {
    let context = dump_context(DatabaseEngine::Mysql);
    let connection = DumpConnection {
        database: Some("--all-databases".to_string()),
        ..dump_connection(DatabaseEngine::Mysql)
    };
    let export = dump_export_request(DumpScope::Database { database: None });
    let error = validate_export_request(&context, &connection, &export)
        .expect_err("option-shaped database name");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);
    assert!(error.message.contains("start with '-'"));

    let context = dump_context(DatabaseEngine::Postgresql);
    let connection = dump_connection(DatabaseEngine::Postgresql);
    let export = dump_export_request(DumpScope::Tables {
        database: None,
        schema: None,
        tables: vec![identifier("--where=1=1")],
    });
    let error = validate_export_request(&context, &connection, &export)
        .expect_err("option-shaped table name");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);
}

#[test]
fn dump_commands_never_place_the_password_in_arguments() {
    let paths = serverdesk_lib::database::dump::DumpPaths {
        work_dir: "/tmp/serverdesk-dump-fixture".to_string(),
        output: Some("/tmp/serverdesk-dump-fixture/dump.partial".to_string()),
        error_log: "/tmp/serverdesk-dump-fixture/stderr.log".to_string(),
        pid: "/tmp/serverdesk-dump-fixture/dump.pid".to_string(),
        credential: "/tmp/serverdesk-dump-fixture/password".to_string(),
        input: None,
    };
    let command = mysql::dump::export_command(
        &dump_connection(DatabaseEngine::Mysql),
        &DumpScope::Database { database: None },
        DumpFormat::PlainSql,
        &paths,
        serverdesk_lib::database::DumpToolVendor::Mysql,
    )
    .expect("mysql dump command");
    assert!(!command.contains("--password"));
    assert!(!command.contains(password_secret()));
    assert!(command.contains("MYSQL_PWD=$(cat '/tmp/serverdesk-dump-fixture/password')"));

    let command = postgres::dump::export_command(
        &dump_connection(DatabaseEngine::Postgresql),
        &DumpScope::Database { database: None },
        DumpFormat::PlainSql,
        false,
        &paths,
        serverdesk_lib::database::DumpToolVendor::Postgresql,
    )
    .expect("postgres dump command");
    assert!(!command.contains("--password"));
    assert!(!command.contains(password_secret()));
    assert!(command.contains("PGPASSWORD=$(cat '/tmp/serverdesk-dump-fixture/password')"));
}

#[test]
fn sql_history_and_snippets_redact_password_literals_that_used_to_leak() {
    for sql in [
        format!("SET PASSWORD FOR 'app'@'%' = '{}';", password_secret()),
        format!(
            "CREATE USER 'app'@'%' IDENTIFIED WITH caching_sha2_password AS '{}';",
            auth_hash()
        ),
        format!("SELECT PASSWORD('{}');", password_secret()),
        format!("ALTER ROLE app PASSWORD '{}';", password_secret()),
    ] {
        let sanitized = sanitize_query_history_sql(&sql);
        assert!(
            !sanitized.contains(password_secret()) && !sanitized.contains(auth_hash()),
            "history sanitizer leaked a secret for {sql:?}: {sanitized}"
        );
        let sanitized = sanitize_saved_query_sql(&sql).expect("snippet");
        assert!(
            !sanitized.contains(password_secret()) && !sanitized.contains(auth_hash()),
            "snippet sanitizer leaked a secret for {sql:?}: {sanitized}"
        );
    }
}

fn new_server() -> NewServer {
    NewServer {
        name: "Production".to_string(),
        host: "198.51.100.24".to_string(),
        port: 22,
        username: "deploy".to_string(),
        auth_type: AuthType::Password,
        credential_ref: Some("serverdesk:ssh:test-server".to_string()),
        group_name: None,
        tags: vec![],
        notes: None,
    }
}

fn new_profile(server_id: &str) -> NewDatabaseConnection {
    NewDatabaseConnection {
        server_id: server_id.to_string(),
        name: "Primary".to_string(),
        engine: DatabaseEngine::Postgresql,
        host: "127.0.0.1".to_string(),
        port: 5432,
        socket: None,
        database_name: Some(DATABASE.to_string()),
        username: "app".to_string(),
        credential_ref: Some("serverdesk:db:00000000-0000-4000-8000-000000000001".to_string()),
        read_only: true,
        ssl_mode: Some("preferred".to_string()),
    }
}

#[test]
fn application_sqlite_never_stores_database_query_secrets() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("serverdesk.sqlite3");
    let database = Database::open(&path).expect("open database");
    let server = database.create_server(&new_server()).expect("server");
    let profile = database
        .create_database_connection(&new_profile(&server.id))
        .expect("profile");

    record_query_history(
        &database,
        &DatabaseQueryHistoryDraft {
            profile: &profile,
            sql: &format!("SET PASSWORD FOR 'app'@'%' = '{}';", password_secret()),
            result: DatabaseQueryResult::Success,
            duration_ms: 1,
            row_count: Some(0),
            error_summary: Some(&format!("pass{}={}", "word", password_secret())),
            started_at: 1,
        },
    )
    .expect("record history");

    create_saved_query(
        &database,
        &NewDatabaseSavedQuery {
            name: "rotate".to_string(),
            engine: DatabaseEngine::Postgresql,
            sql_text: format!(
                "CREATE USER 'app'@'%' IDENTIFIED WITH caching_sha2_password AS '{}';",
                auth_hash()
            ),
        },
    )
    .expect("saved query");

    drop(database);

    let mut raw = Vec::new();
    for candidate in [
        path.clone(),
        path.with_file_name("serverdesk.sqlite3-wal"),
        path.with_file_name("serverdesk.sqlite3-shm"),
    ] {
        if let Ok(bytes) = std::fs::read(&candidate) {
            raw.extend_from_slice(&bytes);
        }
    }
    let text = String::from_utf8_lossy(&raw);
    for secret in [password_secret(), auth_hash()] {
        assert!(
            !text.contains(secret),
            "{secret:?} was stored in SQLite or its sidecars"
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("database metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "database must be private to the OS user");
    }
}

#[test]
fn read_only_analysis_flags_session_and_sequence_mutation_functions() {
    for sql in [
        "SELECT set_config('default_transaction_read_only', 'off', false)",
        "SELECT setval('users_id_seq', 1)",
        "SELECT nextval('users_id_seq')",
        "SELECT pg_terminate_backend(42)",
    ] {
        let analysis = analyze_sql(DatabaseEngine::Postgresql, sql).expect("analysis");
        assert!(analysis.has_writes, "{sql} must be classified as a write");
    }
    let read = analyze_sql(
        DatabaseEngine::Postgresql,
        "SELECT id, name FROM users WHERE id = $1",
    )
    .expect("read");
    assert!(!read.has_writes);
}
