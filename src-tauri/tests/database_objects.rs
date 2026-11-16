//! View, routine, trigger, sequence, and materialized-view tests.
//!
//! The deterministic fake adapter exercises the same validation, capability,
//! preview, confirmation, permission, execution, and verification path as the
//! live adapters. Engine planners are checked separately for exact DDL.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        DatabaseAdapter, DatabaseConnectRequest, DatabaseErrorKind, DatabaseObjectKind,
        DatabasePassword, FakeDatabaseAdapter, MetadataScope, ObjectActionKind,
        ObjectActionRequest, ObjectRefRequest, RoutineKind, SequenceDataType, TriggerBodyRequest,
        TriggerEvent, TriggerTiming, execute_object_with_adapter, preview_object_with_adapter,
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

fn view_request(engine: DatabaseEngine, name: &str, or_replace: bool) -> ObjectActionRequest {
    ObjectActionRequest::CreateView {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: name.to_string(),
        query: "SELECT id, email FROM users WHERE active".to_string(),
        or_replace,
    }
}

fn drop_view_request(
    engine: DatabaseEngine,
    name: &str,
    materialized: bool,
) -> ObjectActionRequest {
    ObjectActionRequest::DropView {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: name.to_string(),
        materialized,
    }
}

fn routine_request(engine: DatabaseEngine, or_replace: bool) -> ObjectActionRequest {
    ObjectActionRequest::CreateRoutine {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: "user_email".to_string(),
        routine_kind: RoutineKind::Function,
        arguments: Some("user_id bigint".to_string()),
        return_type: Some(if engine == DatabaseEngine::Mysql {
            "varchar(255)".to_string()
        } else {
            "text".to_string()
        }),
        language: Some(if engine == DatabaseEngine::Mysql {
            "SQL".to_string()
        } else {
            "sql".to_string()
        }),
        body: "BEGIN\n  RETURN NULL;\nEND".to_string(),
        or_replace,
    }
}

fn trigger_request(engine: DatabaseEngine, or_replace: bool) -> ObjectActionRequest {
    ObjectActionRequest::CreateTrigger {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: "users_audit".to_string(),
        table: "users".to_string(),
        timing: TriggerTiming::After,
        events: vec![TriggerEvent::Insert],
        body: if engine == DatabaseEngine::Mysql {
            TriggerBodyRequest::Statement {
                body: "INSERT INTO audit (user_id) VALUES (NEW.id)".to_string(),
            }
        } else {
            TriggerBodyRequest::Function {
                schema: Some("public".to_string()),
                name: "audit_users".to_string(),
                arguments: None,
            }
        },
        or_replace,
    }
}

fn sequence_request(engine: DatabaseEngine) -> ObjectActionRequest {
    ObjectActionRequest::CreateSequence {
        database: Some("app".to_string()),
        schema: (engine == DatabaseEngine::Postgresql).then(|| "public".to_string()),
        name: "orders_id_seq".to_string(),
        data_type: Some(SequenceDataType::BigInt),
        start: Some(100),
        increment: Some(5),
        minimum: Some(1),
        maximum: None,
        cache: Some(10),
        cycle: false,
    }
}

fn scope(engine: DatabaseEngine) -> MetadataScope {
    MetadataScope::new(
        Some("app"),
        (engine == DatabaseEngine::Postgresql).then_some("public"),
    )
    .expect("scope")
}

#[tokio::test]
async fn mysql_fixture_metadata_lists_views_routines_and_triggers() {
    let adapter = connected(DatabaseEngine::Mysql, false).await;
    let scope = scope(DatabaseEngine::Mysql);

    let views = adapter
        .list_views(&scope, Default::default())
        .await
        .expect("views");
    assert!(views.items.iter().any(|view| view.name == "active_users"));

    let routines = adapter
        .list_routines(&scope, Default::default())
        .await
        .expect("routines");
    let routine = routines
        .items
        .iter()
        .find(|routine| routine.name == "user_email")
        .expect("routine fixture");
    assert_eq!(routine.kind, RoutineKind::Function);
    assert_eq!(routine.language.value().map(String::as_str), Some("SQL"));

    let triggers = adapter
        .list_triggers(&scope, Default::default())
        .await
        .expect("triggers");
    let trigger = triggers
        .items
        .iter()
        .find(|trigger| trigger.name == "users_audit")
        .expect("trigger fixture");
    assert_eq!(trigger.table, "users");
    assert_eq!(trigger.timing, TriggerTiming::After);
    assert_eq!(trigger.events, vec![TriggerEvent::Insert]);
}

#[tokio::test]
async fn postgres_fixture_metadata_includes_materialized_views_and_sequences() {
    let adapter = connected(DatabaseEngine::Postgresql, false).await;
    let scope = scope(DatabaseEngine::Postgresql);

    let views = adapter
        .list_views(&scope, Default::default())
        .await
        .expect("views");
    assert!(views.items.iter().any(|view| view.name == "daily_totals"
        && view.kind == serverdesk_lib::database::ViewKind::MaterializedView));

    let routines = adapter
        .list_routines(&scope, Default::default())
        .await
        .expect("routines");
    assert!(
        routines
            .items
            .iter()
            .any(|routine| routine.name == "user_email")
    );

    let triggers = adapter
        .list_triggers(&scope, Default::default())
        .await
        .expect("triggers");
    assert!(
        triggers
            .items
            .iter()
            .any(|trigger| trigger.name == "users_audit")
    );

    let sequences = adapter
        .list_sequences(&scope, Default::default())
        .await
        .expect("sequences");
    assert!(
        sequences
            .items
            .iter()
            .any(|sequence| sequence.name == "users_id_seq")
    );
}

#[tokio::test]
async fn object_definitions_include_view_query_trigger_events_and_sequence_details() {
    let adapter = connected(DatabaseEngine::Postgresql, false).await;

    let view = adapter
        .object_definition(
            &ObjectRefRequest {
                kind: DatabaseObjectKind::View,
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "active_users".to_string(),
                routine_kind: None,
                arguments: None,
                table: None,
            }
            .validate()
            .expect("view ref"),
        )
        .await
        .expect("view definition");
    assert!(
        view.view_query
            .value()
            .is_some_and(|query| query.contains("SELECT"))
    );
    assert!(
        view.definition
            .value()
            .is_some_and(|definition| definition.contains("CREATE VIEW"))
    );

    let trigger = adapter
        .object_definition(
            &ObjectRefRequest {
                kind: DatabaseObjectKind::Trigger,
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "users_audit".to_string(),
                routine_kind: None,
                arguments: None,
                table: Some("users".to_string()),
            }
            .validate()
            .expect("trigger ref"),
        )
        .await
        .expect("trigger definition");
    let details = trigger.trigger.expect("trigger details");
    assert_eq!(details.table, "users");
    assert_eq!(details.timing, TriggerTiming::After);
    assert_eq!(details.events, vec![TriggerEvent::Insert]);
    assert!(details.statement.is_available());
    assert!(details.function.is_available());

    let sequence = adapter
        .object_definition(
            &ObjectRefRequest {
                kind: DatabaseObjectKind::Sequence,
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "users_id_seq".to_string(),
                routine_kind: None,
                arguments: None,
                table: None,
            }
            .validate()
            .expect("sequence ref"),
        )
        .await
        .expect("sequence definition");
    let details = sequence.sequence.expect("sequence details");
    assert_eq!(details.increment.value().map(String::as_str), Some("1"));
    assert_eq!(
        details.current_value.value().map(String::as_str),
        Some("42")
    );
}

#[tokio::test]
async fn mysql_capabilities_reject_sequences_and_materialized_views() {
    let adapter = connected(DatabaseEngine::Mysql, false).await;

    let sequence = sequence_request(DatabaseEngine::Mysql);
    let error = preview_object_with_adapter(&adapter, &sequence)
        .await
        .expect_err("MySQL sequences are unsupported");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
    assert!(error.message.contains("create_sequence"));

    let materialized = drop_view_request(DatabaseEngine::Mysql, "daily_totals", true);
    let error = preview_object_with_adapter(&adapter, &materialized)
        .await
        .expect_err("MySQL materialized views are unsupported");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);
}

#[tokio::test]
async fn mysql_trigger_planner_rejects_multiple_events_and_statement_bodies_are_rendered() {
    let adapter = connected(DatabaseEngine::Mysql, false).await;
    let request = ObjectActionRequest::CreateTrigger {
        database: Some("app".to_string()),
        schema: None,
        name: "users_audit".to_string(),
        table: "users".to_string(),
        timing: TriggerTiming::Before,
        events: vec![TriggerEvent::Insert, TriggerEvent::Update],
        body: TriggerBodyRequest::Statement {
            body: "SET NEW.updated_at = NOW()".to_string(),
        },
        or_replace: false,
    };
    let error = preview_object_with_adapter(&adapter, &request)
        .await
        .expect_err("MySQL supports one event per trigger");
    assert_eq!(error.kind, DatabaseErrorKind::Capability);

    let preview =
        preview_object_with_adapter(&adapter, &trigger_request(DatabaseEngine::Mysql, false))
            .await
            .expect("trigger preview");
    assert_eq!(preview.action, ObjectActionKind::CreateTrigger);
    assert!(
        preview
            .ddl
            .contains("CREATE TRIGGER `app`.`users_audit` AFTER INSERT ON `app`.`users`")
    );
    assert!(preview.ddl.contains("INSERT INTO audit"));
    assert!(!preview.requires_confirmation);
}

#[tokio::test]
async fn object_ddl_previews_are_engine_specific() {
    let mysql = connected(DatabaseEngine::Mysql, false).await;
    let view = preview_object_with_adapter(
        &mysql,
        &view_request(DatabaseEngine::Mysql, "active_users", true),
    )
    .await
    .expect("mysql view preview");
    assert_eq!(view.action, ObjectActionKind::ReplaceView);
    assert!(
        view.ddl
            .starts_with("CREATE OR REPLACE VIEW `app`.`active_users` AS")
    );
    assert!(view.requires_confirmation);

    let routine =
        preview_object_with_adapter(&mysql, &routine_request(DatabaseEngine::Mysql, false))
            .await
            .expect("mysql routine preview");
    assert!(
        routine
            .ddl
            .starts_with("CREATE FUNCTION `app`.`user_email`(user_id bigint) RETURNS")
    );
    assert!(!routine.ddl.contains("CREATE OR REPLACE"));

    let postgres = connected(DatabaseEngine::Postgresql, false).await;
    let view = preview_object_with_adapter(
        &postgres,
        &view_request(DatabaseEngine::Postgresql, "active_users", true),
    )
    .await
    .expect("postgres view preview");
    assert!(
        view.ddl
            .starts_with("CREATE OR REPLACE VIEW \"public\".\"active_users\" AS")
    );

    let routine = preview_object_with_adapter(
        &postgres,
        &routine_request(DatabaseEngine::Postgresql, true),
    )
    .await
    .expect("postgres routine preview");
    assert!(
        routine
            .ddl
            .starts_with("CREATE OR REPLACE FUNCTION \"public\".\"user_email\"")
    );
    assert!(routine.ddl.contains("LANGUAGE sql"));
    assert!(routine.requires_confirmation);

    let trigger = preview_object_with_adapter(
        &postgres,
        &trigger_request(DatabaseEngine::Postgresql, true),
    )
    .await
    .expect("postgres trigger preview");
    assert!(
        trigger
            .ddl
            .contains("DROP TRIGGER IF EXISTS \"users_audit\" ON \"public\".\"users\"")
    );
    assert!(
        trigger
            .ddl
            .contains("EXECUTE FUNCTION \"public\".\"audit_users\"()")
    );

    let sequence =
        preview_object_with_adapter(&postgres, &sequence_request(DatabaseEngine::Postgresql))
            .await
            .expect("postgres sequence preview");
    assert!(
        sequence
            .ddl
            .contains("CREATE SEQUENCE \"public\".\"orders_id_seq\" AS bigint")
    );
    assert!(sequence.ddl.contains("INCREMENT BY 5"));
    assert!(sequence.ddl.contains("START WITH 100"));
}

#[tokio::test]
async fn drop_and_replace_require_confirmation_before_any_statement_runs() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, false).await;

        let drop = drop_view_request(engine, "active_users", false);
        let error = execute_object_with_adapter(&mut adapter, drop, false)
            .await
            .expect_err("unconfirmed drop");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
        assert_eq!(adapter.object_execution_count(), 0);

        let replace = view_request(engine, "active_users", true);
        let error = execute_object_with_adapter(&mut adapter, replace, false)
            .await
            .expect_err("unconfirmed replace");
        assert_eq!(error.kind, DatabaseErrorKind::Cancelled);
        assert_eq!(adapter.object_execution_count(), 0);

        let create = view_request(engine, "confirmed_view", false);
        let result = execute_object_with_adapter(&mut adapter, create, false)
            .await
            .expect("create does not require confirmation");
        assert!(result.verified);
        assert_eq!(adapter.object_execution_count(), 1);
    }
}

#[tokio::test]
async fn read_only_profiles_reject_object_changes_before_execution() {
    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let mut adapter = connected(engine, true).await;
        let drop = drop_view_request(engine, "active_users", false);
        let error = execute_object_with_adapter(&mut adapter, drop, true)
            .await
            .expect_err("read-only profile");
        assert_eq!(error.kind, DatabaseErrorKind::Permission);
        assert_eq!(adapter.object_execution_count(), 0);
    }
}

#[tokio::test]
async fn execution_verifies_created_objects_and_detects_skipped_work() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;

    let create = ObjectActionRequest::CreateView {
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
        name: "verified_view".to_string(),
        query: "SELECT 1 AS ok".to_string(),
        or_replace: false,
    };
    let result = execute_object_with_adapter(&mut adapter, create, false)
        .await
        .expect("create view");
    assert!(result.verified);
    assert!(result.definition.is_some());
    assert_eq!(result.action, ObjectActionKind::CreateView);

    let mut skipped = FakeDatabaseAdapter::new(DatabaseEngine::Postgresql, Some("16.2"));
    skipped
        .connect(connect_request(DatabaseEngine::Postgresql, false))
        .await
        .expect("connect");
    skipped = skipped.with_skipped_object_execution();
    let create = ObjectActionRequest::CreateView {
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
        name: "missing_view".to_string(),
        query: "SELECT 1".to_string(),
        or_replace: false,
    };
    let result = execute_object_with_adapter(&mut skipped, create, false)
        .await
        .expect("skipped execution still returns a result");
    assert!(!result.verified);
    assert!(result.verification_error.is_some());
}

#[tokio::test]
async fn drop_execution_verifies_absence_and_refresh_keeps_materialized_view() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;

    let drop = drop_view_request(DatabaseEngine::Postgresql, "active_users", false);
    let result = execute_object_with_adapter(&mut adapter, drop, true)
        .await
        .expect("drop view");
    assert!(result.verified);

    let refresh = ObjectActionRequest::RefreshMaterializedView {
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
        name: "daily_totals".to_string(),
    };
    let result = execute_object_with_adapter(&mut adapter, refresh, false)
        .await
        .expect("refresh materialized view");
    assert!(result.verified);
    assert_eq!(result.action, ObjectActionKind::RefreshMaterializedView);
}

#[tokio::test]
async fn sequence_management_previews_and_verifies_postgres_changes() {
    let mut adapter = connected(DatabaseEngine::Postgresql, false).await;

    let create = sequence_request(DatabaseEngine::Postgresql);
    let result = execute_object_with_adapter(&mut adapter, create, false)
        .await
        .expect("create sequence");
    assert!(result.verified);
    let details = result
        .definition
        .expect("sequence definition")
        .sequence
        .expect("details");
    assert_eq!(details.increment.value().map(String::as_str), Some("5"));

    let alter = ObjectActionRequest::AlterSequence {
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
        name: "orders_id_seq".to_string(),
        increment: Some(2),
        minimum: None,
        maximum: None,
        restart: Some(500),
        cache: None,
        cycle: Some(true),
    };
    let result = execute_object_with_adapter(&mut adapter, alter, false)
        .await
        .expect("alter sequence");
    assert!(result.verified);
    let details = result
        .definition
        .expect("sequence definition")
        .sequence
        .expect("details");
    assert_eq!(details.increment.value().map(String::as_str), Some("2"));
    assert_eq!(
        details.current_value.value().map(String::as_str),
        Some("500")
    );
    assert_eq!(details.cycle.value(), Some(&true));
}

#[tokio::test]
async fn invalid_definitions_are_rejected_before_preview_or_execution() {
    let adapter = connected(DatabaseEngine::Postgresql, false).await;

    let multi_statement = ObjectActionRequest::CreateView {
        database: None,
        schema: Some("public".to_string()),
        name: "bad_view".to_string(),
        query: "SELECT 1; DROP TABLE users".to_string(),
        or_replace: false,
    };
    let error = preview_object_with_adapter(&adapter, &multi_statement)
        .await
        .expect_err("multi-statement view");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);

    let ddl = ObjectActionRequest::CreateView {
        database: None,
        schema: Some("public".to_string()),
        name: "bad_view".to_string(),
        query: "DROP TABLE users".to_string(),
        or_replace: false,
    };
    let error = preview_object_with_adapter(&adapter, &ddl)
        .await
        .expect_err("non-select view");
    assert_eq!(error.kind, DatabaseErrorKind::Validation);
}
