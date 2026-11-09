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
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let database = Self::from_connection(connection)?;
        restrict_database_permissions(&path);
        Ok(database)
    }

    pub fn in_memory() -> Result<Self, StorageError> {
        let connection = Connection::open_in_memory()?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, StorageError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "secure_delete", "ON")?;
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

/// Restricts the database and its WAL sidecars to the current OS user.
///
/// SQLite creates `-wal` and `-shm` files next to the database. They hold the
/// same sanitized metadata, but they are still private local application data.
fn restrict_database_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut candidates = vec![path.to_path_buf()];
        if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
            for suffix in ["-wal", "-shm"] {
                candidates.push(path.with_file_name(format!("{file_name}{suffix}")));
            }
        }

        for candidate in candidates {
            if let Ok(metadata) = std::fs::metadata(&candidate) {
                let mut permissions = metadata.permissions();
                permissions.set_mode(0o600);
                let _ = std::fs::set_permissions(candidate, permissions);
            }
        }
    }
}

pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
