//! Structured, fixed remote commands for the first end-to-end vertical slice.
//!
//! Only commands in [`RemoteCommand`] can be executed through this path. The
//! command line is a fixed template, so no user-supplied text is concatenated
//! into a shell command.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::core::{
    AppError,
    ssh::{CommandOutput, ConnectionManager, ConnectionState},
};

/// The read-only remote commands supported by this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommand {
    Hostname,
    Uptime,
}

impl RemoteCommand {
    /// Every command exposed by this structured path.
    pub const ALL: [Self; 2] = [Self::Hostname, Self::Uptime];

    /// The exact command line sent over SSH. These are fixed templates.
    pub const fn template(self) -> &'static str {
        match self {
            Self::Hostname => "hostname",
            Self::Uptime => "uptime",
        }
    }
}

/// A remote command and its complete, structured result.
///
/// A non-zero exit status is not a transport error: it is returned to the
/// caller so the UI can show stdout, stderr, and the exit code together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCommandResult {
    pub server_id: String,
    pub command: RemoteCommand,
    pub template: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
    pub succeeded: bool,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

impl RemoteCommandResult {
    fn from_output(
        server_id: &str,
        command: RemoteCommand,
        output: CommandOutput,
        duration_ms: u64,
        connection_state: ConnectionState,
    ) -> Self {
        Self {
            server_id: server_id.to_string(),
            command,
            template: command.template().to_string(),
            succeeded: output.exit_code == Some(0),
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.exit_code,
            duration_ms,
            connection_state,
        }
    }
}

/// Executes one fixed command over an existing authenticated SSH session.
pub async fn run_remote_command(
    manager: &ConnectionManager,
    server_id: &str,
    command: RemoteCommand,
) -> Result<RemoteCommandResult, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let started = Instant::now();
    let output = manager.execute(server_id, command.template()).await?;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let connection_state = manager.connection_state(server_id)?.state;

    Ok(RemoteCommandResult::from_output(
        server_id,
        command,
        output,
        duration_ms,
        connection_state,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(stdout: &str, stderr: &str, exit_code: Option<u32>) -> CommandOutput {
        CommandOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    #[test]
    fn command_lines_are_fixed_templates_without_arguments() {
        assert_eq!(RemoteCommand::Hostname.template(), "hostname");
        assert_eq!(RemoteCommand::Uptime.template(), "uptime");
        assert_eq!(
            RemoteCommand::ALL.map(RemoteCommand::template),
            ["hostname", "uptime"]
        );
        for command in RemoteCommand::ALL {
            assert!(!command.template().contains(char::is_whitespace));
            assert!(!command.template().contains(';'));
            assert!(!command.template().contains('|'));
            assert!(!command.template().contains('$'));
        }
    }

    #[test]
    fn serializes_typed_results_for_the_frontend() {
        let result = RemoteCommandResult::from_output(
            "server-1",
            RemoteCommand::Uptime,
            output("up 1 day\n", "", Some(0)),
            42,
            ConnectionState::Connected,
        );
        let value = serde_json::to_value(result).expect("serialized result");

        assert_eq!(value["serverId"], "server-1");
        assert_eq!(value["command"], "uptime");
        assert_eq!(value["template"], "uptime");
        assert_eq!(value["stdout"], "up 1 day\n");
        assert_eq!(value["stderr"], "");
        assert_eq!(value["exitCode"], 0);
        assert_eq!(value["succeeded"], true);
        assert_eq!(value["durationMs"], 42);
        assert_eq!(value["connectionState"], "connected");
    }

    #[test]
    fn preserves_stderr_and_nonzero_exit_codes() {
        let result = RemoteCommandResult::from_output(
            "server-1",
            RemoteCommand::Uptime,
            output("partial\n", "uptime: failed\n", Some(7)),
            5,
            ConnectionState::Connected,
        );

        assert_eq!(result.stdout, "partial\n");
        assert_eq!(result.stderr, "uptime: failed\n");
        assert_eq!(result.exit_code, Some(7));
        assert!(!result.succeeded);
    }

    #[test]
    fn missing_exit_status_is_not_treated_as_success() {
        let result = RemoteCommandResult::from_output(
            "server-1",
            RemoteCommand::Hostname,
            output("fixture\n", "", None),
            1,
            ConnectionState::Connected,
        );

        assert!(!result.succeeded);
        assert_eq!(result.exit_code, None);
    }

    #[test]
    fn rejects_unknown_command_names_during_deserialization() {
        assert!(serde_json::from_str::<RemoteCommand>("\"hostname\"").is_ok());
        assert!(serde_json::from_str::<RemoteCommand>("\"rm -rf /\"").is_err());
        assert!(serde_json::from_str::<RemoteCommand>("\"hostname; id\"").is_err());
    }
}
