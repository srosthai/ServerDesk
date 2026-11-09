//! Safe, fixed Laravel Artisan actions.
//!
//! Every action is a closed enum value that maps to one command template. The
//! caller never supplies command text. The detected project path is normalized
//! and shell-quoted as a single `cd` argument so every action runs with that
//! path as its working directory.
//!
//! Output is bounded before it reaches the UI. Lines that look like `.env`
//! assignments, secret assignments, bearer tokens, credentialed URLs, or
//! private-key blocks are redacted as defense in depth. Operation history
//! stores only a safe summary built from the fixed action and path, never raw
//! command output.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::core::{
    AppError,
    command::artisan_command,
    projects::{RemotePath, normalize_remote_path},
    redaction::redact_text,
    ssh::{ConnectionManager, ConnectionState},
};

/// Hard output bound applied to one Artisan action command.
pub const MAX_ARTISAN_OUTPUT_BYTES: usize = 64 * 1024;
const _: () = assert!(MAX_ARTISAN_OUTPUT_BYTES <= 64 * 1024);

/// Hard command-duration bound applied on top of the connection timeout.
pub const ARTISAN_ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum characters kept from each output stream after redaction.
const MAX_ARTISAN_STREAM_CHARS: usize = 24 * 1024;

/// Explanation shown for `queue:restart` in the UI.
pub const QUEUE_RESTART_EXPLANATION: &str = "Signals queue workers to finish their current job and \
     restart gracefully. This is a Laravel-level restart and does not restart Supervisor or a \
     systemd service.";

/// The closed allowlist of Artisan actions exposed by ServerDesk.
///
/// The enum is deserialized from the frontend, so unknown action strings are
/// rejected by Serde before any command template is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtisanAction {
    About,
    RouteList,
    ConfigCache,
    ConfigClear,
    CacheClear,
    OptimizeClear,
    QueueRestart,
    MigrateStatus,
}

impl ArtisanAction {
    pub const ALL: [Self; 8] = [
        Self::About,
        Self::RouteList,
        Self::ConfigCache,
        Self::ConfigClear,
        Self::CacheClear,
        Self::OptimizeClear,
        Self::QueueRestart,
        Self::MigrateStatus,
    ];

    /// The Artisan command name without the `php artisan` prefix.
    pub const fn argument(self) -> &'static str {
        match self {
            Self::About => "about",
            Self::RouteList => "route:list",
            Self::ConfigCache => "config:cache",
            Self::ConfigClear => "config:clear",
            Self::CacheClear => "cache:clear",
            Self::OptimizeClear => "optimize:clear",
            Self::QueueRestart => "queue:restart",
            Self::MigrateStatus => "migrate:status",
        }
    }

    /// The operation name stored in local history.
    pub const fn operation_name(self) -> &'static str {
        match self {
            Self::About => "artisan-about",
            Self::RouteList => "artisan-route-list",
            Self::ConfigCache => "artisan-config-cache",
            Self::ConfigClear => "artisan-config-clear",
            Self::CacheClear => "artisan-cache-clear",
            Self::OptimizeClear => "artisan-optimize-clear",
            Self::QueueRestart => "artisan-queue-restart",
            Self::MigrateStatus => "artisan-migrate-status",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::About => "About",
            Self::RouteList => "Route list",
            Self::ConfigCache => "Config cache",
            Self::ConfigClear => "Config clear",
            Self::CacheClear => "Clear application cache",
            Self::OptimizeClear => "Clear compiled caches",
            Self::QueueRestart => "Restart queue workers",
            Self::MigrateStatus => "Migration status",
        }
    }

    /// Whether the action changes the remote project or interrupts work.
    ///
    /// Destructive or high-impact actions require an explicit confirmation
    /// before the Rust backend builds or sends any command.
    pub const fn destructive(self) -> bool {
        matches!(
            self,
            Self::ConfigCache
                | Self::ConfigClear
                | Self::CacheClear
                | Self::OptimizeClear
                | Self::QueueRestart
        )
    }

    pub const fn requires_confirmation(self) -> bool {
        self.destructive()
    }

    /// Builds the only command line used for this action and project path.
    ///
    /// The path has already been normalized by [`normalize_remote_path`] and is
    /// quoted as one POSIX shell argument. No other user-controlled text is
    /// interpolated.
    pub fn command(self, path: &RemotePath) -> String {
        artisan_command(path.as_str(), self.argument())
    }
}

/// One bounded, redacted stream of command output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SanitizedStream {
    text: String,
    redacted: bool,
    truncated: bool,
}

/// The complete result of one fixed Artisan action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtisanActionResult {
    pub server_id: String,
    pub project_id: String,
    pub action: ArtisanAction,
    pub command: String,
    pub target_path: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
    pub succeeded: bool,
    pub started_at: i64,
    pub duration_ms: u64,
    /// True when at least one secret-like value was replaced.
    pub secrets_redacted: bool,
    /// True when a stream was shortened at the ServerDesk display limit.
    pub output_truncated: bool,
    pub connection_state: ConnectionState,
}

impl ArtisanActionResult {
    /// A history summary that never includes command output.
    pub fn safe_summary(&self) -> String {
        let exit = match self.exit_code {
            Some(code) => format!("exit code {code}"),
            None => "no exit status".to_string(),
        };
        format!(
            "Ran php artisan {} in {} ({exit})",
            self.action.argument(),
            self.target_path
        )
    }
}

/// Runs one fixed Artisan action in the stored project directory.
///
/// Destructive actions are rejected before any remote command is sent unless
/// `confirmed` is true. Command-level failures (including a non-zero exit
/// status) are returned as structured results so the exact output and exit
/// status stay visible. Transport, timeout, and validation failures are
/// returned as [`AppError`].
pub async fn run_artisan_action(
    manager: &ConnectionManager,
    server_id: &str,
    project_id: &str,
    remote_path: &str,
    action: ArtisanAction,
    confirmed: bool,
) -> Result<ArtisanActionResult, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }
    if project_id.trim().is_empty() {
        return Err(AppError::validation("Project ID is required."));
    }

    let path = normalize_remote_path(remote_path)?;
    if action.requires_confirmation() && !confirmed {
        return Err(AppError::validation(format!(
            "{} requires explicit confirmation before it can run.",
            action.label()
        ))
        .with_details(
            "Review the target project path and the exact fixed command, then confirm the \
             action. No remote command was sent.",
        ));
    }

    let command = action.command(&path);
    let started_at = now_unix_seconds();
    let started = Instant::now();
    let output = tokio::time::timeout(
        ARTISAN_ACTION_TIMEOUT,
        manager.execute_bounded(server_id, &command, MAX_ARTISAN_OUTPUT_BYTES),
    )
    .await
    .map_err(|_| {
        AppError::timeout(format!(
            "The Artisan action timed out after {} seconds.",
            ARTISAN_ACTION_TIMEOUT.as_secs()
        ))
        .with_details(
            "The Artisan process may still be running or may already have completed. Verify the \
             project state before retrying an action that changes files or caches.",
        )
    })??;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let stdout = sanitize_stream(&output.stdout);
    let stderr = sanitize_stream(&output.stderr);
    let connection_state = manager.connection_state(server_id)?.state;

    Ok(ArtisanActionResult {
        server_id: server_id.to_string(),
        project_id: project_id.to_string(),
        action,
        command,
        target_path: path.as_str().to_string(),
        stdout: stdout.text,
        stderr: stderr.text,
        exit_code: output.exit_code,
        succeeded: output.exit_code == Some(0),
        started_at,
        duration_ms,
        secrets_redacted: stdout.redacted || stderr.redacted,
        output_truncated: stdout.truncated || stderr.truncated,
        connection_state,
    })
}

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn sanitize_stream(raw: &str) -> SanitizedStream {
    let redacted = redact_text(raw);
    let mut stream = SanitizedStream {
        redacted: redacted.redacted,
        ..SanitizedStream::default()
    };
    let mut text = redacted.text;

    if text.chars().count() > MAX_ARTISAN_STREAM_CHARS {
        stream.truncated = true;
        text = text
            .chars()
            .take(MAX_ARTISAN_STREAM_CHARS)
            .collect::<String>();
        if let Some(index) = text.rfind('\n') {
            text.truncate(index + 1);
        } else {
            text.clear();
        }
        text.push_str("[output truncated at the ServerDesk display limit]\n");
    }

    stream.text = text;
    stream
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> RemotePath {
        normalize_remote_path(value).expect("normalized fixture path")
    }

    #[test]
    fn exposes_only_the_fixed_action_allowlist() {
        let arguments = ArtisanAction::ALL
            .iter()
            .map(|action| action.argument())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            vec![
                "about",
                "route:list",
                "config:cache",
                "config:clear",
                "cache:clear",
                "optimize:clear",
                "queue:restart",
                "migrate:status",
            ]
        );

        for forbidden in [
            "\"migrate\"",
            "\"db:wipe\"",
            "\"about; rm -rf /\"",
            "\"route:list && id\"",
            "\"\"",
            "null",
        ] {
            assert!(
                serde_json::from_str::<ArtisanAction>(forbidden).is_err(),
                "{forbidden:?} must not deserialize into an Artisan action"
            );
        }

        assert_eq!(
            serde_json::to_value(ArtisanAction::RouteList).expect("serialized action"),
            serde_json::json!("route-list")
        );
    }

    #[test]
    fn commands_are_fixed_and_change_into_the_project_directory() {
        let project = path("/var/www/My App's Files");
        let quoted = "'/var/www/My App'\\''s Files'";

        for (action, argument) in [
            (ArtisanAction::About, "about"),
            (ArtisanAction::RouteList, "route:list"),
            (ArtisanAction::ConfigCache, "config:cache"),
            (ArtisanAction::ConfigClear, "config:clear"),
            (ArtisanAction::CacheClear, "cache:clear"),
            (ArtisanAction::OptimizeClear, "optimize:clear"),
            (ArtisanAction::QueueRestart, "queue:restart"),
            (ArtisanAction::MigrateStatus, "migrate:status"),
        ] {
            let command = action.command(&project);
            assert_eq!(command, format!("cd -- {quoted} && php artisan {argument}"));
            assert!(command.contains(quoted));
            for forbidden in [';', '|', '$', '`', '>', '<', '\n'] {
                assert!(
                    !command.contains(forbidden),
                    "{command:?} must not contain {forbidden:?}"
                );
            }
            assert_eq!(
                action.operation_name(),
                format!("artisan-{}", argument.replace(':', "-"))
            );
        }
    }

    #[test]
    fn destructive_actions_require_confirmation() {
        for action in ArtisanAction::ALL {
            let expected = matches!(
                action,
                ArtisanAction::ConfigCache
                    | ArtisanAction::ConfigClear
                    | ArtisanAction::CacheClear
                    | ArtisanAction::OptimizeClear
                    | ArtisanAction::QueueRestart
            );
            assert_eq!(action.destructive(), expected, "{action:?}");
            assert_eq!(action.requires_confirmation(), expected, "{action:?}");
        }

        assert!(!ArtisanAction::About.destructive());
        assert!(!ArtisanAction::RouteList.destructive());
        assert!(!ArtisanAction::MigrateStatus.destructive());
        assert!(ArtisanAction::CacheClear.destructive());
        assert!(ArtisanAction::QueueRestart.destructive());
    }

    #[test]
    fn redacts_dotenv_lines_and_common_secret_assignments() {
        let output = sanitize_stream(
            "APP_NAME=Fixture\n\
             DB_PASSWORD=hunter2\n\
             QUEUE_CONNECTION=redis\n\
             password: hunter2\n\
             Authorization: Bearer abc123\n\
             database_url=mysql://deploy:db-secret@127.0.0.1/app\n",
        );

        assert!(output.redacted);
        for secret in [
            "Fixture",
            "hunter2",
            "redis",
            "abc123",
            "db-secret",
            "DB_PASSWORD=",
            "APP_NAME=",
        ] {
            assert!(
                !output.text.contains(secret),
                "redacted output still contains {secret:?}: {}",
                output.text
            );
        }
        assert!(output.text.contains("[redacted environment line]"));
        assert!(output.text.contains("password:<redacted>"));
        assert!(output.text.contains("Bearer <redacted>"));
        assert!(
            output
                .text
                .contains("database_url=mysql://deploy:<redacted>@127.0.0.1/app")
        );
    }

    #[test]
    fn redacts_private_key_blocks_and_strips_ansi_sequences() {
        let output = sanitize_stream(
            "\u{1b}[32mroute list\u{1b}[0m\n\
             -----BEGIN OPENSSH PRIVATE KEY-----\n\
             super-secret-key-body\n\
             -----END OPENSSH PRIVATE KEY-----\n\
             done\n",
        );

        assert!(output.redacted);
        assert!(!output.text.contains("super-secret-key-body"));
        assert!(!output.text.contains("BEGIN OPENSSH"));
        assert!(output.text.contains("[redacted private key]"));
        assert!(output.text.contains("route list"));
        assert!(output.text.contains("done"));
        assert!(!output.text.contains('\u{1b}'));
    }

    #[test]
    fn bounds_and_flags_long_output() {
        let line = "x".repeat(MAX_ARTISAN_STREAM_CHARS + 32);
        let output = sanitize_stream(&line);
        assert!(output.truncated);
        assert!(output.text.chars().count() <= MAX_ARTISAN_STREAM_CHARS + 100);
        assert!(
            output
                .text
                .contains("[output truncated at the ServerDesk display limit]")
        );

        let normal = sanitize_stream("short output\n");
        assert!(!normal.truncated);
        assert!(!normal.redacted);
        assert_eq!(normal.text, "short output\n");
    }

    #[test]
    fn safe_summary_never_includes_output() {
        let result = ArtisanActionResult {
            server_id: "server-1".to_string(),
            project_id: "project-1".to_string(),
            action: ArtisanAction::CacheClear,
            command: "cd -- '/var/www/storefront' && php artisan cache:clear".to_string(),
            target_path: "/var/www/storefront".to_string(),
            stdout: "APP_KEY=base64:super-secret\n".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            succeeded: true,
            started_at: 1_762_665_720,
            duration_ms: 42,
            secrets_redacted: true,
            output_truncated: false,
            connection_state: ConnectionState::Connected,
        };

        let summary = result.safe_summary();
        assert!(summary.contains("php artisan cache:clear"));
        assert!(summary.contains("/var/www/storefront"));
        assert!(summary.contains("exit code 0"));
        assert!(!summary.contains("super-secret"));
        assert!(!summary.contains("APP_KEY"));
    }

    #[test]
    fn queue_restart_is_explained_as_a_graceful_worker_restart() {
        assert!(QUEUE_RESTART_EXPLANATION.contains("gracefully"));
        assert!(QUEUE_RESTART_EXPLANATION.contains("does not restart Supervisor"));
        assert_eq!(
            ArtisanAction::QueueRestart.command(&path("/var/www/storefront")),
            "cd -- '/var/www/storefront' && php artisan queue:restart"
        );
    }
}
