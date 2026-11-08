use std::{
    path::Path,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;

use crate::storage::{StorageError, migrations::run_migrations};

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self, StorageError> {
        let connection = Connection::open_in_memory()?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, StorageError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(Duration::from_secs(5))?;

        let database = Self {
            connection: Mutex::new(connection),
        };
        database.migrate()?;
        Ok(database)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let mut connection = self.lock()?;
        run_migrations(&mut connection)
    }

    pub fn schema_version(&self) -> Result<i32, StorageError> {
        let connection = self.lock()?;
        Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    pub(crate) fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let connection = self.lock()?;
        operation(&connection)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StorageError> {
        self.connection
            .lock()
            .map_err(|_| StorageError::LockPoisoned)
    }
}

pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
