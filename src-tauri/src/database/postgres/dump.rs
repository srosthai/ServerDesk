//! PostgreSQL dump and restore command construction.
//!
//! `pg_dump`, `pg_dumpall`, `pg_restore`, and `psql` are invoked with fixed
//! options and validated identifiers. The database password is read from a
//! mode-0600 file into `PGPASSWORD`; it never appears in the command line.

use crate::{
    core::command::shell_quote,
    database::{
        dialect::Identifier,
        dump::{
            DumpConnection, DumpFormat, DumpPaths, DumpScope, DumpToolVendor,
            wrap_tool_command_with_environment,
        },
        errors::DatabaseError,
    },
};

/// Builds the fixed PostgreSQL export command for one scope and format.
pub fn export_command(
    connection: &DumpConnection,
    scope: &DumpScope,
    format: DumpFormat,
    no_owner: bool,
    paths: &DumpPaths,
    _vendor: DumpToolVendor,
) -> Result<String, DatabaseError> {
    let output = paths
        .output
        .as_deref()
        .ok_or_else(|| DatabaseError::internal("The PostgreSQL dump output path is missing."))?;

    if matches!(scope, DumpScope::Cluster) {
        if format != DumpFormat::PlainSql {
            return Err(DatabaseError::capability(
                "pg_dumpall exports plain SQL only.",
            ));
        }
        let database = connection.database.as_deref().unwrap_or("postgres");
        let mut arguments = connection_arguments(connection);
        arguments.push(format!("--dbname={}", shell_quote(database)));
        return Ok(wrap_tool_command_with_environment(
            "pg_dumpall",
            "PGPASSWORD",
            &arguments,
            &connection_environment(connection),
            paths,
            output,
            None,
        ));
    }

    let database = scope
        .database(connection.database.as_deref())
        .ok_or_else(|| {
            DatabaseError::validation(
                "Choose the PostgreSQL database to dump; the profile has no default database.",
            )
        })?;
    let mut arguments = connection_arguments(connection);
    arguments.push(format!("--dbname={}", shell_quote(database)));
    arguments.push(format!("--format={}", format_name(format)?));
    if no_owner {
        arguments.push("--no-owner".to_string());
    }
    match scope {
        DumpScope::Database { .. } => {}
        DumpScope::Schema { schema, .. } => {
            arguments.push(format!(
                "--schema={}",
                shell_quote(&identifier_pattern(schema))
            ));
        }
        DumpScope::Tables { schema, tables, .. } => {
            for table in tables {
                let pattern = match schema {
                    Some(schema) => format!(
                        "{}.{}",
                        identifier_pattern(schema),
                        identifier_pattern(table)
                    ),
                    None => identifier_pattern(table),
                };
                arguments.push(format!("--table={}", shell_quote(&pattern)));
            }
        }
        DumpScope::Cluster => unreachable!("cluster scope is handled above"),
    }

    Ok(wrap_tool_command_with_environment(
        "pg_dump",
        "PGPASSWORD",
        &arguments,
        &connection_environment(connection),
        paths,
        output,
        None,
    ))
}

/// Builds the fixed PostgreSQL restore command for one format.
pub fn restore_command(
    connection: &DumpConnection,
    format: DumpFormat,
    target_database: &Identifier,
    clean: bool,
    no_owner: bool,
    paths: &DumpPaths,
    _vendor: DumpToolVendor,
) -> Result<String, DatabaseError> {
    let input = paths
        .input
        .as_deref()
        .ok_or_else(|| DatabaseError::internal("The PostgreSQL restore input path is missing."))?;

    match format {
        DumpFormat::PlainSql => {
            if clean || no_owner {
                return Err(DatabaseError::validation(
                    "Plain SQL restores do not support clean or no-owner options.",
                ));
            }
            let mut arguments = connection_arguments(connection);
            arguments.push("--no-psqlrc".to_string());
            arguments.push("--set=ON_ERROR_STOP=on".to_string());
            arguments.push(format!(
                "--dbname={}",
                shell_quote(target_database.as_str())
            ));
            Ok(wrap_tool_command_with_environment(
                "psql",
                "PGPASSWORD",
                &arguments,
                &connection_environment(connection),
                paths,
                "/dev/null",
                Some(input),
            ))
        }
        DumpFormat::Custom | DumpFormat::Tar => {
            let mut arguments = connection_arguments(connection);
            arguments.push("--exit-on-error".to_string());
            arguments.push(format!("--format={}", format_name(format)?));
            arguments.push(format!(
                "--dbname={}",
                shell_quote(target_database.as_str())
            ));
            if clean {
                arguments.push("--clean".to_string());
                arguments.push("--if-exists".to_string());
            }
            if no_owner {
                arguments.push("--no-owner".to_string());
            }
            arguments.push(shell_quote(input));
            Ok(wrap_tool_command_with_environment(
                "pg_restore",
                "PGPASSWORD",
                &arguments,
                &connection_environment(connection),
                paths,
                "/dev/null",
                None,
            ))
        }
    }
}

fn connection_arguments(connection: &DumpConnection) -> Vec<String> {
    let mut arguments = vec!["--no-password".to_string()];
    arguments.push(format!("--host={}", shell_quote(&connection.host)));
    arguments.push(format!("--port={}", connection.port));
    arguments.push(format!("--username={}", shell_quote(&connection.username)));
    arguments
}

fn connection_environment(connection: &DumpConnection) -> Vec<String> {
    ssl_mode(connection.ssl_mode.as_deref())
        .map(|mode| vec![format!("PGSSLMODE={mode}")])
        .unwrap_or_default()
}

fn ssl_mode(mode: Option<&str>) -> Option<&'static str> {
    match mode? {
        "disabled" => Some("disable"),
        "preferred" => Some("prefer"),
        "required" => Some("require"),
        "verify-ca" => Some("verify-ca"),
        "verify-identity" => Some("verify-full"),
        _ => None,
    }
}

fn format_name(format: DumpFormat) -> Result<&'static str, DatabaseError> {
    match format {
        DumpFormat::PlainSql => Ok("plain"),
        DumpFormat::Custom => Ok("custom"),
        DumpFormat::Tar => Ok("tar"),
    }
}

fn identifier_pattern(identifier: &Identifier) -> String {
    format!("\"{}\"", identifier.as_str().replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> DumpConnection {
        DumpConnection {
            engine: crate::storage::DatabaseEngine::Postgresql,
            host: "127.0.0.1".to_string(),
            port: 5432,
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            ssl_mode: None,
            read_only: false,
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
    fn database_scope_uses_pg_dump_and_no_password_argument() {
        let command = export_command(
            &connection(),
            &DumpScope::Database { database: None },
            DumpFormat::PlainSql,
            false,
            &paths(),
            DumpToolVendor::Postgresql,
        )
        .expect("command");

        assert!(command.contains("exec pg_dump --no-password"));
        assert!(command.contains("--dbname='app'"));
        assert!(command.contains("--format=plain"));
        assert!(command.contains("PGPASSWORD=$(cat '/tmp/serverdesk-dump-fixture/password')"));
        assert!(!command.contains("--password"));
        assert!(!command.contains("hunter2"));
    }

    #[test]
    fn schema_and_table_scope_quote_identifiers_as_patterns() {
        let schema = export_command(
            &connection(),
            &DumpScope::Schema {
                database: None,
                schema: Identifier::new("Public Data").expect("schema"),
            },
            DumpFormat::Custom,
            true,
            &paths(),
            DumpToolVendor::Postgresql,
        )
        .expect("schema command");
        assert!(schema.contains("--format=custom"));
        assert!(schema.contains("--no-owner"));
        assert!(schema.contains("--schema='\"Public Data\"'"));

        let tables = export_command(
            &connection(),
            &DumpScope::Tables {
                database: None,
                schema: Some(Identifier::new("public").expect("schema")),
                tables: vec![
                    Identifier::new("users").expect("table"),
                    Identifier::new("Order Items").expect("table"),
                ],
            },
            DumpFormat::Tar,
            false,
            &paths(),
            DumpToolVendor::Postgresql,
        )
        .expect("table command");
        assert!(tables.contains("--format=tar"));
        assert!(tables.contains("--table='\"public\".\"users\"'"));
        assert!(tables.contains("--table='\"public\".\"Order Items\"'"));
    }

    #[test]
    fn cluster_scope_uses_pg_dumpall_and_plain_sql_only() {
        let command = export_command(
            &connection(),
            &DumpScope::Cluster,
            DumpFormat::PlainSql,
            false,
            &paths(),
            DumpToolVendor::Postgresql,
        )
        .expect("command");
        assert!(command.contains("exec pg_dumpall --no-password"));
        assert!(command.contains("--dbname='app'"));

        let error = export_command(
            &connection(),
            &DumpScope::Cluster,
            DumpFormat::Custom,
            false,
            &paths(),
            DumpToolVendor::Postgresql,
        )
        .expect_err("unsupported");
        assert_eq!(
            error.kind,
            crate::database::errors::DatabaseErrorKind::Capability
        );
    }

    #[test]
    fn plain_restore_uses_psql_and_stops_on_error() {
        let mut paths = paths();
        paths.input = Some("/tmp/serverdesk-dump-fixture/restore.input".to_string());
        let command = restore_command(
            &connection(),
            DumpFormat::PlainSql,
            &Identifier::new("app").expect("database"),
            false,
            false,
            &paths,
            DumpToolVendor::Postgresql,
        )
        .expect("command");

        assert!(command.contains("exec psql --no-password"));
        assert!(command.contains("--no-psqlrc"));
        assert!(command.contains("--set=ON_ERROR_STOP=on"));
        assert!(command.contains("< '/tmp/serverdesk-dump-fixture/restore.input'"));
        assert!(!command.contains("--password"));
    }

    #[test]
    fn archive_restore_uses_pg_restore_with_fixed_flags() {
        let mut paths = paths();
        paths.input = Some("/tmp/serverdesk-dump-fixture/restore.input".to_string());
        let command = restore_command(
            &connection(),
            DumpFormat::Custom,
            &Identifier::new("app").expect("database"),
            true,
            true,
            &paths,
            DumpToolVendor::Postgresql,
        )
        .expect("command");

        assert!(command.contains("exec pg_restore --no-password"));
        assert!(command.contains("--exit-on-error"));
        assert!(command.contains("--format=custom"));
        assert!(command.contains("--clean --if-exists"));
        assert!(command.contains("--no-owner"));
        assert!(command.contains("'/tmp/serverdesk-dump-fixture/restore.input'"));
    }

    #[test]
    fn hostile_database_name_stays_inside_one_shell_argument() {
        let mut connection = connection();
        connection.database = Some("app'; touch /tmp/pwn; --".to_string());
        let command = export_command(
            &connection,
            &DumpScope::Database { database: None },
            DumpFormat::PlainSql,
            false,
            &paths(),
            DumpToolVendor::Postgresql,
        )
        .expect("command");

        assert!(command.contains("'app'\\''; touch /tmp/pwn; --'"));
        assert!(!command.contains("; touch /tmp/pwn; -- --format"));
    }

    #[test]
    fn generated_commands_parse_as_posix_shell() {
        let commands = [
            export_command(
                &connection(),
                &DumpScope::Database { database: None },
                DumpFormat::Custom,
                false,
                &paths(),
                DumpToolVendor::Postgresql,
            )
            .expect("export command"),
            {
                let mut paths = paths();
                paths.input = Some("/tmp/serverdesk-dump-fixture/restore.input".to_string());
                restore_command(
                    &connection(),
                    DumpFormat::Custom,
                    &Identifier::new("app").expect("database"),
                    true,
                    true,
                    &paths,
                    DumpToolVendor::Postgresql,
                )
                .expect("restore command")
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
    fn ssl_mode_is_mapped_to_the_client_spelling() {
        let mut connection = connection();
        connection.ssl_mode = Some("verify-identity".to_string());
        let command = export_command(
            &connection,
            &DumpScope::Database { database: None },
            DumpFormat::PlainSql,
            false,
            &paths(),
            DumpToolVendor::Postgresql,
        )
        .expect("command");
        assert!(command.contains("PGSSLMODE=verify-full"));
    }
}
