//! Lightweight database health samples.
//!
//! A sample combines the server version and capabilities already observed at
//! connect time, one bounded round-trip ping, and catalog metadata for the
//! active database. Adapters must never count rows, sample table data, or run
//! `ANALYZE` for this path: sizes and row estimates come from catalog
//! statistics and are therefore labeled as estimates in the UI.
//!
//! Bytes, row estimates, and statistics timestamps are [`MetadataValue`]s so an
//! engine that cannot report them, or an account without permission, receives
//! an explicit `unavailable` instead of a guessed zero.

use serde::{Deserialize, Serialize};

use crate::{
    core::{AppError, database_profiles::now_millis},
    storage::DatabaseEngine,
};

use super::{
    errors::DatabaseError,
    pagination::Bounded,
    service::DatabaseConnectionRegistry,
    traits::{DatabaseConnectionInfo, DatabaseVersionInfo, MetadataValue, TableKind},
};

/// Table sizes included in one sample when the caller does not choose a limit.
pub const DEFAULT_HEALTH_TABLE_LIMIT: u32 = 12;
/// Largest table-size list one sample will return. The query asks for one
/// extra row so truncation is disclosed without reading the rest of the
/// catalog.
pub const MAX_HEALTH_TABLE_LIMIT: u32 = 50;

/// One table's catalog size and row estimate. The size is the engine's
/// allocated bytes for the table and its indexes; it is not computed by
/// scanning rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseTableSize {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: TableKind,
    pub size_bytes: MetadataValue<u64>,
    /// Engine row estimate from catalog statistics, never an exact count.
    pub estimated_rows: MetadataValue<u64>,
    /// Server-reported time when statistics were last refreshed. `Unavailable`
    /// means the engine, storage engine, or account does not expose it.
    pub metadata_refreshed_at: MetadataValue<String>,
}

/// Catalog portion of one health sample. The full table list is bounded, and
/// the list itself can be unavailable when the account cannot read catalog
/// metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseHealthMetrics {
    pub database_size_bytes: MetadataValue<u64>,
    pub table_sizes: MetadataValue<Bounded<DatabaseTableSize>>,
}

/// One manual health snapshot. It contains no credentials, SQL, connection
/// string, query text, or driver diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseHealthSnapshot {
    pub profile_id: String,
    pub engine: DatabaseEngine,
    /// Active database reported by the live connection.
    pub database: Option<String>,
    pub version: DatabaseVersionInfo,
    /// Unix epoch milliseconds when the sample finished.
    pub sampled_at: i64,
    /// Round-trip ping measured for this sample.
    pub latency_ms: MetadataValue<u64>,
    pub database_size_bytes: MetadataValue<u64>,
    pub table_sizes: MetadataValue<Bounded<DatabaseTableSize>>,
}

impl DatabaseHealthSnapshot {
    pub fn new(
        profile_id: String,
        info: DatabaseConnectionInfo,
        latency_ms: MetadataValue<u64>,
        metrics: DatabaseHealthMetrics,
        sampled_at: i64,
    ) -> Self {
        Self {
            profile_id,
            engine: info.engine,
            database: info.database,
            version: info.version,
            sampled_at,
            latency_ms,
            database_size_bytes: metrics.database_size_bytes,
            table_sizes: metrics.table_sizes,
        }
    }
}

/// Builds health samples from the live connection registry. Sampling is always
/// explicit: no background timer, polling loop, or automatic refresh exists.
pub struct DatabaseHealthService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseHealthService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    /// Takes one sample for a connected profile. A ping permission failure
    /// records unavailable latency and still returns catalog metrics; a lost
    /// connection fails the whole sample so the UI can keep the previous one
    /// and show a refresh error.
    pub async fn snapshot(
        &self,
        profile_id: &str,
        limit: Option<u32>,
    ) -> Result<DatabaseHealthSnapshot, AppError> {
        let limit = limit
            .unwrap_or(DEFAULT_HEALTH_TABLE_LIMIT)
            .clamp(1, MAX_HEALTH_TABLE_LIMIT);
        let info = self.connections.info(profile_id).await?;
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;

        let latency_ms = match adapter.ping().await {
            Ok(ping) => MetadataValue::available(ping.latency_ms),
            Err(error)
                if matches!(
                    error.kind,
                    super::errors::DatabaseErrorKind::Permission
                        | super::errors::DatabaseErrorKind::Capability
                ) =>
            {
                MetadataValue::Unavailable
            }
            Err(error) => return Err(error.into()),
        };
        let metrics = adapter
            .health_metrics(limit)
            .await
            .map_err(AppError::from)?;

        Ok(DatabaseHealthSnapshot::new(
            profile_id.to_string(),
            info,
            latency_ms,
            metrics,
            now_millis(),
        ))
    }
}

/// Converts one engine catalog row into the shared table-size record. Values
/// that the query could not read are unavailable rather than zero.
pub(crate) fn table_size_from_parts(
    database: Option<String>,
    schema: Option<String>,
    name: String,
    kind: TableKind,
    size_bytes: Option<i64>,
    estimated_rows: Option<i64>,
    metadata_refreshed_at: Option<String>,
) -> DatabaseTableSize {
    DatabaseTableSize {
        database,
        schema,
        name,
        kind,
        size_bytes: size_bytes
            .and_then(|value| u64::try_from(value).ok())
            .into(),
        estimated_rows: estimated_rows
            .and_then(|value| u64::try_from(value).ok())
            .into(),
        metadata_refreshed_at: metadata_refreshed_at.into(),
    }
}

/// Shared permission fallback for catalog reads. A restricted account sees an
/// unavailable section instead of a failed sample; connection and timeout
/// failures still propagate.
pub(crate) fn unavailable_on_permission<T>(
    result: Result<T, DatabaseError>,
) -> Result<MetadataValue<T>, DatabaseError> {
    match result {
        Ok(value) => Ok(MetadataValue::available(value)),
        Err(error) if error.kind == super::errors::DatabaseErrorKind::Permission => {
            Ok(MetadataValue::Unavailable)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> DatabaseConnectionInfo {
        DatabaseConnectionInfo {
            engine: DatabaseEngine::Postgresql,
            version: DatabaseVersionInfo {
                version: super::super::capabilities::DatabaseVersion::new(16, 2, 0),
                raw: "16.2".to_string(),
            },
            database: Some("app".to_string()),
            read_only: false,
            capabilities: super::super::capabilities::DatabaseCapabilities::with_version(
                DatabaseEngine::Postgresql,
                Some(super::super::capabilities::DatabaseVersion::new(16, 2, 0)),
            )
            .snapshot(),
        }
    }

    #[test]
    fn snapshot_serializes_estimates_and_never_connection_details() {
        let metrics = DatabaseHealthMetrics {
            database_size_bytes: MetadataValue::available(8_192),
            table_sizes: MetadataValue::available(Bounded::new(
                vec![table_size_from_parts(
                    Some("app".to_string()),
                    Some("public".to_string()),
                    "users".to_string(),
                    TableKind::Table,
                    Some(8_192),
                    Some(42),
                    Some("2026-01-02T03:04:05Z".to_string()),
                )],
                12,
            )),
        };
        let snapshot = DatabaseHealthSnapshot::new(
            "profile-1".to_string(),
            info(),
            MetadataValue::available(7),
            metrics,
            1_700_000_000_000,
        );
        let json = serde_json::to_string(&snapshot).expect("snapshot json");

        assert!(json.contains("\"sampledAt\":1700000000000"));
        assert!(json.contains("\"latencyMs\":{\"status\":\"available\",\"value\":7}"));
        assert!(json.contains("\"estimatedRows\":{\"status\":\"available\",\"value\":42}"));
        assert!(!json.contains("password"));
        assert!(!json.contains("127.0.0.1"));
        assert!(!json.contains("SELECT"));
    }

    #[test]
    fn unavailable_catalog_values_are_not_zero() {
        let size = table_size_from_parts(
            Some("app".to_string()),
            Some("public".to_string()),
            "users".to_string(),
            TableKind::Table,
            None,
            None,
            None,
        );

        assert_eq!(size.size_bytes, MetadataValue::Unavailable);
        assert_eq!(size.estimated_rows, MetadataValue::Unavailable);
        assert_eq!(size.metadata_refreshed_at, MetadataValue::Unavailable);
    }

    #[test]
    fn permission_errors_become_unavailable_sections() {
        let unavailable = unavailable_on_permission::<u64>(Err(DatabaseError::permission(
            "The account cannot read catalog sizes.",
        )))
        .expect("permission fallback");
        assert_eq!(unavailable, MetadataValue::Unavailable);

        assert!(
            unavailable_on_permission::<u64>(Err(DatabaseError::timeout(
                "The catalog read timed out."
            )))
            .is_err()
        );
    }
}
