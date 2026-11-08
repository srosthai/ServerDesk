use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::storage::{
    NewProject, Project, StorageError,
    database::{Database, now_unix},
};

const PROJECT_COLUMNS: &str = "id, server_id, name, remote_path, project_type, detected_at";

impl Database {
    pub fn create_project(&self, input: &NewProject) -> Result<Project, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_unix();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO projects (
                    id, server_id, name, remote_path, project_type, detected_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    input.server_id,
                    input.name,
                    input.remote_path,
                    input.project_type,
                    now,
                ],
            )?;

            get_project(connection, &id)?.ok_or(StorageError::NotFound)
        })
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>, StorageError> {
        self.with_connection(|connection| get_project(connection, id))
    }

    pub fn list_projects(&self, server_id: &str) -> Result<Vec<Project>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM projects WHERE server_id = ?1 ORDER BY name COLLATE NOCASE"
            ))?;
            let projects = statement
                .query_map(params![server_id], row_to_project)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(projects)
        })
    }

    pub fn delete_project(&self, id: &str) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
            Ok(())
        })
    }
}

fn get_project(connection: &Connection, id: &str) -> Result<Option<Project>, StorageError> {
    Ok(connection
        .query_row(
            &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"),
            params![id],
            row_to_project,
        )
        .optional()?)
}

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        server_id: row.get(1)?,
        name: row.get(2)?,
        remote_path: row.get(3)?,
        project_type: row.get(4)?,
        detected_at: row.get(5)?,
    })
}
