//! Read-only Laravel queue worker and Supervisor detection.
//!
//! Every command in this module is fixed and read-only. Supervisor
//! configuration is never read into ServerDesk and never rewritten. When
//! Supervisor is absent, installed but unreachable, or its status cannot be
//! parsed, the typed result reports `missing` or `unavailable` together with an
//! explicit detail instead of inventing program names or worker states.
//!
//! Queue worker status comes from the remote process list, which is the only
//! generally available source without modifying Supervisor or the project.
//! Supervisor program states are reported separately so the UI can distinguish
//! a Laravel-level queue restart from restarting Supervisor itself.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::{
    AppError, ErrorCode,
    redaction::redact_text,
    ssh::{CommandOutput, ConnectionManager, ConnectionState},
};

/// Hard output bounds for each fixed read-only detection command.
pub const MAX_SUPERVISOR_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_QUEUE_PROCESS_OUTPUT_BYTES: usize = 64 * 1024;
const _: () = assert!(MAX_SUPERVISOR_OUTPUT_BYTES <= 64 * 1024);
const _: () = assert!(MAX_QUEUE_PROCESS_OUTPUT_BYTES <= 64 * 1024);

/// Hard duration bounds applied to each detection command.
pub const QUEUE_SUPERVISOR_TIMEOUT: Duration = Duration::from_secs(10);

/// Fixed, read-only presence probe for the Supervisor binaries.
///
/// The shell only runs `command -v`; no package installation, service control,
/// or configuration file read is attempted.
pub const SUPERVISOR_PROBE_COMMAND: &str = "if command -v supervisorctl >/dev/null 2>&1 || \
     command -v supervisord >/dev/null 2>&1; then echo __installed__; \
     else echo __missing__; fi";

/// The single fixed read-only command used to read Supervisor program names
/// and states. It never starts, stops, restarts, rereads, or updates Supervisor.
pub const SUPERVISOR_STATUS_COMMAND: &str = "supervisorctl status";

/// The fixed, read-only process listing used to find queue workers.
///
/// `pid=` and `etimes=` suppress headers; `args=` prints the command line.
pub const QUEUE_PROCESS_COMMAND: &str = "ps -eo pid=,etimes=,args=";

/// Explanation shared by the backend and shown in the UI. It must always keep
/// the Laravel-level restart distinct from a Supervisor service restart.
pub const QUEUE_RESTART_DISTINCTION: &str = "A Laravel queue restart runs php artisan \
     queue:restart inside the project directory. It signals the application's queue workers to \
     finish their current job and restart gracefully. It does not restart Supervisor, supervisord, \
     or the supervisor.service systemd unit, and ServerDesk never rewrites Supervisor \
     configuration.";

const MAX_PROCESSING_LINES: usize = 4096;
const MAX_PROGRAMS: usize = 256;
const MAX_WORKERS: usize = 256;
const MAX_PROGRAM_NAME_CHARS: usize = 255;
const MAX_STATE_CHARS: usize = 32;
const MAX_DETAIL_CHARS: usize = 240;
const MAX_OPTION_CHARS: usize = 120;

/// Program-name fragments that usually identify Laravel queue workers,
/// schedulers, or Horizon. Matching is used only to highlight programs whose
/// names make them relevant; it never assigns a state or invents a program.
const QUEUE_PROGRAM_NEEDLES: [&str; 6] = [
    "queue",
    "worker",
    "horizon",
    "laravel",
    "artisan",
    "scheduler",
];

/// Whether Supervisor itself is present and its programs could be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorStatus {
    /// Supervisor is installed and `supervisorctl status` succeeded.
    Available,
    /// Neither `supervisorctl` nor `supervisord` was found on the server.
    Missing,
    /// Supervisor may be installed, but its status could not be read safely.
    Unavailable,
}

/// Normalized Supervisor program state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorProgramState {
    Running,
    Stopped,
    Starting,
    Backoff,
    Fatal,
    Exited,
    Unknown,
}

/// One Supervisor program reported by `supervisorctl status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorProgram {
    pub name: String,
    pub state: SupervisorProgramState,
    /// The raw state text reported by Supervisor, bounded and sanitized.
    pub state_text: String,
    /// True when the program name looks queue, worker, Horizon, or scheduler
    /// related. This is a display hint only, not a claim about the program.
    pub queue_related: bool,
    /// Bounded Supervisor detail such as `pid 1234, uptime 0:10:12`.
    pub detail: Option<String>,
}

/// The complete Supervisor detection result.
///
/// `programs` is always empty unless `status` is `available`, so missing data
/// is never rendered as a guessed program list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorDetection {
    pub status: SupervisorStatus,
    /// `Some(true)` when the binaries were found, `Some(false)` when they were
    /// not, and `None` when the presence probe itself failed.
    pub installed: Option<bool>,
    pub programs: Vec<SupervisorProgram>,
    pub detail: Option<String>,
}

/// Whether queue worker processes could be read from the remote system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueWorkerStatus {
    /// The process list was read successfully. An empty list means no matching
    /// worker processes were found at sample time.
    Available,
    /// The process list could not be read or parsed.
    Unavailable,
}

/// The Laravel queue command identified in a worker process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueWorkerKind {
    QueueWork,
    QueueListen,
    Horizon,
}

/// One queue worker process found in the remote process list.
///
/// Only the PID, elapsed time, canonical command kind, and bounded option
/// values are kept. The raw argv is intentionally not stored or returned so
/// process arguments can never leak credentials into the UI or history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorkerProcess {
    pub pid: u32,
    pub elapsed_seconds: u64,
    pub kind: QueueWorkerKind,
    pub queue: Option<String>,
    pub connection: Option<String>,
}

/// Queue worker processes read from the remote process list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueWorkerDetection {
    pub status: QueueWorkerStatus,
    pub workers: Vec<QueueWorkerProcess>,
    pub truncated: bool,
    pub detail: Option<String>,
}

/// The complete typed queue and Supervisor snapshot for one server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSupervisorStatus {
    pub server_id: String,
    pub supervisor: SupervisorDetection,
    pub queue_workers: QueueWorkerDetection,
    pub sampled_at: i64,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

/// Discovers Supervisor presence, program names, and queue worker processes.
///
/// Only fixed read-only commands are sent. Command-level failures become typed
/// `missing` or `unavailable` results with a bounded detail; transport and
/// connection failures remain [`AppError`] values.
pub async fn discover_queue_supervisor(
    manager: &ConnectionManager,
    server_id: &str,
) -> Result<QueueSupervisorStatus, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let started = Instant::now();
    let supervisor = detect_supervisor(manager, server_id).await?;
    let queue_workers = detect_queue_workers(manager, server_id).await?;
    let sampled_at = now_millis();
    let connection_state = manager.connection_state(server_id)?.state;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(QueueSupervisorStatus {
        server_id: server_id.to_string(),
        supervisor,
        queue_workers,
        sampled_at,
        duration_ms,
        connection_state,
    })
}

async fn detect_supervisor(
    manager: &ConnectionManager,
    server_id: &str,
) -> Result<SupervisorDetection, AppError> {
    let probe = match run_bounded(
        manager,
        server_id,
        SUPERVISOR_PROBE_COMMAND,
        MAX_SUPERVISOR_OUTPUT_BYTES,
        "Supervisor detection",
    )
    .await
    {
        Ok(output) => output,
        Err(error) if error.code == ErrorCode::Timeout => {
            return Ok(supervisor_unavailable(
                None,
                format!(
                    "Supervisor presence could not be checked within {} seconds.",
                    QUEUE_SUPERVISOR_TIMEOUT.as_secs()
                ),
            ));
        }
        Err(error) if is_unavailable_detection_error(&error) => {
            return Ok(supervisor_unavailable(
                None,
                format!(
                    "Supervisor presence could not be checked: {}",
                    bounded_error_message(&error.message)
                ),
            ));
        }
        Err(error) => return Err(error),
    };

    if probe.exit_code != Some(0) {
        return Ok(supervisor_unavailable(
            None,
            format!(
                "Supervisor presence could not be checked ({}).",
                exit_suffix(&probe)
            ),
        ));
    }

    if probe
        .stdout
        .lines()
        .any(|line| line.trim() == "__missing__")
    {
        return Ok(SupervisorDetection {
            status: SupervisorStatus::Missing,
            installed: Some(false),
            programs: Vec::new(),
            detail: Some(
                "Supervisor is not installed: supervisorctl and supervisord were not found."
                    .to_string(),
            ),
        });
    }

    if !probe
        .stdout
        .lines()
        .any(|line| line.trim() == "__installed__")
    {
        return Ok(supervisor_unavailable(
            None,
            "Supervisor presence could not be determined from the remote probe.".to_string(),
        ));
    }

    let status = match run_bounded(
        manager,
        server_id,
        SUPERVISOR_STATUS_COMMAND,
        MAX_SUPERVISOR_OUTPUT_BYTES,
        "Supervisor status",
    )
    .await
    {
        Ok(output) => output,
        Err(error) if error.code == ErrorCode::Timeout => {
            return Ok(supervisor_unavailable(
                Some(true),
                format!(
                    "Supervisor is installed, but program status did not return within {} seconds. Program names are unavailable.",
                    QUEUE_SUPERVISOR_TIMEOUT.as_secs()
                ),
            ));
        }
        Err(error) if is_unavailable_detection_error(&error) => {
            return Ok(supervisor_unavailable(
                Some(true),
                format!(
                    "Supervisor is installed, but program status could not be read: {}. Program names are unavailable.",
                    bounded_error_message(&error.message)
                ),
            ));
        }
        Err(error) => return Err(error),
    };

    if status.exit_code != Some(0) {
        return Ok(supervisor_unavailable(
            Some(true),
            format!(
                "Supervisor is installed, but program status could not be read ({}). Program names are unavailable.",
                failure_detail(&status)
            ),
        ));
    }

    let parsed = parse_supervisor_status(&status.stdout);
    let detail = if parsed.truncated {
        Some(
            "The Supervisor program list was truncated at the ServerDesk safety limit.".to_string(),
        )
    } else {
        None
    };

    Ok(SupervisorDetection {
        status: SupervisorStatus::Available,
        installed: Some(true),
        programs: parsed.programs,
        detail,
    })
}

async fn detect_queue_workers(
    manager: &ConnectionManager,
    server_id: &str,
) -> Result<QueueWorkerDetection, AppError> {
    let output = match run_bounded(
        manager,
        server_id,
        QUEUE_PROCESS_COMMAND,
        MAX_QUEUE_PROCESS_OUTPUT_BYTES,
        "Queue worker detection",
    )
    .await
    {
        Ok(output) => output,
        Err(error) if error.code == ErrorCode::Timeout => {
            return Ok(queue_workers_unavailable(format!(
                "Queue worker processes did not return within {} seconds.",
                QUEUE_SUPERVISOR_TIMEOUT.as_secs()
            )));
        }
        Err(error) if is_unavailable_detection_error(&error) => {
            return Ok(queue_workers_unavailable(format!(
                "Queue worker processes could not be read: {}.",
                bounded_error_message(&error.message)
            )));
        }
        Err(error) => return Err(error),
    };

    if output.exit_code != Some(0) {
        return Ok(queue_workers_unavailable(format!(
            "Queue worker processes could not be read ({}).",
            failure_detail(&output)
        )));
    }

    let parsed = parse_queue_workers(&output.stdout);
    Ok(QueueWorkerDetection {
        status: QueueWorkerStatus::Available,
        workers: parsed.workers,
        truncated: parsed.truncated,
        detail: if parsed.truncated {
            Some(
                "The queue worker process list was truncated at the ServerDesk safety limit."
                    .to_string(),
            )
        } else {
            None
        },
    })
}

/// Command-level failures that should become typed missing data instead of
/// aborting the whole read-only snapshot. Connection, authentication, and
/// transport health failures still propagate as errors.
fn is_unavailable_detection_error(error: &AppError) -> bool {
    matches!(error.code, ErrorCode::Remote | ErrorCode::Permission)
}

fn bounded_error_message(message: &str) -> String {
    redact_text(message)
        .text
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .take(MAX_DETAIL_CHARS)
        .collect()
}

fn supervisor_unavailable(installed: Option<bool>, detail: String) -> SupervisorDetection {
    SupervisorDetection {
        status: SupervisorStatus::Unavailable,
        installed,
        programs: Vec::new(),
        detail: Some(detail),
    }
}

fn queue_workers_unavailable(detail: String) -> QueueWorkerDetection {
    QueueWorkerDetection {
        status: QueueWorkerStatus::Unavailable,
        workers: Vec::new(),
        truncated: false,
        detail: Some(detail),
    }
}

#[derive(Debug, Default)]
struct ParsedPrograms {
    programs: Vec<SupervisorProgram>,
    truncated: bool,
}

fn parse_supervisor_status(stdout: &str) -> ParsedPrograms {
    let mut parsed = ParsedPrograms::default();

    for (index, line) in stdout.lines().enumerate() {
        if index >= MAX_PROCESSING_LINES {
            parsed.truncated = true;
            break;
        }

        let Some(program) = parse_supervisor_program_line(line) else {
            continue;
        };

        if parsed.programs.len() >= MAX_PROGRAMS {
            parsed.truncated = true;
            break;
        }

        parsed.programs.push(program);
    }

    parsed
}

fn parse_supervisor_program_line(line: &str) -> Option<SupervisorProgram> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // `supervisorctl` can print human notices such as "No programs present."
    // These are not program rows.
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("no program") || lower.starts_with("supervisorctl") {
        return None;
    }

    let mut fields = line.split_whitespace();
    let name = sanitize_program_name(fields.next()?)?;
    let state_text = bounded_token(fields.next()?, MAX_STATE_CHARS)?;
    let detail = bounded_detail(&fields.collect::<Vec<_>>().join(" "));
    let state = classify_program_state(&state_text);

    Some(SupervisorProgram {
        queue_related: is_queue_related_program(&name),
        name,
        state,
        state_text,
        detail,
    })
}

fn sanitize_program_name(value: &str) -> Option<String> {
    let name = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_PROGRAM_NAME_CHARS)
        .collect::<String>();
    if name.is_empty() || name.len() > MAX_PROGRAM_NAME_CHARS {
        return None;
    }
    Some(name)
}

fn classify_program_state(state_text: &str) -> SupervisorProgramState {
    match state_text.to_ascii_uppercase().as_str() {
        "RUNNING" => SupervisorProgramState::Running,
        "STOPPED" => SupervisorProgramState::Stopped,
        "STARTING" => SupervisorProgramState::Starting,
        "BACKOFF" => SupervisorProgramState::Backoff,
        "FATAL" => SupervisorProgramState::Fatal,
        "EXITED" => SupervisorProgramState::Exited,
        _ => SupervisorProgramState::Unknown,
    }
}

fn is_queue_related_program(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    QUEUE_PROGRAM_NEEDLES
        .iter()
        .any(|needle| name.contains(needle))
}

#[derive(Debug, Default)]
struct ParsedWorkers {
    workers: Vec<QueueWorkerProcess>,
    truncated: bool,
}

fn parse_queue_workers(stdout: &str) -> ParsedWorkers {
    let mut parsed = ParsedWorkers::default();

    for (index, line) in stdout.lines().enumerate() {
        if index >= MAX_PROCESSING_LINES {
            parsed.truncated = true;
            break;
        }

        let Some(worker) = parse_queue_worker_line(line) else {
            continue;
        };

        if parsed.workers.len() >= MAX_WORKERS {
            parsed.truncated = true;
            break;
        }

        parsed.workers.push(worker);
    }

    parsed.workers.sort_by_key(|worker| worker.pid);
    parsed
}

fn parse_queue_worker_line(line: &str) -> Option<QueueWorkerProcess> {
    let mut fields = line.split_whitespace();
    let pid = fields.next()?.parse::<u32>().ok()?;
    let elapsed_seconds = fields.next()?.parse::<u64>().ok()?;
    let args = fields.collect::<Vec<_>>().join(" ");
    if args.is_empty() {
        return None;
    }

    let lower = args.to_ascii_lowercase();
    if !lower.contains("artisan") {
        return None;
    }

    let kind = if lower.contains("artisan horizon") {
        QueueWorkerKind::Horizon
    } else if lower.contains("queue:listen") {
        QueueWorkerKind::QueueListen
    } else if lower.contains("queue:work") {
        QueueWorkerKind::QueueWork
    } else {
        return None;
    };

    Some(QueueWorkerProcess {
        pid,
        elapsed_seconds,
        kind,
        queue: argument_value(&args, "queue"),
        connection: argument_value(&args, "connection"),
    })
}

/// Reads a bounded `--name=value` or `--name value` option from an argv string.
fn argument_value(args: &str, name: &str) -> Option<String> {
    let flag = format!("--{name}");
    let equals_prefix = format!("--{name}=");
    let mut tokens = args.split_whitespace().peekable();

    while let Some(token) = tokens.next() {
        let value = if let Some(value) = token.strip_prefix(&equals_prefix) {
            value
        } else if token == flag {
            let Some(value) = tokens.next() else {
                break;
            };
            value
        } else {
            continue;
        };

        let value = value
            .trim_matches(|character| matches!(character, '"' | '\''))
            .chars()
            .filter(|character| !character.is_control() && !character.is_whitespace())
            .take(MAX_OPTION_CHARS)
            .collect::<String>();
        if !value.is_empty() {
            return Some(value);
        }
    }

    None
}

fn bounded_token(value: &str, max_chars: usize) -> Option<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect::<String>();
    if value.is_empty() { None } else { Some(value) }
}

fn bounded_detail(value: &str) -> Option<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .take(MAX_DETAIL_CHARS)
        .collect::<String>();
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn failure_detail(output: &CommandOutput) -> String {
    let stderr = output
        .stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| {
            redact_text(line)
                .text
                .chars()
                .filter(|character| !character.is_control() || *character == '\t')
                .take(MAX_DETAIL_CHARS)
                .collect::<String>()
        })
        .unwrap_or_default();

    if stderr.is_empty() {
        exit_suffix(output)
    } else {
        format!("{}: {}", exit_suffix(output), stderr.trim())
    }
}

fn exit_suffix(output: &CommandOutput) -> String {
    match output.exit_code {
        Some(code) => format!("exit code {code}"),
        None => "no exit status".to_string(),
    }
}

async fn run_bounded(
    manager: &ConnectionManager,
    server_id: &str,
    command: &str,
    max_output_bytes: usize,
    label: &str,
) -> Result<CommandOutput, AppError> {
    tokio::time::timeout(
        QUEUE_SUPERVISOR_TIMEOUT,
        manager.execute_bounded(server_id, command, max_output_bytes),
    )
    .await
    .map_err(|_| {
        AppError::timeout(format!(
            "{label} timed out after {} seconds.",
            QUEUE_SUPERVISOR_TIMEOUT.as_secs()
        ))
    })?
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

    const SUPERVISOR_FIXTURE: &str = "\
laravel-worker:laravel-worker_00   RUNNING   pid 2101, uptime 0:10:12
laravel-worker:laravel-worker_01   STOPPED   Nov 09 10:00 AM
horizon                            RUNNING   pid 2102, uptime 0:10:10
nightly-report                     FATAL     Exited too quickly (process log may have details)
custom-backup                      BACKOFF   Exited too quickly
odd-job                            WEIRD     unexpected state
";

    #[test]
    fn parses_supervisor_programs_and_marks_queue_related_names() {
        let parsed = parse_supervisor_status(SUPERVISOR_FIXTURE);
        assert!(!parsed.truncated);
        assert_eq!(parsed.programs.len(), 6);

        let worker = parsed
            .programs
            .iter()
            .find(|program| program.name == "laravel-worker:laravel-worker_00")
            .expect("worker program");
        assert_eq!(worker.state, SupervisorProgramState::Running);
        assert_eq!(worker.state_text, "RUNNING");
        assert!(worker.queue_related);
        assert_eq!(worker.detail.as_deref(), Some("pid 2101, uptime 0:10:12"));

        let stopped = parsed
            .programs
            .iter()
            .find(|program| program.name == "laravel-worker:laravel-worker_01")
            .expect("stopped program");
        assert_eq!(stopped.state, SupervisorProgramState::Stopped);

        let horizon = parsed
            .programs
            .iter()
            .find(|program| program.name == "horizon")
            .expect("horizon program");
        assert!(horizon.queue_related);

        let report = parsed
            .programs
            .iter()
            .find(|program| program.name == "nightly-report")
            .expect("fatal program");
        assert_eq!(report.state, SupervisorProgramState::Fatal);
        assert!(!report.queue_related);

        let backup = parsed
            .programs
            .iter()
            .find(|program| program.name == "custom-backup")
            .expect("backoff program");
        assert_eq!(backup.state, SupervisorProgramState::Backoff);

        let odd = parsed
            .programs
            .iter()
            .find(|program| program.name == "odd-job")
            .expect("unknown program");
        assert_eq!(odd.state, SupervisorProgramState::Unknown);
        assert_eq!(odd.state_text, "WEIRD");
    }

    #[test]
    fn skips_notices_and_malformed_supervisor_rows() {
        let parsed = parse_supervisor_status(
            "No programs present.\n\
             supervisorctl: warning\n\
             \n\
             only-one-field\n\
             valid-program RUNNING pid 1, uptime 0:00:01\n",
        );
        assert_eq!(parsed.programs.len(), 1);
        assert_eq!(parsed.programs[0].name, "valid-program");
    }

    #[test]
    fn parses_queue_workers_without_returning_raw_argv() {
        let parsed = parse_queue_workers(
            " 2101 120 php artisan queue:work --queue=high,default --connection=redis --token=super-secret\n\
             2102 300 php artisan horizon\n\
             2103 45 php artisan queue:listen --queue=low\n\
             2104 10 /usr/bin/redis-server *:6379\n\
             2105 11 php artisan schedule:work\n\
             not-a-pid 12 php artisan queue:work\n",
        );

        assert!(!parsed.truncated);
        assert_eq!(parsed.workers.len(), 3);

        let work = &parsed.workers[0];
        assert_eq!(work.pid, 2101);
        assert_eq!(work.elapsed_seconds, 120);
        assert_eq!(work.kind, QueueWorkerKind::QueueWork);
        assert_eq!(work.queue.as_deref(), Some("high,default"));
        assert_eq!(work.connection.as_deref(), Some("redis"));

        let horizon = &parsed.workers[1];
        assert_eq!(horizon.kind, QueueWorkerKind::Horizon);
        assert_eq!(horizon.queue, None);
        assert_eq!(horizon.connection, None);

        let listen = &parsed.workers[2];
        assert_eq!(listen.kind, QueueWorkerKind::QueueListen);
        assert_eq!(listen.queue.as_deref(), Some("low"));

        let serialized = serde_json::to_string(&parsed.workers).expect("serialized workers");
        assert!(!serialized.contains("super-secret"));
        assert!(!serialized.contains("--token"));
        assert!(!serialized.contains("redis-server"));
    }

    #[test]
    fn recognizes_space_separated_option_values() {
        assert_eq!(
            argument_value(
                "php artisan queue:work --queue default --connection redis",
                "queue"
            )
            .as_deref(),
            Some("default")
        );
        assert_eq!(
            argument_value("php artisan queue:work --queue='high'", "queue").as_deref(),
            Some("high")
        );
        assert_eq!(argument_value("php artisan queue:work", "queue"), None);
    }

    #[test]
    fn no_matching_workers_is_a_successful_empty_sample() {
        let parsed = parse_queue_workers(
            "1 10 /usr/sbin/sshd\n2 22 nginx: worker process\n3 30 php-fpm: pool www\n",
        );
        assert!(!parsed.truncated);
        assert!(parsed.workers.is_empty());

        let detection = QueueWorkerDetection {
            status: QueueWorkerStatus::Available,
            workers: parsed.workers,
            truncated: parsed.truncated,
            detail: None,
        };
        let value = serde_json::to_value(&detection).expect("serialized detection");
        assert_eq!(value["status"], "available");
        assert_eq!(value["workers"], serde_json::json!([]));
        assert_eq!(value["truncated"], false);
    }

    #[test]
    fn bounds_program_and_worker_lists() {
        let programs = (0..MAX_PROGRAMS + 3)
            .map(|index| format!("program-{index} RUNNING pid {index}, uptime 0:00:01\n"))
            .collect::<String>();
        let parsed = parse_supervisor_status(&programs);
        assert_eq!(parsed.programs.len(), MAX_PROGRAMS);
        assert!(parsed.truncated);

        let workers = (0..MAX_WORKERS + 3)
            .map(|index| format!("{index} 10 php artisan queue:work --queue=default\n"))
            .collect::<String>();
        let parsed = parse_queue_workers(&workers);
        assert_eq!(parsed.workers.len(), MAX_WORKERS);
        assert!(parsed.truncated);

        let too_many_lines = (0..MAX_PROCESSING_LINES + 1)
            .map(|index| format!("program-{index} RUNNING\n"))
            .collect::<String>();
        let parsed = parse_supervisor_status(&too_many_lines);
        assert!(parsed.truncated);
    }

    #[test]
    fn unavailable_supervisor_results_never_include_guessed_programs() {
        let unavailable = supervisor_unavailable(
            Some(true),
            "Supervisor is installed, but program status could not be read.".to_string(),
        );
        assert_eq!(unavailable.status, SupervisorStatus::Unavailable);
        assert_eq!(unavailable.installed, Some(true));
        assert!(unavailable.programs.is_empty());
        assert!(
            unavailable
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("could not be read"))
        );

        let missing = SupervisorDetection {
            status: SupervisorStatus::Missing,
            installed: Some(false),
            programs: Vec::new(),
            detail: Some("Supervisor is not installed.".to_string()),
        };
        let value = serde_json::to_value(&missing).expect("serialized detection");
        assert_eq!(value["status"], "missing");
        assert_eq!(value["installed"], false);
        assert_eq!(value["programs"], serde_json::json!([]));
    }

    #[test]
    fn serializes_the_typed_queue_and_supervisor_snapshot() {
        let status = QueueSupervisorStatus {
            server_id: "server-1".to_string(),
            supervisor: SupervisorDetection {
                status: SupervisorStatus::Available,
                installed: Some(true),
                programs: vec![SupervisorProgram {
                    name: "laravel-worker:laravel-worker_00".to_string(),
                    state: SupervisorProgramState::Running,
                    state_text: "RUNNING".to_string(),
                    queue_related: true,
                    detail: Some("pid 2101, uptime 0:10:12".to_string()),
                }],
                detail: None,
            },
            queue_workers: QueueWorkerDetection {
                status: QueueWorkerStatus::Available,
                workers: vec![QueueWorkerProcess {
                    pid: 2101,
                    elapsed_seconds: 120,
                    kind: QueueWorkerKind::QueueWork,
                    queue: Some("default".to_string()),
                    connection: Some("redis".to_string()),
                }],
                truncated: false,
                detail: None,
            },
            sampled_at: 1_762_665_720_000,
            duration_ms: 42,
            connection_state: ConnectionState::Connected,
        };

        let value = serde_json::to_value(&status).expect("serialized queue status");
        assert_eq!(value["serverId"], "server-1");
        assert_eq!(value["supervisor"]["status"], "available");
        assert_eq!(value["supervisor"]["installed"], true);
        assert_eq!(
            value["supervisor"]["programs"][0]["name"],
            "laravel-worker:laravel-worker_00"
        );
        assert_eq!(value["supervisor"]["programs"][0]["state"], "running");
        assert_eq!(value["supervisor"]["programs"][0]["stateText"], "RUNNING");
        assert_eq!(value["supervisor"]["programs"][0]["queueRelated"], true);
        assert_eq!(value["queueWorkers"]["status"], "available");
        assert_eq!(value["queueWorkers"]["workers"][0]["pid"], 2101);
        assert_eq!(value["queueWorkers"]["workers"][0]["kind"], "queue_work");
        assert_eq!(value["queueWorkers"]["workers"][0]["queue"], "default");
        assert_eq!(value["queueWorkers"]["workers"][0]["connection"], "redis");
        assert_eq!(value["sampledAt"], 1_762_665_720_000_i64);
        assert_eq!(value["connectionState"], "connected");
    }

    #[test]
    fn commands_are_fixed_and_read_only() {
        assert!(SUPERVISOR_PROBE_COMMAND.contains("command -v supervisorctl"));
        assert!(SUPERVISOR_PROBE_COMMAND.contains("command -v supervisord"));
        assert_eq!(SUPERVISOR_STATUS_COMMAND, "supervisorctl status");
        assert_eq!(QUEUE_PROCESS_COMMAND, "ps -eo pid=,etimes=,args=");

        for command in [
            SUPERVISOR_PROBE_COMMAND,
            SUPERVISOR_STATUS_COMMAND,
            QUEUE_PROCESS_COMMAND,
        ] {
            for forbidden in [
                "reread",
                "update",
                "restart",
                "start ",
                "stop ",
                ">>",
                "tee",
                "sed -i",
                "/etc/supervisor",
            ] {
                assert!(
                    !command.contains(forbidden),
                    "{command:?} must not contain {forbidden:?}"
                );
            }
        }
        assert!(QUEUE_SUPERVISOR_TIMEOUT <= Duration::from_secs(10));
    }

    #[test]
    fn queue_restart_difference_is_explicitly_documented() {
        assert!(QUEUE_RESTART_DISTINCTION.contains("php artisan queue:restart"));
        assert!(QUEUE_RESTART_DISTINCTION.contains("does not restart Supervisor"));
        assert!(QUEUE_RESTART_DISTINCTION.contains("never rewrites Supervisor configuration"));
    }
}
