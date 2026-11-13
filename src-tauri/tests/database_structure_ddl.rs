//! Golden DDL tests for engine-aware structure previews.
//!
//! Every request goes through the fake adapter, so these tests exercise the
//! same validation, capability, connection, and planning path the command uses.
//! The expected SQL files are checked in and must change only when the DDL
//! contract deliberately changes.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        ColumnDefaultRequest, ColumnDefinitionRequest, ColumnIdentity, ColumnPositionRequest,
        ConstraintRequest, DatabaseAdapter, DatabaseConnectRequest, DatabaseErrorKind,
        DatabasePassword, FakeDatabaseAdapter, GeneratedColumnRequest, ReferentialAction,
        StructureActionRequest, TableRef, preview_structure_with_adapter,
    },
    storage::DatabaseEngine,
};

const MYSQL_GOLDEN: &str = include_str!("fixtures/database_structure/mysql.golden.sql");
const POSTGRES_GOLDEN: &str = include_str!("fixtures/database_structure/postgres.golden.sql");

fn column(name: &str, data_type: &str, nullable: bool) -> ColumnDefinitionRequest {
    ColumnDefinitionRequest {
        name: name.to_string(),
        data_type: data_type.to_string(),
        nullable,
        default: None,
        comment: None,
        generated: None,
        identity: None,
    }
}

fn base_table(database: &str, schema: Option<&str>, table: &str) -> TableRef {
    TableRef::new(Some(database), schema, table).expect("table ref")
}

async fn connected(engine: DatabaseEngine, version: &str) -> FakeDatabaseAdapter {
    let mut adapter = FakeDatabaseAdapter::new(engine, Some(version));
    adapter
        .connect(DatabaseConnectRequest {
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
            read_only: false,
            ssl_mode: None,
            connect_timeout: Duration::from_secs(5),
        })
        .await
        .expect("fake connect");
    adapter
}

async fn render_golden(
    engine: DatabaseEngine,
    version: &str,
    cases: Vec<(&str, StructureActionRequest)>,
) -> String {
    let adapter = connected(engine, version).await;
    let mut output = String::new();
    for (label, request) in cases {
        let preview = preview_structure_with_adapter(&adapter, &request)
            .await
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        output.push_str("-- ");
        output.push_str(label);
        output.push('\n');
        output.push_str(&preview.ddl);
        output.push_str(";\n\n");
    }
    output
}

fn mysql_cases() -> Vec<(&'static str, StructureActionRequest)> {
    let database = Some("app db".to_string());
    let mut id = column("ID", "bigint unsigned", false);
    id.identity = Some(ColumnIdentity::AutoIncrement);
    let mut slug = column("Slug Value", "varchar(180)", true);
    slug.generated = Some(GeneratedColumnRequest {
        expression: "lower(`Email Address`)".to_string(),
        stored: true,
    });
    let mut email = column("Email Address", "varchar(320)", false);
    email.default = Some(ColumnDefaultRequest::String {
        value: "n/a".to_string(),
    });
    email.comment = Some("Login address".to_string());

    vec![
        (
            "create_table users",
            StructureActionRequest::CreateTable {
                database: database.clone(),
                schema: None,
                name: "Users".to_string(),
                columns: vec![id, slug, email],
                primary_key: vec!["ID".to_string()],
            },
        ),
        (
            "rename_table users to accounts",
            StructureActionRequest::RenameTable {
                database: database.clone(),
                schema: None,
                name: "Users".to_string(),
                new_name: "Accounts".to_string(),
            },
        ),
        (
            "add_column display_name",
            StructureActionRequest::AddColumn {
                database: database.clone(),
                schema: None,
                table: "Users".to_string(),
                column: {
                    let mut added = column("Display Name", "varchar(120)", false);
                    added.default = Some(ColumnDefaultRequest::String {
                        value: "Robert'; DROP TABLE users; --".to_string(),
                    });
                    added
                },
                position: Some(ColumnPositionRequest::After {
                    column: "Email Address".to_string(),
                }),
            },
        ),
        (
            "alter_column email",
            StructureActionRequest::AlterColumn {
                database: database.clone(),
                schema: None,
                table: "Users".to_string(),
                name: "Email Address".to_string(),
                data_type: Some("varchar(500)".to_string()),
                nullable: Some(false),
                default: Some(ColumnDefaultRequest::Null),
                drop_default: false,
                comment: Some("Primary address".to_string()),
                drop_comment: false,
            },
        ),
        (
            "rename_column email",
            StructureActionRequest::RenameColumn {
                database: database.clone(),
                schema: None,
                table: "Users".to_string(),
                name: "Email Address".to_string(),
                new_name: "Email".to_string(),
            },
        ),
        (
            "reorder_column id first",
            StructureActionRequest::ReorderColumn {
                database: database.clone(),
                schema: None,
                table: "Users".to_string(),
                column: column("ID", "bigint unsigned", false),
                position: ColumnPositionRequest::First,
            },
        ),
        (
            "create_index users_email",
            StructureActionRequest::CreateIndex {
                database: database.clone(),
                schema: None,
                table: "Users".to_string(),
                name: "Users Email Index".to_string(),
                columns: vec!["Email".to_string()],
                unique: true,
                method: Some("btree".to_string()),
                predicate: None,
            },
        ),
        (
            "rename_index users_email",
            StructureActionRequest::RenameIndex {
                database: database.clone(),
                schema: None,
                table: "Users".to_string(),
                name: "Users Email Index".to_string(),
                new_name: "users_email_idx".to_string(),
            },
        ),
        (
            "add_foreign_key orders_user",
            StructureActionRequest::AddConstraint {
                database: database.clone(),
                schema: None,
                table: "Orders".to_string(),
                constraint: ConstraintRequest::ForeignKey {
                    name: "orders_user_fk".to_string(),
                    columns: vec!["User ID".to_string()],
                    referenced_schema: database.clone(),
                    referenced_table: "Users".to_string(),
                    referenced_columns: vec!["ID".to_string()],
                    on_delete: Some(ReferentialAction::Cascade),
                    on_update: Some(ReferentialAction::NoAction),
                },
            },
        ),
        (
            "drop_foreign_key orders_user",
            StructureActionRequest::DropConstraint {
                database: database.clone(),
                schema: None,
                table: "Orders".to_string(),
                name: "orders_user_fk".to_string(),
                kind: serverdesk_lib::database::ConstraintKind::ForeignKey,
            },
        ),
        (
            "drop_table accounts",
            StructureActionRequest::DropTable {
                database,
                schema: None,
                name: "Accounts".to_string(),
            },
        ),
    ]
}

fn postgres_cases() -> Vec<(&'static str, StructureActionRequest)> {
    let database = Some("app".to_string());
    let schema = Some("public".to_string());
    let mut id = column("ID", "bigint", false);
    id.identity = Some(ColumnIdentity::ByDefault);
    let mut slug = column("Slug Value", "text", false);
    slug.generated = Some(GeneratedColumnRequest {
        expression: "lower(\"Email Address\")".to_string(),
        stored: true,
    });
    let mut email = column("Email Address", "varchar(320)", false);
    email.default = Some(ColumnDefaultRequest::String {
        value: "n/a".to_string(),
    });
    email.comment = Some("Login address".to_string());

    vec![
        (
            "create_table users",
            StructureActionRequest::CreateTable {
                database: database.clone(),
                schema: schema.clone(),
                name: "Users".to_string(),
                columns: vec![id, slug, email],
                primary_key: vec!["ID".to_string()],
            },
        ),
        (
            "rename_table users to accounts",
            StructureActionRequest::RenameTable {
                database: database.clone(),
                schema: schema.clone(),
                name: "Users".to_string(),
                new_name: "Accounts".to_string(),
            },
        ),
        (
            "add_column display_name",
            StructureActionRequest::AddColumn {
                database: database.clone(),
                schema: schema.clone(),
                table: "Users".to_string(),
                column: {
                    let mut added = column("Display Name", "varchar(120)", false);
                    added.default = Some(ColumnDefaultRequest::Boolean { value: false });
                    added.comment = Some("Shown in lists".to_string());
                    added
                },
                position: None,
            },
        ),
        (
            "alter_column email",
            StructureActionRequest::AlterColumn {
                database: database.clone(),
                schema: schema.clone(),
                table: "Users".to_string(),
                name: "Email Address".to_string(),
                data_type: Some("varchar(500)".to_string()),
                nullable: Some(false),
                default: Some(ColumnDefaultRequest::String {
                    value: "n/a".to_string(),
                }),
                drop_default: false,
                comment: Some("Primary address".to_string()),
                drop_comment: false,
            },
        ),
        (
            "rename_column email",
            StructureActionRequest::RenameColumn {
                database: database.clone(),
                schema: schema.clone(),
                table: "Users".to_string(),
                name: "Email Address".to_string(),
                new_name: "Email".to_string(),
            },
        ),
        (
            "create_partial_index users_active",
            StructureActionRequest::CreateIndex {
                database: database.clone(),
                schema: schema.clone(),
                table: "Users".to_string(),
                name: "Users Active Index".to_string(),
                columns: vec!["Email".to_string()],
                unique: true,
                method: Some("btree".to_string()),
                predicate: Some("\"active\" = TRUE".to_string()),
            },
        ),
        (
            "add_check_constraint users_email",
            StructureActionRequest::AddConstraint {
                database: database.clone(),
                schema: schema.clone(),
                table: "Users".to_string(),
                constraint: ConstraintRequest::Check {
                    name: "users_email_check".to_string(),
                    expression: "\"Email\" <> ''".to_string(),
                },
            },
        ),
        (
            "drop_check_constraint users_email",
            StructureActionRequest::DropConstraint {
                database: database.clone(),
                schema: schema.clone(),
                table: "Users".to_string(),
                name: "users_email_check".to_string(),
                kind: serverdesk_lib::database::ConstraintKind::Check,
            },
        ),
        (
            "drop_table accounts",
            StructureActionRequest::DropTable {
                database,
                schema,
                name: "Accounts".to_string(),
            },
        ),
    ]
}

#[tokio::test]
async fn mysql_ddl_matches_the_golden_fixture() {
    let actual = render_golden(DatabaseEngine::Mysql, "8.0.36", mysql_cases()).await;
    assert_eq!(
        actual, MYSQL_GOLDEN,
        "MySQL structure DDL changed unexpectedly"
    );
}

#[tokio::test]
async fn postgres_ddl_matches_the_golden_fixture() {
    let actual = render_golden(DatabaseEngine::Postgresql, "16.2", postgres_cases()).await;
    assert_eq!(
        actual, POSTGRES_GOLDEN,
        "PostgreSQL structure DDL changed unexpectedly"
    );
}

#[tokio::test]
async fn previews_carry_object_destructive_and_quoting_metadata() {
    let mysql = connected(DatabaseEngine::Mysql, "8.0.36").await;
    let mut add = column("Mixed Case`Name", "varchar(20)", false);
    add.default = Some(ColumnDefaultRequest::String {
        value: "quoted 'value'".to_string(),
    });
    let request = StructureActionRequest::AddColumn {
        database: Some("app".to_string()),
        schema: None,
        table: "Mixed Table".to_string(),
        column: add,
        position: None,
    };
    let preview = preview_structure_with_adapter(&mysql, &request)
        .await
        .expect("preview");

    assert_eq!(preview.engine, DatabaseEngine::Mysql);
    assert_eq!(preview.object.table, "Mixed Table");
    assert_eq!(preview.object.name.as_deref(), Some("Mixed Case`Name"));
    assert_eq!(preview.action.as_str(), "add_column");
    assert!(!preview.destructive);
    assert!(!preview.requires_confirmation);
    assert!(preview.ddl.contains("`Mixed Case``Name`"));
    assert!(preview.ddl.contains("DEFAULT 'quoted ''value'''"));
    assert!(!preview.ddl.contains("?"));
    assert!(!preview.ddl.contains("$1"));

    let drop = StructureActionRequest::DropTable {
        database: Some("app".to_string()),
        schema: None,
        name: "Mixed Table".to_string(),
    };
    let preview = preview_structure_with_adapter(&mysql, &drop)
        .await
        .expect("drop preview");
    assert!(preview.destructive);
    assert!(preview.requires_confirmation);
}

#[tokio::test]
async fn unsupported_structure_actions_fail_before_ddl_open() {
    let mysql = connected(DatabaseEngine::Mysql, "5.7.44").await;
    let check = StructureActionRequest::AddConstraint {
        database: Some("app".to_string()),
        schema: None,
        table: "users".to_string(),
        constraint: ConstraintRequest::Check {
            name: "users_email_check".to_string(),
            expression: "email <> ''".to_string(),
        },
    };
    let error = preview_structure_with_adapter(&mysql, &check)
        .await
        .expect_err("MySQL 5.7 has no check constraints");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert!(error.message.contains("create_check_constraint"));

    let postgres = connected(DatabaseEngine::Postgresql, "9.6.24").await;
    let identity = StructureActionRequest::AddColumn {
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
        table: "users".to_string(),
        column: {
            let mut id = column("id", "bigint", false);
            id.identity = Some(ColumnIdentity::Always);
            id
        },
        position: None,
    };
    let error = preview_structure_with_adapter(&postgres, &identity)
        .await
        .expect_err("PostgreSQL 9.6 has no identity columns");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);

    let reorder = StructureActionRequest::ReorderColumn {
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
        table: "users".to_string(),
        column: column("email", "text", false),
        position: ColumnPositionRequest::First,
    };
    let error = preview_structure_with_adapter(&postgres, &reorder)
        .await
        .expect_err("PostgreSQL cannot reorder stored columns");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
}

#[tokio::test]
async fn preview_requires_a_live_connection_but_never_executes_ddl() {
    let adapter = FakeDatabaseAdapter::new(DatabaseEngine::Mysql, Some("8.0.36"));
    let request = StructureActionRequest::DropTable {
        database: Some("app".to_string()),
        schema: None,
        name: "users".to_string(),
    };
    let error = preview_structure_with_adapter(&adapter, &request)
        .await
        .expect_err("preview requires a connection");
    assert_eq!(error.kind, DatabaseErrorKind::Connection);
    assert_eq!(adapter.connect_count(), 0);
}

#[tokio::test]
async fn placeholder_like_identifiers_are_quoted_not_bound_as_values() {
    let mysql = connected(DatabaseEngine::Mysql, "8.0.36").await;
    let request = StructureActionRequest::CreateTable {
        database: Some("app".to_string()),
        schema: None,
        name: "Params".to_string(),
        columns: vec![column("?", "int", false), column("$1", "int", true)],
        primary_key: Vec::new(),
    };
    let preview = preview_structure_with_adapter(&mysql, &request)
        .await
        .expect("preview");
    assert!(preview.ddl.contains("`?`"));
    assert!(preview.ddl.contains("`$1`"));
    assert!(!preview.ddl.contains(';'));

    let postgres = connected(DatabaseEngine::Postgresql, "16.2").await;
    let preview = preview_structure_with_adapter(
        &postgres,
        &StructureActionRequest::CreateTable {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "Params".to_string(),
            columns: vec![column("?", "int", false), column("$1", "int", true)],
            primary_key: Vec::new(),
        },
    )
    .await
    .expect("preview");
    assert!(preview.ddl.contains("\"?\""));
    assert!(preview.ddl.contains("\"$1\""));
    assert!(!preview.ddl.contains(';'));
}

#[test]
fn table_ref_fixture_keeps_quoted_identifiers_intact() {
    let table = base_table("app db", None, "Mixed Case");
    assert_eq!(table.name.as_str(), "Mixed Case");
}
