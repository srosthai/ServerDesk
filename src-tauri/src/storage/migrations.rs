use rusqlite::Connection;

use crate::storage::StorageError;

const MIGRATIONS: &[(&str, &str)] = &[(
    "0001_initial",
    include_str!("../../../migrations/0001_initial.sql"),
)];

pub(crate) fn run_migrations(connection: &mut Connection) -> Result<(), StorageError> {
    let current_version: i32 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (index, (name, sql)) in MIGRATIONS.iter().enumerate() {
        let version = (index + 1) as i32;
        if version <= current_version {
            continue;
        }

        let transaction = connection.transaction()?;
        transaction
            .execute_batch(sql)
            .map_err(|error| StorageError::Migration(format!("{name}: {error}")))?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }

    Ok(())
}
