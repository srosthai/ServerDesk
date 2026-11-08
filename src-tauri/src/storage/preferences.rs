use rusqlite::{OptionalExtension, Row, params};

use crate::storage::{
    Preference, StorageError,
    database::{Database, now_unix},
};

impl Database {
    pub fn set_preference(&self, key: &str, value: &str) -> Result<Preference, StorageError> {
        let now = now_unix();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO preferences (key, value, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at",
                params![key, value, now],
            )?;

            connection
                .query_row(
                    "SELECT key, value, updated_at FROM preferences WHERE key = ?1",
                    params![key],
                    row_to_preference,
                )
                .optional()?
                .ok_or(StorageError::NotFound)
        })
    }

    pub fn get_preference(&self, key: &str) -> Result<Option<Preference>, StorageError> {
        self.with_connection(|connection| {
            Ok(connection
                .query_row(
                    "SELECT key, value, updated_at FROM preferences WHERE key = ?1",
                    params![key],
                    row_to_preference,
                )
                .optional()?)
        })
    }

    pub fn list_preferences(&self) -> Result<Vec<Preference>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT key, value, updated_at FROM preferences ORDER BY key COLLATE NOCASE",
            )?;
            let preferences = statement
                .query_map([], row_to_preference)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(preferences)
        })
    }

    pub fn delete_preference(&self, key: &str) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute("DELETE FROM preferences WHERE key = ?1", params![key])?;
            Ok(())
        })
    }
}

fn row_to_preference(row: &Row<'_>) -> rusqlite::Result<Preference> {
    Ok(Preference {
        key: row.get(0)?,
        value: row.get(1)?,
        updated_at: row.get(2)?,
    })
}
