use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use serverdesk_lib::{
    core::{
        ErrorCode,
        artisan::{ArtisanAction, run_artisan_action},
        projects::normalize_remote_path,
        ssh::{
            Authentication, CommandOutput, ConnectionEventSink, ConnectionManager,
            ConnectionRequest, ConnectionSnapshot, ConnectionTimeouts, SshSession, SshTransport,
            TransportError,
        },
    },
    storage::{Database, NewOperationRecord},
};

struct LocalSession {
    path_env: String,
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
            .env("PATH", &self.path_env)
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
    path_env: String,
    commands: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SshTransport for LocalTransport {
    async fn connect(
        &self,
        _request: &ConnectionRequest,
    ) -> Result<Arc<dyn SshSession>, TransportError> {
        Ok(Arc::new(LocalSession {
            path_env: self.path_env.clone(),
            commands: Arc::clone(&self.commands),
            closed: AtomicBool::new(false),
        }))
    }
}

struct RecordingSink;

impl ConnectionEventSink for RecordingSink {
    fn emit(&self, _snapshot: &ConnectionSnapshot) {}
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directory");
    }
    fs::write(path, contents).expect("write fixture file");
}

fn write_executable(path: &Path, contents: &str) {
    write_file(path, contents);
    let mut permissions = fs::metadata(path).expect("fixture metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fixture executable");
}

fn laravel_fixture(root: &Path) -> PathBuf {
    let project = root.join("storefront");
    write_file(
        &project.join("artisan"),
        "#!/usr/bin/env php\n<?php // fixture artisan\n",
    );
    write_file(
        &project.join("composer.json"),
        r#"{"require":{"php":"^8.3","laravel/framework":"^11.0"}}"#,
    );
    fs::create_dir_all(project.join("bootstrap")).expect("bootstrap directory");
    fs::create_dir_all(project.join("storage/logs")).expect("storage directory");
    write_file(
        &project.join(".env"),
        "APP_NAME=Fixture\nDB_PASSWORD=fixture-secret\nQUEUE_CONNECTION=redis\n",
    );
    fs::canonicalize(project).expect("canonical fixture path")
}

fn fake_php(root: &Path) -> String {
    let bin = root.join("bin");
    write_executable(
        &bin.join("php"),
        r#"#!/bin/sh
case "$*" in
  *"about"*)
    echo "pwd=$(pwd)"
    cat .env
    exit 0
    ;;
  *"route:list"*)
    echo "route output"
    echo "route failure detail" >&2
    exit 3
    ;;
  *"migrate:status"*)
    yes x | head -c 70000
    exit 0
    ;;
  *)
    echo "pwd=$(pwd) args=$*"
    exit 0
    ;;
esac
"#,
    );

    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string())
    )
}

fn manager(root: &Path) -> (Arc<ConnectionManager>, Arc<Mutex<Vec<String>>>) {
    let path_env = fake_php(root);
    let commands = Arc::new(Mutex::new(Vec::new()));
    let manager = Arc::new(ConnectionManager::new(
        Arc::new(LocalTransport {
            path_env,
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

#[tokio::test]
async fn runs_every_action_inside_the_detected_project_directory() {
    let root = tempfile::tempdir().expect("temporary directory");
    let project = laravel_fixture(root.path());
    let project_path = project.to_string_lossy().to_string();
    let (manager, _commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let result = run_artisan_action(
        &manager,
        "fixture-server",
        "project-1",
        &project_path,
        ArtisanAction::About,
        false,
    )
    .await
    .expect("run about");

    assert_eq!(
        result.command,
        format!("cd -- '{project_path}' && php artisan about")
    );
    assert_eq!(
        result.target_path,
        normalize_remote_path(&project_path).expect("path").as_str()
    );
    assert_eq!(result.exit_code, Some(0));
    assert!(result.succeeded);
    assert!(
        result.stdout.contains(&format!("pwd={project_path}")),
        "the command did not run in the project directory: {}",
        result.stdout
    );
    assert!(result.stdout.contains("[redacted environment line]"));
    for secret in ["fixture-secret", "APP_NAME=Fixture", "DB_PASSWORD="] {
        assert!(
            !result.stdout.contains(secret),
            "output exposed .env content {secret:?}: {}",
            result.stdout
        );
    }
    assert!(result.secrets_redacted);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn failed_commands_keep_bounded_output_and_exit_status() {
    let root = tempfile::tempdir().expect("temporary directory");
    let project = laravel_fixture(root.path());
    let (manager, _commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let result = run_artisan_action(
        &manager,
        "fixture-server",
        "project-1",
        &project.to_string_lossy(),
        ArtisanAction::RouteList,
        false,
    )
    .await
    .expect("run route list");

    assert_eq!(result.exit_code, Some(3));
    assert!(!result.succeeded);
    assert!(result.stdout.contains("route output"));
    assert!(result.stderr.contains("route failure detail"));
    assert!(result.duration_ms < 10_000);

    let summary = result.safe_summary();
    assert!(summary.contains("/storefront"));
    assert!(summary.contains("exit code 3"));
    assert!(!summary.contains("route failure detail"));

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn destructive_actions_are_rejected_before_a_command_is_sent() {
    let root = tempfile::tempdir().expect("temporary directory");
    let project = laravel_fixture(root.path());
    let (manager, commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");
    let before = commands.lock().expect("command log").len();

    let error = run_artisan_action(
        &manager,
        "fixture-server",
        "project-1",
        &project.to_string_lossy(),
        ArtisanAction::CacheClear,
        false,
    )
    .await
    .expect_err("cache clear without confirmation must be rejected");

    assert_eq!(error.code, ErrorCode::Validation);
    assert!(error.message.contains("explicit confirmation"));
    assert_eq!(
        commands.lock().expect("command log").len(),
        before,
        "no remote command may be sent before confirmation"
    );

    let confirmed = run_artisan_action(
        &manager,
        "fixture-server",
        "project-1",
        &project.to_string_lossy(),
        ArtisanAction::CacheClear,
        true,
    )
    .await
    .expect("confirmed cache clear");
    assert_eq!(confirmed.exit_code, Some(0));
    let last = commands
        .lock()
        .expect("command log")
        .last()
        .cloned()
        .expect("confirmed command");
    assert_eq!(
        last,
        format!(
            "cd -- '{}' && php artisan cache:clear",
            project.to_string_lossy()
        )
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn queue_restart_runs_with_confirmation_and_uses_the_graceful_command() {
    let root = tempfile::tempdir().expect("temporary directory");
    let project = laravel_fixture(root.path());
    let (manager, _commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let result = run_artisan_action(
        &manager,
        "fixture-server",
        "project-1",
        &project.to_string_lossy(),
        ArtisanAction::QueueRestart,
        true,
    )
    .await
    .expect("run queue restart");

    assert_eq!(
        result.command,
        format!(
            "cd -- '{}' && php artisan queue:restart",
            project.to_string_lossy()
        )
    );
    assert_eq!(result.exit_code, Some(0));
    assert!(serverdesk_lib::core::artisan::QUEUE_RESTART_EXPLANATION.contains("gracefully"));

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn oversized_output_is_rejected_instead_of_unbounded_buffering() {
    let root = tempfile::tempdir().expect("temporary directory");
    let project = laravel_fixture(root.path());
    let (manager, _commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let error = run_artisan_action(
        &manager,
        "fixture-server",
        "project-1",
        &project.to_string_lossy(),
        ArtisanAction::MigrateStatus,
        false,
    )
    .await
    .expect_err("oversized output must not be returned");

    assert_eq!(error.code, ErrorCode::Remote);

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn history_summary_never_contains_env_values() {
    let root = tempfile::tempdir().expect("temporary directory");
    let project = laravel_fixture(root.path());
    let (manager, _commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let result = run_artisan_action(
        &manager,
        "fixture-server",
        "project-1",
        &project.to_string_lossy(),
        ArtisanAction::About,
        false,
    )
    .await
    .expect("run about");
    assert!(result.stdout.contains("[redacted environment line]"));

    let database = Database::in_memory().expect("in-memory database");
    database
        .record_operation(&NewOperationRecord {
            server_id: None,
            project_id: None,
            operation: result.action.operation_name().to_string(),
            safe_summary: result.safe_summary(),
            exit_code: result.exit_code.and_then(|code| i32::try_from(code).ok()),
            started_at: result.started_at,
            duration_ms: i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
        })
        .expect("record history");
    let history = database.list_operations(10).expect("list history");
    assert_eq!(history.len(), 1);
    assert!(history[0].safe_summary.contains("php artisan about"));
    for secret in [
        "fixture-secret",
        "APP_NAME",
        "DB_PASSWORD",
        "QUEUE_CONNECTION",
    ] {
        assert!(
            !history[0].safe_summary.contains(secret),
            "history exposed secret-like .env content {secret:?}"
        );
    }

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}
