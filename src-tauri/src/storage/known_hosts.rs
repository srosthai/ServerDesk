use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::storage::{
    KnownHost, StorageError,
    database::{Database, now_unix},
};

const KNOWN_HOST_COLUMNS: &str = "id, server_id, key_type, fingerprint, first_seen_at, verified_at";

impl Database {
    /// Stores a host key only after the user explicitly trusts it.
    ///
    /// Existing records are never overwritten. A different fingerprint for the
    /// same key type is a conflict so a changed key cannot be silently trusted.
    pub fn insert_known_host(
        &self,
        server_id: &str,
        key_type: &str,
        fingerprint: &str,
    ) -> Result<KnownHost, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_unix();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO known_hosts (
                    id, server_id, key_type, fingerprint, first_seen_at, verified_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT (server_id, key_type) DO NOTHING",
                params![id, server_id, key_type, fingerprint, now, now],
            )?;

            let host =
                get_known_host(connection, server_id, key_type)?.ok_or(StorageError::NotFound)?;
            if host.fingerprint != fingerprint {
                return Err(StorageError::Conflict(
                    "a different host key is already trusted for this server".to_string(),
                ));
            }

            Ok(host)
        })
    }

    pub fn get_known_host(
        &self,
        server_id: &str,
        key_type: &str,
    ) -> Result<Option<KnownHost>, StorageError> {
        self.with_connection(|connection| get_known_host(connection, server_id, key_type))
    }

    pub fn list_known_hosts(&self, server_id: &str) -> Result<Vec<KnownHost>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {KNOWN_HOST_COLUMNS} FROM known_hosts WHERE server_id = ?1 ORDER BY key_type"
            ))?;
            let hosts = statement
                .query_map(params![server_id], row_to_known_host)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(hosts)
        })
    }

    pub fn delete_known_host(&self, server_id: &str, key_type: &str) -> Result<bool, StorageError> {
        self.with_connection(|connection| {
            let deleted = connection.execute(
                "DELETE FROM known_hosts WHERE server_id = ?1 AND key_type = ?2",
                params![server_id, key_type],
            )?;
            Ok(deleted > 0)
        })
    }
}

fn get_known_host(
    connection: &Connection,
    server_id: &str,
    key_type: &str,
) -> Result<Option<KnownHost>, StorageError> {
    Ok(connection
        .query_row(
            &format!(
                "SELECT {KNOWN_HOST_COLUMNS} FROM known_hosts WHERE server_id = ?1 AND key_type = ?2"
            ),
            params![server_id, key_type],
            row_to_known_host,
        )
        .optional()?)
}

fn row_to_known_host(row: &Row<'_>) -> rusqlite::Result<KnownHost> {
    Ok(KnownHost {
        id: row.get(0)?,
        server_id: row.get(1)?,
        key_type: row.get(2)?,
        fingerprint: row.get(3)?,
        first_seen_at: row.get(4)?,
        verified_at: row.get(5)?,
    })
}
