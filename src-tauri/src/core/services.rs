//! Systemd service discovery.
//!
//! Discovery runs one fixed, read-only `systemctl` command over an already
//! authenticated SSH session. Output is bounded before it is parsed, units are
//! parsed generically from the command output, and systems without systemd
//! return a typed unsupported result instead of an empty list presented as a
//! successful discovery.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::core::{
    AppError, ErrorCode,
    command::{service_action_command, service_status_command},
    redaction::redact_text,
    ssh::{CommandOutput, ConnectionManager, ConnectionState},
};

pub use crate::core::command::{
    SERVICE_STATUS_PROPERTIES, SERVICE_STATUS_PROPERTY_FLAGS, SUDO_BINARY, SYSTEMCTL_BINARY,
};

/// Hard output bound applied to the service discovery command.
pub const MAX_SERVICE_OUTPUT_BYTES: usize = 64 * 1024;
const _: () = assert!(MAX_SERVICE_OUTPUT_BYTES <= 64 * 1024);

/// Hard output bounds applied to service status and action commands.
pub const MAX_SERVICE_STATUS_OUTPUT_BYTES: usize = 16 * 1024;
pub const MAX_SERVICE_ACTION_OUTPUT_BYTES: usize = 64 * 1024;
const _: () = assert!(MAX_SERVICE_STATUS_OUTPUT_BYTES <= 64 * 1024);
const _: () = assert!(MAX_SERVICE_ACTION_OUTPUT_BYTES <= 64 * 1024);

/// Hard command-duration bound applied on top of the connection timeout.
pub const SERVICE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
pub const SERVICE_STATUS_TIMEOUT: Duration = Duration::from_secs(10);
pub const SERVICE_ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// The single fixed command sent during service discovery.
///
/// `--full` keeps long unit names from being shortened and `--no-legend`
/// removes the header so every line is either a unit row or empty.
pub const SYSTEMD_SERVICES_COMMAND: &str =
    "systemctl list-units --type=service --all --no-pager --no-legend --full";

const MAX_PROCESSING_LINES: usize = 4096;
const MAX_SERVICES: usize = 512;
const MAX_UNIT_NAME_CHARS: usize = 255;
const MAX_STATE_CHARS: usize = 32;
const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_DETAIL_CHARS: usize = 180;

/// The normalized state shown for one discovered unit.
///
/// systemd exposes more active states than the MVP needs to style separately.
/// Anything outside active, inactive, and failed remains `Unknown` together
/// with its raw `active_state`, so it is never confused with failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Active,
    Inactive,
    Failed,
    Unknown,
}

/// Whether service discovery produced a usable unit list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceDiscoveryStatus {
    /// The remote systemd command succeeded and its units were parsed.
    Available,
    /// The remote system does not run systemd or has no usable systemd bus.
    Unsupported,
    /// systemd is present but the discovery command failed for another reason.
    Unavailable,
}

/// One service unit discovered from the remote system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceUnit {
    pub name: String,
    pub state: ServiceState,
    /// The raw systemd active state, for example `active` or `activating`.
    pub active_state: String,
    pub sub_state: Option<String>,
    pub load_state: Option<String>,
    pub description: Option<String>,
}

/// The complete result of one service-discovery attempt.
///
/// `services` is empty whenever `status` is not `available`. A successful but
/// empty list means systemd reported no service units; it does not cause
/// placeholder services to be invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDiscovery {
    pub server_id: String,
    pub status: ServiceDiscoveryStatus,
    pub services: Vec<ServiceUnit>,
    pub detail: Option<String>,
    pub truncated: bool,
    pub sampled_at: i64,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedServices {
    services: Vec<ServiceUnit>,
    truncated: bool,
}

/// Discovers systemd service units over an existing authenticated SSH session.
///
/// The command has a fixed template, a 64 KiB output bound, and a 10 second
/// duration bound. A failed command becomes a typed `unsupported`,
/// `unavailable`, or transport error result instead of a fabricated empty
/// successful discovery.
pub async fn discover_systemd_services(
    manager: &ConnectionManager,
    server_id: &str,
) -> Result<ServiceDiscovery, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let started = Instant::now();
    let output = tokio::time::timeout(
        SERVICE_DISCOVERY_TIMEOUT,
        manager.execute_bounded(
            server_id,
            SYSTEMD_SERVICES_COMMAND,
            MAX_SERVICE_OUTPUT_BYTES,
        ),
    )
    .await
    .map_err(|_| {
        AppError::timeout(format!(
            "Systemd service discovery timed out after {} seconds.",
            SERVICE_DISCOVERY_TIMEOUT.as_secs()
        ))
    })??;

    let sampled_at = now_millis();
    let connection_state = manager.connection_state(server_id)?.state;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    if output.exit_code != Some(0) {
        let status = classify_failure(&output);
        return Ok(ServiceDiscovery {
            server_id: server_id.to_string(),
            status,
            services: Vec::new(),
            detail: Some(failure_detail(&output, status)),
            truncated: false,
            sampled_at,
            duration_ms,
            connection_state,
        });
    }

    let parsed = parse_services(&output.stdout);

    Ok(ServiceDiscovery {
        server_id: server_id.to_string(),
        status: ServiceDiscoveryStatus::Available,
        services: parsed.services,
        detail: None,
        truncated: parsed.truncated,
        sampled_at,
        duration_ms,
        connection_state,
    })
}

fn classify_failure(output: &CommandOutput) -> ServiceDiscoveryStatus {
    if output.exit_code == Some(127) {
        return ServiceDiscoveryStatus::Unsupported;
    }

    let stderr = output.stderr.to_ascii_lowercase();
    if [
        "system has not been booted with systemd",
        "systemd is not running",
        "not running systemd",
        "failed to connect to bus",
        "failed to get d-bus connection",
        "systemctl: not found",
        "command not found",
    ]
    .iter()
    .any(|needle| stderr.contains(needle))
    {
        ServiceDiscoveryStatus::Unsupported
    } else {
        ServiceDiscoveryStatus::Unavailable
    }
}

fn failure_detail(output: &CommandOutput, status: ServiceDiscoveryStatus) -> String {
    let exit = match output.exit_code {
        Some(code) => format!("exit code {code}"),
        None => "no exit status".to_string(),
    };
    let stderr = output
        .stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| bounded_detail(redact_text(line.trim()).text.trim()))
        .unwrap_or_default();

    match status {
        ServiceDiscoveryStatus::Unsupported => {
            if stderr.is_empty() {
                format!("Systemd is not available on this server ({exit}).")
            } else {
                format!("Systemd is not available on this server ({exit}: {stderr}).")
            }
        }
        ServiceDiscoveryStatus::Available | ServiceDiscoveryStatus::Unavailable => {
            if stderr.is_empty() {
                format!("Systemd service discovery failed with {exit}.")
            } else {
                format!("Systemd service discovery failed with {exit}: {stderr}")
            }
        }
    }
}

fn parse_services(stdout: &str) -> ParsedServices {
    let mut parsed = ParsedServices::default();

    for (index, line) in stdout.lines().enumerate() {
        if index >= MAX_PROCESSING_LINES {
            parsed.truncated = true;
            break;
        }

        let Some(service) = parse_service_line(line) else {
            continue;
        };

        if parsed.services.len() >= MAX_SERVICES {
            parsed.truncated = true;
            break;
        }

        parsed.services.push(service);
    }

    parsed
}

fn parse_service_line(line: &str) -> Option<ServiceUnit> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Some systemd versions prefix failed units with a marker glyph.
    let line = line.trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '●' | '○' | '*' | '×')
    });
    if line.is_empty() || line.starts_with("0 loaded units listed") {
        return None;
    }

    let mut fields = line.split_whitespace();
    let name = sanitize_unit_name(fields.next()?)?;
    let load_state = fields.next().and_then(sanitize_token);
    let active_state = fields
        .next()
        .and_then(sanitize_token)
        .unwrap_or_else(|| "unknown".to_string());
    let sub_state = fields.next().and_then(sanitize_token);
    let description = bounded_description(&fields.collect::<Vec<_>>().join(" "));
    let state = classify_state(&active_state);

    Some(ServiceUnit {
        name,
        state,
        active_state,
        sub_state,
        load_state,
        description,
    })
}

fn classify_state(active_state: &str) -> ServiceState {
    match active_state {
        "active" => ServiceState::Active,
        "inactive" => ServiceState::Inactive,
        "failed" => ServiceState::Failed,
        _ => ServiceState::Unknown,
    }
}

fn sanitize_unit_name(value: &str) -> Option<String> {
    if value.is_empty()
        || !value.ends_with(".service")
        || value.chars().count() > MAX_UNIT_NAME_CHARS
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return None;
    }

    Some(value.to_string())
}

fn sanitize_token(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_STATE_CHARS
        || value.chars().any(char::is_control)
    {
        return None;
    }

    Some(value.to_string())
}

fn bounded_description(value: &str) -> Option<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DESCRIPTION_CHARS)
        .collect::<String>();
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn bounded_detail(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DETAIL_CHARS)
        .collect()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// A validated systemd service unit name.
///
/// The type can only be built through [`validate_service_unit_name`], so every
/// command template receives a value from a strict ASCII allowlist instead of
/// an arbitrary string from the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceUnitName(String);

impl ServiceUnitName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Rejects unit names that could change the command being executed.
///
/// systemd unit names are restricted to a small ASCII allowlist for this MVP.
/// Shell operators, whitespace, slashes, control characters, option-like names,
/// and names that do not end in `.service` never reach an SSH command.
pub fn validate_service_unit_name(value: &str) -> Result<ServiceUnitName, AppError> {
    if value.is_empty() || value.chars().count() > MAX_UNIT_NAME_CHARS {
        return Err(invalid_unit_name_error());
    }

    let Some(stem) = value.strip_suffix(".service") else {
        return Err(invalid_unit_name_error());
    };

    if stem.is_empty()
        || stem.starts_with('.')
        || stem.starts_with('-')
        || stem.ends_with('.')
        || stem.contains("..")
        || stem.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '.' | '@' | '-'))
        })
    {
        return Err(invalid_unit_name_error());
    }

    Ok(ServiceUnitName(value.to_string()))
}

fn invalid_unit_name_error() -> AppError {
    AppError::validation("The service unit name is not valid.").with_details(
        "Unit names must end in .service and contain only ASCII letters, digits, ':', '_', \
         '.', '@', or '-'. Whitespace, shell operators, slashes, control characters, and \
         option-like names are rejected before any remote command is sent.",
    )
}

/// The fixed service operations exposed by the typed action path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Status,
    Start,
    Stop,
    Restart,
}

impl ServiceAction {
    pub const ALL: [Self; 4] = [Self::Status, Self::Start, Self::Stop, Self::Restart];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }

    pub const fn verb(self) -> &'static str {
        match self {
            Self::Status => "inspect",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }

    /// The state a successful command should produce, if there is one.
    ///
    /// Status inspection has no expected state. The verified state is always
    /// reported regardless of this expectation.
    pub const fn expected_state(self) -> Option<ServiceState> {
        match self {
            Self::Status => None,
            Self::Start | Self::Restart => Some(ServiceState::Active),
            Self::Stop => Some(ServiceState::Inactive),
        }
    }

    /// Whether the action can interrupt workloads and therefore requires
    /// explicit confirmation naming the server, unit, and operation.
    pub const fn requires_confirmation(self) -> bool {
        matches!(self, Self::Stop | Self::Restart)
    }

    /// Builds the only command line used for this action and unit.
    ///
    /// The unit has already passed [`validate_service_unit_name`]. `--` is
    /// passed to both sudo and systemctl so neither can interpret the unit name
    /// as an option.
    pub fn command(self, unit: &ServiceUnitName) -> String {
        match self {
            Self::Status => service_status_command(unit.as_str()),
            Self::Start | Self::Stop | Self::Restart => {
                service_action_command(self.as_str(), unit.as_str())
            }
        }
    }
}

/// A parsed, verified state for one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub server_id: String,
    pub unit: String,
    pub state: ServiceState,
    pub active_state: String,
    pub sub_state: Option<String>,
    pub load_state: Option<String>,
    pub unit_file_state: Option<String>,
    pub description: Option<String>,
    pub sampled_at: i64,
    pub duration_ms: u64,
}

/// The complete result of one fixed service action.
///
/// A non-zero action exit status is a structured, typed result rather than a
/// transport error, so stdout, stderr, and the exit code stay visible. The
/// state is re-queried after every action; `verified_status` is the only state
/// the UI may present as verified.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceActionResult {
    pub server_id: String,
    pub unit: String,
    pub action: ServiceAction,
    pub command: String,
    pub succeeded: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
    pub duration_ms: u64,
    pub permission_denied: bool,
    pub sudo_guidance: Option<String>,
    pub verified_status: Option<ServiceStatus>,
    /// `None` when the action was status or the state could not be verified.
    pub state_verified: Option<bool>,
    pub verification_error: Option<String>,
    pub error: Option<AppError>,
    pub connection_state: ConnectionState,
}

struct StatusQuery {
    output: CommandOutput,
    duration_ms: u64,
    status: Option<ServiceStatus>,
    parse_error: Option<String>,
    permission_denied: bool,
}

/// Inspects one unit through the fixed status command and waits for the result.
pub async fn inspect_service_status(
    manager: &ConnectionManager,
    server_id: &str,
    unit_name: &str,
) -> Result<ServiceStatus, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let unit = validate_service_unit_name(unit_name)?;
    let query = query_service_status(manager, server_id, &unit).await?;

    match query.status {
        Some(status) => Ok(status),
        None => Err(status_error(&unit, &query)),
    }
}

/// Executes one fixed service action and re-queries the unit afterwards.
///
/// Command-level failures (including sudo denial and systemctl errors) are
/// returned as [`ServiceActionResult`] values so the caller can show the exact
/// output and exit status. Transport, timeout, and validation failures are
/// returned as [`AppError`]. No action ever reports success from the button
/// click alone.
pub async fn run_service_action(
    manager: &ConnectionManager,
    server_id: &str,
    unit_name: &str,
    action: ServiceAction,
    confirmed: bool,
) -> Result<ServiceActionResult, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let unit = validate_service_unit_name(unit_name)?;
    if action.requires_confirmation() && !confirmed {
        return Err(AppError::validation(format!(
            "{} {} on server {} requires explicit confirmation before it can run.",
            action.as_str(),
            unit.as_str(),
            server_id
        ))
        .with_details(
            "Review the server, target unit, and operation, then confirm the action. No remote command was sent.",
        ));
    }

    if action == ServiceAction::Status {
        let query = query_service_status(manager, server_id, &unit).await?;
        let connection_state = manager.connection_state(server_id)?.state;
        let command = action.command(&unit);
        let succeeded = query.output.exit_code == Some(0) && query.status.is_some();
        let (error, permission_denied) = if succeeded {
            (None, false)
        } else {
            let error = command_error(&action, &unit, &command, &query.output, query.duration_ms);
            let permission_denied = error.code == ErrorCode::Permission;
            (Some(error), permission_denied)
        };
        let sudo_guidance = permission_denied.then(|| sudo_guidance(&unit));

        return Ok(ServiceActionResult {
            server_id: server_id.to_string(),
            unit: unit.as_str().to_string(),
            action,
            command,
            succeeded,
            stdout: sanitize_stream(&query.output.stdout),
            stderr: sanitize_stream(&query.output.stderr),
            exit_code: query.output.exit_code,
            duration_ms: query.duration_ms,
            permission_denied,
            sudo_guidance,
            verified_status: query.status,
            state_verified: None,
            verification_error: query.parse_error,
            error,
            connection_state,
        });
    }

    let command = action.command(&unit);
    let started = Instant::now();
    let output = tokio::time::timeout(
        SERVICE_ACTION_TIMEOUT,
        manager.execute_bounded(server_id, &command, MAX_SERVICE_ACTION_OUTPUT_BYTES),
    )
    .await
    .map_err(|_| {
        AppError::timeout(format!(
            "The {} action timed out after {} seconds.",
            action.verb(),
            SERVICE_ACTION_TIMEOUT.as_secs()
        ))
        .with_details(
            "The remote command may still be running or may already have completed. Verify the \
             unit state before retrying a start, stop, or restart.",
        )
    })??;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let (verified_status, verification_error) =
        match query_service_status(manager, server_id, &unit).await {
            Ok(query) => (query.status, query.parse_error),
            Err(error) => (None, Some(with_details(&error))),
        };

    let connection_state = manager.connection_state(server_id)?.state;
    let action_succeeded = output.exit_code == Some(0);
    let (error, permission_denied) = if action_succeeded {
        (None, false)
    } else {
        let error = command_error(&action, &unit, &command, &output, duration_ms);
        let permission_denied = error.code == ErrorCode::Permission;
        (Some(error), permission_denied)
    };
    let sudo_guidance = permission_denied.then(|| sudo_guidance(&unit));
    let state_verified = match (&verified_status, action.expected_state()) {
        (Some(status), Some(expected)) => Some(status.state == expected),
        _ => None,
    };

    Ok(ServiceActionResult {
        server_id: server_id.to_string(),
        unit: unit.as_str().to_string(),
        action,
        command,
        succeeded: action_succeeded,
        stdout: sanitize_stream(&output.stdout),
        stderr: sanitize_stream(&output.stderr),
        exit_code: output.exit_code,
        duration_ms,
        permission_denied,
        sudo_guidance,
        verified_status,
        state_verified,
        verification_error,
        error,
        connection_state,
    })
}

async fn query_service_status(
    manager: &ConnectionManager,
    server_id: &str,
    unit: &ServiceUnitName,
) -> Result<StatusQuery, AppError> {
    let command = ServiceAction::Status.command(unit);
    let started = Instant::now();
    let output = tokio::time::timeout(
        SERVICE_STATUS_TIMEOUT,
        manager.execute_bounded(server_id, &command, MAX_SERVICE_STATUS_OUTPUT_BYTES),
    )
    .await
    .map_err(|_| {
        AppError::timeout(format!(
            "The service status query timed out after {} seconds.",
            SERVICE_STATUS_TIMEOUT.as_secs()
        ))
    })??;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    if output.exit_code != Some(0) {
        let permission_denied = is_permission_denied(&output);
        return Ok(StatusQuery {
            parse_error: Some(describe_failure(&output)),
            output,
            duration_ms,
            status: None,
            permission_denied,
        });
    }

    match parse_service_status(server_id, unit, &output.stdout, duration_ms) {
        Ok(status) => Ok(StatusQuery {
            output,
            duration_ms,
            status: Some(status),
            parse_error: None,
            permission_denied: false,
        }),
        Err(message) => Ok(StatusQuery {
            output,
            duration_ms,
            status: None,
            parse_error: Some(message),
            permission_denied: false,
        }),
    }
}

fn parse_service_status(
    server_id: &str,
    unit: &ServiceUnitName,
    stdout: &str,
    duration_ms: u64,
) -> Result<ServiceStatus, String> {
    let mut active_state = None;
    let mut sub_state = None;
    let mut load_state = None;
    let mut unit_file_state = None;
    let mut description = None;

    for line in stdout.lines().take(MAX_PROCESSING_LINES) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if value.chars().any(char::is_control) {
            continue;
        }

        match key.trim() {
            "ActiveState" => active_state = sanitize_status_value(value),
            "SubState" => sub_state = sanitize_status_value(value),
            "LoadState" => load_state = sanitize_status_value(value),
            "UnitFileState" => unit_file_state = sanitize_status_value(value),
            "Description" => description = bounded_description(value),
            _ => {}
        }
    }

    let Some(active_state) = active_state else {
        return Err(format!(
            "Systemd did not report ActiveState for {}.",
            unit.as_str()
        ));
    };
    let state = classify_state(&active_state);

    Ok(ServiceStatus {
        server_id: server_id.to_string(),
        unit: unit.as_str().to_string(),
        state,
        active_state,
        sub_state,
        load_state,
        unit_file_state,
        description,
        sampled_at: now_millis(),
        duration_ms,
    })
}

fn sanitize_status_value(value: &str) -> Option<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_STATE_CHARS)
        .collect::<String>();
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn status_error(unit: &ServiceUnitName, query: &StatusQuery) -> AppError {
    let command = ServiceAction::Status.command(unit);
    if query.permission_denied {
        permission_denied_error(
            ServiceAction::Status,
            unit,
            &command,
            &query.output,
            query.duration_ms,
        )
    } else {
        AppError::remote(format!(
            "Could not inspect {} {}.",
            unit.as_str(),
            exit_suffix(&query.output)
        ))
        .with_details(command_diagnostics(
            &command,
            &query.output,
            query.duration_ms,
        ))
    }
}

fn command_error(
    action: &ServiceAction,
    unit: &ServiceUnitName,
    command: &str,
    output: &CommandOutput,
    duration_ms: u64,
) -> AppError {
    if is_permission_denied(output) {
        permission_denied_error(*action, unit, command, output, duration_ms)
    } else {
        AppError::remote(format!(
            "Could not {} {} {}.",
            action.verb(),
            unit.as_str(),
            exit_suffix(output)
        ))
        .with_details(command_diagnostics(command, output, duration_ms))
    }
}

fn permission_denied_error(
    action: ServiceAction,
    unit: &ServiceUnitName,
    command: &str,
    output: &CommandOutput,
    duration_ms: u64,
) -> AppError {
    AppError::permission(format!(
        "The SSH account is not allowed to {} {}. Grant the dedicated non-root account the \
         narrowly scoped passwordless sudo rule shown in the details; do not use unrestricted sudo.",
        action.verb(),
        unit.as_str()
    ))
    .with_details(format!(
        "{}\n\nRemote diagnostics:\n{}",
        sudo_guidance(unit),
        command_diagnostics(command, output, duration_ms)
    ))
}

/// Guidance for a narrowly scoped sudoers rule for exactly one unit.
///
/// The placeholder account name makes it clear that the rule belongs to the
/// dedicated ServerDesk SSH account, not to an interactive administrator.
pub fn sudo_guidance(unit: &ServiceUnitName) -> String {
    format!(
        "Create and validate a rule for the dedicated non-root ServerDesk SSH account (replace \
         <serverdesk-ssh-user>):\n\
         <serverdesk-ssh-user> ALL=(root) NOPASSWD: \
         /usr/bin/systemctl show --no-pager {SERVICE_STATUS_PROPERTY_FLAGS} -- {unit}, \
         /usr/bin/systemctl start -- {unit}, \
         /usr/bin/systemctl stop -- {unit}, \
         /usr/bin/systemctl restart -- {unit}\n\
         Never grant ALL=(ALL) NOPASSWD: ALL or unrestricted sudo.",
        unit = unit.as_str()
    )
}

fn is_permission_denied(output: &CommandOutput) -> bool {
    if output.exit_code == Some(0) {
        return false;
    }

    let combined = format!(
        "{}\n{}",
        output.stderr.to_ascii_lowercase(),
        output.stdout.to_ascii_lowercase()
    );

    [
        "a password is required",
        "a terminal is required",
        "must have a tty",
        "no tty present",
        "not allowed to execute",
        "not allowed to run sudo",
        "not in the sudoers file",
        "sudoers",
        "permission denied",
        "access denied",
        "authentication is required",
        "interactive authentication required",
        "not authorized",
        "polkit",
        "command not allowed",
    ]
    .iter()
    .any(|needle| combined.contains(needle))
}

fn describe_failure(output: &CommandOutput) -> String {
    let stderr = output
        .stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| bounded_detail(redact_text(line.trim()).text.trim()))
        .unwrap_or_default();
    let stdout = output
        .stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| bounded_detail(redact_text(line.trim()).text.trim()))
        .unwrap_or_default();

    match (stderr.is_empty(), stdout.is_empty()) {
        (false, _) => format!("{} ({})", stderr, exit_suffix(output).trim_start()),
        (true, false) => format!("{} ({})", stdout, exit_suffix(output).trim_start()),
        (true, true) => format!("The remote command failed {}.", exit_suffix(output)),
    }
}

fn exit_suffix(output: &CommandOutput) -> String {
    match output.exit_code {
        Some(code) => format!("(exit code {code})"),
        None => "(no exit status)".to_string(),
    }
}

fn sanitize_stream(value: &str) -> String {
    redact_text(value).text
}

fn command_diagnostics(command: &str, output: &CommandOutput, duration_ms: u64) -> String {
    let stdout = bounded_diagnostic(&redact_text(&output.stdout).text);
    let stderr = bounded_diagnostic(&redact_text(&output.stderr).text);
    format!(
        "Command: {command}\nExit code: {}\nDuration: {duration_ms} ms\n\nstdout:\n{}\n\nstderr:\n{}",
        match output.exit_code {
            Some(code) => code.to_string(),
            None => "no exit status".to_string(),
        },
        if stdout.is_empty() {
            "(empty)"
        } else {
            stdout.as_str()
        },
        if stderr.is_empty() {
            "(empty)"
        } else {
            stderr.as_str()
        }
    )
}

fn bounded_diagnostic(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(MAX_DETAIL_CHARS * 16)
        .collect()
}

fn with_details(error: &AppError) -> String {
    match error.details.as_deref() {
        Some(details) if !details.is_empty() => format!("{} {details}", error.message),
        _ => error.message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEMD_FIXTURE: &str = r#"  nginx.service               loaded    active   running    A high performance web server
  apache2.service             loaded    failed   failed     The Apache HTTP Server
  mysql.service               loaded    inactive dead       MySQL Community Server
  postgresql.service          loaded    active   exited     PostgreSQL RDBMS
  redis-server.service        loaded    activating start     Redis persistent key-value database
  supervisor.service          loaded    inactive dead       Supervisor process control system for UNIX
  php8.3-fpm.service          loaded    active   running    The PHP 8.3 FastCGI Process Manager
  custom-worker.service       loaded    active   running    A service that is not in any common list
  device@1.service            not-found inactive dead     A referenced but missing unit
"#;

    #[test]
    fn parses_common_versioned_and_unlisted_units_generically() {
        let parsed = parse_services(SYSTEMD_FIXTURE);
        assert!(!parsed.truncated);
        assert_eq!(parsed.services.len(), 9);

        let names = parsed
            .services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>();
        for expected in [
            "nginx.service",
            "apache2.service",
            "mysql.service",
            "postgresql.service",
            "redis-server.service",
            "supervisor.service",
            "php8.3-fpm.service",
            "custom-worker.service",
            "device@1.service",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }

        let php = parsed
            .services
            .iter()
            .find(|service| service.name == "php8.3-fpm.service")
            .expect("versioned PHP-FPM unit");
        assert_eq!(php.state, ServiceState::Active);
        assert_eq!(php.active_state, "active");
        assert_eq!(php.sub_state.as_deref(), Some("running"));
        assert_eq!(php.load_state.as_deref(), Some("loaded"));
        assert_eq!(
            php.description.as_deref(),
            Some("The PHP 8.3 FastCGI Process Manager")
        );

        let custom = parsed
            .services
            .iter()
            .find(|service| service.name == "custom-worker.service")
            .expect("unlisted unit");
        assert_eq!(custom.state, ServiceState::Active);
    }

    #[test]
    fn active_inactive_failed_and_unknown_states_stay_distinct() {
        let parsed = parse_services(SYSTEMD_FIXTURE);

        let state_of = |name: &str| {
            parsed
                .services
                .iter()
                .find(|service| service.name == name)
                .map(|service| service.state)
                .expect("fixture unit")
        };

        assert_eq!(state_of("nginx.service"), ServiceState::Active);
        assert_eq!(state_of("mysql.service"), ServiceState::Inactive);
        assert_eq!(state_of("apache2.service"), ServiceState::Failed);
        assert_eq!(state_of("redis-server.service"), ServiceState::Unknown);
        assert_ne!(
            state_of("apache2.service"),
            state_of("redis-server.service")
        );

        let redis = parsed
            .services
            .iter()
            .find(|service| service.name == "redis-server.service")
            .expect("activating unit");
        assert_eq!(redis.active_state, "activating");
        assert_eq!(redis.sub_state.as_deref(), Some("start"));
    }

    #[test]
    fn discovery_does_not_invent_absent_common_services() {
        let parsed =
            parse_services("  custom.service loaded active running Only the fixture unit\n");
        assert_eq!(parsed.services.len(), 1);
        assert_eq!(parsed.services[0].name, "custom.service");
        assert!(
            parsed
                .services
                .iter()
                .all(|service| service.name != "nginx.service")
        );
    }

    #[test]
    fn empty_successful_output_returns_no_units() {
        let parsed = parse_services("");
        assert!(parsed.services.is_empty());
        assert!(!parsed.truncated);
    }

    #[test]
    fn skips_malformed_rows_and_headers() {
        let output = "UNIT LOAD ACTIVE SUB DESCRIPTION\n\
            not-a-unit loaded active running ignored\n\
            0 loaded units listed.\n\
            missing.service\n\
            short.service loaded\n\
            valid.service loaded active running Kept description\n\
            bad@.service loaded active running Still valid\n";
        let parsed = parse_services(output);

        assert!(!parsed.services.is_empty());
        assert!(
            parsed
                .services
                .iter()
                .all(|service| service.name.ends_with(".service"))
        );
        assert!(
            parsed
                .services
                .iter()
                .any(|service| service.name == "valid.service")
        );
        assert!(
            parsed
                .services
                .iter()
                .any(|service| service.name == "bad@.service")
        );
    }

    #[test]
    fn handles_marker_glyphs_and_control_characters_safely() {
        let output = "● failed.service loaded failed failed Failed service\n\
            valid.service loaded active running Valid\u{0007} description\n";
        let parsed = parse_services(output);

        let failed = parsed
            .services
            .iter()
            .find(|service| service.name == "failed.service")
            .expect("marked failed unit");
        assert_eq!(failed.state, ServiceState::Failed);

        let valid = parsed
            .services
            .iter()
            .find(|service| service.name == "valid.service")
            .expect("valid unit");
        assert_eq!(
            valid.description.as_deref(),
            Some("Valid description"),
            "control characters must not reach the UI"
        );
    }

    #[test]
    fn unit_list_and_processing_are_bounded() {
        let output = (0..MAX_SERVICES + 20)
            .map(|index| format!("unit{index}.service loaded active running Unit {index}\n"))
            .collect::<String>();
        let parsed = parse_services(&output);
        assert_eq!(parsed.services.len(), MAX_SERVICES);
        assert!(parsed.truncated);

        let too_many_lines = (0..MAX_PROCESSING_LINES + 1)
            .map(|index| format!("unit{index}.service loaded active running\n"))
            .collect::<String>();
        let parsed = parse_services(&too_many_lines);
        assert!(parsed.truncated);
        assert!(parsed.services.len() <= MAX_SERVICES);

        let overlong_name = format!(
            "  {}.service loaded active running\n",
            "x".repeat(MAX_UNIT_NAME_CHARS)
        );
        let parsed = parse_services(&overlong_name);
        assert!(parsed.services.is_empty());

        let long_description = format!(
            "  nginx.service loaded active running {}\n",
            "d".repeat(MAX_DESCRIPTION_CHARS + 50)
        );
        let parsed = parse_services(&long_description);
        assert_eq!(
            parsed.services[0].description.as_deref().map(str::len),
            Some(MAX_DESCRIPTION_CHARS)
        );
    }

    #[test]
    fn classifies_missing_systemd_as_unsupported() {
        let no_binary = CommandOutput {
            stdout: String::new(),
            stderr: "sh: systemctl: not found\n".to_string(),
            exit_code: Some(127),
        };
        assert_eq!(
            classify_failure(&no_binary),
            ServiceDiscoveryStatus::Unsupported
        );
        assert!(
            failure_detail(&no_binary, ServiceDiscoveryStatus::Unsupported).contains("Systemd")
        );

        let no_init = CommandOutput {
            stdout: String::new(),
            stderr:
                "System has not been booted with systemd as init system (PID 1). Can't operate.\n"
                    .to_string(),
            exit_code: Some(1),
        };
        assert_eq!(
            classify_failure(&no_init),
            ServiceDiscoveryStatus::Unsupported
        );

        let no_bus = CommandOutput {
            stdout: String::new(),
            stderr: "Failed to connect to bus: No such file or directory\n".to_string(),
            exit_code: Some(1),
        };
        assert_eq!(
            classify_failure(&no_bus),
            ServiceDiscoveryStatus::Unsupported
        );

        let transient = CommandOutput {
            stdout: String::new(),
            stderr: "Access denied\n".to_string(),
            exit_code: Some(1),
        };
        assert_eq!(
            classify_failure(&transient),
            ServiceDiscoveryStatus::Unavailable
        );
        assert!(
            failure_detail(&transient, ServiceDiscoveryStatus::Unavailable)
                .contains("Access denied")
        );
    }

    #[test]
    fn serializes_typed_discovery_for_the_frontend() {
        let discovery = ServiceDiscovery {
            server_id: "server-1".to_string(),
            status: ServiceDiscoveryStatus::Available,
            services: vec![ServiceUnit {
                name: "php8.3-fpm.service".to_string(),
                state: ServiceState::Active,
                active_state: "active".to_string(),
                sub_state: Some("running".to_string()),
                load_state: Some("loaded".to_string()),
                description: Some("The PHP 8.3 FastCGI Process Manager".to_string()),
            }],
            detail: None,
            truncated: false,
            sampled_at: 1_762_665_720_000,
            duration_ms: 42,
            connection_state: ConnectionState::Connected,
        };
        let value = serde_json::to_value(&discovery).expect("serialized discovery");

        assert_eq!(value["serverId"], "server-1");
        assert_eq!(value["status"], "available");
        assert_eq!(value["services"][0]["name"], "php8.3-fpm.service");
        assert_eq!(value["services"][0]["state"], "active");
        assert_eq!(value["services"][0]["activeState"], "active");
        assert_eq!(value["services"][0]["subState"], "running");
        assert_eq!(value["services"][0]["loadState"], "loaded");
        assert_eq!(value["truncated"], false);
        assert_eq!(value["sampledAt"], 1_762_665_720_000_i64);
        assert_eq!(value["durationMs"], 42);
        assert_eq!(value["connectionState"], "connected");

        let unsupported = ServiceDiscovery {
            status: ServiceDiscoveryStatus::Unsupported,
            services: Vec::new(),
            detail: Some("Systemd is not available on this server.".to_string()),
            ..discovery
        };
        let value = serde_json::to_value(&unsupported).expect("serialized unsupported discovery");
        assert_eq!(value["status"], "unsupported");
        assert_eq!(value["detail"], "Systemd is not available on this server.");
        assert_eq!(value["services"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn command_and_bounds_are_fixed() {
        assert_eq!(
            SYSTEMD_SERVICES_COMMAND,
            "systemctl list-units --type=service --all --no-pager --no-legend --full"
        );
        for forbidden in [';', '|', '&', '$', '`', '>', '<', '\n'] {
            assert!(
                !SYSTEMD_SERVICES_COMMAND.contains(forbidden),
                "{SYSTEMD_SERVICES_COMMAND:?} must not contain {forbidden:?}"
            );
        }
        assert!(SERVICE_DISCOVERY_TIMEOUT <= Duration::from_secs(10));
    }

    fn unit(name: &str) -> ServiceUnitName {
        validate_service_unit_name(name).expect("valid fixture unit")
    }

    fn output(stdout: &str, stderr: &str, exit_code: Option<u32>) -> CommandOutput {
        CommandOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    #[test]
    fn rejects_invalid_unit_names_before_any_command_is_built() {
        for invalid in [
            "",
            "nginx",
            "nginx.socket",
            "nginx.service ",
            " nginx.service",
            "nginx.service\n",
            "nginx.service; id",
            "nginx.service && reboot",
            "nginx.service|cat",
            "nginx.service$(id)",
            "nginx.service`id`",
            "nginx\".service",
            "nginx'.service",
            "../../etc/passwd.service",
            "/usr/lib/systemd/system/nginx.service",
            "-H.service",
            ".hidden.service",
            "nginx..service",
            "nginx service.service",
            "nginx.service\t",
            "nïginx.service",
            ".service",
        ] {
            assert!(
                validate_service_unit_name(invalid).is_err(),
                "{invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn accepts_versioned_template_and_custom_unit_names() {
        for valid in [
            "nginx.service",
            "php8.3-fpm.service",
            "getty@tty1.service",
            "user@1000.service",
            "custom_worker:1.service",
            "a.service",
        ] {
            assert_eq!(
                validate_service_unit_name(valid).expect(valid).as_str(),
                valid
            );
        }

        let too_long = format!("{}.service", "x".repeat(MAX_UNIT_NAME_CHARS));
        assert!(validate_service_unit_name(&too_long).is_err());
    }

    #[test]
    fn action_commands_are_fixed_and_use_scoped_non_interactive_sudo() {
        let nginx = unit("nginx.service");

        assert_eq!(
            ServiceAction::Status.command(&nginx),
            "/usr/bin/sudo -n -- /usr/bin/systemctl show --no-pager \
             --property=Id --property=LoadState --property=ActiveState --property=SubState \
             --property=UnitFileState --property=Description -- nginx.service"
        );
        assert_eq!(
            ServiceAction::Start.command(&nginx),
            "/usr/bin/sudo -n -- /usr/bin/systemctl start -- nginx.service"
        );
        assert_eq!(
            ServiceAction::Stop.command(&nginx),
            "/usr/bin/sudo -n -- /usr/bin/systemctl stop -- nginx.service"
        );
        assert_eq!(
            ServiceAction::Restart.command(&nginx),
            "/usr/bin/sudo -n -- /usr/bin/systemctl restart -- nginx.service"
        );

        for action in ServiceAction::ALL {
            let command = action.command(&nginx);
            assert!(command.starts_with("/usr/bin/sudo -n -- /usr/bin/systemctl "));
            assert!(command.contains(" -- nginx.service"));
            for forbidden in [';', '|', '&', '$', '`', '>', '<', '\n', '*', '?', '(', ')'] {
                assert!(
                    !command.contains(forbidden),
                    "{command:?} must not contain {forbidden:?}"
                );
            }
        }
    }

    #[test]
    fn status_output_is_parsed_into_a_verified_state() {
        let status = parse_service_status(
            "server-1",
            &unit("nginx.service"),
            "Id=nginx.service\nLoadState=loaded\nActiveState=active\nSubState=running\n\
             UnitFileState=enabled\nDescription=A high performance web server\n",
            12,
        )
        .expect("parsed status");

        assert_eq!(status.server_id, "server-1");
        assert_eq!(status.unit, "nginx.service");
        assert_eq!(status.state, ServiceState::Active);
        assert_eq!(status.active_state, "active");
        assert_eq!(status.sub_state.as_deref(), Some("running"));
        assert_eq!(status.load_state.as_deref(), Some("loaded"));
        assert_eq!(status.unit_file_state.as_deref(), Some("enabled"));
        assert_eq!(
            status.description.as_deref(),
            Some("A high performance web server")
        );
        assert!(status.sampled_at > 0);

        let missing = parse_service_status(
            "server-1",
            &unit("nginx.service"),
            "LoadState=loaded\nSubState=running\n",
            5,
        )
        .expect_err("ActiveState is required");
        assert!(missing.contains("ActiveState"));

        let unknown = parse_service_status(
            "server-1",
            &unit("nginx.service"),
            "LoadState=loaded\nActiveState=activating\nSubState=start\n",
            5,
        )
        .expect("unknown state is still parsed");
        assert_eq!(unknown.state, ServiceState::Unknown);
        assert_eq!(unknown.active_state, "activating");
        assert_eq!(unknown.sub_state.as_deref(), Some("start"));
    }

    #[test]
    fn permission_denial_maps_to_permission_error_with_sudo_guidance() {
        let nginx = unit("nginx.service");
        let command = ServiceAction::Restart.command(&nginx);
        let denied = output(
            "",
            "sudo: a password is required\nsudo: no tty present and no askpass program specified\n",
            Some(1),
        );

        assert!(is_permission_denied(&denied));
        let error = command_error(&ServiceAction::Restart, &nginx, &command, &denied, 17);

        assert_eq!(error.code, ErrorCode::Permission);
        assert!(error.message.contains("narrowly scoped"));
        assert!(error.message.contains("nginx.service"));
        let details = error.details.expect("sudo guidance details");
        assert!(details.contains("NOPASSWD"));
        assert!(details.contains("/usr/bin/systemctl restart -- nginx.service"));
        assert!(details.contains("/usr/bin/systemctl start -- nginx.service"));
        assert!(details.contains("Never grant ALL=(ALL) NOPASSWD: ALL"));
        assert!(details.contains("sudo: a password is required"));
        assert!(details.contains("Exit code: 1"));
        assert!(details.contains("Duration: 17 ms"));
    }

    #[test]
    fn access_denied_and_success_are_classified_correctly() {
        assert!(is_permission_denied(&output(
            "",
            "Failed to restart nginx.service: Access denied\n",
            Some(1),
        )));
        assert!(is_permission_denied(&output(
            "",
            "Interactive authentication required.\n",
            Some(1),
        )));
        assert!(!is_permission_denied(&output(
            "",
            "Job for nginx.service failed because the control process exited with error code.\n",
            Some(1),
        )));
        assert!(!is_permission_denied(&output(
            "",
            "permission denied\n",
            Some(0),
        )));
    }

    #[test]
    fn failed_action_keeps_output_exit_code_and_duration() {
        let nginx = unit("nginx.service");
        let command = ServiceAction::Start.command(&nginx);
        let failed = output(
            "starting\n",
            "Job for nginx.service failed because the control process exited with error code.\n",
            Some(5),
        );

        let error = command_error(&ServiceAction::Start, &nginx, &command, &failed, 33);
        assert_eq!(error.code, ErrorCode::Remote);
        assert!(error.message.contains("exit code 5"));
        let details = error.details.expect("failure details");
        assert!(
            details
                .contains("Command: /usr/bin/sudo -n -- /usr/bin/systemctl start -- nginx.service")
        );
        assert!(details.contains("stdout:\nstarting"));
        assert!(details.contains("stderr:\nJob for nginx.service failed"));
        assert!(details.contains("Exit code: 5"));
        assert!(details.contains("Duration: 33 ms"));
    }

    #[test]
    fn action_names_are_a_closed_typed_set() {
        for (value, expected) in [
            ("\"status\"", ServiceAction::Status),
            ("\"start\"", ServiceAction::Start),
            ("\"stop\"", ServiceAction::Stop),
            ("\"restart\"", ServiceAction::Restart),
        ] {
            assert_eq!(
                serde_json::from_str::<ServiceAction>(value).expect("known action"),
                expected
            );
        }

        for invalid in [
            "\"reload\"",
            "\"start; id\"",
            "\"rm -rf /\"",
            "\"\"",
            "null",
        ] {
            assert!(
                serde_json::from_str::<ServiceAction>(invalid).is_err(),
                "{invalid:?} must not deserialize into a service action"
            );
        }
    }

    #[test]
    fn serializes_action_results_with_verified_state_and_nested_error() {
        let nginx = unit("nginx.service");
        let command = ServiceAction::Restart.command(&nginx);
        let denied = output("", "sudo: a password is required\n", Some(1));
        let error = command_error(&ServiceAction::Restart, &nginx, &command, &denied, 5);
        let result = ServiceActionResult {
            server_id: "server-1".to_string(),
            unit: "nginx.service".to_string(),
            action: ServiceAction::Restart,
            command,
            succeeded: false,
            stdout: String::new(),
            stderr: "sudo: a password is required\n".to_string(),
            exit_code: Some(1),
            duration_ms: 5,
            permission_denied: true,
            sudo_guidance: Some(sudo_guidance(&nginx)),
            verified_status: Some(ServiceStatus {
                server_id: "server-1".to_string(),
                unit: "nginx.service".to_string(),
                state: ServiceState::Inactive,
                active_state: "inactive".to_string(),
                sub_state: Some("dead".to_string()),
                load_state: Some("loaded".to_string()),
                unit_file_state: Some("enabled".to_string()),
                description: Some("A high performance web server".to_string()),
                sampled_at: 1_762_665_720_000,
                duration_ms: 3,
            }),
            state_verified: Some(false),
            verification_error: None,
            error: Some(error),
            connection_state: ConnectionState::Connected,
        };
        let value = serde_json::to_value(&result).expect("serialized action result");

        assert_eq!(value["serverId"], "server-1");
        assert_eq!(value["unit"], "nginx.service");
        assert_eq!(value["action"], "restart");
        assert_eq!(value["succeeded"], false);
        assert_eq!(value["permissionDenied"], true);
        assert_eq!(value["exitCode"], 1);
        assert_eq!(value["stateVerified"], false);
        assert_eq!(value["verifiedStatus"]["state"], "inactive");
        assert_eq!(value["error"]["code"], "permission");
        assert!(
            value["sudoGuidance"]
                .as_str()
                .expect("guidance")
                .contains("NOPASSWD")
        );
    }
}
