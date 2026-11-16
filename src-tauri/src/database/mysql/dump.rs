//! MySQL dump and restore command construction.
//!
//! Every argument is either a fixed literal or a validated value quoted as one
//! POSIX shell argument. The database password is read from a mode-0600 file
//! into `MYSQL_PWD`; it never appears in the command line.

use crate::{
    core::command::shell_quote,
    database::{
        dialect::Identifier,
        dump::{
            DumpConnection, DumpFormat, DumpPaths, DumpScope, DumpToolVendor, wrap_tool_command,
        },
        errors::DatabaseError,
    },
};

/// Builds the fixed `mysqldump` command for one export.
pub fn export_command(
    connection: &DumpConnection,
    scope: &DumpScope,
    format: DumpFormat,
    paths: &DumpPaths,
    vendor: DumpToolVendor,
) -> Result<String, DatabaseError> {
    if format != DumpFormat::PlainSql {
        return Err(DatabaseError::capability(
            "mysqldump exports plain SQL only.",
        ));
    }
    let output = paths
        .output
        .as_deref()
        .ok_or_else(|| DatabaseError::internal("The MySQL dump output path is missing."))?;
    let database = scope
        .database(connection.database.as_deref())
        .ok_or_else(|| {
            DatabaseError::validation(
                "Choose the MySQL database to dump; the profile has no default database.",
            )
        })?;

    let mut arguments = connection_arguments(connection, vendor);
    match scope {
        DumpScope::Database { .. } => {
            arguments.push(shell_quote(database));
        }
        DumpScope::Tables { tables, .. } => {
            arguments.push(shell_quote(database));
            for table in tables {
                arguments.push(shell_quote(table.as_str()));
            }
        }
        DumpScope::Cluster | DumpScope::Schema { .. } => {
            return Err(DatabaseError::capability(
                "Use a database or table scope for a MySQL dump.",
            ));
        }
    }
    arguments.push("--single-transaction".to_string());
    arguments.push("--quick".to_string());
    arguments.push("--triggers".to_string());
    arguments.push("--hex-blob".to_string());

    Ok(wrap_tool_command(
        "mysqldump",
        "MYSQL_PWD",
        &arguments,
        paths,
        output,
        None,
    ))
}

/// Builds the fixed `mysql` restore command for one plain SQL dump.
pub fn restore_command(
    connection: &DumpConnection,
    format: DumpFormat,
    target_database: &Identifier,
    paths: &DumpPaths,
    vendor: DumpToolVendor,
) -> Result<String, DatabaseError> {
    if format != DumpFormat::PlainSql {
        return Err(DatabaseError::capability(
            "The mysql client can restore plain SQL dumps only.",
        ));
    }
    let input = paths
        .input
        .as_deref()
        .ok_or_else(|| DatabaseError::internal("The MySQL restore input path is missing."))?;
    let mut arguments = connection_arguments(connection, vendor);
    arguments.push(format!(
        "--database={}",
        shell_quote(target_database.as_str())
    ));

    Ok(wrap_tool_command(
        "mysql",
        "MYSQL_PWD",
        &arguments,
        paths,
        "/dev/null",
        Some(input),
    ))
}

fn connection_arguments(connection: &DumpConnection, vendor: DumpToolVendor) -> Vec<String> {
    let mut arguments = vec!["--no-defaults".to_string()];
    if let Some(socket) = connection.socket.as_deref() {
        arguments.push(format!("--socket={}", shell_quote(socket)));
    } else {
        arguments.push(format!("--host={}", shell_quote(&connection.host)));
        arguments.push(format!("--port={}", connection.port));
    }
    arguments.push(format!("--user={}", shell_quote(&connection.username)));
    arguments.extend(ssl_arguments(connection.ssl_mode.as_deref(), vendor));
    arguments
}

fn ssl_arguments(mode: Option<&str>, vendor: DumpToolVendor) -> Vec<String> {
    let Some(mode) = mode else {
        return Vec::new();
    };
    match vendor {
        DumpToolVendor::Mariadb => match mode {
            "disabled" => vec!["--skip-ssl".to_string()],
            "preferred" | "required" => vec!["--ssl".to_string()],
            "verify-ca" | "verify-identity" => {
                vec!["--ssl".to_string(), "--ssl-verify-server-cert".to_string()]
            }
            _ => Vec::new(),
        },
        _ => {
            let value = match mode {
                "disabled" => "DISABLED",
                "preferred" => "PREFERRED",
                "required" => "REQUIRED",
                "verify-ca" => "VERIFY_CA",
                "verify-identity" => "VERIFY_IDENTITY",
                _ => return Vec::new(),
            };
            vec![format!("--ssl-mode={value}")]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> DumpConnection {
        DumpConnection {
            engine: crate::storage::DatabaseEngine::Mysql,
            host: "127.0.0.1".to_string(),
            port: 3306,
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            ssl_mode: None,
        }
    }

    fn paths() -> DumpPaths {
        DumpPaths {
            work_dir: "/tmp/serverdesk-dump-fixture".to_string(),
            output: Some("/tmp/serverdesk-dump-fixture/dump.partial".to_string()),
            error_log: "/tmp/serverdesk-dump-fixture/stderr.log".to_string(),
            pid: "/tmp/serverdesk-dump-fixture/dump.pid".to_string(),
            credential: "/tmp/serverdesk-dump-fixture/password".to_string(),
            input: None,
        }
    }

    #[test]
    fn database_scope_quotes_the_database_and_no_password_argument() {
        let command = export_command(
            &connection(),
            &DumpScope::Database { database: None },
            DumpFormat::PlainSql,
            &paths(),
            DumpToolVendor::Mysql,
        )
        .expect("command");

        assert!(command.contains("exec mysqldump --no-defaults"));
        assert!(command.contains("mysqldump --no-defaults"));
        assert!(command.contains("'app'"));
        assert!(!command.contains("--databases"));
        assert!(command.contains("--single-transaction"));
        assert!(command.contains("MYSQL_PWD=$(cat '/tmp/serverdesk-dump-fixture/password')"));
        assert!(!command.contains("--password"));
        assert!(!command.contains("hunter2"));
        assert!(command.contains("> '/tmp/serverdesk-dump-fixture/dump.partial'"));
    }

    #[test]
    fn table_scope_quotes_every_table_and_database() {
        let command = export_command(
            &connection(),
            &DumpScope::Tables {
                database: Some(Identifier::new("app").expect("database")),
                schema: None,
                tables: vec![
                    Identifier::new("users").expect("table"),
                    Identifier::new("My Table's Data").expect("table"),
                ],
            },
            DumpFormat::PlainSql,
            &paths(),
            DumpToolVendor::Mysql,
        )
        .expect("command");

        assert!(command.contains("mysqldump --no-defaults"));
        assert!(command.contains("'app' 'users' 'My Table'\\''s Data'"));
        assert!(!command.contains("--databases"));
    }

    #[test]
    fn hostile_database_name_stays_inside_one_shell_argument() {
        let mut connection = connection();
        connection.database = Some("app'; touch /tmp/pwn; --".to_string());
        let command = export_command(
            &connection,
            &DumpScope::Database { database: None },
            DumpFormat::PlainSql,
            &paths(),
            DumpToolVendor::Mysql,
        )
        .expect("command");

        assert!(command.contains("'app'\\''; touch /tmp/pwn; --'"));
        assert!(!command.contains("; touch /tmp/pwn; -- --databases"));
    }

    #[test]
    fn restore_uses_stdin_redirect_and_target_database() {
        let mut paths = paths();
        paths.input = Some("/tmp/serverdesk-dump-fixture/restore.input".to_string());
        let command = restore_command(
            &connection(),
            DumpFormat::PlainSql,
            &Identifier::new("app").expect("database"),
            &paths,
            DumpToolVendor::Mysql,
        )
        .expect("command");

        assert!(command.contains("exec mysql --no-defaults"));
        assert!(command.contains("--database='app'"));
        assert!(command.contains("< '/tmp/serverdesk-dump-fixture/restore.input'"));
        assert!(command.contains("> '/dev/null'"));
        assert!(!command.contains("--password"));
    }

    #[test]
    fn mariadb_uses_its_ssl_flags() {
        let mut connection = connection();
        connection.ssl_mode = Some("required".to_string());
        let command = export_command(
            &connection,
            &DumpScope::Database { database: None },
            DumpFormat::PlainSql,
            &paths(),
            DumpToolVendor::Mariadb,
        )
        .expect("command");

        assert!(command.contains("--ssl"));
        assert!(!command.contains("--ssl-mode"));
    }

    #[test]
    fn generated_commands_parse_as_posix_shell() {
        let commands = [
            export_command(
                &connection(),
                &DumpScope::Database { database: None },
                DumpFormat::PlainSql,
                &paths(),
                DumpToolVendor::Mysql,
            )
            .expect("command"),
            {
                let mut paths = paths();
                paths.input = Some("/tmp/serverdesk-dump-fixture/restore.input".to_string());
                restore_command(
                    &connection(),
                    DumpFormat::PlainSql,
                    &Identifier::new("app").expect("database"),
                    &paths,
                    DumpToolVendor::Mysql,
                )
                .expect("command")
            },
        ];
        for command in commands {
            let status = std::process::Command::new("sh")
                .arg("-n")
                .arg("-c")
                .arg(&command)
                .status()
                .expect("run sh");
            assert!(status.success(), "invalid shell command: {command}");
        }
    }

    #[test]
    fn unsupported_format_is_rejected() {
        let error = export_command(
            &connection(),
            &DumpScope::Database { database: None },
            DumpFormat::Custom,
            &paths(),
            DumpToolVendor::Mysql,
        )
        .expect_err("unsupported format");
        assert_eq!(
            error.kind,
            crate::database::errors::DatabaseErrorKind::Capability
        );
    }
}
