//! Systemd service discovery.
//!
//! Discovery runs one fixed, read-only `systemctl` command over an already
//! authenticated SSH session. Output is bounded before it is parsed, units are
//! parsed generically from the command output, and systems without systemd
//! return a typed unsupported result instead of an empty list presented as a
//! successful discovery.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::{
    AppError,
    ssh::{CommandOutput, ConnectionManager, ConnectionState},
};

/// Hard output bound applied to the service discovery command.
pub const MAX_SERVICE_OUTPUT_BYTES: usize = 64 * 1024;
const _: () = assert!(MAX_SERVICE_OUTPUT_BYTES <= 64 * 1024);

/// Hard command-duration bound applied on top of the connection timeout.
pub const SERVICE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

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
        .map(|line| bounded_detail(line.trim()))
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
}
