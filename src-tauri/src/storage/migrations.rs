use rusqlite::Connection;

use crate::storage::StorageError;

const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_initial",
        include_str!("../../../migrations/0001_initial.sql"),
    ),
    (
        "0002_unique_server_names",
        include_str!("../../../migrations/0002_unique_server_names.sql"),
    ),
    (
        "0003_project_detection",
        include_str!("../../../migrations/0003_project_detection.sql"),
    ),
    (
        "0004_operation_history",
        include_str!("../../../migrations/0004_operation_history.sql"),
    ),
    (
        "0005_database_manager",
        include_str!("../../../migrations/0005_database_manager.sql"),
    ),
];

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

#[cfg(test)]
mod tests {
    use super::*;

    fn user_version(connection: &Connection) -> i32 {
        connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read user_version")
    }

    #[test]
    fn upgrades_an_existing_database_and_reapplies_the_latest_migration() {
        let mut connection = Connection::open_in_memory().expect("in-memory database");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("enable foreign keys");

        for (index, (name, sql)) in MIGRATIONS.iter().enumerate().take(4) {
            connection
                .execute_batch(sql)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            connection
                .pragma_update(None, "user_version", (index + 1) as i32)
                .expect("set version");
        }

        assert_eq!(user_version(&connection), 4);

        run_migrations(&mut connection).expect("upgrade existing database");

        assert_eq!(user_version(&connection), 5);

        // The migration runner skips applied versions, but every object in the
        // latest migration is also guarded so a manual reapply is safe.
        connection
            .execute_batch(MIGRATIONS[4].1)
            .expect("reapply database manager migration");
        assert_eq!(user_version(&connection), 5);
    }
}
