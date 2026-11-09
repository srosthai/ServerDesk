use serverdesk_lib::{
    core::history::{MAX_OUTPUT_SUMMARY_CHARS, record_operation, summarize_output},
    storage::{
        AuthType, Database, NewOperationRecord, NewServer, OperationCategory,
        OperationHistoryFilter, OperationResult,
    },
};

fn server_input(name: &str) -> NewServer {
    NewServer {
        name: name.to_string(),
        host: "198.51.100.24".to_string(),
        port: 22,
        username: "deploy".to_string(),
        auth_type: AuthType::Password,
        credential_ref: Some(format!("credential:{name}")),
        group_name: None,
        tags: Vec::new(),
        notes: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn record(
    database: &Database,
    server_id: Option<String>,
    category: OperationCategory,
    operation: &str,
    result: OperationResult,
    exit_code: Option<i32>,
    started_at: i64,
    output_summary: Option<&str>,
) {
    record_operation(
        database,
        &NewOperationRecord {
            server_id,
            project_id: None,
            category,
            operation: operation.to_string(),
            safe_summary: format!("{operation} summary"),
            path: Some("/var/www/storefront".to_string()),
            output_summary: output_summary.map(str::to_string),
            result,
            exit_code,
            started_at,
            duration_ms: 42,
        },
    )
    .expect("record operation");
}

#[test]
fn records_successful_and_failed_operations() {
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");

    record(
        &database,
        Some(server.id.clone()),
        OperationCategory::Service,
        "service-restart",
        OperationResult::Success,
        Some(0),
        1_700_000_100,
        Some("restarted nginx.service"),
    );
    record(
        &database,
        Some(server.id.clone()),
        OperationCategory::Service,
        "service-stop",
        OperationResult::Failure,
        Some(1),
        1_700_000_200,
        Some("sudo: a password is required"),
    );

    let history = database
        .list_operations_filtered(&OperationHistoryFilter::default())
        .expect("list history");

    assert_eq!(history.len(), 2);
    assert_eq!(history[0].operation, "service-stop");
    assert_eq!(history[0].result, OperationResult::Failure);
    assert_eq!(history[0].exit_code, Some(1));
    assert_eq!(history[1].operation, "service-restart");
    assert_eq!(history[1].result, OperationResult::Success);
    assert_eq!(history[1].started_at, 1_700_000_100);
    assert_eq!(history[1].duration_ms, 42);
}

#[test]
fn redacts_secrets_from_every_recorded_field() {
    let database = Database::in_memory().expect("in-memory database");
    let server = database
        .create_server(&server_input("Production"))
        .expect("create server");

    let safe_summary = "Deploy DB_PASSWORD=fixture-password passphrase: fixture-passphrase";
    let output_summary = "APP_KEY=base64:fixture-app-key\n\
        Authorization: Bearer fixture-bearer-token\n\
        DATABASE_URL=mysql://deploy:fixture-db-password@127.0.0.1/app\n\
        -----BEGIN OPENSSH PRIVATE KEY-----\n\
        fixture-private-key-body\n\
        -----END OPENSSH PRIVATE KEY-----\n\
        api_key=sk-live-fixture-token-123456";

    let stored = record_operation(
        &database,
        &NewOperationRecord {
            server_id: Some(server.id.clone()),
            project_id: None,
            category: OperationCategory::Command,
            operation: "remote-hostname".to_string(),
            safe_summary: safe_summary.to_string(),
            path: Some("/var/www/storefront?token=fixture-url-token".to_string()),
            output_summary: Some(output_summary.to_string()),
            result: OperationResult::Success,
            exit_code: Some(0),
            started_at: 1_700_000_000,
            duration_ms: 5,
        },
    )
    .expect("record sanitized operation");

    let serialized = serde_json::to_string(&stored).expect("serialized record");
    for secret in [
        "fixture-password",
        "fixture-passphrase",
        "fixture-app-key",
        "fixture-bearer-token",
        "fixture-db-password",
        "fixture-private-key-body",
        "fixture-url-token",
        "sk-live-fixture-token-123456",
    ] {
        assert!(
            !serialized.contains(secret),
            "history field exposed {secret:?}: {serialized}"
        );
    }
    assert!(serialized.contains("[redacted environment line]"));
    assert!(serialized.contains("[redacted private key]"));

    let history = database.list_operations(10).expect("list history");
    assert_eq!(history.len(), 1);
    for secret in ["fixture-password", "fixture-private-key-body"] {
        for field in [
            &history[0].operation,
            &history[0].safe_summary,
            history[0].path.as_deref().unwrap_or_default(),
            history[0].output_summary.as_deref().unwrap_or_default(),
        ] {
            assert!(!field.contains(secret));
        }
    }
}

#[test]
fn filters_by_server_category_result_and_date() {
    let database = Database::in_memory().expect("in-memory database");
    let production = database
        .create_server(&server_input("Production"))
        .expect("create production");
    let staging = database
        .create_server(&server_input("Staging"))
        .expect("create staging");

    record(
        &database,
        Some(production.id.clone()),
        OperationCategory::Service,
        "service-start",
        OperationResult::Success,
        Some(0),
        1_700_000_100,
        None,
    );
    record(
        &database,
        Some(production.id.clone()),
        OperationCategory::Artisan,
        "artisan-migrate-status",
        OperationResult::Failure,
        Some(1),
        1_700_000_200,
        None,
    );
    record(
        &database,
        Some(staging.id.clone()),
        OperationCategory::Sftp,
        "sftp-upload",
        OperationResult::Success,
        None,
        1_700_000_300,
        None,
    );

    let by_server = database
        .list_operations_filtered(&OperationHistoryFilter {
            server_id: Some(production.id.clone()),
            ..OperationHistoryFilter::default()
        })
        .expect("filter server");
    assert_eq!(by_server.len(), 2);
    assert!(
        by_server
            .iter()
            .all(|row| row.server_id.as_deref() == Some(production.id.as_str()))
    );

    let by_category = database
        .list_operations_filtered(&OperationHistoryFilter {
            category: Some(OperationCategory::Sftp),
            ..OperationHistoryFilter::default()
        })
        .expect("filter category");
    assert_eq!(by_category.len(), 1);
    assert_eq!(by_category[0].operation, "sftp-upload");

    let by_result = database
        .list_operations_filtered(&OperationHistoryFilter {
            result: Some(OperationResult::Failure),
            ..OperationHistoryFilter::default()
        })
        .expect("filter result");
    assert_eq!(by_result.len(), 1);
    assert_eq!(by_result[0].operation, "artisan-migrate-status");

    let by_date = database
        .list_operations_filtered(&OperationHistoryFilter {
            started_after: Some(1_700_000_150),
            started_before: Some(1_700_000_250),
            ..OperationHistoryFilter::default()
        })
        .expect("filter date");
    assert_eq!(by_date.len(), 1);
    assert_eq!(by_date[0].operation, "artisan-migrate-status");

    let combined = database
        .list_operations_filtered(&OperationHistoryFilter {
            server_id: Some(production.id),
            category: Some(OperationCategory::Service),
            result: Some(OperationResult::Success),
            ..OperationHistoryFilter::default()
        })
        .expect("filter combined");
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].operation, "service-start");
}

#[test]
fn history_survives_reopen_and_is_bounded_then_clearable() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("serverdesk.sqlite3");
    let database = Database::open(&path).expect("open database");

    for index in 0..1_005 {
        record(
            &database,
            None,
            OperationCategory::Command,
            &format!("remote-run-{index}"),
            OperationResult::Success,
            Some(0),
            1_700_000_000 + index,
            Some("ok"),
        );
    }

    let bounded = database
        .list_operations(2_000)
        .expect("list bounded history");
    assert_eq!(bounded.len(), 1_000);
    assert_eq!(bounded[0].operation, "remote-run-1004");
    assert_eq!(bounded[999].operation, "remote-run-5");
    assert!(!bounded.iter().any(|row| row.operation == "remote-run-0"));

    drop(database);
    let reopened = Database::open(&path).expect("reopen database");
    let persisted = reopened.list_operations(10).expect("list reopened history");
    assert_eq!(persisted.len(), 10);
    assert_eq!(persisted[0].operation, "remote-run-1004");

    reopened
        .clear_operation_history()
        .expect("clear operation history");
    assert!(
        reopened
            .list_operations(10)
            .expect("list after clear")
            .is_empty()
    );
}

#[test]
fn large_output_is_summarized_instead_of_stored_wholesale() {
    let output = (0..500)
        .map(|index| format!("line {index}: {}\n", "x".repeat(200)))
        .collect::<String>();
    let summary = summarize_output(&output, "").expect("summary");

    assert!(summary.chars().count() <= MAX_OUTPUT_SUMMARY_CHARS);
    assert!(summary.contains("line 0"));
    assert!(!summary.contains("line 499"));
}
