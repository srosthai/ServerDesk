//! Safe, user-selected Laravel project discovery.
//!
//! Discovery never scans the server automatically. Every browse and scan call
//! receives one absolute remote path chosen by the user, and child scanning is
//! limited to the immediate subdirectories of that path. Paths are normalized
//! and shell-quoted before they are placed in a fixed command template.
//!
//! Detection reads only file and directory metadata plus a small allowlist of
//! bounded metadata files. Secrets from `.env` are never read: the queue query
//! extracts only the `QUEUE_CONNECTION` line and the value is never persisted
//! with credentials.

use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{
    core::{
        AppError,
        ssh::{CommandOutput, ConnectionManager, ConnectionState},
    },
    storage::{OperationRecord, Project},
};

/// Maximum accepted length of one normalized absolute remote path.
pub const MAX_PROJECT_PATH_CHARS: usize = 1024;
const _: () = assert!(MAX_PROJECT_PATH_CHARS <= 4096);
/// Hard output bound for browsing one remote directory.
pub const MAX_BROWSE_OUTPUT_BYTES: usize = 64 * 1024;
/// Hard output bound for the indicator probe of one candidate path.
pub const MAX_PROJECT_PROBE_OUTPUT_BYTES: usize = 8 * 1024;
/// Hard output bound for Composer's installed package metadata.
pub const MAX_PROJECT_METADATA_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_QUEUE_OUTPUT_BYTES: usize = 4 * 1024;
const MAX_CRON_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_PHP_OUTPUT_BYTES: usize = 8 * 1024;

/// A browse request only lists the immediate children of the selected path.
pub const PROJECT_BROWSE_TIMEOUT: Duration = Duration::from_secs(15);
pub const PROJECT_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
pub const PROJECT_METADATA_TIMEOUT: Duration = Duration::from_secs(20);

/// Maximum number of immediate child directories listed in one browse.
pub const MAX_DIRECTORY_ENTRIES: usize = 200;
/// Maximum number of candidate paths probed in one scan.
pub const MAX_SCAN_CANDIDATES: usize = 100;
/// Recent operations shown for each project on the dashboard.
pub const RECENT_PROJECT_OPERATIONS: u32 = 5;

const MAX_PROJECT_NAME_CHARS: usize = 255;
const MAX_VERSION_CHARS: usize = 64;
const MAX_HINT_CHARS: usize = 200;
const MAX_DETAIL_CHARS: usize = 300;

/// The four required Laravel project indicators.
pub const LARAVEL_INDICATORS: [&str; 4] = ["artisan", "composer.json", "bootstrap", "storage"];

/// The fixed PHP version command. No project file is executed.
pub const PHP_VERSION_COMMAND: &str = "php --version";

/// A validated, normalized absolute remote path.
///
/// Normalization collapses duplicate and trailing slashes, removes `.`
/// segments, and rejects `..` segments. The value is safe for display and can
/// be shell-quoted as one argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePath(String);

impl RemotePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The display name for a project root, or `root` for `/`.
    pub fn project_name(&self) -> String {
        if self.0 == "/" {
            return "root".to_string();
        }

        let name = self
            .0
            .rsplit_once('/')
            .map(|(_, name)| name)
            .unwrap_or(&self.0);
        let bounded = name
            .chars()
            .filter(|character| !character.is_control())
            .take(MAX_PROJECT_NAME_CHARS)
            .collect::<String>();

        if bounded.is_empty() {
            "root".to_string()
        } else {
            bounded
        }
    }
}

/// Normalizes and validates one user-selected absolute remote path.
pub fn normalize_remote_path(value: &str) -> Result<RemotePath, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(invalid_path_error("A remote directory path is required."));
    }
    if !trimmed.starts_with('/') {
        return Err(invalid_path_error("Remote project paths must be absolute."));
    }
    if trimmed.chars().count() > MAX_PROJECT_PATH_CHARS {
        return Err(invalid_path_error(
            "The remote path is longer than the supported limit.",
        ));
    }
    if trimmed
        .chars()
        .any(|character| character.is_control() || character == '\0')
    {
        return Err(invalid_path_error(
            "The remote path contains a control character.",
        ));
    }

    let mut segments = Vec::new();
    for segment in trimmed.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                return Err(invalid_path_error(
                    "The remote path must not contain parent directory segments.",
                ));
            }
            _ => segments.push(segment),
        }
    }

    let normalized = if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    };

    if normalized.chars().count() > MAX_PROJECT_PATH_CHARS {
        return Err(invalid_path_error(
            "The normalized remote path is longer than the supported limit.",
        ));
    }

    Ok(RemotePath(normalized))
}

fn invalid_path_error(message: &str) -> AppError {
    AppError::validation(message).with_details(
        "Use an absolute Linux path such as /var/www/storefront. Relative paths, \
         parent segments, and control characters are rejected before any remote command is sent.",
    )
}

/// Quotes one value as a single POSIX shell argument.
///
/// The path validator has already rejected control characters. This function
/// keeps spaces, quotes, and other printable characters usable without ever
/// letting them escape the argument.
pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The fixed command used to list immediate child directories.
pub fn browse_command(path: &RemotePath) -> String {
    format!(
        "find {} -mindepth 1 -maxdepth 1 \\( -type d -o -xtype d \\) -print0",
        shell_quote(path.as_str())
    )
}

/// The fixed command used to probe one candidate project path.
pub fn project_probe_command(path: &RemotePath) -> String {
    let path = shell_quote(path.as_str());
    format!(
        "if [ -d {path} ]; then echo __directory__; else echo __missing__; fi; \
         if [ -f {path}/artisan ]; then echo artisan; fi; \
         if [ -f {path}/composer.json ]; then echo composer.json; fi; \
         if [ -d {path}/bootstrap ]; then echo bootstrap; fi; \
         if [ -d {path}/storage ]; then echo storage; fi; \
         if [ -f {path}/routes/console.php ]; then echo routes/console.php; fi; \
         if [ -f {path}/app/Console/Kernel.php ]; then echo app/Console/Kernel.php; fi; \
         if [ -f {path}/config/queue.php ]; then echo config/queue.php; fi"
    )
}

/// The fixed command used to read Composer's installed package metadata.
pub fn laravel_package_command(path: &RemotePath) -> String {
    let path = shell_quote(path.as_str());
    format!(
        "if [ -f {path}/vendor/composer/installed.json ]; then cat {path}/vendor/composer/installed.json; fi"
    )
}

/// The fixed command used to read only the queue connection name from `.env`.
pub fn queue_connection_command(path: &RemotePath) -> String {
    let path = shell_quote(path.as_str());
    format!("if [ -f {path}/.env ]; then sed -n 's/^QUEUE_CONNECTION=//p' {path}/.env; fi")
}

/// The fixed command used to find a schedule:run cron entry for this path.
pub fn scheduler_cron_command(path: &RemotePath) -> String {
    let path = shell_quote(path.as_str());
    format!("crontab -l 2>/dev/null | grep -F -- 'schedule:run' | grep -F -- {path} || true")
}

/// One immediate child directory of a browsed path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDirectory {
    pub name: String,
    pub path: String,
}

/// The result of one explicit directory browse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDirectoryListing {
    pub server_id: String,
    pub path: String,
    pub directories: Vec<RemoteDirectory>,
    pub truncated: bool,
    pub sampled_at: i64,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

/// One Laravel project detected at an absolute remote path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedProject {
    pub name: String,
    pub remote_path: String,
    pub project_type: String,
    pub laravel_version: Option<String>,
    pub php_version: Option<String>,
    pub queue_hint: Option<String>,
    pub scheduler_hint: Option<String>,
}

/// The result of one explicit project scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScan {
    pub server_id: String,
    pub path: String,
    pub include_children: bool,
    pub scanned_paths: usize,
    pub rejected_paths: usize,
    pub truncated: bool,
    pub projects: Vec<DetectedProject>,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

/// A recent operation shown on the project dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOperationSummary {
    pub id: String,
    pub operation: String,
    pub safe_summary: String,
    pub status: ProjectOperationStatus,
    pub exit_code: Option<i32>,
    pub started_at: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOperationStatus {
    Succeeded,
    Failed,
    Unknown,
}

/// A persisted project and the dashboard data derived from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub remote_path: String,
    pub project_type: String,
    pub laravel_version: Option<String>,
    pub php_version: Option<String>,
    pub queue_hint: Option<String>,
    pub scheduler_hint: Option<String>,
    pub detected_at: i64,
    pub log_path: String,
    pub recent_operations: Vec<ProjectOperationSummary>,
}

/// The persisted result of one explicit project scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScanResult {
    pub server_id: String,
    pub path: String,
    pub include_children: bool,
    pub scanned_paths: usize,
    pub rejected_paths: usize,
    pub truncated: bool,
    pub projects: Vec<ProjectRecord>,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

impl ProjectScanResult {
    pub fn from_scan(scan: ProjectScan, projects: Vec<ProjectRecord>) -> Self {
        Self {
            server_id: scan.server_id,
            path: scan.path,
            include_children: scan.include_children,
            scanned_paths: scan.scanned_paths,
            rejected_paths: scan.rejected_paths,
            truncated: scan.truncated,
            projects,
            duration_ms: scan.duration_ms,
            connection_state: scan.connection_state,
        }
    }
}

/// Builds the typed dashboard record for one stored project.
pub fn project_record(project: Project, operations: Vec<OperationRecord>) -> ProjectRecord {
    let log_path = if project.remote_path == "/" {
        "/storage/logs/laravel.log".to_string()
    } else {
        format!("{}/storage/logs/laravel.log", project.remote_path)
    };

    ProjectRecord {
        id: project.id,
        server_id: project.server_id,
        name: project.name,
        remote_path: project.remote_path,
        project_type: project.project_type,
        laravel_version: project.laravel_version,
        php_version: project.php_version,
        queue_hint: project.queue_hint,
        scheduler_hint: project.scheduler_hint,
        detected_at: project.detected_at,
        log_path,
        recent_operations: operations
            .into_iter()
            .map(|operation| ProjectOperationSummary {
                status: match operation.exit_code {
                    Some(0) => ProjectOperationStatus::Succeeded,
                    Some(_) => ProjectOperationStatus::Failed,
                    None => ProjectOperationStatus::Unknown,
                },
                id: operation.id,
                operation: operation.operation,
                safe_summary: operation.safe_summary,
                exit_code: operation.exit_code,
                started_at: operation.started_at,
                duration_ms: operation.duration_ms,
            })
            .collect(),
    }
}

/// Lists only the immediate child directories of one explicitly selected path.
pub async fn list_remote_directories(
    manager: &ConnectionManager,
    server_id: &str,
    path: &str,
) -> Result<RemoteDirectoryListing, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }
    let path = normalize_remote_path(path)?;
    let started = Instant::now();
    let output = run_bounded(
        manager,
        server_id,
        &browse_command(&path),
        MAX_BROWSE_OUTPUT_BYTES,
        PROJECT_BROWSE_TIMEOUT,
        "Remote directory browsing",
    )
    .await?;
    let duration_ms = duration_millis(started);

    if output.exit_code != Some(0) {
        return Err(remote_command_error(
            "Could not list the selected remote directory.",
            &output,
        ));
    }

    let (directories, truncated) = parse_directory_listing(&output.stdout);
    let connection_state = manager.connection_state(server_id)?.state;

    Ok(RemoteDirectoryListing {
        server_id: server_id.to_string(),
        path: path.as_str().to_string(),
        directories,
        truncated,
        sampled_at: now_millis(),
        duration_ms,
        connection_state,
    })
}

/// Scans one explicitly selected path, optionally including its immediate
/// child directories. It never walks deeper and never scans another path.
pub async fn scan_remote_projects(
    manager: &ConnectionManager,
    server_id: &str,
    path: &str,
    include_children: bool,
) -> Result<ProjectScan, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let base = normalize_remote_path(path)?;
    let started = Instant::now();
    let mut candidates = vec![base.clone()];
    let mut truncated = false;

    if include_children {
        let listing = list_remote_directories(manager, server_id, base.as_str()).await?;
        truncated = listing.truncated;
        for directory in listing.directories {
            if candidates.len() >= MAX_SCAN_CANDIDATES {
                truncated = true;
                break;
            }
            if let Ok(candidate) = normalize_remote_path(&directory.path)
                && candidate != base
                && !candidates.contains(&candidate)
            {
                candidates.push(candidate);
            }
        }
    }

    candidates.truncate(MAX_SCAN_CANDIDATES);

    let mut projects = Vec::new();
    let mut scanned_paths = 0;
    let mut rejected_paths = 0;
    let mut php_version = None;
    let mut php_version_fetched = false;

    for candidate in candidates {
        scanned_paths += 1;
        let probe = probe_path(manager, server_id, &candidate).await?;

        if !probe.exists {
            if candidate == base {
                return Err(AppError::not_found(format!(
                    "The remote path {} does not exist or is not a directory.",
                    candidate.as_str()
                ))
                .with_details(
                    "Browse to the parent directory or enter another absolute path. No project was recorded.",
                ));
            }
            rejected_paths += 1;
            continue;
        }

        if !probe.is_laravel() {
            rejected_paths += 1;
            continue;
        }

        if !php_version_fetched {
            php_version = detect_php_version(manager, server_id).await;
            php_version_fetched = true;
        }

        let packages = read_packages(manager, server_id, &candidate).await;
        let queue = read_queue_connection(manager, server_id, &candidate).await;
        let cron_scheduler = detect_scheduler_cron(manager, server_id, &candidate).await;

        projects.push(DetectedProject {
            name: candidate.project_name(),
            remote_path: candidate.as_str().to_string(),
            project_type: "laravel".to_string(),
            laravel_version: packages.as_ref().and_then(|packages| {
                packages
                    .laravel_framework_version
                    .as_ref()
                    .map(|version| version.to_string())
            }),
            php_version: php_version.clone(),
            queue_hint: queue_hint(queue.as_deref(), packages.as_ref(), &probe),
            scheduler_hint: scheduler_hint(cron_scheduler, &probe),
        });
    }

    let duration_ms = duration_millis(started);
    let connection_state = manager.connection_state(server_id)?.state;

    Ok(ProjectScan {
        server_id: server_id.to_string(),
        path: base.as_str().to_string(),
        include_children,
        scanned_paths,
        rejected_paths,
        truncated,
        projects,
        duration_ms,
        connection_state,
    })
}

struct PathProbe {
    exists: bool,
    indicators: BTreeSet<String>,
}

impl PathProbe {
    fn is_laravel(&self) -> bool {
        LARAVEL_INDICATORS
            .iter()
            .all(|indicator| self.indicators.contains(*indicator))
    }

    fn has_scheduler_definitions(&self) -> bool {
        self.indicators.contains("routes/console.php")
            || self.indicators.contains("app/Console/Kernel.php")
    }

    fn has_queue_config(&self) -> bool {
        self.indicators.contains("config/queue.php")
    }
}

async fn probe_path(
    manager: &ConnectionManager,
    server_id: &str,
    path: &RemotePath,
) -> Result<PathProbe, AppError> {
    let output = run_bounded(
        manager,
        server_id,
        &project_probe_command(path),
        MAX_PROJECT_PROBE_OUTPUT_BYTES,
        PROJECT_PROBE_TIMEOUT,
        "Project detection",
    )
    .await?;

    if output.exit_code != Some(0) {
        return Err(remote_command_error(
            "Could not inspect the selected remote directory.",
            &output,
        ));
    }

    Ok(parse_project_probe(&output.stdout))
}

fn parse_project_probe(stdout: &str) -> PathProbe {
    let mut exists = false;
    let mut indicators = BTreeSet::new();

    for line in stdout.lines() {
        match line.trim() {
            "__directory__" => exists = true,
            "__missing__" => {}
            indicator if LARAVEL_INDICATORS.contains(&indicator) => {
                indicators.insert(indicator.to_string());
            }
            "routes/console.php" | "app/Console/Kernel.php" | "config/queue.php" => {
                indicators.insert(line.trim().to_string());
            }
            _ => {}
        }
    }

    PathProbe { exists, indicators }
}

fn parse_directory_listing(stdout: &str) -> (Vec<RemoteDirectory>, bool) {
    let mut directories = Vec::new();
    let mut seen = BTreeSet::new();
    let mut truncated = false;

    for raw in stdout.split('\0') {
        if raw.is_empty() {
            continue;
        }
        let Ok(path) = normalize_remote_path(raw) else {
            continue;
        };
        if !seen.insert(path.as_str().to_string()) {
            continue;
        }
        if directories.len() >= MAX_DIRECTORY_ENTRIES {
            truncated = true;
            break;
        }
        directories.push(RemoteDirectory {
            name: path.project_name(),
            path: path.as_str().to_string(),
        });
    }

    directories.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });

    (directories, truncated)
}

#[derive(Debug, Default)]
struct InstalledPackages {
    laravel_framework_version: Option<String>,
    horizon_installed: bool,
}

async fn read_packages(
    manager: &ConnectionManager,
    server_id: &str,
    path: &RemotePath,
) -> Option<InstalledPackages> {
    let stdout = optional_stdout(
        manager,
        server_id,
        &laravel_package_command(path),
        MAX_PROJECT_METADATA_OUTPUT_BYTES,
        PROJECT_METADATA_TIMEOUT,
    )
    .await?;
    parse_installed_packages(&stdout)
}

fn parse_installed_packages(stdout: &str) -> Option<InstalledPackages> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let packages = value
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array())?;

    let mut parsed = InstalledPackages::default();
    for package in packages {
        let Some(name) = package.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let version = package
            .get("version")
            .and_then(serde_json::Value::as_str)
            .and_then(sanitize_version);

        match name {
            "laravel/framework" => {
                if parsed.laravel_framework_version.is_none() {
                    parsed.laravel_framework_version = version;
                }
            }
            "laravel/horizon" => parsed.horizon_installed = true,
            _ => {}
        }
    }

    Some(parsed)
}

async fn detect_php_version(manager: &ConnectionManager, server_id: &str) -> Option<String> {
    let stdout = optional_stdout(
        manager,
        server_id,
        PHP_VERSION_COMMAND,
        MAX_PHP_OUTPUT_BYTES,
        PROJECT_METADATA_TIMEOUT,
    )
    .await?;
    parse_php_version(&stdout)
}

fn parse_php_version(stdout: &str) -> Option<String> {
    let line = stdout.lines().next()?.trim();
    let rest = line.strip_prefix("PHP ")?;
    let version = rest.split_whitespace().next()?;
    sanitize_version(version)
}

async fn read_queue_connection(
    manager: &ConnectionManager,
    server_id: &str,
    path: &RemotePath,
) -> Option<String> {
    let stdout = optional_stdout(
        manager,
        server_id,
        &queue_connection_command(path),
        MAX_QUEUE_OUTPUT_BYTES,
        PROJECT_METADATA_TIMEOUT,
    )
    .await?;
    parse_queue_connection(&stdout)
}

fn parse_queue_connection(stdout: &str) -> Option<String> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    if line.contains('=') && !line.starts_with("QUEUE_CONNECTION=") {
        return None;
    }
    let line = line.strip_prefix("QUEUE_CONNECTION=").unwrap_or(line);
    let value = line.trim_matches(|character| matches!(character, '"' | '\''));
    sanitize_hint(value)
}

async fn detect_scheduler_cron(
    manager: &ConnectionManager,
    server_id: &str,
    path: &RemotePath,
) -> bool {
    let Some(stdout) = optional_stdout(
        manager,
        server_id,
        &scheduler_cron_command(path),
        MAX_CRON_OUTPUT_BYTES,
        PROJECT_METADATA_TIMEOUT,
    )
    .await
    else {
        return false;
    };

    stdout.lines().any(|line| !line.trim().is_empty())
}

fn queue_hint(
    connection: Option<&str>,
    packages: Option<&InstalledPackages>,
    probe: &PathProbe,
) -> Option<String> {
    let horizon = packages.is_some_and(|packages| packages.horizon_installed);

    match (connection, horizon, probe.has_queue_config()) {
        (Some(connection), true, _) => Some(format!(
            "Queue connection: {connection}; Laravel Horizon is installed"
        )),
        (Some(connection), false, _) => Some(format!("Queue connection: {connection}")),
        (None, true, _) => {
            Some("Laravel Horizon is installed; queue connection was not detected".to_string())
        }
        (None, false, true) => {
            Some("Queue configuration found; queue connection was not detected".to_string())
        }
        (None, false, false) => None,
    }
}

fn scheduler_hint(cron_detected: bool, probe: &PathProbe) -> Option<String> {
    if cron_detected {
        Some("A cron entry runs schedule:run for this path".to_string())
    } else if probe.has_scheduler_definitions() {
        Some("Scheduler files found; no schedule:run cron entry references this path".to_string())
    } else {
        None
    }
}

fn sanitize_version(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_VERSION_CHARS
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return None;
    }

    Some(value.to_string())
}

fn sanitize_hint(value: &str) -> Option<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_HINT_CHARS)
        .collect::<String>();
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

async fn optional_stdout(
    manager: &ConnectionManager,
    server_id: &str,
    command: &str,
    max_output_bytes: usize,
    timeout: Duration,
) -> Option<String> {
    match run_bounded(
        manager,
        server_id,
        command,
        max_output_bytes,
        timeout,
        "Project metadata",
    )
    .await
    {
        Ok(output) if output.exit_code == Some(0) => Some(output.stdout),
        _ => None,
    }
}

async fn run_bounded(
    manager: &ConnectionManager,
    server_id: &str,
    command: &str,
    max_output_bytes: usize,
    timeout: Duration,
    label: &str,
) -> Result<CommandOutput, AppError> {
    tokio::time::timeout(
        timeout,
        manager.execute_bounded(server_id, command, max_output_bytes),
    )
    .await
    .map_err(|_| {
        AppError::timeout(format!(
            "{label} timed out after {} seconds.",
            timeout.as_secs()
        ))
    })?
}

fn remote_command_error(message: &str, output: &CommandOutput) -> AppError {
    let exit = match output.exit_code {
        Some(code) => format!("exit code {code}"),
        None => "no exit status".to_string(),
    };
    let stderr = output
        .stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| {
            line.chars()
                .filter(|character| !character.is_control() || *character == '\t')
                .take(MAX_DETAIL_CHARS)
                .collect::<String>()
        })
        .unwrap_or_default();

    if stderr.is_empty() {
        AppError::remote(format!("{message} ({exit})"))
    } else {
        AppError::remote(format!("{message} ({exit}: {stderr})"))
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn duration_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn normalizes_absolute_paths_and_rejects_unsafe_forms() {
        assert_eq!(
            normalize_remote_path(" /var/www//storefront/ ")
                .expect("normalized")
                .as_str(),
            "/var/www/storefront"
        );
        assert_eq!(
            normalize_remote_path("/var/www/./storefront")
                .expect("normalized")
                .as_str(),
            "/var/www/storefront"
        );
        assert_eq!(normalize_remote_path("/").expect("root").as_str(), "/");
        assert_eq!(normalize_remote_path("//").expect("root").as_str(), "/");

        for invalid in [
            "",
            "   ",
            "var/www/storefront",
            "../storefront",
            "/var/www/../etc",
            "/var/www/\nstorefront",
            "/var/www/\0storefront",
        ] {
            assert!(
                normalize_remote_path(invalid).is_err(),
                "{invalid:?} must be rejected"
            );
        }

        let too_long = format!("/{}", "a".repeat(MAX_PROJECT_PATH_CHARS));
        assert!(normalize_remote_path(&too_long).is_err());
    }

    #[test]
    fn project_names_come_from_the_normalized_path() {
        assert_eq!(
            normalize_remote_path("/var/www/storefront/")
                .expect("path")
                .project_name(),
            "storefront"
        );
        assert_eq!(
            normalize_remote_path("/").expect("root").project_name(),
            "root"
        );
    }

    #[test]
    fn shell_quoting_keeps_spaces_and_quotes_inside_one_argument() {
        assert_eq!(shell_quote("/var/www/app"), "'/var/www/app'");
        assert_eq!(
            shell_quote("/var/www/My App's Files"),
            "'/var/www/My App'\\''s Files'"
        );

        let directory = tempfile::tempdir().expect("temporary directory");
        let project = directory
            .path()
            .join("project with 'quotes'; touch injected");
        fs::create_dir(&project).expect("create fixture directory");
        let path =
            normalize_remote_path(&project.to_string_lossy()).expect("normalized fixture path");

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("test -d {}", shell_quote(path.as_str())))
            .output()
            .expect("run local shell");
        assert!(output.status.success());

        let marker = directory.path().join("injected");
        let malicious = directory.path().join("project'; touch injected; echo '");
        fs::create_dir(&malicious).expect("create malicious fixture directory");
        let malicious_path =
            normalize_remote_path(&malicious.to_string_lossy()).expect("normalized malicious path");
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(project_probe_command(&malicious_path))
            .current_dir(directory.path())
            .output()
            .expect("run quoted probe");
        assert!(output.status.success());
        assert!(
            !marker.exists(),
            "the quoted path must never execute an injected command"
        );
    }

    #[test]
    fn probe_requires_all_four_laravel_indicators() {
        let complete = parse_project_probe(
            "__directory__\nartisan\ncomposer.json\nbootstrap\nstorage\n\
             routes/console.php\nconfig/queue.php\n",
        );
        assert!(complete.exists);
        assert!(complete.is_laravel());
        assert!(complete.has_scheduler_definitions());
        assert!(complete.has_queue_config());

        for missing in LARAVEL_INDICATORS {
            let output = [
                "__directory__",
                "artisan",
                "composer.json",
                "bootstrap",
                "storage",
            ]
            .iter()
            .filter(|indicator| **indicator != missing)
            .map(|indicator| format!("{indicator}\n"))
            .collect::<String>();
            let probe = parse_project_probe(&output);
            assert!(
                probe.exists && !probe.is_laravel(),
                "missing {missing:?} must reject the directory"
            );
        }

        let non_laravel = parse_project_probe("__directory__\ncomposer.json\n");
        assert!(non_laravel.exists);
        assert!(!non_laravel.is_laravel());

        let missing = parse_project_probe("__missing__\n");
        assert!(!missing.exists);
    }

    #[test]
    fn probe_command_is_fixed_and_quotes_the_selected_path() {
        let path = normalize_remote_path("/var/www/My App").expect("path");
        let command = project_probe_command(&path);
        assert!(command.contains("'/var/www/My App'/artisan"));
        assert!(command.contains("echo artisan"));
        assert!(command.contains("echo storage"));
        assert!(!command.contains("$("));
        assert!(!command.contains("`"));
        assert!(!command.contains("/var/www/My App/artisan;"));
    }

    #[test]
    fn installed_packages_provide_laravel_version_and_horizon() {
        let packages = parse_installed_packages(
            r#"{
                "packages": [
                    {"name": "laravel/framework", "version": "v11.30.0"},
                    {"name": "laravel/horizon", "version": "v5.32.0"},
                    {"name": "symfony/console", "version": "v7.2.0"}
                ]
            }"#,
        )
        .expect("parsed packages");
        assert_eq!(
            packages.laravel_framework_version.as_deref(),
            Some("v11.30.0")
        );
        assert!(packages.horizon_installed);

        let old_format =
            parse_installed_packages(r#"[{"name": "laravel/framework", "version": "10.48.0"}]"#)
                .expect("legacy installed.json array");
        assert_eq!(
            old_format.laravel_framework_version.as_deref(),
            Some("10.48.0")
        );
        assert!(!old_format.horizon_installed);

        assert!(parse_installed_packages("not json").is_none());
        assert!(parse_installed_packages("{}").is_none());
    }

    #[test]
    fn parses_php_version_from_the_fixed_command_output() {
        assert_eq!(
            parse_php_version("PHP 8.3.14 (cli) (built: Nov 21 2024 00:00:00) (NTS)\n").as_deref(),
            Some("8.3.14")
        );
        assert_eq!(
            parse_php_version("PHP 8.2.0RC2 (cli)\n").as_deref(),
            Some("8.2.0RC2")
        );
        assert_eq!(parse_php_version("not php\n"), None);
        assert_eq!(parse_php_version(""), None);
        assert_eq!(parse_php_version("PHP "), None);
    }

    #[test]
    fn parses_only_the_queue_connection_value() {
        assert_eq!(
            parse_queue_connection("QUEUE_CONNECTION=redis\n").as_deref(),
            Some("redis")
        );
        assert_eq!(
            parse_queue_connection("\nQUEUE_CONNECTION=\"database\"\n").as_deref(),
            Some("database")
        );
        assert_eq!(
            parse_queue_connection("QUEUE_CONNECTION='sqs'\n").as_deref(),
            Some("sqs")
        );
        assert_eq!(parse_queue_connection(""), None);
        assert_eq!(parse_queue_connection("MAIL_PASSWORD=secret\n"), None);
    }

    #[test]
    fn dashboard_record_contains_links_hints_and_recent_operation_status() {
        let project = Project {
            id: "project-1".to_string(),
            server_id: "server-1".to_string(),
            name: "storefront".to_string(),
            remote_path: "/var/www/storefront".to_string(),
            project_type: "laravel".to_string(),
            laravel_version: Some("v11.30.0".to_string()),
            php_version: Some("8.3.14".to_string()),
            queue_hint: Some("Queue connection: redis".to_string()),
            scheduler_hint: Some("A cron entry runs schedule:run for this path".to_string()),
            detected_at: 1_762_665_720,
        };
        let operations = vec![
            OperationRecord {
                id: "operation-1".to_string(),
                server_id: Some("server-1".to_string()),
                project_id: Some("project-1".to_string()),
                category: crate::storage::OperationCategory::Artisan,
                operation: "artisan-about".to_string(),
                safe_summary: "Read Artisan about".to_string(),
                path: Some("/var/www/storefront".to_string()),
                output_summary: Some("Laravel Framework 11.30.0".to_string()),
                result: crate::storage::OperationResult::Success,
                exit_code: Some(0),
                started_at: 1_762_665_720,
                duration_ms: 42,
            },
            OperationRecord {
                id: "operation-2".to_string(),
                server_id: Some("server-1".to_string()),
                project_id: Some("project-1".to_string()),
                category: crate::storage::OperationCategory::Queue,
                operation: "queue-restart".to_string(),
                safe_summary: "Restarted queue workers".to_string(),
                path: Some("/var/www/storefront".to_string()),
                output_summary: Some("Broadcasting queue restart signal".to_string()),
                result: crate::storage::OperationResult::Failure,
                exit_code: Some(1),
                started_at: 1_762_665_700,
                duration_ms: 17,
            },
        ];

        let record = project_record(project, operations);
        assert_eq!(
            record.log_path,
            "/var/www/storefront/storage/logs/laravel.log"
        );
        assert_eq!(record.recent_operations.len(), 2);
        assert_eq!(
            record.recent_operations[0].status,
            ProjectOperationStatus::Succeeded
        );
        assert_eq!(
            record.recent_operations[1].status,
            ProjectOperationStatus::Failed
        );
    }

    #[test]
    fn directory_listing_parses_nul_separated_absolute_paths() {
        let (directories, truncated) = parse_directory_listing(
            "/var/www/storefront\0/var/www/blog\0/var/www//api/\0relative/path\0",
        );
        assert!(!truncated);
        assert_eq!(directories.len(), 3);
        assert_eq!(directories[0].name, "api");
        assert_eq!(directories[0].path, "/var/www/api");
        assert_eq!(directories[1].name, "blog");
        assert_eq!(directories[2].name, "storefront");

        let mut output = String::new();
        for index in 0..MAX_DIRECTORY_ENTRIES + 2 {
            output.push_str(&format!("/var/www/dir{index}\0"));
        }
        let (directories, truncated) = parse_directory_listing(&output);
        assert_eq!(directories.len(), MAX_DIRECTORY_ENTRIES);
        assert!(truncated);
    }

    #[test]
    fn scheduler_and_queue_hints_never_invent_a_setup() {
        let probe = parse_project_probe(
            "__directory__\nartisan\ncomposer.json\nbootstrap\nstorage\nconfig/queue.php\n",
        );
        assert_eq!(
            queue_hint(None, None, &probe).as_deref(),
            Some("Queue configuration found; queue connection was not detected")
        );
        assert_eq!(
            scheduler_hint(false, &probe),
            None,
            "a queue config file is not scheduler evidence"
        );

        let scheduler_probe = parse_project_probe(
            "__directory__\nartisan\ncomposer.json\nbootstrap\nstorage\nroutes/console.php\n",
        );
        assert!(
            scheduler_hint(false, &scheduler_probe)
                .expect("scheduler hint")
                .contains("no schedule:run cron entry")
        );
        assert!(
            scheduler_hint(true, &scheduler_probe)
                .expect("cron hint")
                .contains("runs schedule:run")
        );

        let packages = InstalledPackages {
            laravel_framework_version: Some("v11.30.0".to_string()),
            horizon_installed: true,
        };
        assert!(
            queue_hint(Some("redis"), Some(&packages), &probe)
                .expect("queue hint")
                .contains("Horizon")
        );
    }

    #[test]
    fn serializes_scan_and_dashboard_records_for_the_frontend() {
        let scan = ProjectScan {
            server_id: "server-1".to_string(),
            path: "/var/www".to_string(),
            include_children: true,
            scanned_paths: 3,
            rejected_paths: 2,
            truncated: false,
            projects: vec![DetectedProject {
                name: "storefront".to_string(),
                remote_path: "/var/www/storefront".to_string(),
                project_type: "laravel".to_string(),
                laravel_version: Some("v11.30.0".to_string()),
                php_version: Some("8.3.14".to_string()),
                queue_hint: None,
                scheduler_hint: None,
            }],
            duration_ms: 25,
            connection_state: ConnectionState::Connected,
        };
        let value = serde_json::to_value(scan).expect("serialized scan");
        assert_eq!(value["serverId"], "server-1");
        assert_eq!(value["includeChildren"], true);
        assert_eq!(value["rejectedPaths"], 2);
        assert_eq!(value["projects"][0]["remotePath"], "/var/www/storefront");
        assert_eq!(value["projects"][0]["laravelVersion"], "v11.30.0");

        let record = project_record(
            Project {
                id: "project-1".to_string(),
                server_id: "server-1".to_string(),
                name: "storefront".to_string(),
                remote_path: "/var/www/storefront".to_string(),
                project_type: "laravel".to_string(),
                laravel_version: None,
                php_version: None,
                queue_hint: None,
                scheduler_hint: None,
                detected_at: 1_762_665_720,
            },
            Vec::new(),
        );
        let value = serde_json::to_value(record).expect("serialized project");
        assert_eq!(value["remotePath"], "/var/www/storefront");
        assert_eq!(
            value["logPath"],
            "/var/www/storefront/storage/logs/laravel.log"
        );
        assert_eq!(value["recentOperations"], serde_json::json!([]));
    }

    #[test]
    fn command_templates_remain_fixed_and_bounded() {
        assert_eq!(PHP_VERSION_COMMAND, "php --version");
        assert!(PROJECT_BROWSE_TIMEOUT <= Duration::from_secs(15));
        assert!(PROJECT_METADATA_TIMEOUT <= Duration::from_secs(20));

        let path = normalize_remote_path("/var/www/storefront").expect("path");
        for command in [
            browse_command(&path),
            project_probe_command(&path),
            laravel_package_command(&path),
            queue_connection_command(&path),
            scheduler_cron_command(&path),
        ] {
            assert!(command.contains("/var/www/storefront"));
            assert!(!command.contains("rm -rf"));
        }
    }
}
