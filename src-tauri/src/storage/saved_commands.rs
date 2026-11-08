use rusqlite::{Row, params};
use uuid::Uuid;

use crate::storage::{Database, NewSavedCommand, SavedCommand, StorageError};

const SAVED_COMMAND_COLUMNS: &str = "id, name, category, command_template, requires_confirmation";

impl Database {
    pub fn create_saved_command(
        &self,
        input: &NewSavedCommand,
    ) -> Result<SavedCommand, StorageError> {
        let id = Uuid::new_v4().to_string();

        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO saved_commands (
                    id, name, category, command_template, requires_confirmation
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    input.name,
                    input.category,
                    input.command_template,
                    if input.requires_confirmation { 1 } else { 0 },
                ],
            )?;

            let command = connection.query_row(
                &format!("SELECT {SAVED_COMMAND_COLUMNS} FROM saved_commands WHERE id = ?1"),
                params![id],
                row_to_saved_command,
            )?;
            Ok(command)
        })
    }

    pub fn list_saved_commands(&self) -> Result<Vec<SavedCommand>, StorageError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {SAVED_COMMAND_COLUMNS} FROM saved_commands ORDER BY category, name COLLATE NOCASE"
            ))?;
            let commands = statement
                .query_map([], row_to_saved_command)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(commands)
        })
    }

    pub fn delete_saved_command(&self, id: &str) -> Result<(), StorageError> {
        self.with_connection(|connection| {
            connection.execute("DELETE FROM saved_commands WHERE id = ?1", params![id])?;
            Ok(())
        })
    }
}

fn row_to_saved_command(row: &Row<'_>) -> rusqlite::Result<SavedCommand> {
    Ok(SavedCommand {
        id: row.get(0)?,
        name: row.get(1)?,
        category: row.get(2)?,
        command_template: row.get(3)?,
        requires_confirmation: row.get::<_, i64>(4)? != 0,
    })
}
