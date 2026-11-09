use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use serverdesk_lib::core::{
    artisan::{ArtisanAction, QUEUE_RESTART_EXPLANATION, run_artisan_action},
    queue::{
        QUEUE_PROCESS_COMMAND, QUEUE_RESTART_DISTINCTION, QueueWorkerKind, QueueWorkerStatus,
        SUPERVISOR_PROBE_COMMAND, SUPERVISOR_STATUS_COMMAND, SupervisorProgramState,
        SupervisorStatus, discover_queue_supervisor,
    },
    ssh::{
        Authentication, CommandOutput, ConnectionEventSink, ConnectionManager, ConnectionRequest,
        ConnectionSnapshot, ConnectionTimeouts, SshSession, SshTransport, TransportError,
    },
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

        let output = std::process::Command::new("/bin/sh")
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("fixture metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("make fixture executable");
    }
}

fn bin_path(root: &Path) -> String {
    root.join("bin").display().to_string()
}

fn write_ps_fake(root: &Path, stdout: &str) {
    write_executable(
        &root.join("bin/ps"),
        &format!(
            "#!/bin/sh\n/bin/cat <<'SERVERDESK_PS_FIXTURE'\n{stdout}\nSERVERDESK_PS_FIXTURE\n"
        ),
    );
}

fn write_supervisor_fake(root: &Path, contents: &str) {
    write_executable(&root.join("bin/supervisorctl"), contents);
    write_executable(&root.join("bin/supervisord"), "#!/bin/sh\nexit 0\n");
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
    fs::canonicalize(project).expect("canonical fixture path")
}

fn write_php_fake(root: &Path) {
    write_executable(
        &root.join("bin/php"),
        r#"#!/bin/sh
echo "pwd=$(pwd) args=$*"
exit 0
"#,
    );
}

fn manager(root: &Path) -> (Arc<ConnectionManager>, Arc<Mutex<Vec<String>>>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let manager = Arc::new(ConnectionManager::new(
        Arc::new(LocalTransport {
            path_env: bin_path(root),
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
async fn detects_supervisor_programs_and_queue_workers_from_read_only_commands() {
    let root = tempfile::tempdir().expect("temporary directory");
    fs::create_dir_all(root.path().join("bin")).expect("fixture bin");
    write_ps_fake(
        root.path(),
        " 2101 120 php artisan queue:work --queue=high --connection=redis\n\
         2102 300 php artisan horizon\n\
         3100 15 nginx: worker process\n",
    );
    write_supervisor_fake(
        root.path(),
        "#!/bin/sh
if [ \"$1\" = \"status\" ]; then
  /bin/cat <<'SERVERDESK_SUPERVISOR_FIXTURE'
laravel-worker:laravel-worker_00   RUNNING   pid 2101, uptime 0:10:12
laravel-worker:laravel-worker_01   STOPPED   Nov 09 10:00 AM
horizon                            RUNNING   pid 2102, uptime 0:10:10
nightly-report                     FATAL     Exited too quickly
SERVERDESK_SUPERVISOR_FIXTURE
  exit 0
fi
exit 1
",
    );

    let (manager, commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let status = discover_queue_supervisor(&manager, "fixture-server")
        .await
        .expect("discover queue and supervisor");

    assert_eq!(status.supervisor.status, SupervisorStatus::Available);
    assert_eq!(status.supervisor.installed, Some(true));
    assert_eq!(status.supervisor.programs.len(), 4);
    let worker = status
        .supervisor
        .programs
        .iter()
        .find(|program| program.name == "laravel-worker:laravel-worker_00")
        .expect("worker program");
    assert_eq!(worker.state, SupervisorProgramState::Running);
    assert!(worker.queue_related);
    let report = status
        .supervisor
        .programs
        .iter()
        .find(|program| program.name == "nightly-report")
        .expect("unrelated program");
    assert_eq!(report.state, SupervisorProgramState::Fatal);
    assert!(!report.queue_related);

    assert_eq!(status.queue_workers.status, QueueWorkerStatus::Available);
    assert_eq!(status.queue_workers.workers.len(), 2);
    assert_eq!(status.queue_workers.workers[0].pid, 2101);
    assert_eq!(
        status.queue_workers.workers[0].kind,
        QueueWorkerKind::QueueWork
    );
    assert_eq!(
        status.queue_workers.workers[0].queue.as_deref(),
        Some("high")
    );
    assert_eq!(
        status.queue_workers.workers[0].connection.as_deref(),
        Some("redis")
    );
    assert_eq!(
        status.queue_workers.workers[1].kind,
        QueueWorkerKind::Horizon
    );
    assert_eq!(
        status.connection_state,
        serverdesk_lib::core::ssh::ConnectionState::Connected
    );

    let commands = commands
        .lock()
        .expect("command log")
        .iter()
        .filter(|command| *command != "true")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        vec![
            SUPERVISOR_PROBE_COMMAND.to_string(),
            SUPERVISOR_STATUS_COMMAND.to_string(),
            QUEUE_PROCESS_COMMAND.to_string(),
        ]
    );
    for command in &commands {
        for forbidden in ["reread", "update", "/etc/supervisor", "restart supervisor"] {
            assert!(
                !command.contains(forbidden),
                "queue detection must never modify Supervisor: {command}"
            );
        }
    }

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn missing_supervisor_is_reported_as_missing_without_guessed_programs() {
    let root = tempfile::tempdir().expect("temporary directory");
    fs::create_dir_all(root.path().join("bin")).expect("fixture bin");
    write_ps_fake(root.path(), " 3100 15 nginx: worker process\n");

    let (manager, commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let status = discover_queue_supervisor(&manager, "fixture-server")
        .await
        .expect("discover without supervisor");

    assert_eq!(status.supervisor.status, SupervisorStatus::Missing);
    assert_eq!(status.supervisor.installed, Some(false));
    assert!(status.supervisor.programs.is_empty());
    assert!(
        status
            .supervisor
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("not installed"))
    );
    assert_eq!(status.queue_workers.status, QueueWorkerStatus::Available);
    assert!(status.queue_workers.workers.is_empty());
    assert!(
        status.queue_workers.detail.is_none(),
        "an empty successful process list is not an error"
    );

    let commands = commands
        .lock()
        .expect("command log")
        .iter()
        .filter(|command| *command != "true")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        vec![
            SUPERVISOR_PROBE_COMMAND.to_string(),
            QUEUE_PROCESS_COMMAND.to_string(),
        ],
        "supervisorctl status must not run when Supervisor is absent"
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn unreadable_supervisor_status_is_unavailable_with_no_program_names() {
    let root = tempfile::tempdir().expect("temporary directory");
    fs::create_dir_all(root.path().join("bin")).expect("fixture bin");
    write_ps_fake(root.path(), "");
    write_supervisor_fake(
        root.path(),
        "#!/bin/sh
echo 'unix:///var/run/supervisor.sock no such file' >&2
exit 2
",
    );

    let (manager, _commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let status = discover_queue_supervisor(&manager, "fixture-server")
        .await
        .expect("discover with unreadable supervisor");

    assert_eq!(status.supervisor.status, SupervisorStatus::Unavailable);
    assert_eq!(status.supervisor.installed, Some(true));
    assert!(status.supervisor.programs.is_empty());
    let detail = status.supervisor.detail.expect("unavailable detail");
    assert!(detail.contains("Program names are unavailable"));
    assert!(detail.contains("supervisor.sock"));

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn queue_restart_runs_in_the_project_directory_and_is_not_a_supervisor_restart() {
    let root = tempfile::tempdir().expect("temporary directory");
    fs::create_dir_all(root.path().join("bin")).expect("fixture bin");
    write_php_fake(root.path());
    let project = laravel_fixture(root.path());

    let (manager, commands) = manager(root.path());
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

    let expected = format!(
        "cd -- '{}' && php artisan queue:restart",
        project.to_string_lossy()
    );
    assert_eq!(result.command, expected);
    assert!(
        result
            .stdout
            .contains(&format!("pwd={}", project.to_string_lossy()))
    );
    assert_eq!(result.exit_code, Some(0));
    assert!(result.succeeded);

    for forbidden in ["supervisorctl", "systemctl", "service supervisor"] {
        assert!(
            !result.command.contains(forbidden),
            "queue:restart must never restart Supervisor: {}",
            result.command
        );
    }
    assert!(QUEUE_RESTART_EXPLANATION.contains("does not restart Supervisor"));
    assert!(QUEUE_RESTART_DISTINCTION.contains("php artisan queue:restart"));
    assert!(QUEUE_RESTART_DISTINCTION.contains("does not restart Supervisor"));

    let commands = commands.lock().expect("command log").clone();
    assert_eq!(commands.last(), Some(&expected));
    assert!(
        commands
            .iter()
            .all(|command| !command.contains("supervisor") && !command.contains("systemctl"))
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}
