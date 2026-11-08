use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::storage::{
    AuthType, NewServer, Server, StorageError,
    database::{Database, now_unix},
};

const SERVER_COLUMNS: &str = "id, name, host, port, username, auth_type, credential_ref, group_name, tags, notes, created_at, updated_at";

impl Database {
    pub fn create_server(&self, input: &NewServer) -> Result<Server, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_unix();
        let tags = serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".to_string());

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO servers (
                    id, name, host, port, username, auth_type, credential_ref,
                    group_name, tags, notes, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    input.name,
                    input.host,
                    input.port,
                    input.username,
                    input.auth_type.as_str(),
                    input.credential_ref,
                    input.group_name,
                    tags,
                    input.notes,
                    now,
                    now,
                ],
            )?;

            get_server(connection, &id)?.ok_or(StorageError::NotFound)
        })
    }

    pub fn get_server(&self, id: &str) -> Result<Option<Server>, StorageError> {
        self.with_connection(|connection| get_server(connection, id))
    }

    pub fn list_servers(&self) -> Result<Vec<Server>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {SERVER_COLUMNS} FROM servers ORDER BY name COLLATE NOCASE, host"
            ))?;
            let servers = statement
                .query_map([], row_to_server)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(servers)
        })
    }

    pub fn update_server(&self, id: &str, input: &NewServer) -> Result<Server, StorageError> {
        let now = now_unix();
        let tags = serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".to_string());

        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE servers SET
                    name = ?1, host = ?2, port = ?3, username = ?4, auth_type = ?5,
                    credential_ref = ?6, group_name = ?7, tags = ?8, notes = ?9, updated_at = ?10
                 WHERE id = ?11",
                params![
                    input.name,
                    input.host,
                    input.port,
                    input.username,
                    input.auth_type.as_str(),
                    input.credential_ref,
                    input.group_name,
                    tags,
                    input.notes,
                    now,
                    id,
                ],
            )?;

            if changed == 0 {
                return Err(StorageError::NotFound);
            }

            get_server(connection, id)?.ok_or(StorageError::NotFound)
        })
    }

    pub fn delete_server(&self, id: &str) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute("DELETE FROM servers WHERE id = ?1", params![id])?;
            Ok(())
        })
    }
}

fn get_server(connection: &Connection, id: &str) -> Result<Option<Server>, StorageError> {
    Ok(connection
        .query_row(
            &format!("SELECT {SERVER_COLUMNS} FROM servers WHERE id = ?1"),
            params![id],
            row_to_server,
        )
        .optional()?)
}

fn row_to_server(row: &Row<'_>) -> rusqlite::Result<Server> {
    let auth_raw: String = row.get(5)?;
    let auth_type = AuthType::from_str(&auth_raw).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid auth_type",
            )),
        )
    })?;
    let port: i64 = row.get(3)?;
    let tags_raw: String = row.get(8)?;

    Ok(Server {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: u16::try_from(port).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "port out of range",
                )),
            )
        })?,
        username: row.get(4)?,
        auth_type,
        credential_ref: row.get(6)?,
        group_name: row.get(7)?,
        tags: serde_json::from_str(&tags_raw).unwrap_or_default(),
        notes: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

impl Database {
    pub fn server_name_exists(
        &self,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<bool, StorageError> {
        self.with_connection(|connection| {
            let exists: i64 = match exclude_id {
                Some(id) => connection.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM servers WHERE name = ?1 COLLATE NOCASE AND id <> ?2
                    )",
                    params![name, id],
                    |row| row.get(0),
                )?,
                None => connection.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM servers WHERE name = ?1 COLLATE NOCASE
                    )",
                    params![name],
                    |row| row.get(0),
                )?,
            };

            Ok(exists != 0)
        })
    }
}
