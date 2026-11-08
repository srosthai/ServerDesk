//! Bounded, read-only log tails for common system logs and Laravel logs.
//!
//! ServerDesk never downloads a whole log file and never accepts an arbitrary
//! remote path. A request must resolve to either one of the fixed system log
//! paths below or a `.log` file directly inside a Laravel `storage/logs`
//! directory. The remote command tails a bounded number of bytes, and the
//! Rust side applies the requested line limit before returning text to the UI.
//!
//! Missing, unreadable, non-regular, rotating, and truncated logs are reported
//! as typed results so the UI never presents an empty read as a successful
//! log tail.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::{
    AppError,
    projects::{RemotePath, normalize_remote_path, shell_quote},
    ssh::{CommandOutput, ConnectionManager, ConnectionState},
};

/// Lines shown by default when the caller does not choose a limit.
pub const DEFAULT_LOG_LINES: usize = 200;
/// Hard per-request line limit. Larger requests are clamped to this value.
pub const MAX_LOG_LINES: usize = 2_000;
/// Hard per-request byte limit applied to the returned log content.
pub const MAX_LOG_CONTENT_BYTES: usize = 64 * 1024;
/// Transport bound covering the content plus a small metadata budget.
pub const MAX_LOG_COMMAND_OUTPUT_BYTES: usize = MAX_LOG_CONTENT_BYTES + 8 * 1024;
const _: () = assert!(MAX_LOG_CONTENT_BYTES <= 64 * 1024);
const _: () = assert!(MAX_LOG_COMMAND_OUTPUT_BYTES > MAX_LOG_CONTENT_BYTES);

/// Hard command-duration bound applied on top of the connection timeout.
pub const LOG_READ_TIMEOUT: Duration = Duration::from_secs(15);

const MAX_LOG_FILE_NAME_CHARS: usize = 255;
const MAX_LOG_DETAIL_CHARS: usize = 300;

const REAL_MARKER: &str = "__SERVERDESK_LOG_REAL__";
const BEFORE_MARKER: &str = "__SERVERDESK_LOG_BEFORE__";
const AFTER_MARKER: &str = "__SERVERDESK_LOG_AFTER__";
const MISSING_MARKER: &str = "__SERVERDESK_LOG_MISSING__";
const NOT_FILE_MARKER: &str = "__SERVERDESK_LOG_NOT_FILE__";
const UNREADABLE_MARKER: &str = "__SERVERDESK_LOG_UNREADABLE__";
const READ_FAILED_MARKER: &str = "__SERVERDESK_LOG_READ_FAILED__";
const GONE_MARKER: &str = "__SERVERDESK_LOG_GONE__";

/// One common system log offered by ServerDesk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemLogSource {
    pub path: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

/// The fixed allowlist of common system logs.
///
/// This is a closed list. A path that is not one of these exact normalized
/// paths must match the Laravel `storage/logs/*.log` shape instead.
pub const SYSTEM_LOG_SOURCES: [SystemLogSource; 12] = [
    SystemLogSource {
        path: "/var/log/syslog",
        label: "System log (syslog)",
        description: "General system messages on Debian and Ubuntu servers.",
    },
    SystemLogSource {
        path: "/var/log/auth.log",
        label: "Authentication log",
        description: "SSH logins, sudo attempts, and authentication events.",
    },
    SystemLogSource {
        path: "/var/log/messages",
        label: "System messages",
        description: "General system messages on RHEL-family distributions.",
    },
    SystemLogSource {
        path: "/var/log/kern.log",
        label: "Kernel log",
        description: "Kernel messages on Debian and Ubuntu servers.",
    },
    SystemLogSource {
        path: "/var/log/dpkg.log",
        label: "Package log",
        description: "Package installation, upgrade, and removal history.",
    },
    SystemLogSource {
        path: "/var/log/nginx/access.log",
        label: "Nginx access log",
        description: "HTTP requests handled by Nginx.",
    },
    SystemLogSource {
        path: "/var/log/nginx/error.log",
        label: "Nginx error log",
        description: "Nginx startup and request errors.",
    },
    SystemLogSource {
        path: "/var/log/apache2/access.log",
        label: "Apache access log",
        description: "HTTP requests handled by Apache.",
    },
    SystemLogSource {
        path: "/var/log/apache2/error.log",
        label: "Apache error log",
        description: "Apache startup and request errors.",
    },
    SystemLogSource {
        path: "/var/log/mysql/error.log",
        label: "MySQL error log",
        description: "MySQL server startup and runtime errors.",
    },
    SystemLogSource {
        path: "/var/log/redis/redis-server.log",
        label: "Redis log",
        description: "Redis server startup and runtime messages.",
    },
    SystemLogSource {
        path: "/var/log/ufw.log",
        label: "Firewall log (UFW)",
        description: "Blocked and allowed packets recorded by UFW.",
    },
];

/// Whether an approved path is a system log or a Laravel application log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogSourceKind {
    System,
    Laravel,
}

/// The honest outcome of one log-tail request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogTailStatus {
    /// The file was read and the bounded tail is available.
    Available,
    /// The path does not exist, for example after rotation or cleanup.
    Missing,
    /// The path exists but is not a regular file.
    NotFile,
    /// The SSH account does not have permission to read the file.
    Unreadable,
    /// The file exists but the read failed for another reason.
    Unavailable,
}

/// One validated log path inside an approved location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedLogPath {
    pub path: RemotePath,
    pub kind: LogSourceKind,
}

/// A bounded, typed log tail returned to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogTail {
    pub server_id: String,
    pub requested_path: String,
    pub resolved_path: Option<String>,
    pub source_kind: LogSourceKind,
    pub status: LogTailStatus,
    pub detail: Option<String>,
    pub content: String,
    pub returned_lines: usize,
    pub returned_bytes: usize,
    pub file_size_bytes: Option<u64>,
    pub line_limit: usize,
    pub byte_limit: usize,
    pub truncated: bool,
    pub truncated_bytes: bool,
    pub truncated_lines: bool,
    /// True when a partial first line was dropped from a byte-bounded tail.
    pub partial_first_line: bool,
    pub rotated: bool,
    pub rotation_detail: Option<String>,
    pub retrieved_at: i64,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

/// Normalizes and approves one log path.
///
/// Parent segments, relative paths, and control characters are rejected before
/// any remote command is sent. Paths outside the fixed system log allowlist and
/// outside a Laravel `storage/logs` directory are rejected as not approved.
pub fn normalize_log_path(value: &str) -> Result<ApprovedLogPath, AppError> {
    let path = normalize_remote_path(value).map_err(|_| invalid_log_path_error())?;

    if SYSTEM_LOG_SOURCES
        .iter()
        .any(|source| source.path == path.as_str())
    {
        return Ok(ApprovedLogPath {
            path,
            kind: LogSourceKind::System,
        });
    }

    if is_laravel_log_path(path.as_str()) {
        return Ok(ApprovedLogPath {
            path,
            kind: LogSourceKind::Laravel,
        });
    }

    Err(not_approved_log_path_error())
}

/// Verifies that a symlink-resolved remote path still points inside an
/// approved location.
///
/// System log symlinks must stay under `/var/log`. Laravel log symlinks must
/// stay under a `storage/logs` directory. A symlink from an approved path to an
/// arbitrary file is rejected before any content is read.
pub fn validate_resolved_log_path(
    approved: &ApprovedLogPath,
    resolved: &str,
) -> Result<RemotePath, AppError> {
    let resolved = normalize_remote_path(resolved).map_err(|_| resolved_outside_error())?;
    let allowed = match approved.kind {
        LogSourceKind::System => {
            resolved.as_str() == "/var/log" || resolved.as_str().starts_with("/var/log/")
        }
        LogSourceKind::Laravel => is_under_laravel_logs(resolved.as_str()),
    };

    if allowed {
        Ok(resolved)
    } else {
        Err(resolved_outside_error())
    }
}

fn is_laravel_log_path(path: &str) -> bool {
    let segments = path_segments(path);
    let Some(file_name) = segments.last() else {
        return false;
    };

    segments.len() >= 3
        && segments[segments.len() - 3] == "storage"
        && segments[segments.len() - 2] == "logs"
        && is_log_file_name(file_name)
}

fn is_under_laravel_logs(path: &str) -> bool {
    let segments = path_segments(path);
    segments.len() >= 3
        && segments[segments.len() - 3] == "storage"
        && segments[segments.len() - 2] == "logs"
}

fn is_log_file_name(name: &str) -> bool {
    name.chars().count() <= MAX_LOG_FILE_NAME_CHARS
        && name.ends_with(".log")
        && name != ".log"
        && !name.starts_with('.')
}

fn path_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn invalid_log_path_error() -> AppError {
    AppError::validation("The log path is not valid.").with_details(
        "Use an absolute Linux log path. Relative paths, parent segments, control characters, \
         and oversized paths are rejected before any remote command is sent.",
    )
}

fn not_approved_log_path_error() -> AppError {
    AppError::validation("This path is not an approved log location.").with_details(
        "Choose one of the common system logs or a .log file directly inside a Laravel \
         storage/logs directory. ServerDesk does not read arbitrary remote files.",
    )
}

fn resolved_outside_error() -> AppError {
    AppError::validation("The log path resolves outside approved log locations.").with_details(
        "ServerDesk follows the remote symlink only when it still points under /var/log for a \
         system log or under a Laravel storage/logs directory. No file content was read.",
    )
}

/// Builds the only remote command used for one log tail.
///
/// The path is quoted as one shell argument and the byte count is a fixed
/// numeric bound. The command emits small metadata markers on stderr and the
/// actual bounded tail on stdout.
pub fn log_tail_command(path: &str, byte_limit: usize) -> String {
    let quoted = shell_quote(path);
    format!(
        "log_path={quoted}; \
         if [ ! -e \"$log_path\" ]; then printf '%s\\n' '{MISSING_MARKER}' >&2; exit 41; fi; \
         if [ ! -f \"$log_path\" ]; then printf '%s\\n' '{NOT_FILE_MARKER}' >&2; exit 42; fi; \
         resolved=$(readlink -f -- \"$log_path\" 2>/dev/null) || resolved=''; \
         if [ -z \"$resolved\" ]; then printf '%s\\n' '{MISSING_MARKER}' >&2; exit 41; fi; \
         printf '%s%s\\n' '{REAL_MARKER}' \"$resolved\" >&2; \
         if [ ! -r \"$resolved\" ]; then printf '%s\\n' '{UNREADABLE_MARKER}' >&2; exit 43; fi; \
         if ! before=$(stat -c '%d:%i:%s:%Y' -- \"$resolved\" 2>/dev/null); then \
           printf '%s\\n' '{UNREADABLE_MARKER}' >&2; exit 43; \
         fi; \
         printf '%s%s\\n' '{BEFORE_MARKER}' \"$before\" >&2; \
         tail -c {byte_limit} -- \"$resolved\"; \
         read_status=$?; \
         if [ \"$read_status\" -ne 0 ]; then \
           printf '%s%s\\n' '{READ_FAILED_MARKER}' \"$read_status\" >&2; exit 44; \
         fi; \
         if ! after=$(stat -c '%d:%i:%s:%Y' -- \"$resolved\" 2>/dev/null); then \
           printf '%s\\n' '{GONE_MARKER}' >&2; exit 0; \
         fi; \
         printf '%s%s\\n' '{AFTER_MARKER}' \"$after\" >&2"
    )
}

/// Reads one bounded log tail over an existing authenticated SSH session.
pub async fn read_log_tail(
    manager: &ConnectionManager,
    server_id: &str,
    path: &str,
    lines: Option<usize>,
) -> Result<LogTail, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let approved = normalize_log_path(path)?;
    let line_limit = lines.unwrap_or(DEFAULT_LOG_LINES).clamp(1, MAX_LOG_LINES);
    let started = Instant::now();
    let output = tokio::time::timeout(
        LOG_READ_TIMEOUT,
        manager.execute_bounded(
            server_id,
            &log_tail_command(approved.path.as_str(), MAX_LOG_CONTENT_BYTES),
            MAX_LOG_COMMAND_OUTPUT_BYTES,
        ),
    )
    .await
    .map_err(|_| {
        AppError::timeout(format!(
            "The log tail timed out after {} seconds.",
            LOG_READ_TIMEOUT.as_secs()
        ))
    })??;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let retrieved_at = now_millis();
    let connection_state = manager.connection_state(server_id)?.state;
    let metadata = parse_remote_log_metadata(&output.stderr);

    let resolved_path = match metadata.resolved_path.as_deref() {
        Some(value) => Some(
            validate_resolved_log_path(&approved, value)?
                .as_str()
                .to_string(),
        ),
        None => None,
    };
    let file_size_bytes = metadata.before.map(|before| before.size);
    let (rotated, rotation_detail) = rotation_state(
        metadata.before.as_ref(),
        metadata.after.as_ref(),
        metadata.gone,
    );
    let (status, detail) = classify_status(&output, &metadata);

    let mut tail = LogTail {
        server_id: server_id.to_string(),
        requested_path: approved.path.as_str().to_string(),
        resolved_path,
        source_kind: approved.kind,
        status,
        detail,
        content: String::new(),
        returned_lines: 0,
        returned_bytes: 0,
        file_size_bytes,
        line_limit,
        byte_limit: MAX_LOG_CONTENT_BYTES,
        truncated: false,
        truncated_bytes: false,
        truncated_lines: false,
        partial_first_line: false,
        rotated,
        rotation_detail,
        retrieved_at,
        duration_ms,
        connection_state,
    };

    if tail.status == LogTailStatus::Available {
        let bounded = bounded_tail(
            &output.stdout,
            file_size_bytes,
            MAX_LOG_CONTENT_BYTES,
            line_limit,
        );
        tail.content = bounded.content;
        tail.returned_lines = bounded.returned_lines;
        tail.returned_bytes = bounded.returned_bytes;
        tail.truncated_bytes = bounded.truncated_bytes;
        tail.truncated_lines = bounded.truncated_lines;
        tail.partial_first_line = bounded.partial_first_line;
        tail.truncated = bounded.truncated_bytes || bounded.truncated_lines;
    }

    Ok(tail)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileMeta {
    device: u64,
    inode: u64,
    size: u64,
    modified_at: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RemoteLogMetadata {
    resolved_path: Option<String>,
    before: Option<FileMeta>,
    after: Option<FileMeta>,
    missing: bool,
    not_file: bool,
    unreadable: bool,
    gone: bool,
    read_failed: Option<u32>,
    error_text: Option<String>,
}

fn parse_remote_log_metadata(stderr: &str) -> RemoteLogMetadata {
    let mut metadata = RemoteLogMetadata::default();

    for raw_line in stderr.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix(REAL_MARKER) {
            let value = value.trim();
            if !value.is_empty() {
                metadata.resolved_path = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix(BEFORE_MARKER) {
            metadata.before = parse_file_meta(value.trim());
        } else if let Some(value) = line.strip_prefix(AFTER_MARKER) {
            metadata.after = parse_file_meta(value.trim());
        } else if line == MISSING_MARKER {
            metadata.missing = true;
        } else if line == NOT_FILE_MARKER {
            metadata.not_file = true;
        } else if line == UNREADABLE_MARKER {
            metadata.unreadable = true;
        } else if line == GONE_MARKER {
            metadata.gone = true;
        } else if let Some(value) = line.strip_prefix(READ_FAILED_MARKER) {
            metadata.read_failed = value.trim().parse().ok();
        } else if !line.trim().is_empty() && metadata.error_text.is_none() {
            metadata.error_text = Some(bounded_detail(line));
        }
    }

    metadata
}

fn parse_file_meta(value: &str) -> Option<FileMeta> {
    let mut parts = value.split(':');
    let device = parts.next()?.parse().ok()?;
    let inode = parts.next()?.parse().ok()?;
    let size = parts.next()?.parse().ok()?;
    let modified_at = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }

    Some(FileMeta {
        device,
        inode,
        size,
        modified_at,
    })
}

fn rotation_state(
    before: Option<&FileMeta>,
    after: Option<&FileMeta>,
    gone: bool,
) -> (bool, Option<String>) {
    if gone {
        return (
            true,
            Some(
                "The log file was removed or replaced while it was being read. The tail may be \
                 incomplete."
                    .to_string(),
            ),
        );
    }

    let (Some(before), Some(after)) = (before, after) else {
        return (false, None);
    };

    if before.device != after.device || before.inode != after.inode {
        return (
            true,
            Some(
                "The log file was rotated while it was being read. This tail may come from the old \
                 file, the new file, or a mix of both."
                    .to_string(),
            ),
        );
    }

    if after.size < before.size || after.modified_at < before.modified_at {
        return (
            true,
            Some(
                "The log file changed while it was being read (likely copy-truncate rotation). This \
                 tail may not be a continuous view."
                    .to_string(),
            ),
        );
    }

    (false, None)
}

fn classify_status(
    output: &CommandOutput,
    metadata: &RemoteLogMetadata,
) -> (LogTailStatus, Option<String>) {
    if metadata.missing {
        return (
            LogTailStatus::Missing,
            Some(
                "The log file does not exist on the server. Rotation, cleanup, or a different log \
                 path may explain the missing file."
                    .to_string(),
            ),
        );
    }
    if metadata.not_file {
        return (
            LogTailStatus::NotFile,
            Some(
                "The selected path is not a regular file, so it cannot be read as a log tail."
                    .to_string(),
            ),
        );
    }
    if metadata.unreadable {
        return (LogTailStatus::Unreadable, Some(unreadable_detail(metadata)));
    }
    if let Some(code) = metadata.read_failed {
        let status = infer_failure_status(metadata.error_text.as_deref());
        return (
            status,
            Some(failure_detail(
                status,
                Some(code),
                metadata.error_text.as_deref(),
            )),
        );
    }
    if output.exit_code != Some(0) {
        let status = infer_failure_status(metadata.error_text.as_deref());
        return (
            status,
            Some(failure_detail(
                status,
                output.exit_code,
                metadata.error_text.as_deref(),
            )),
        );
    }

    (LogTailStatus::Available, None)
}

fn unreadable_detail(metadata: &RemoteLogMetadata) -> String {
    let base = "The SSH account cannot read this log file. ServerDesk does not elevate with sudo; \
                grant read access to the account or choose another approved log.";
    match metadata.error_text.as_deref() {
        Some(detail) if !detail.is_empty() => format!("{base} Remote detail: {detail}"),
        _ => base.to_string(),
    }
}

fn infer_failure_status(error_text: Option<&str>) -> LogTailStatus {
    let Some(text) = error_text else {
        return LogTailStatus::Unavailable;
    };
    let lower = text.to_ascii_lowercase();
    if lower.contains("permission denied") {
        LogTailStatus::Unreadable
    } else if lower.contains("no such file") || lower.contains("cannot open") {
        LogTailStatus::Missing
    } else if lower.contains("is a directory") || lower.contains("not a regular file") {
        LogTailStatus::NotFile
    } else {
        LogTailStatus::Unavailable
    }
}

fn failure_detail(
    status: LogTailStatus,
    exit_code: Option<u32>,
    error_text: Option<&str>,
) -> String {
    let label = match status {
        LogTailStatus::Unreadable => "The SSH account cannot read this log file",
        LogTailStatus::Missing => "The log file could not be opened",
        LogTailStatus::NotFile => "The selected path is not a regular file",
        LogTailStatus::Available | LogTailStatus::Unavailable => "The log tail command failed",
    };
    let exit = match exit_code {
        Some(code) => format!("exit code {code}"),
        None => "no exit status".to_string(),
    };
    match error_text {
        Some(text) if !text.is_empty() => format!("{label} ({exit}: {text})."),
        _ => format!("{label} ({exit})."),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct BoundedTail {
    content: String,
    returned_lines: usize,
    returned_bytes: usize,
    truncated_bytes: bool,
    truncated_lines: bool,
    partial_first_line: bool,
}

fn bounded_tail(
    raw: &str,
    file_size: Option<u64>,
    byte_limit: usize,
    line_limit: usize,
) -> BoundedTail {
    let byte_limit_u64 = u64::try_from(byte_limit).unwrap_or(u64::MAX);
    let truncated_bytes = match file_size {
        Some(size) => size > byte_limit_u64,
        None => raw.len() >= byte_limit,
    };

    // A byte-bounded tail can start in the middle of one line. Drop that
    // partial line instead of displaying corrupted text.
    let remainder = if truncated_bytes {
        match raw.find('\n') {
            Some(index) => &raw[index + 1..],
            None => "",
        }
    } else {
        raw
    };

    let ends_with_newline = remainder.ends_with('\n');
    let mut lines = remainder.lines().collect::<Vec<_>>();
    let truncated_lines = lines.len() > line_limit;
    if truncated_lines {
        lines = lines.split_off(lines.len() - line_limit);
    }

    let mut content = lines.join("\n");
    if ends_with_newline && !content.is_empty() {
        content.push('\n');
    }

    BoundedTail {
        returned_lines: content.lines().count(),
        returned_bytes: content.len(),
        content,
        truncated_bytes,
        truncated_lines,
        partial_first_line: truncated_bytes,
    }
}

fn bounded_detail(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .take(MAX_LOG_DETAIL_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approves_system_and_laravel_logs_and_rejects_everything_else() {
        let syslog = normalize_log_path(" /var/log//syslog ").expect("syslog");
        assert_eq!(syslog.path.as_str(), "/var/log/syslog");
        assert_eq!(syslog.kind, LogSourceKind::System);

        let laravel = normalize_log_path("/var/www/storefront/storage/logs/laravel.log")
            .expect("laravel log");
        assert_eq!(
            laravel.path.as_str(),
            "/var/www/storefront/storage/logs/laravel.log"
        );
        assert_eq!(laravel.kind, LogSourceKind::Laravel);

        for valid in [
            "/storage/logs/laravel.log",
            "/var/www/storefront/storage/logs/laravel-2026-11-09.log",
            "/srv/apps/api/storage/logs/worker.log",
        ] {
            assert!(
                normalize_log_path(valid).is_ok(),
                "{valid:?} must be approved"
            );
        }

        for invalid in [
            "",
            "var/log/syslog",
            "/etc/passwd",
            "/var/log/../etc/passwd",
            "/var/log/syslog/extra",
            "/var/www/storefront/storage/logs/../../../../etc/passwd",
            "/var/www/storefront/storage/laravel.log",
            "/var/www/storefront/logs/laravel.log",
            "/var/www/storefront/storage/logs/laravel.txt",
            "/var/www/storefront/storage/logs/.log",
            "/var/www/storefront/storage/logs/.hidden.log",
            "/var/log/sys\nlog",
        ] {
            assert!(
                normalize_log_path(invalid).is_err(),
                "{invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn resolved_symlinks_must_stay_inside_approved_locations() {
        let syslog = normalize_log_path("/var/log/nginx/access.log").expect("system path");
        assert!(
            validate_resolved_log_path(&syslog, "/var/log/nginx/access.log.1").is_ok(),
            "a rotated system log is still under /var/log"
        );
        assert!(validate_resolved_log_path(&syslog, "/etc/shadow").is_err());

        let laravel =
            normalize_log_path("/var/www/app/storage/logs/laravel.log").expect("laravel path");
        assert!(
            validate_resolved_log_path(&laravel, "/srv/app/storage/logs/laravel.log.1").is_ok(),
            "a resolved Laravel log may be an older rotated file in storage/logs"
        );
        assert!(validate_resolved_log_path(&laravel, "/etc/passwd").is_err());
        assert!(validate_resolved_log_path(&laravel, "/srv/app/logs/laravel.log").is_err());
    }

    #[test]
    fn command_quotes_the_path_and_applies_the_byte_limit() {
        let path = normalize_log_path("/var/www/My App's Files/storage/logs/laravel.log")
            .expect("approved path");
        let command = log_tail_command(path.path.as_str(), MAX_LOG_CONTENT_BYTES);

        assert!(
            command.contains("log_path='/var/www/My App'\\''s Files/storage/logs/laravel.log'")
        );
        assert!(command.contains("readlink -f --"));
        assert!(command.contains("tail -c 65536 -- \"$resolved\""));
        assert!(command.contains("stat -c '%d:%i:%s:%Y'"));
        assert!(!command.contains("cat /var/www/My App's Files"));
        assert!(!command.contains("rm -rf"));
    }

    #[test]
    fn parses_metadata_and_detects_rotation() {
        let metadata = parse_remote_log_metadata(
            "__SERVERDESK_LOG_REAL__/var/www/app/storage/logs/laravel.log\n\
             __SERVERDESK_LOG_BEFORE__8:100:2048:1762665700\n\
             __SERVERDESK_LOG_AFTER__8:101:128:1762665701\n",
        );
        assert_eq!(
            metadata.resolved_path.as_deref(),
            Some("/var/www/app/storage/logs/laravel.log")
        );
        assert_eq!(
            metadata.before,
            Some(FileMeta {
                device: 8,
                inode: 100,
                size: 2048,
                modified_at: 1_762_665_700,
            })
        );
        let (rotated, detail) =
            rotation_state(metadata.before.as_ref(), metadata.after.as_ref(), false);
        assert!(rotated);
        assert!(detail.expect("rotation detail").contains("rotated"));

        let copy_truncate = parse_remote_log_metadata(
            "__SERVERDESK_LOG_BEFORE__8:100:2048:1762665700\n\
             __SERVERDESK_LOG_AFTER__8:100:128:1762665701\n",
        );
        let (rotated, detail) = rotation_state(
            copy_truncate.before.as_ref(),
            copy_truncate.after.as_ref(),
            false,
        );
        assert!(rotated);
        assert!(
            detail
                .expect("copy truncate detail")
                .contains("copy-truncate")
        );

        let stable = parse_remote_log_metadata(
            "__SERVERDESK_LOG_BEFORE__8:100:2048:1762665700\n\
             __SERVERDESK_LOG_AFTER__8:100:4096:1762665701\n",
        );
        assert_eq!(
            rotation_state(stable.before.as_ref(), stable.after.as_ref(), false),
            (false, None)
        );

        let gone = parse_remote_log_metadata("__SERVERDESK_LOG_GONE__\n");
        let (rotated, detail) = rotation_state(None, None, gone.gone);
        assert!(rotated);
        assert!(detail.expect("gone detail").contains("removed or replaced"));
    }

    #[test]
    fn reports_missing_unreadable_and_non_regular_files() {
        let output = CommandOutput {
            stdout: String::new(),
            stderr: "__SERVERDESK_LOG_MISSING__\n".to_string(),
            exit_code: Some(41),
        };
        let (status, detail) = classify_status(&output, &parse_remote_log_metadata(&output.stderr));
        assert_eq!(status, LogTailStatus::Missing);
        assert!(detail.expect("missing detail").contains("does not exist"));

        let output = CommandOutput {
            stdout: String::new(),
            stderr: "__SERVERDESK_LOG_UNREADABLE__\n".to_string(),
            exit_code: Some(43),
        };
        let (status, detail) = classify_status(&output, &parse_remote_log_metadata(&output.stderr));
        assert_eq!(status, LogTailStatus::Unreadable);
        assert!(
            detail
                .expect("unreadable detail")
                .contains("does not elevate with sudo")
        );

        let output = CommandOutput {
            stdout: String::new(),
            stderr: "__SERVERDESK_LOG_NOT_FILE__\n".to_string(),
            exit_code: Some(42),
        };
        let (status, _) = classify_status(&output, &parse_remote_log_metadata(&output.stderr));
        assert_eq!(status, LogTailStatus::NotFile);

        let output = CommandOutput {
            stdout: String::new(),
            stderr: "tail: cannot open '/var/log/x' for reading: Permission denied\n".to_string(),
            exit_code: Some(1),
        };
        let (status, detail) = classify_status(&output, &parse_remote_log_metadata(&output.stderr));
        assert_eq!(status, LogTailStatus::Unreadable);
        assert!(
            detail
                .expect("permission detail")
                .contains("Permission denied")
        );
    }

    #[test]
    fn bounds_bytes_and_lines_and_drops_a_partial_first_line() {
        let normal = bounded_tail("first\nsecond\n", Some(13), MAX_LOG_CONTENT_BYTES, 10);
        assert_eq!(normal.content, "first\nsecond\n");
        assert_eq!(normal.returned_lines, 2);
        assert_eq!(normal.returned_bytes, 13);
        assert!(!normal.truncated_bytes);
        assert!(!normal.truncated_lines);
        assert!(!normal.partial_first_line);

        let line_bounded = bounded_tail("one\ntwo\nthree\nfour\n", Some(20), 1024, 2);
        assert_eq!(line_bounded.content, "three\nfour\n");
        assert_eq!(line_bounded.returned_lines, 2);
        assert!(line_bounded.truncated_lines);
        assert!(!line_bounded.truncated_bytes);

        let byte_bounded = bounded_tail("partial\nsecond\nthird\n", Some(4096), 12, 100);
        assert_eq!(byte_bounded.content, "second\nthird\n");
        assert!(byte_bounded.truncated_bytes);
        assert!(byte_bounded.partial_first_line);
        assert_eq!(byte_bounded.returned_lines, 2);

        let only_partial_line = bounded_tail("one huge line without newline", Some(4096), 8, 100);
        assert_eq!(only_partial_line.content, "");
        assert_eq!(only_partial_line.returned_lines, 0);
        assert!(only_partial_line.truncated_bytes);
        assert!(only_partial_line.partial_first_line);
    }

    #[test]
    fn serializes_the_typed_log_tail_for_the_frontend() {
        let tail = LogTail {
            server_id: "server-1".to_string(),
            requested_path: "/var/www/app/storage/logs/laravel.log".to_string(),
            resolved_path: Some("/var/www/app/storage/logs/laravel.log".to_string()),
            source_kind: LogSourceKind::Laravel,
            status: LogTailStatus::Available,
            detail: None,
            content: "line\n".to_string(),
            returned_lines: 1,
            returned_bytes: 5,
            file_size_bytes: Some(5),
            line_limit: 200,
            byte_limit: MAX_LOG_CONTENT_BYTES,
            truncated: false,
            truncated_bytes: false,
            truncated_lines: false,
            partial_first_line: false,
            rotated: false,
            rotation_detail: None,
            retrieved_at: 1_762_665_720_000,
            duration_ms: 12,
            connection_state: ConnectionState::Connected,
        };
        let value = serde_json::to_value(tail).expect("serialized log tail");
        assert_eq!(value["sourceKind"], "laravel");
        assert_eq!(value["status"], "available");
        assert_eq!(
            value["requestedPath"],
            "/var/www/app/storage/logs/laravel.log"
        );
        assert_eq!(value["byteLimit"], MAX_LOG_CONTENT_BYTES);
    }
}
