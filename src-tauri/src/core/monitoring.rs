//! Lightweight, snapshot-based monitoring metrics.
//!
//! Collection uses a small set of fixed, read-only command templates and
//! bounded output. Every metric group carries its own collection timestamp and
//! an explicit status, so an unsupported or failed source is never presented as
//! live data.

use std::time::Instant;

use serde::Serialize;

use crate::core::{
    AppError,
    redaction::redact_text,
    ssh::{CommandOutput, ConnectionManager, ConnectionState},
};

/// Hard output bound applied to every monitoring command.
pub const MAX_MONITORING_OUTPUT_BYTES: usize = 32 * 1024;
const _: () = assert!(MAX_MONITORING_OUTPUT_BYTES <= 64 * 1024);

const MAX_DETAIL_CHARS: usize = 180;
const MAX_LABEL_CHARS: usize = 256;
const MAX_INTERFACE_CHARS: usize = 32;
const MAX_FILESYSTEMS: usize = 64;
const MAX_NETWORK_INTERFACES: usize = 64;
const MAX_PROCESSING_LINES: usize = 4096;

pub const LOADAVG_COMMAND: &str = "cat /proc/loadavg";
pub const NPROC_COMMAND: &str = "nproc";
pub const MEMINFO_COMMAND: &str = "cat /proc/meminfo";
pub const DF_COMMAND: &str = "df -Pk";
pub const NETDEV_COMMAND: &str = "cat /proc/net/dev";

/// Every fixed command sent during one monitoring sample.
pub const MONITORING_COMMANDS: [&str; 5] = [
    LOADAVG_COMMAND,
    NPROC_COMMAND,
    MEMINFO_COMMAND,
    DF_COMMAND,
    NETDEV_COMMAND,
];

/// How trustworthy one metric group is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus {
    /// The source was read and parsed successfully.
    Available,
    /// The remote system does not expose this metric at all.
    Unsupported,
    /// The source exists but could not be collected or parsed this time.
    Unavailable,
}

/// A metric group together with its collection time and status.
///
/// `detail` explains an unsupported or unavailable group. It is `None` for an
/// available group.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricGroup<T> {
    pub status: MetricStatus,
    pub sampled_at: i64,
    pub data: Option<T>,
    pub detail: Option<String>,
}

impl<T> MetricGroup<T> {
    fn available(data: T, sampled_at: i64) -> Self {
        Self {
            status: MetricStatus::Available,
            sampled_at,
            data: Some(data),
            detail: None,
        }
    }

    fn unsupported(detail: String, sampled_at: i64) -> Self {
        Self {
            status: MetricStatus::Unsupported,
            sampled_at,
            data: None,
            detail: Some(detail),
        }
    }

    fn unavailable(detail: String, sampled_at: i64) -> Self {
        Self {
            status: MetricStatus::Unavailable,
            sampled_at,
            data: None,
            detail: Some(detail),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
    pub logical_cpus: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesystemMetrics {
    pub filesystem: Option<String>,
    pub mount: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceMetrics {
    pub interface: String,
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
    pub received_packets: u64,
    pub transmitted_packets: u64,
}

/// A point-in-time sample of the remote server resources.
///
/// `sampled_at` is the local Unix timestamp in milliseconds when the sample
/// completed. Each metric group also keeps the time its own source was read.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringSnapshot {
    pub server_id: String,
    pub sampled_at: i64,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
    pub cpu: MetricGroup<CpuMetrics>,
    pub memory: MetricGroup<MemoryMetrics>,
    pub filesystems: MetricGroup<Vec<FilesystemMetrics>>,
    pub network: MetricGroup<Vec<NetworkInterfaceMetrics>>,
}

enum CommandReading {
    Output(String),
    Failed {
        status: MetricStatus,
        detail: String,
    },
}

/// Samples CPU, memory, filesystem, and network metrics over an existing
/// authenticated SSH session.
///
/// A transport failure aborts the whole sample; the caller must keep its last
/// successful snapshot and mark it stale. A failing individual source only
/// marks that metric group unavailable or unsupported so partial data remains
/// useful and honest.
pub async fn sample_monitoring_snapshot(
    manager: &ConnectionManager,
    server_id: &str,
) -> Result<MonitoringSnapshot, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let started = Instant::now();

    let cpu = collect_cpu(manager, server_id).await?;
    let memory = collect_metric(manager, server_id, MEMINFO_COMMAND, parse_memory).await?;
    let filesystems = collect_metric(manager, server_id, DF_COMMAND, parse_filesystems).await?;
    let network =
        collect_metric(manager, server_id, NETDEV_COMMAND, parse_network_interfaces).await?;

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let connection_state = manager.connection_state(server_id)?.state;

    Ok(MonitoringSnapshot {
        server_id: server_id.to_string(),
        sampled_at: now_millis(),
        duration_ms,
        connection_state,
        cpu,
        memory,
        filesystems,
        network,
    })
}

async fn collect_cpu(
    manager: &ConnectionManager,
    server_id: &str,
) -> Result<MetricGroup<CpuMetrics>, AppError> {
    let load = read_command(manager, server_id, LOADAVG_COMMAND).await?;
    let cpu_count = read_command(manager, server_id, NPROC_COMMAND).await?;
    let sampled_at = now_millis();

    match load {
        CommandReading::Output(stdout) => match parse_loadavg(&stdout) {
            Ok((load_1m, load_5m, load_15m)) => {
                let logical_cpus = match cpu_count {
                    CommandReading::Output(stdout) => parse_cpu_count(&stdout),
                    CommandReading::Failed { .. } => None,
                };
                Ok(MetricGroup::available(
                    CpuMetrics {
                        load_1m,
                        load_5m,
                        load_15m,
                        logical_cpus,
                    },
                    sampled_at,
                ))
            }
            Err(detail) => Ok(MetricGroup::unavailable(detail, sampled_at)),
        },
        CommandReading::Failed { status, detail } => {
            Ok(unavailable_group(status, detail, sampled_at))
        }
    }
}

async fn collect_metric<T, F>(
    manager: &ConnectionManager,
    server_id: &str,
    command: &str,
    parser: F,
) -> Result<MetricGroup<T>, AppError>
where
    F: FnOnce(&str) -> Result<T, String>,
{
    match read_command(manager, server_id, command).await? {
        CommandReading::Output(stdout) => {
            let sampled_at = now_millis();
            match parser(&stdout) {
                Ok(data) => Ok(MetricGroup::available(data, sampled_at)),
                Err(detail) => Ok(MetricGroup::unavailable(detail, sampled_at)),
            }
        }
        CommandReading::Failed { status, detail } => {
            Ok(unavailable_group(status, detail, now_millis()))
        }
    }
}

fn unavailable_group<T>(status: MetricStatus, detail: String, sampled_at: i64) -> MetricGroup<T> {
    match status {
        MetricStatus::Unsupported => MetricGroup::unsupported(detail, sampled_at),
        MetricStatus::Available | MetricStatus::Unavailable => {
            MetricGroup::unavailable(detail, sampled_at)
        }
    }
}

async fn read_command(
    manager: &ConnectionManager,
    server_id: &str,
    command: &str,
) -> Result<CommandReading, AppError> {
    let output = manager
        .execute_bounded(server_id, command, MAX_MONITORING_OUTPUT_BYTES)
        .await?;

    if output.exit_code == Some(0) {
        return Ok(CommandReading::Output(output.stdout));
    }

    Ok(CommandReading::Failed {
        status: classify_failure(&output),
        detail: failure_detail(command, &output),
    })
}

fn classify_failure(output: &CommandOutput) -> MetricStatus {
    if output.exit_code == Some(127) {
        return MetricStatus::Unsupported;
    }

    let stderr = output.stderr.to_ascii_lowercase();
    if [
        "not found",
        "no such file",
        "not supported",
        "unsupported",
        "not available",
        "does not exist",
    ]
    .iter()
    .any(|needle| stderr.contains(needle))
    {
        MetricStatus::Unsupported
    } else {
        MetricStatus::Unavailable
    }
}

fn failure_detail(command: &str, output: &CommandOutput) -> String {
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

    if stderr.is_empty() {
        format!("The remote command {command:?} failed with {exit}.")
    } else {
        format!("The remote command {command:?} failed with {exit}: {stderr}")
    }
}

fn bounded_detail(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DETAIL_CHARS)
        .collect()
}

fn parse_loadavg(stdout: &str) -> Result<(f64, f64, f64), String> {
    let mut tokens = stdout.split_whitespace();
    let mut values = [0.0; 3];

    for value in &mut values {
        let token = tokens
            .next()
            .ok_or_else(|| "The load average output was incomplete.".to_string())?;
        if token.len() > 32 {
            return Err("The load average output could not be parsed.".to_string());
        }

        *value = token
            .parse::<f64>()
            .map_err(|_| "The load average output could not be parsed.".to_string())?;
        if !value.is_finite() || *value < 0.0 {
            return Err("The load average output could not be parsed.".to_string());
        }
    }

    Ok((values[0], values[1], values[2]))
}

fn parse_cpu_count(stdout: &str) -> Option<u32> {
    let count = stdout.trim().parse::<u32>().ok()?;
    (1..=4096).contains(&count).then_some(count)
}

fn parse_memory(stdout: &str) -> Result<MemoryMetrics, String> {
    let mut total = None;
    let mut available = None;

    for line in stdout.lines().take(MAX_PROCESSING_LINES) {
        let Some((key, raw_value)) = line.split_once(':') else {
            continue;
        };

        match key.trim() {
            "MemTotal" => total = parse_kib_value(raw_value),
            "MemAvailable" => available = parse_kib_value(raw_value),
            _ => {}
        }
    }

    let total = total.ok_or_else(|| {
        "The memory snapshot did not include a usable MemTotal value.".to_string()
    })?;
    let available = available.ok_or_else(|| {
        "The memory snapshot did not include a usable MemAvailable value.".to_string()
    })?;
    if total == 0 || available > total {
        return Err("The memory snapshot contained inconsistent values.".to_string());
    }

    let used = total - available;
    let used_percent = percentage(used, total);

    Ok(MemoryMetrics {
        total_bytes: total,
        used_bytes: used,
        available_bytes: available,
        used_percent,
    })
}

fn parse_kib_value(raw: &str) -> Option<u64> {
    let mut parts = raw.split_whitespace();
    let value = parts.next()?.parse::<u64>().ok()?;
    let unit = parts.next().unwrap_or_default().to_ascii_lowercase();

    match unit.as_str() {
        "" | "b" => Some(value),
        "kb" | "kib" => value.checked_mul(1024),
        "mb" | "mib" => value.checked_mul(1024 * 1024),
        "gb" | "gib" => value.checked_mul(1024 * 1024 * 1024),
        _ => None,
    }
}

fn parse_filesystems(stdout: &str) -> Result<Vec<FilesystemMetrics>, String> {
    let mut filesystems = Vec::new();

    for line in stdout.lines().skip(1).take(MAX_PROCESSING_LINES) {
        let Some(filesystem) = parse_filesystem_line(line) else {
            continue;
        };
        filesystems.push(filesystem);
        if filesystems.len() >= MAX_FILESYSTEMS {
            break;
        }
    }

    if filesystems.is_empty() {
        Err("No filesystem usage data was returned.".to_string())
    } else {
        Ok(filesystems)
    }
}

fn parse_filesystem_line(line: &str) -> Option<FilesystemMetrics> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 6 {
        return None;
    }

    let total_blocks = tokens[1].parse::<u64>().ok()?;
    let used_blocks = tokens[2].parse::<u64>().ok()?;
    let available_blocks = tokens[3].parse::<u64>().ok()?;
    if total_blocks == 0 {
        return None;
    }

    let total_bytes = total_blocks.checked_mul(1024)?;
    let used_bytes = used_blocks.checked_mul(1024)?;
    let available_bytes = available_blocks.checked_mul(1024)?;
    let mount = sanitize_label(&tokens[5..].join(" "), MAX_LABEL_CHARS)?;
    let filesystem = sanitize_label(tokens[0], MAX_LABEL_CHARS);
    let capacity = tokens[4]
        .strip_suffix('%')
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value));
    let used_percent = capacity
        .unwrap_or_else(|| percentage(used_bytes, used_bytes.saturating_add(available_bytes)));

    Some(FilesystemMetrics {
        filesystem,
        mount,
        total_bytes,
        used_bytes,
        available_bytes,
        used_percent: round_percentage(used_percent),
    })
}

fn parse_network_interfaces(stdout: &str) -> Result<Vec<NetworkInterfaceMetrics>, String> {
    let mut interfaces = Vec::new();

    for line in stdout.lines().skip(2).take(MAX_PROCESSING_LINES) {
        let Some((name, raw_fields)) = line.split_once(':') else {
            continue;
        };
        let Some(interface) = sanitize_label(name, MAX_INTERFACE_CHARS) else {
            continue;
        };
        let fields = raw_fields.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 16 {
            continue;
        }

        let Some(received_bytes) = fields[0].parse::<u64>().ok() else {
            continue;
        };
        let Some(received_packets) = fields[1].parse::<u64>().ok() else {
            continue;
        };
        let Some(transmitted_bytes) = fields[8].parse::<u64>().ok() else {
            continue;
        };
        let Some(transmitted_packets) = fields[9].parse::<u64>().ok() else {
            continue;
        };

        interfaces.push(NetworkInterfaceMetrics {
            interface,
            received_bytes,
            transmitted_bytes,
            received_packets,
            transmitted_packets,
        });
        if interfaces.len() >= MAX_NETWORK_INTERFACES {
            break;
        }
    }

    if interfaces.is_empty() {
        Err("No network interface counters were returned.".to_string())
    } else {
        Ok(interfaces)
    }
}

fn sanitize_label(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        return None;
    }

    Some(value.to_string())
}

fn percentage(used: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }

    round_percentage(used as f64 / total as f64 * 100.0)
}

fn round_percentage(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
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

    const LOADAVG_FIXTURE: &str = "0.42 0.31 0.28 2/512 123456\n";
    const NPROC_FIXTURE: &str = "8\n";
    const MEMINFO_FIXTURE: &str = r#"MemTotal:       16316172 kB
MemFree:          845624 kB
MemAvailable:    9247520 kB
Buffers:          123456 kB
Cached:          5678901 kB
SwapCached:            0 kB
Active:          3456789 kB
Inactive:        2345678 kB
SwapTotal:       2097148 kB
SwapFree:        2097148 kB
"#;
    const DF_FIXTURE: &str = r#"Filesystem     1024-blocks      Used Available Capacity Mounted on
/dev/vda1         41020672  12345678  26566840      32% /
/dev/vdb1        103080624  23456789  74345678      24% /mnt/data
tmpfs               812344         0    812344       0% /run/user/1000
"#;
    const NETDEV_FIXTURE: &str = r#"Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo:    1000      10    0    0    0     0          0         0     1000      10    0    0    0     0       0          0
  eth0: 123456789   98765    0    0    0     0          0         0 987654321   54321    0    0    0     0       0          0
"#;

    #[test]
    fn parses_load_average_and_logical_cpu_count() {
        let (load_1m, load_5m, load_15m) = parse_loadavg(LOADAVG_FIXTURE).expect("load average");
        assert!((load_1m - 0.42).abs() < f64::EPSILON);
        assert!((load_5m - 0.31).abs() < f64::EPSILON);
        assert!((load_15m - 0.28).abs() < f64::EPSILON);
        assert_eq!(parse_cpu_count(NPROC_FIXTURE), Some(8));
        assert_eq!(parse_cpu_count("0\n"), None);
        assert_eq!(parse_cpu_count("not-a-count\n"), None);
    }

    #[test]
    fn malformed_load_average_is_unavailable_instead_of_guessed() {
        assert!(parse_loadavg("0.42 0.31\n").is_err());
        assert!(parse_loadavg("nan 0.31 0.28\n").is_err());
        assert!(parse_loadavg("-1 0.31 0.28\n").is_err());
        assert!(parse_loadavg("").is_err());
    }

    #[test]
    fn parses_linux_memory_fixture_with_percentage() {
        let memory = parse_memory(MEMINFO_FIXTURE).expect("memory");
        assert_eq!(memory.total_bytes, 16_316_172 * 1024);
        assert_eq!(memory.available_bytes, 9_247_520 * 1024);
        assert_eq!(
            memory.used_bytes,
            memory.total_bytes - memory.available_bytes
        );
        assert!((memory.used_percent - 43.33).abs() < 0.01);
    }

    #[test]
    fn memory_requires_mem_available_and_consistent_values() {
        let missing = parse_memory("MemTotal: 1024 kB\nMemFree: 512 kB\n")
            .expect_err("MemAvailable is required");
        assert!(missing.contains("MemAvailable"));

        let inconsistent = parse_memory("MemTotal: 1024 kB\nMemAvailable: 2048 kB\n")
            .expect_err("available cannot exceed total");
        assert!(inconsistent.contains("inconsistent"));

        let zero = parse_memory("MemTotal: 0 kB\nMemAvailable: 0 kB\n")
            .expect_err("zero total is not usable");
        assert!(zero.contains("inconsistent"));
    }

    #[test]
    fn parses_filesystem_fixture_with_mounts_and_percentages() {
        let filesystems = parse_filesystems(DF_FIXTURE).expect("filesystems");
        assert_eq!(filesystems.len(), 3);

        let root = &filesystems[0];
        assert_eq!(root.filesystem.as_deref(), Some("/dev/vda1"));
        assert_eq!(root.mount, "/");
        assert_eq!(root.total_bytes, 41_020_672 * 1024);
        assert_eq!(root.used_bytes, 12_345_678 * 1024);
        assert_eq!(root.available_bytes, 26_566_840 * 1024);
        assert!((root.used_percent - 32.0).abs() < f64::EPSILON);

        assert_eq!(filesystems[1].mount, "/mnt/data");
        assert_eq!(filesystems[2].mount, "/run/user/1000");
    }

    #[test]
    fn filesystem_mount_names_with_spaces_are_preserved() {
        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/vdb1 100 40 60 40% /mnt/my data\n";
        let filesystems = parse_filesystems(output).expect("filesystem with spaces");
        assert_eq!(filesystems[0].mount, "/mnt/my data");
    }

    #[test]
    fn invalid_filesystem_rows_are_skipped() {
        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\nnot-a-row\n/dev/vda1 -1 2 3 4% /\n";
        assert!(parse_filesystems(output).is_err());

        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/vda1 100 40 60 40% /\n";
        let filesystems = parse_filesystems(output).expect("valid filesystem");
        assert_eq!(filesystems.len(), 1);
    }

    #[test]
    fn parses_network_interface_counters() {
        let interfaces = parse_network_interfaces(NETDEV_FIXTURE).expect("interfaces");
        assert_eq!(interfaces.len(), 2);

        let eth0 = interfaces
            .iter()
            .find(|interface| interface.interface == "eth0")
            .expect("eth0");
        assert_eq!(eth0.received_bytes, 123_456_789);
        assert_eq!(eth0.received_packets, 98_765);
        assert_eq!(eth0.transmitted_bytes, 987_654_321);
        assert_eq!(eth0.transmitted_packets, 54_321);
    }

    #[test]
    fn malformed_network_rows_are_skipped_without_guessing() {
        let output = "Inter-| Receive | Transmit\n face |bytes packets\neth0: 1 2 3\neth1: 10 20 0 0 0 0 0 0 30 40 0 0 0 0 0 0\n";
        let interfaces = parse_network_interfaces(output).expect("interfaces");
        assert_eq!(interfaces.len(), 1);
        assert_eq!(interfaces[0].interface, "eth1");

        assert!(
            parse_network_interfaces("Inter-| Receive | Transmit\n face |bytes packets\n").is_err()
        );
    }

    #[test]
    fn classifies_missing_sources_as_unsupported() {
        let unsupported = CommandOutput {
            stdout: String::new(),
            stderr: "cat: /proc/net/dev: No such file or directory\n".to_string(),
            exit_code: Some(1),
        };
        assert_eq!(classify_failure(&unsupported), MetricStatus::Unsupported);

        let missing_command = CommandOutput {
            stdout: String::new(),
            stderr: "sh: nproc: not found\n".to_string(),
            exit_code: Some(127),
        };
        assert_eq!(
            classify_failure(&missing_command),
            MetricStatus::Unsupported
        );

        let transient = CommandOutput {
            stdout: String::new(),
            stderr: "Permission denied\n".to_string(),
            exit_code: Some(1),
        };
        assert_eq!(classify_failure(&transient), MetricStatus::Unavailable);
    }

    #[test]
    fn metric_groups_serialize_status_timestamp_and_data() {
        let cpu = MetricGroup::available(
            CpuMetrics {
                load_1m: 0.42,
                load_5m: 0.31,
                load_15m: 0.28,
                logical_cpus: Some(8),
            },
            1_762_665_720_000,
        );
        let value = serde_json::to_value(&cpu).expect("serialized cpu group");
        assert_eq!(value["status"], "available");
        assert_eq!(value["sampledAt"], 1_762_665_720_000_i64);
        assert_eq!(value["data"]["load1m"], 0.42);
        assert_eq!(value["data"]["logicalCpus"], 8);
        assert!(value["detail"].is_null());

        let unavailable: MetricGroup<CpuMetrics> =
            MetricGroup::unavailable("The load average output was incomplete.".to_string(), 7);
        let value = serde_json::to_value(&unavailable).expect("serialized unavailable group");
        assert_eq!(value["status"], "unavailable");
        assert_eq!(value["sampledAt"], 7);
        assert!(value["data"].is_null());
        assert_eq!(value["detail"], "The load average output was incomplete.");

        let unsupported: MetricGroup<MemoryMetrics> = MetricGroup::unsupported(
            "The remote system does not expose /proc/meminfo.".to_string(),
            8,
        );
        let value = serde_json::to_value(&unsupported).expect("serialized unsupported group");
        assert_eq!(value["status"], "unsupported");
        assert!(value["data"].is_null());
    }

    #[test]
    fn monitoring_commands_are_fixed_and_bounded() {
        assert_eq!(
            MONITORING_COMMANDS,
            [
                "cat /proc/loadavg",
                "nproc",
                "cat /proc/meminfo",
                "df -Pk",
                "cat /proc/net/dev"
            ]
        );
        for command in MONITORING_COMMANDS {
            for forbidden in [';', '|', '&', '$', '`', '>', '<', '\n'] {
                assert!(
                    !command.contains(forbidden),
                    "{command:?} must not contain {forbidden:?}"
                );
            }
        }
    }
}
