use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use serverdesk_lib::core::{
    ErrorCode,
    logs::{
        DEFAULT_LOG_LINES, LogSourceKind, LogTailStatus, MAX_LOG_CONTENT_BYTES, MAX_LOG_LINES,
        read_log_tail,
    },
    ssh::{
        Authentication, CommandOutput, ConnectionEventSink, ConnectionManager, ConnectionRequest,
        ConnectionSnapshot, ConnectionState, ConnectionTimeouts, SshSession, SshTransport,
        TransportError,
    },
};

struct LocalSession {
    commands: Arc<Mutex<Vec<String>>>,
    closed: AtomicBool,
}

#[async_trait]
impl SshSession for LocalSession {
    async fn execute(&self, command: &str) -> Result<CommandOutput, TransportError> {
        self.commands
            .lock()
            .expect("command log")
            .push(command.to_string());

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|error| TransportError::Internal(error.to_string()))?;

        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output
                .status
                .code()
                .and_then(|code| u32::try_from(code).ok()),
        })
    }

    async fn is_healthy(&self) -> bool {
        !self.closed.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct LocalTransport {
    commands: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SshTransport for LocalTransport {
    async fn connect(
        &self,
        _request: &ConnectionRequest,
    ) -> Result<Arc<dyn SshSession>, TransportError> {
        Ok(Arc::new(LocalSession {
            commands: Arc::clone(&self.commands),
            closed: AtomicBool::new(false),
        }))
    }
}

struct ScriptedSession {
    output: CommandOutput,
    commands: Arc<Mutex<Vec<String>>>,
    closed: AtomicBool,
}

#[async_trait]
impl SshSession for ScriptedSession {
    async fn execute(&self, command: &str) -> Result<CommandOutput, TransportError> {
        self.commands
            .lock()
            .expect("command log")
            .push(command.to_string());

        if command == "true" {
            return Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(0),
            });
        }

        Ok(self.output.clone())
    }

    async fn is_healthy(&self) -> bool {
        !self.closed.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct ScriptedTransport {
    output: CommandOutput,
    commands: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SshTransport for ScriptedTransport {
    async fn connect(
        &self,
        _request: &ConnectionRequest,
    ) -> Result<Arc<dyn SshSession>, TransportError> {
        Ok(Arc::new(ScriptedSession {
            output: self.output.clone(),
            commands: Arc::clone(&self.commands),
            closed: AtomicBool::new(false),
        }))
    }
}

struct RecordingSink;

impl ConnectionEventSink for RecordingSink {
    fn emit(&self, _snapshot: &ConnectionSnapshot) {}
}

fn request() -> ConnectionRequest {
    ConnectionRequest {
        server_id: "fixture-server".to_string(),
        host: "127.0.0.1".to_string(),
        port: 22,
        username: "deploy".to_string(),
        authentication: Authentication::Password("fixture-password".to_string()),
        trusted_host_keys: Vec::new(),
    }
}

fn local_manager() -> (Arc<ConnectionManager>, Arc<Mutex<Vec<String>>>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let manager = Arc::new(ConnectionManager::new(
        Arc::new(LocalTransport {
            commands: Arc::clone(&commands),
        }),
        Arc::new(RecordingSink),
        ConnectionTimeouts {
            connect: Duration::from_secs(5),
            command: Duration::from_secs(10),
            close: Duration::from_secs(1),
        },
    ));
    (manager, commands)
}

fn scripted_manager(output: CommandOutput) -> (Arc<ConnectionManager>, Arc<Mutex<Vec<String>>>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let manager = Arc::new(ConnectionManager::new(
        Arc::new(ScriptedTransport {
            output,
            commands: Arc::clone(&commands),
        }),
        Arc::new(RecordingSink),
        ConnectionTimeouts {
            connect: Duration::from_secs(5),
            command: Duration::from_secs(10),
            close: Duration::from_secs(1),
        },
    ));
    (manager, commands)
}

fn write_log(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create log directory");
    }
    fs::write(path, contents).expect("write log fixture");
}

#[tokio::test]
async fn returns_a_bounded_tail_with_truncation_metadata() {
    let root = tempfile::tempdir().expect("temporary directory");
    let log_path = root.path().join("app/storage/logs/laravel.log");
    let mut contents = String::new();
    for index in 0..5000 {
        contents.push_str(&format!(
            "2026-11-09 10:00:00 production.INFO line {index} payload=abcdefghijklmnopqrstuvwxyz\n"
        ));
    }
    write_log(&log_path, &contents);
    let requested = log_path.to_string_lossy().to_string();

    let (manager, commands) = local_manager();
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let tail = read_log_tail(&manager, "fixture-server", &requested, Some(120))
        .await
        .expect("read bounded tail");

    assert_eq!(tail.status, LogTailStatus::Available);
    assert_eq!(tail.source_kind, LogSourceKind::Laravel);
    assert_eq!(tail.requested_path, requested);
    assert_eq!(tail.resolved_path.as_deref(), Some(requested.as_str()));
    assert_eq!(tail.line_limit, 120);
    assert_eq!(tail.byte_limit, MAX_LOG_CONTENT_BYTES);
    assert!(tail.truncated);
    assert!(tail.truncated_bytes);
    assert!(tail.partial_first_line);
    assert!(tail.returned_lines <= 120);
    assert!(tail.returned_bytes <= MAX_LOG_CONTENT_BYTES);
    assert!(
        tail.file_size_bytes
            .is_some_and(|size| size == contents.len() as u64),
        "file size must come from the remote stat output"
    );
    assert!(tail.content.contains("line 4999"));
    assert!(tail.retrieved_at > 0);
    assert!(tail.duration_ms < 10_000);
    assert_eq!(tail.connection_state, ConnectionState::Connected);

    let last_command = commands
        .lock()
        .expect("command log")
        .last()
        .cloned()
        .expect("tail command");
    assert!(last_command.contains("readlink -f --"));
    assert!(last_command.contains("tail -c 65536 --"));
    assert!(last_command.contains(&requested));

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn applies_the_default_line_limit_and_keeps_the_last_lines() {
    let root = tempfile::tempdir().expect("temporary directory");
    let log_path = root.path().join("app/storage/logs/laravel.log");
    let mut contents = String::new();
    for index in 0..(DEFAULT_LOG_LINES + 50) {
        contents.push_str(&format!("line-{index}\n"));
    }
    write_log(&log_path, &contents);
    let requested = log_path.to_string_lossy().to_string();

    let (manager, _commands) = local_manager();
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let tail = read_log_tail(&manager, "fixture-server", &requested, None)
        .await
        .expect("read default tail");

    assert_eq!(tail.status, LogTailStatus::Available);
    assert_eq!(tail.line_limit, DEFAULT_LOG_LINES);
    assert!(tail.truncated_lines);
    assert!(!tail.truncated_bytes);
    assert_eq!(tail.returned_lines, DEFAULT_LOG_LINES);
    assert!(tail.content.starts_with("line-50\n"));
    assert!(
        tail.content
            .contains(&format!("line-{}\n", DEFAULT_LOG_LINES + 49))
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn clamps_oversized_line_requests_to_the_hard_limit() {
    let root = tempfile::tempdir().expect("temporary directory");
    let log_path = root.path().join("app/storage/logs/laravel.log");
    let mut contents = String::new();
    for index in 0..(MAX_LOG_LINES + 25) {
        contents.push_str(&format!("line-{index}\n"));
    }
    write_log(&log_path, &contents);
    let requested = log_path.to_string_lossy().to_string();

    let (manager, _commands) = local_manager();
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let tail = read_log_tail(&manager, "fixture-server", &requested, Some(10_000))
        .await
        .expect("read clamped tail");

    assert_eq!(tail.status, LogTailStatus::Available);
    assert_eq!(tail.line_limit, MAX_LOG_LINES);
    assert_eq!(tail.returned_lines, MAX_LOG_LINES);
    assert!(tail.truncated_lines);
    assert!(!tail.truncated_bytes);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn reports_missing_logs_without_fabricating_content() {
    let root = tempfile::tempdir().expect("temporary directory");
    let missing = root
        .path()
        .join("app/storage/logs/missing.log")
        .to_string_lossy()
        .to_string();

    let (manager, _commands) = local_manager();
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let tail = read_log_tail(&manager, "fixture-server", &missing, Some(100))
        .await
        .expect("a missing file is a typed result");

    assert_eq!(tail.status, LogTailStatus::Missing);
    assert!(tail.content.is_empty());
    assert_eq!(tail.returned_lines, 0);
    assert!(tail.file_size_bytes.is_none());
    assert!(
        tail.detail
            .as_deref()
            .is_some_and(|detail| detail.contains("does not exist"))
    );
    assert!(tail.retrieved_at > 0);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn reports_unreadable_logs_with_clear_permission_guidance() {
    let root = tempfile::tempdir().expect("temporary directory");
    let log_path = root.path().join("app/storage/logs/laravel.log");
    write_log(&log_path, "secret line\n");
    let mut permissions = fs::metadata(&log_path)
        .expect("fixture metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&log_path, permissions).expect("remove read permission");
    let requested = log_path.to_string_lossy().to_string();

    let (manager, _commands) = local_manager();
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let tail = read_log_tail(&manager, "fixture-server", &requested, Some(100))
        .await
        .expect("an unreadable file is a typed result");

    assert_eq!(tail.status, LogTailStatus::Unreadable);
    assert!(tail.content.is_empty());
    assert!(
        tail.detail
            .as_deref()
            .is_some_and(|detail| detail.contains("does not elevate with sudo"))
    );
    assert!(tail.detail.as_deref().is_some_and(|detail| {
        detail.to_ascii_lowercase().contains("permission denied") || detail.contains("cannot read")
    }));

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn rejects_path_traversal_before_any_remote_command_is_sent() {
    let (manager, commands) = local_manager();
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");
    let before = commands.lock().expect("command log").len();

    for rejected in [
        "/etc/passwd",
        "/var/www/app/storage/logs/../../../../etc/passwd",
        "/var/www/app/storage/logs/not-a-log.txt",
        "relative/storage/logs/laravel.log",
    ] {
        let error = read_log_tail(&manager, "fixture-server", rejected, Some(50))
            .await
            .expect_err("unsafe path must be rejected");
        assert_eq!(error.code, ErrorCode::Validation, "{rejected}");
    }

    assert_eq!(
        commands.lock().expect("command log").len(),
        before,
        "path validation must happen before a remote command is sent"
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn scripted_rotation_is_reported_with_the_returned_tail() {
    let output = CommandOutput {
        stdout: "new line 1\nnew line 2\n".to_string(),
        stderr: "__SERVERDESK_LOG_REAL__/var/www/app/storage/logs/laravel.log\n\
                 __SERVERDESK_LOG_BEFORE__8:100:2048:1762665700\n\
                 __SERVERDESK_LOG_AFTER__8:101:64:1762665701\n"
            .to_string(),
        exit_code: Some(0),
    };
    let (manager, _commands) = scripted_manager(output);
    manager
        .connect(request())
        .await
        .expect("connect scripted fixture transport");

    let tail = read_log_tail(
        &manager,
        "fixture-server",
        "/var/www/app/storage/logs/laravel.log",
        Some(100),
    )
    .await
    .expect("a rotated file still returns an honest tail");

    assert_eq!(tail.status, LogTailStatus::Available);
    assert!(tail.rotated);
    assert!(
        tail.rotation_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("rotated"))
    );
    assert_eq!(tail.content, "new line 1\nnew line 2\n");
    assert_eq!(tail.file_size_bytes, Some(2048));

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}
