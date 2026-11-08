use rusqlite::{Row, params};
use uuid::Uuid;

use crate::storage::{Database, NewPortForward, PortForward, StorageError, database::now_unix};

const PORT_FORWARD_COLUMNS: &str =
    "id, server_id, bind_host, local_port, remote_host, remote_port, created_at";

impl Database {
    pub fn create_port_forward(&self, input: &NewPortForward) -> Result<PortForward, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_unix();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO port_forwards (
                    id, server_id, bind_host, local_port, remote_host, remote_port, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    input.server_id,
                    input.bind_host,
                    input.local_port,
                    input.remote_host,
                    input.remote_port,
                    now,
                ],
            )?;

            connection
                .query_row(
                    &format!("SELECT {PORT_FORWARD_COLUMNS} FROM port_forwards WHERE id = ?1"),
                    params![id],
                    row_to_port_forward,
                )
                .map_err(StorageError::from)
        })
    }

    pub fn list_port_forwards(&self, server_id: &str) -> Result<Vec<PortForward>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {PORT_FORWARD_COLUMNS} FROM port_forwards WHERE server_id = ?1 ORDER BY local_port"
            ))?;
            let forwards = statement
                .query_map(params![server_id], row_to_port_forward)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(forwards)
        })
    }

    pub fn delete_port_forward(&self, id: &str) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute("DELETE FROM port_forwards WHERE id = ?1", params![id])?;
            Ok(())
        })
    }
}

fn row_to_port_forward(row: &Row<'_>) -> rusqlite::Result<PortForward> {
    let local_port: i64 = row.get(3)?;
    let remote_port: i64 = row.get(5)?;

    Ok(PortForward {
        id: row.get(0)?,
        server_id: row.get(1)?,
        bind_host: row.get(2)?,
        local_port: u16::try_from(local_port).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "local_port out of range",
                )),
            )
        })?,
        remote_host: row.get(4)?,
        remote_port: u16::try_from(remote_port).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "remote_port out of range",
                )),
            )
        })?,
        created_at: row.get(6)?,
    })
}
