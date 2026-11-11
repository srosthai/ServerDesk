use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, Row, params, types::Type};
use uuid::Uuid;

use crate::storage::{
    Database, DatabaseConnection, DatabaseEngine, DatabaseQueryHistory, DatabaseQueryHistoryFilter,
    DatabaseQueryResult, DatabaseSavedQuery, NewDatabaseConnection, NewDatabaseQueryHistory,
    NewDatabaseSavedQuery, StorageError, database::now_unix,
};

const DATABASE_CONNECTION_COLUMNS: &str = "id, server_id, name, engine, host, port, socket, \
     database_name, username, credential_ref, read_only, ssl_mode, created_at, updated_at, \
     last_used_at";

const DATABASE_QUERY_HISTORY_COLUMNS: &str = "id, server_id, connection_id, engine, \
     database_name, sql_text, result, duration_ms, row_count, error_summary, started_at";

const DATABASE_SAVED_QUERY_COLUMNS: &str = "id, name, engine, sql_text, created_at, updated_at";

/// Default number of history rows returned when a filter does not set a limit.
pub const DEFAULT_DATABASE_QUERY_HISTORY_LIMIT: u32 = 200;

/// Hard cap for one history query so a single call cannot load the whole table.
pub const MAX_DATABASE_QUERY_HISTORY_LIMIT: u32 = 1_000;

impl Database {
    pub fn create_database_connection(
        &self,
        input: &NewDatabaseConnection,
    ) -> Result<DatabaseConnection, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_unix();

        self.with_connection(|connection| {
            if database_connection_name_exists(connection, &input.server_id, &input.name, None)? {
                return Err(StorageError::Conflict(
                    "a database connection with this name already exists for the server"
                        .to_string(),
                ));
            }

            connection.execute(
                "INSERT INTO database_connections (
                    id, server_id, name, engine, host, port, socket, database_name,
                    username, credential_ref, read_only, ssl_mode, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    id,
                    input.server_id,
                    input.name,
                    input.engine.as_str(),
                    input.host,
                    input.port,
                    input.socket,
                    input.database_name,
                    input.username,
                    input.credential_ref,
                    if input.read_only { 1 } else { 0 },
                    input.ssl_mode,
                    now,
                    now,
                ],
            )?;

            get_database_connection(connection, &id)?.ok_or(StorageError::NotFound)
        })
    }

    pub fn get_database_connection(
        &self,
        id: &str,
    ) -> Result<Option<DatabaseConnection>, StorageError> {
        self.with_connection(|connection| get_database_connection(connection, id))
    }

    pub fn list_database_connections(
        &self,
        server_id: &str,
    ) -> Result<Vec<DatabaseConnection>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {DATABASE_CONNECTION_COLUMNS} FROM database_connections \
                 WHERE server_id = ?1 ORDER BY name COLLATE NOCASE, host, port"
            ))?;
            let connections = statement
                .query_map(params![server_id], row_to_database_connection)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(connections)
        })
    }

    /// Lists every saved database credential reference for `--purge-data`.
    ///
    /// The credential index normally contains these references too, but reading
    /// the database closes the gap if the index was lost or is malformed.
    pub fn list_database_connection_credential_refs(&self) -> Result<Vec<String>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT credential_ref FROM database_connections \
                 WHERE credential_ref IS NOT NULL ORDER BY credential_ref",
            )?;
            let references = statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            Ok(references)
        })
    }

    pub fn update_database_connection(
        &self,
        id: &str,
        input: &NewDatabaseConnection,
    ) -> Result<DatabaseConnection, StorageError> {
        let now = now_unix();

        self.with_connection(|connection| {
            if database_connection_name_exists(connection, &input.server_id, &input.name, Some(id))?
            {
                return Err(StorageError::Conflict(
                    "a database connection with this name already exists for the server"
                        .to_string(),
                ));
            }

            let changed = connection.execute(
                "UPDATE database_connections SET
                    server_id = ?1, name = ?2, engine = ?3, host = ?4, port = ?5,
                    socket = ?6, database_name = ?7, username = ?8, credential_ref = ?9,
                    read_only = ?10, ssl_mode = ?11, updated_at = ?12
                 WHERE id = ?13",
                params![
                    input.server_id,
                    input.name,
                    input.engine.as_str(),
                    input.host,
                    input.port,
                    input.socket,
                    input.database_name,
                    input.username,
                    input.credential_ref,
                    if input.read_only { 1 } else { 0 },
                    input.ssl_mode,
                    now,
                    id,
                ],
            )?;

            if changed == 0 {
                return Err(StorageError::NotFound);
            }

            get_database_connection(connection, id)?.ok_or(StorageError::NotFound)
        })
    }

    /// Records a successful connection use without changing the profile fields.
    pub fn mark_database_connection_used(
        &self,
        id: &str,
    ) -> Result<DatabaseConnection, StorageError> {
        let now = now_unix();

        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE database_connections SET last_used_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;

            if changed == 0 {
                return Err(StorageError::NotFound);
            }

            get_database_connection(connection, id)?.ok_or(StorageError::NotFound)
        })
    }

    pub fn delete_database_connection(&self, id: &str) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute(
                "DELETE FROM database_connections WHERE id = ?1",
                params![id],
            )?;
            Ok(())
        })
    }

    pub fn database_connection_name_exists(
        &self,
        server_id: &str,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<bool, StorageError> {
        self.with_connection(|connection| {
            database_connection_name_exists(connection, server_id, name, exclude_id)
        })
    }

    pub fn record_database_query(
        &self,
        input: &NewDatabaseQueryHistory,
    ) -> Result<DatabaseQueryHistory, StorageError> {
        let id = Uuid::new_v4().to_string();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO database_query_history (
                    id, server_id, connection_id, engine, database_name, sql_text,
                    result, duration_ms, row_count, error_summary, started_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    input.server_id,
                    input.connection_id,
                    input.engine.as_str(),
                    input.database_name,
                    input.sql_text,
                    input.result.as_str(),
                    input.duration_ms,
                    input.row_count,
                    input.error_summary,
                    input.started_at,
                ],
            )?;

            get_database_query_history(connection, &id)?.ok_or(StorageError::NotFound)
        })
    }

    pub fn list_database_query_history(
        &self,
        filter: &DatabaseQueryHistoryFilter,
    ) -> Result<Vec<DatabaseQueryHistory>, StorageError> {
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_DATABASE_QUERY_HISTORY_LIMIT)
            .clamp(1, MAX_DATABASE_QUERY_HISTORY_LIMIT);
        let result = filter.result.map(DatabaseQueryResult::as_str);

        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {DATABASE_QUERY_HISTORY_COLUMNS} FROM database_query_history \
                 WHERE (?1 IS NULL OR server_id = ?1) \
                 AND (?2 IS NULL OR connection_id = ?2) \
                 AND (?3 IS NULL OR result = ?3) \
                 AND (?4 IS NULL OR started_at >= ?4) \
                 AND (?5 IS NULL OR started_at <= ?5) \
                 ORDER BY started_at DESC, id DESC LIMIT ?6"
            ))?;
            let history = statement
                .query_map(
                    params![
                        filter.server_id,
                        filter.connection_id,
                        result,
                        filter.started_after,
                        filter.started_before,
                        i64::from(limit),
                    ],
                    row_to_database_query_history,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(history)
        })
    }

    pub fn clear_database_query_history(&self) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute("DELETE FROM database_query_history", [])?;
            Ok(())
        })
    }

    pub fn create_database_saved_query(
        &self,
        input: &NewDatabaseSavedQuery,
    ) -> Result<DatabaseSavedQuery, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_unix();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO database_saved_queries (
                    id, name, engine, sql_text, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    input.name,
                    input.engine.as_str(),
                    input.sql_text,
                    now,
                    now
                ],
            )?;

            get_database_saved_query(connection, &id)?.ok_or(StorageError::NotFound)
        })
    }

    pub fn get_database_saved_query(
        &self,
        id: &str,
    ) -> Result<Option<DatabaseSavedQuery>, StorageError> {
        self.with_connection(|connection| get_database_saved_query(connection, id))
    }

    pub fn list_database_saved_queries(&self) -> Result<Vec<DatabaseSavedQuery>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {DATABASE_SAVED_QUERY_COLUMNS} FROM database_saved_queries \
                 ORDER BY engine, name COLLATE NOCASE"
            ))?;
            let queries = statement
                .query_map([], row_to_database_saved_query)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(queries)
        })
    }

    pub fn update_database_saved_query(
        &self,
        id: &str,
        input: &NewDatabaseSavedQuery,
    ) -> Result<DatabaseSavedQuery, StorageError> {
        let now = now_unix();

        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE database_saved_queries SET
                    name = ?1, engine = ?2, sql_text = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![input.name, input.engine.as_str(), input.sql_text, now, id],
            )?;

            if changed == 0 {
                return Err(StorageError::NotFound);
            }

            get_database_saved_query(connection, id)?.ok_or(StorageError::NotFound)
        })
    }

    pub fn delete_database_saved_query(&self, id: &str) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute(
                "DELETE FROM database_saved_queries WHERE id = ?1",
                params![id],
            )?;
            Ok(())
        })
    }
}

fn get_database_connection(
    connection: &Connection,
    id: &str,
) -> Result<Option<DatabaseConnection>, StorageError> {
    Ok(connection
        .query_row(
            &format!(
                "SELECT {DATABASE_CONNECTION_COLUMNS} FROM database_connections WHERE id = ?1"
            ),
            params![id],
            row_to_database_connection,
        )
        .optional()?)
}

fn database_connection_name_exists(
    connection: &Connection,
    server_id: &str,
    name: &str,
    exclude_id: Option<&str>,
) -> Result<bool, StorageError> {
    let exists: i64 = match exclude_id {
        Some(id) => connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM database_connections
                WHERE server_id = ?1 AND name = ?2 COLLATE NOCASE AND id <> ?3
            )",
            params![server_id, name, id],
            |row| row.get(0),
        )?,
        None => connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM database_connections
                WHERE server_id = ?1 AND name = ?2 COLLATE NOCASE
            )",
            params![server_id, name],
            |row| row.get(0),
        )?,
    };

    Ok(exists != 0)
}

fn get_database_query_history(
    connection: &Connection,
    id: &str,
) -> Result<Option<DatabaseQueryHistory>, StorageError> {
    Ok(connection
        .query_row(
            &format!(
                "SELECT {DATABASE_QUERY_HISTORY_COLUMNS} FROM database_query_history WHERE id = ?1"
            ),
            params![id],
            row_to_database_query_history,
        )
        .optional()?)
}

fn get_database_saved_query(
    connection: &Connection,
    id: &str,
) -> Result<Option<DatabaseSavedQuery>, StorageError> {
    Ok(connection
        .query_row(
            &format!(
                "SELECT {DATABASE_SAVED_QUERY_COLUMNS} FROM database_saved_queries WHERE id = ?1"
            ),
            params![id],
            row_to_database_saved_query,
        )
        .optional()?)
}

fn row_to_database_connection(row: &Row<'_>) -> rusqlite::Result<DatabaseConnection> {
    Ok(DatabaseConnection {
        id: row.get(0)?,
        server_id: row.get(1)?,
        name: row.get(2)?,
        engine: database_engine(row, 3)?,
        host: row.get(4)?,
        port: u16_port(row, 5)?,
        socket: row.get(6)?,
        database_name: row.get(7)?,
        username: row.get(8)?,
        credential_ref: row.get(9)?,
        read_only: row.get::<_, i64>(10)? != 0,
        ssl_mode: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        last_used_at: row.get(14)?,
    })
}

fn row_to_database_query_history(row: &Row<'_>) -> rusqlite::Result<DatabaseQueryHistory> {
    let result_raw: String = row.get(6)?;
    let result = DatabaseQueryResult::from_str(&result_raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(error))
    })?;

    Ok(DatabaseQueryHistory {
        id: row.get(0)?,
        server_id: row.get(1)?,
        connection_id: row.get(2)?,
        engine: database_engine(row, 3)?,
        database_name: row.get(4)?,
        sql_text: row.get(5)?,
        result,
        duration_ms: row.get(7)?,
        row_count: row.get(8)?,
        error_summary: row.get(9)?,
        started_at: row.get(10)?,
    })
}

fn row_to_database_saved_query(row: &Row<'_>) -> rusqlite::Result<DatabaseSavedQuery> {
    Ok(DatabaseSavedQuery {
        id: row.get(0)?,
        name: row.get(1)?,
        engine: database_engine(row, 2)?,
        sql_text: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn database_engine(row: &Row<'_>, index: usize) -> rusqlite::Result<DatabaseEngine> {
    let raw: String = row.get(index)?;
    DatabaseEngine::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

fn u16_port(row: &Row<'_>, index: usize) -> rusqlite::Result<u16> {
    let port: i64 = row.get(index)?;
    u16::try_from(port).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "database port out of range",
            )),
        )
    })
}
