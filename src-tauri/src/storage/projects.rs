use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::storage::{
    NewProject, Project, StorageError,
    database::{Database, now_unix},
};

const PROJECT_COLUMNS: &str = "id, server_id, name, remote_path, project_type, laravel_version, \
     php_version, queue_hint, scheduler_hint, detected_at";

impl Database {
    /// Inserts a project or refreshes the existing record for the same server
    /// and normalized remote path.
    ///
    /// The unique `(server_id, remote_path)` constraint guarantees that
    /// rescanning a path never creates a duplicate record. The existing ID is
    /// kept and only the detected metadata and detection time are updated.
    pub fn upsert_project(&self, input: &NewProject) -> Result<Project, StorageError> {
        let id = Uuid::new_v4().to_string();
        let now = now_unix();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO projects (
                    id, server_id, name, remote_path, project_type,
                    laravel_version, php_version, queue_hint, scheduler_hint, detected_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                ON CONFLICT (server_id, remote_path) DO UPDATE SET
                    name = excluded.name,
                    project_type = excluded.project_type,
                    laravel_version = excluded.laravel_version,
                    php_version = excluded.php_version,
                    queue_hint = excluded.queue_hint,
                    scheduler_hint = excluded.scheduler_hint,
                    detected_at = excluded.detected_at",
                params![
                    id,
                    input.server_id,
                    input.name,
                    input.remote_path,
                    input.project_type,
                    input.laravel_version,
                    input.php_version,
                    input.queue_hint,
                    input.scheduler_hint,
                    now,
                ],
            )?;

            get_project_by_path(connection, &input.server_id, &input.remote_path)?
                .ok_or(StorageError::NotFound)
        })
    }

    pub fn create_project(&self, input: &NewProject) -> Result<Project, StorageError> {
        self.upsert_project(input)
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>, StorageError> {
        self.with_connection(|connection| get_project(connection, id))
    }

    pub fn get_project_by_path(
        &self,
        server_id: &str,
        remote_path: &str,
    ) -> Result<Option<Project>, StorageError> {
        self.with_connection(|connection| get_project_by_path(connection, server_id, remote_path))
    }

    pub fn list_projects(&self, server_id: &str) -> Result<Vec<Project>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM projects WHERE server_id = ?1 \
                 ORDER BY name COLLATE NOCASE, remote_path"
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

fn get_project_by_path(
    connection: &Connection,
    server_id: &str,
    remote_path: &str,
) -> Result<Option<Project>, StorageError> {
    Ok(connection
        .query_row(
            &format!(
                "SELECT {PROJECT_COLUMNS} FROM projects WHERE server_id = ?1 AND remote_path = ?2"
            ),
            params![server_id, remote_path],
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
        laravel_version: row.get(5)?,
        php_version: row.get(6)?,
        queue_hint: row.get(7)?,
        scheduler_hint: row.get(8)?,
        detected_at: row.get(9)?,
    })
}
