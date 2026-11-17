//! Database health sampling tests.
//!
//! The fake adapter proves the shared contract and the metadata shapes without
//! a database container. SQL safety tests assert that sampling uses catalog
//! metadata only: no row counts, table scans, or statistics maintenance.

use std::time::Duration;

use serverdesk_lib::{
    database::{
        Bounded, DatabaseAdapter, DatabaseConnectRequest, DatabaseError, DatabaseErrorKind,
        DatabaseHealthMetrics, DatabaseHealthSnapshot, DatabasePassword, DatabaseTableSize,
        FakeDatabaseAdapter, MetadataValue, TableKind,
    },
    storage::DatabaseEngine,
};

fn connect_request(engine: DatabaseEngine) -> DatabaseConnectRequest {
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
        read_only: false,
        ssl_mode: None,
        connect_timeout: Duration::from_secs(5),
    }
}

async fn connected(engine: DatabaseEngine) -> FakeDatabaseAdapter {
    let mut adapter = FakeDatabaseAdapter::new(
        engine,
        Some(if engine == DatabaseEngine::Mysql {
            "8.0.36"
        } else {
            "16.2"
        }),
    );
    adapter
        .connect(connect_request(engine))
        .await
        .expect("fake connect");
    adapter
}

#[tokio::test]
async fn mysql_health_matches_catalog_fixtures() {
    let adapter = connected(DatabaseEngine::Mysql).await;
    let metrics = adapter.health_metrics(12).await.expect("health metrics");

    assert_eq!(metrics.database_size_bytes, MetadataValue::available(4_096));
    let tables = metrics
        .table_sizes
        .value()
        .expect("table size list available");
    assert_eq!(tables.items.len(), 2);
    assert_eq!(tables.items[0].name, "orders");
    assert_eq!(tables.items[0].size_bytes, MetadataValue::available(16_384));
    assert_eq!(tables.items[0].estimated_rows, MetadataValue::available(0));
    assert_eq!(
        tables.items[0].metadata_refreshed_at,
        MetadataValue::Unavailable
    );
    assert_eq!(tables.items[1].name, "users");
    assert_eq!(tables.items[1].kind, TableKind::Table);
}

#[tokio::test]
async fn postgres_health_reports_statistics_refresh_time_and_sorts_by_size() {
    let adapter = connected(DatabaseEngine::Postgresql).await;
    let metrics = adapter.health_metrics(12).await.expect("health metrics");

    assert_eq!(metrics.database_size_bytes, MetadataValue::available(8_192));
    let tables = metrics.table_sizes.value().expect("table sizes");
    assert_eq!(tables.items.len(), 2);
    assert!(
        tables
            .items
            .iter()
            .all(|table| table.kind == TableKind::Table)
    );
    assert_eq!(tables.items[0].name, "orders");
    assert_eq!(
        tables.items[0].metadata_refreshed_at,
        MetadataValue::Unavailable
    );
    assert_eq!(
        tables.items[1].metadata_refreshed_at,
        MetadataValue::available("2026-01-02T03:04:05Z".to_string())
    );
}

#[tokio::test]
async fn health_table_lists_are_bounded_and_disclose_truncation() {
    let adapter = connected(DatabaseEngine::Mysql).await;
    let metrics = adapter.health_metrics(1).await.expect("health metrics");
    let tables = metrics.table_sizes.value().expect("table sizes");

    assert_eq!(tables.limit, 1);
    assert_eq!(tables.items.len(), 1);
    assert!(tables.truncated);
}

#[tokio::test]
async fn unsupported_metrics_are_unavailable_instead_of_zero() {
    let metrics = DatabaseHealthMetrics {
        database_size_bytes: MetadataValue::Unavailable,
        table_sizes: MetadataValue::available(Bounded::new(
            vec![DatabaseTableSize {
                database: Some("app".to_string()),
                schema: Some("public".to_string()),
                name: "users".to_string(),
                kind: TableKind::Table,
                size_bytes: MetadataValue::Unavailable,
                estimated_rows: MetadataValue::Unavailable,
                metadata_refreshed_at: MetadataValue::Unavailable,
            }],
            12,
        )),
    };
    let adapter = connected(DatabaseEngine::Postgresql)
        .await
        .with_health_metrics(metrics);

    let sampled = adapter.health_metrics(12).await.expect("health metrics");
    assert_eq!(sampled.database_size_bytes, MetadataValue::Unavailable);
    let tables = sampled.table_sizes.value().expect("table sizes");
    assert_eq!(tables.items[0].size_bytes, MetadataValue::Unavailable);
    assert_eq!(tables.items[0].estimated_rows, MetadataValue::Unavailable);
}

#[tokio::test]
async fn health_sampling_propagates_connection_and_timeout_failures() {
    let adapter = connected(DatabaseEngine::Postgresql)
        .await
        .with_health_metrics_error(DatabaseError::timeout("The catalog read timed out."));

    let error = adapter
        .health_metrics(12)
        .await
        .expect_err("health failure");
    assert_eq!(error.kind, DatabaseErrorKind::Timeout);
}

#[test]
fn health_queries_are_catalog_only_and_bounded() {
    use serverdesk_lib::database::{mysql::sql as mysql_sql, postgres::sql as postgres_sql};

    for statement in [
        mysql_sql::HEALTH_DATABASE_SIZE,
        mysql_sql::HEALTH_TABLE_SIZES,
        postgres_sql::HEALTH_DATABASE_SIZE,
        postgres_sql::HEALTH_TABLE_SIZES,
    ] {
        let upper = statement.to_ascii_uppercase();
        assert!(!upper.contains("COUNT(*)"), "row counting is not allowed");
        assert!(
            !upper.contains("ANALYZE TABLE"),
            "statistics maintenance is not allowed"
        );
        assert!(!upper.contains("SELECT *"), "table data must not be read");
    }

    assert!(mysql_sql::HEALTH_TABLE_SIZES.contains("information_schema.TABLES"));
    assert!(mysql_sql::HEALTH_TABLE_SIZES.contains("LIMIT ?"));
    assert!(postgres_sql::HEALTH_TABLE_SIZES.contains("pg_total_relation_size"));
    assert!(postgres_sql::HEALTH_TABLE_SIZES.contains("has_table_privilege"));
    assert!(postgres_sql::HEALTH_TABLE_SIZES.contains("LIMIT $1"));
}

#[test]
fn health_snapshot_serialization_omits_secrets() {
    let metrics = DatabaseHealthMetrics {
        database_size_bytes: MetadataValue::available(1_024),
        table_sizes: MetadataValue::available(Bounded::new(Vec::new(), 12)),
    };
    let info = serverdesk_lib::database::DatabaseConnectionInfo {
        engine: DatabaseEngine::Postgresql,
        version: serverdesk_lib::database::DatabaseVersionInfo {
            version: serverdesk_lib::database::DatabaseVersion::new(16, 2, 0),
            raw: "PostgreSQL 16.2".to_string(),
        },
        database: Some("app".to_string()),
        read_only: false,
        capabilities: serverdesk_lib::database::DatabaseCapabilities::with_version(
            DatabaseEngine::Postgresql,
            Some(serverdesk_lib::database::DatabaseVersion::new(16, 2, 0)),
        )
        .snapshot(),
    };
    let snapshot = DatabaseHealthSnapshot::new(
        "profile-1".to_string(),
        info,
        MetadataValue::available(4),
        metrics,
        1_700_000_000_000,
    );
    let json = serde_json::to_string(&snapshot).expect("snapshot json");

    assert!(json.contains("\"sampledAt\":1700000000000"));
    assert!(json.contains("\"database\":\"app\""));
    assert!(!json.contains("password"));
    assert!(!json.contains("fixture-password"));
    assert!(!json.contains("SELECT"));
    assert!(!json.contains("127.0.0.1"));
}
