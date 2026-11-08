//! Lightweight remote OS and system identity discovery.
//!
//! Discovery runs a small set of fixed, read-only command templates over an
//! already authenticated SSH session. Every command output is bounded before
//! it is parsed, and missing or unsupported fields remain `None` instead of
//! being guessed from unrelated data.

use std::time::Instant;

use serde::Serialize;

use crate::core::{
    AppError,
    ssh::{ConnectionManager, ConnectionState},
};

/// Hard output bound applied to every identity command.
pub const MAX_IDENTITY_OUTPUT_BYTES: usize = 16 * 1024;
const _: () = assert!(MAX_IDENTITY_OUTPUT_BYTES <= 64 * 1024);

const MAX_IDENTITY_LINES: usize = 512;
const MAX_VALUE_CHARS: usize = 256;
const MAX_KEY_CHARS: usize = 64;

pub const HOSTNAME_COMMAND: &str = "hostname";
pub const OS_RELEASE_COMMAND: &str = "cat /etc/os-release";
pub const UNAME_COMMAND: &str = "uname -srm";
pub const UPTIME_COMMAND: &str = "cat /proc/uptime";

/// Every fixed command sent during one identity sample.
pub const IDENTITY_COMMANDS: [&str; 4] = [
    HOSTNAME_COMMAND,
    OS_RELEASE_COMMAND,
    UNAME_COMMAND,
    UPTIME_COMMAND,
];

/// A point-in-time sample of the remote server identity.
///
/// Fields are `None` when the remote system does not expose them or the
/// command output cannot be parsed without guessing. `sampled_at` is the
/// local Unix timestamp in milliseconds when the sample completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemIdentity {
    pub server_id: String,
    pub hostname: Option<String>,
    pub distribution: Option<String>,
    pub distribution_version: Option<String>,
    pub kernel: Option<String>,
    pub architecture: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub sampled_at: i64,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OsRelease {
    name: Option<String>,
    version_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct UnameDetails {
    kernel: Option<String>,
    architecture: Option<String>,
}

/// Samples identity fields over an existing authenticated SSH session.
///
/// Each command uses the connection manager's configured command timeout and
/// a strict output bound. A non-zero exit status marks only that field group
/// unavailable so partial identity can still be shown. Transport failures are
/// returned as errors because the session itself is no longer reliable.
pub async fn sample_system_identity(
    manager: &ConnectionManager,
    server_id: &str,
) -> Result<SystemIdentity, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let started = Instant::now();

    let hostname = command_stdout(manager, server_id, HOSTNAME_COMMAND)
        .await?
        .and_then(|stdout| parse_hostname(&stdout));

    let os_release = match command_stdout(manager, server_id, OS_RELEASE_COMMAND).await? {
        Some(stdout) => parse_os_release(&stdout),
        None => OsRelease::default(),
    };

    let uname = match command_stdout(manager, server_id, UNAME_COMMAND).await? {
        Some(stdout) => parse_uname(&stdout),
        None => UnameDetails::default(),
    };

    let uptime_seconds = command_stdout(manager, server_id, UPTIME_COMMAND)
        .await?
        .and_then(|stdout| parse_uptime_seconds(&stdout));

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let connection_state = manager.connection_state(server_id)?.state;

    Ok(SystemIdentity {
        server_id: server_id.to_string(),
        hostname,
        distribution: os_release.name,
        distribution_version: os_release.version_id,
        kernel: uname.kernel,
        architecture: uname.architecture,
        uptime_seconds,
        sampled_at: now_millis(),
        duration_ms,
        connection_state,
    })
}

async fn command_stdout(
    manager: &ConnectionManager,
    server_id: &str,
    command: &str,
) -> Result<Option<String>, AppError> {
    let output = manager
        .execute_bounded(server_id, command, MAX_IDENTITY_OUTPUT_BYTES)
        .await?;

    if output.exit_code != Some(0) {
        return Ok(None);
    }

    Ok(Some(output.stdout))
}

fn parse_hostname(stdout: &str) -> Option<String> {
    sanitize_value(stdout.trim())
}

fn parse_os_release(stdout: &str) -> OsRelease {
    let mut release = OsRelease::default();

    for line in stdout.lines().take(MAX_IDENTITY_LINES) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty()
            || key.chars().count() > MAX_KEY_CHARS
            || !key
                .chars()
                .all(|character| character.is_ascii_uppercase() || character == '_')
        {
            continue;
        }

        let value = parse_os_release_value(raw_value);
        match key {
            "NAME" => release.name = value,
            "VERSION_ID" => release.version_id = value,
            _ => {}
        }
    }

    release
}

fn parse_os_release_value(raw: &str) -> Option<String> {
    let raw = raw.trim();

    let value = if let Some(inner) = raw.strip_prefix('"') {
        unescape_double_quoted(inner.strip_suffix('"')?)
    } else if let Some(inner) = raw.strip_prefix('\'') {
        inner.strip_suffix('\'')?.to_string()
    } else {
        if raw.contains('"') || raw.contains('\'') {
            return None;
        }
        raw.to_string()
    };

    sanitize_value(&value)
}

fn unescape_double_quoted(value: &str) -> String {
    let mut unescaped = String::with_capacity(value.len());
    let mut characters = value.chars();

    while let Some(character) = characters.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }

        match characters.next() {
            Some(escaped @ ('"' | '\\' | '$' | '`')) => unescaped.push(escaped),
            Some(escaped) => {
                unescaped.push('\\');
                unescaped.push(escaped);
            }
            None => unescaped.push('\\'),
        }
    }

    unescaped
}

fn parse_uname(stdout: &str) -> UnameDetails {
    let mut fields = stdout.split_whitespace();
    let (Some(_sysname), Some(release), Some(machine), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return UnameDetails::default();
    };

    UnameDetails {
        kernel: sanitize_value(release),
        architecture: sanitize_value(machine),
    }
}

fn parse_uptime_seconds(stdout: &str) -> Option<u64> {
    let token = stdout.split_whitespace().next()?;
    if token.len() > 32 {
        return None;
    }

    let seconds = token.parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }

    Some(seconds.floor() as u64)
}

fn sanitize_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_VALUE_CHARS
        || value.chars().any(char::is_control)
    {
        return None;
    }

    Some(value.to_string())
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

    const UBUNTU_OS_RELEASE: &str = r#"PRETTY_NAME="Ubuntu 24.04.1 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04.1 LTS (Noble Numbat)"
VERSION_CODENAME=noble
ID=ubuntu
ID_LIKE=debian
HOME_URL="https://www.ubuntu.com/"
SUPPORT_URL="https://help.ubuntu.com/"
BUG_REPORT_URL="https://bugs.launchpad.net/ubuntu/"
PRIVACY_POLICY_URL="https://www.ubuntu.com/legal/terms-and-policies/privacy-policy"
UBUNTU_CODENAME=noble
LOGO=ubuntu-logo
"#;

    const DEBIAN_OS_RELEASE: &str = r#"PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"
VERSION_ID="12"
VERSION="12 (bookworm)"
VERSION_CODENAME=bookworm
ID=debian
HOME_URL="https://www.debian.org/"
SUPPORT_URL="https://www.debian.org/support"
BUG_REPORT_URL="https://bugs.debian.org/"
"#;

    const FEDORA_OS_RELEASE: &str = r#"NAME="Fedora Linux"
VERSION="41 (Workstation Edition)"
RELEASE_TYPE=stable
ID=fedora
VERSION_ID=41
VERSION_CODENAME=""
PLATFORM_ID="platform:f41"
PRETTY_NAME="Fedora Linux 41 (Workstation Edition)"
ANSI_COLOR="0;38;2;60;110;180"
LOGO=fedora-logo-icon
CPE_NAME="cpe:/o:fedoraproject:fedora:41"
DEFAULT_HOSTNAME="fedora"
HOME_URL="https://fedoraproject.org/"
DOCUMENTATION_URL="https://docs.fedoraproject.org/en-US/fedora/final/"
SUPPORT_URL="https://fedoraproject.org/wiki/Communicating_and_getting_help"
BUG_REPORT_URL="https://bugzilla.redhat.com/"
REDHAT_BUGZILLA_PRODUCT="Fedora"
REDHAT_BUGZILLA_PRODUCT_VERSION=41
REDHAT_SUPPORT_PRODUCT="Fedora"
REDHAT_SUPPORT_PRODUCT_VERSION=41
SUPPORT_END=2025-11-25
"#;

    #[test]
    fn parses_ubuntu_identity_fixtures() {
        let release = parse_os_release(UBUNTU_OS_RELEASE);
        assert_eq!(release.name.as_deref(), Some("Ubuntu"));
        assert_eq!(release.version_id.as_deref(), Some("24.04"));

        let uname = parse_uname("Linux 6.8.0-45-generic x86_64\n");
        assert_eq!(uname.kernel.as_deref(), Some("6.8.0-45-generic"));
        assert_eq!(uname.architecture.as_deref(), Some("x86_64"));
    }

    #[test]
    fn parses_debian_identity_fixtures() {
        let release = parse_os_release(DEBIAN_OS_RELEASE);
        assert_eq!(release.name.as_deref(), Some("Debian GNU/Linux"));
        assert_eq!(release.version_id.as_deref(), Some("12"));

        let uname = parse_uname("Linux 6.1.0-26-amd64 x86_64\n");
        assert_eq!(uname.kernel.as_deref(), Some("6.1.0-26-amd64"));
        assert_eq!(uname.architecture.as_deref(), Some("x86_64"));
    }

    #[test]
    fn parses_fedora_identity_fixtures() {
        let release = parse_os_release(FEDORA_OS_RELEASE);
        assert_eq!(release.name.as_deref(), Some("Fedora Linux"));
        assert_eq!(release.version_id.as_deref(), Some("41"));

        let uname = parse_uname("Linux 6.11.4-301.fc41.x86_64 x86_64\n");
        assert_eq!(uname.kernel.as_deref(), Some("6.11.4-301.fc41.x86_64"));
        assert_eq!(uname.architecture.as_deref(), Some("x86_64"));
    }

    #[test]
    fn parses_hostname_and_uptime_fixtures() {
        assert_eq!(
            parse_hostname("production-1\n").as_deref(),
            Some("production-1")
        );
        assert_eq!(
            parse_uptime_seconds("372063.12 1234567.89\n"),
            Some(372_063)
        );
    }

    #[test]
    fn os_release_parser_handles_quotes_comments_and_escapes() {
        let release = parse_os_release(
            "# generated fixture\nNAME=\"Example \\\"Linux\\\"\"\nVERSION_ID='9'\nBROKEN=value\n",
        );

        assert_eq!(release.name.as_deref(), Some("Example \"Linux\""));
        assert_eq!(release.version_id.as_deref(), Some("9"));
    }

    #[test]
    fn unsupported_or_missing_fields_stay_unavailable() {
        let release = parse_os_release("NAME=\"Custom Linux\"\n");
        assert_eq!(release.name.as_deref(), Some("Custom Linux"));
        assert_eq!(release.version_id, None);

        let release = parse_os_release("");
        assert_eq!(release.name, None);
        assert_eq!(release.version_id, None);

        let uname = parse_uname("Linux 6.8.0-45-generic\n");
        assert_eq!(uname.kernel, None);
        assert_eq!(uname.architecture, None);

        assert_eq!(parse_hostname("\n"), None);
        assert_eq!(parse_hostname("bad\nhostname\n"), None);
        assert_eq!(parse_uptime_seconds("not-a-number 1.0\n"), None);
        assert_eq!(parse_uptime_seconds("-1.0 1.0\n"), None);
        assert_eq!(parse_uptime_seconds(""), None);
    }

    #[test]
    fn oversized_values_are_rejected_instead_of_truncated() {
        let long_name = format!("NAME=\"{}\"\n", "x".repeat(MAX_VALUE_CHARS + 1));
        let release = parse_os_release(&long_name);
        assert_eq!(release.name, None);

        let long_uname = format!("Linux {} x86_64\n", "k".repeat(MAX_VALUE_CHARS + 1));
        let uname = parse_uname(&long_uname);
        assert_eq!(uname.kernel, None);
        assert_eq!(uname.architecture.as_deref(), Some("x86_64"));
    }

    #[test]
    fn identity_commands_are_fixed_and_bounded() {
        assert_eq!(
            IDENTITY_COMMANDS,
            [
                "hostname",
                "cat /etc/os-release",
                "uname -srm",
                "cat /proc/uptime"
            ]
        );
        for command in IDENTITY_COMMANDS {
            for forbidden in [';', '|', '&', '$', '`', '>', '<', '\n'] {
                assert!(
                    !command.contains(forbidden),
                    "{command:?} must not contain {forbidden:?}"
                );
            }
        }
    }

    #[test]
    fn serializes_identity_for_the_frontend() {
        let identity = SystemIdentity {
            server_id: "server-1".to_string(),
            hostname: Some("production-1".to_string()),
            distribution: Some("Ubuntu".to_string()),
            distribution_version: Some("24.04".to_string()),
            kernel: Some("6.8.0-45-generic".to_string()),
            architecture: Some("x86_64".to_string()),
            uptime_seconds: Some(372_063),
            sampled_at: 1_762_665_720_000,
            duration_ms: 42,
            connection_state: ConnectionState::Connected,
        };
        let value = serde_json::to_value(&identity).expect("serialized identity");

        assert_eq!(value["serverId"], "server-1");
        assert_eq!(value["hostname"], "production-1");
        assert_eq!(value["distribution"], "Ubuntu");
        assert_eq!(value["distributionVersion"], "24.04");
        assert_eq!(value["kernel"], "6.8.0-45-generic");
        assert_eq!(value["architecture"], "x86_64");
        assert_eq!(value["uptimeSeconds"], 372_063);
        assert_eq!(value["sampledAt"], 1_762_665_720_000_i64);
        assert_eq!(value["durationMs"], 42);
        assert_eq!(value["connectionState"], "connected");

        let unavailable = SystemIdentity {
            hostname: None,
            distribution: None,
            distribution_version: None,
            kernel: None,
            architecture: None,
            uptime_seconds: None,
            ..identity
        };
        let value = serde_json::to_value(unavailable).expect("serialized unavailable identity");
        assert!(value["hostname"].is_null());
        assert!(value["distribution"].is_null());
        assert!(value["uptimeSeconds"].is_null());
    }
}
