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
use serverdesk_lib::core::{
    projects::{list_remote_directories, normalize_remote_path, scan_remote_projects},
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
        &project.join("routes/console.php"),
        "<?php // fixture schedule\n",
    );
    write_file(&project.join("config/queue.php"), "<?php return [];\n");
    write_file(
        &project.join(".env"),
        "APP_NAME=Fixture\nQUEUE_CONNECTION=redis\nMAIL_PASSWORD=fixture-secret\n",
    );
    write_file(
        &project.join("vendor/composer/installed.json"),
        r#"{
            "packages": [
                {"name": "laravel/framework", "version": "v11.30.0"},
                {"name": "laravel/horizon", "version": "v5.32.0"}
            ]
        }"#,
    );
    project
}

fn non_laravel_fixture(root: &Path) -> PathBuf {
    let project = root.join("plain-php");
    write_file(
        &project.join("composer.json"),
        r#"{"require":{"php":"^8.3"}}"#,
    );
    write_file(&project.join("index.php"), "<?php echo 'plain';\n");
    project
}

fn manager(fixture_root: &Path) -> (Arc<ConnectionManager>, Arc<Mutex<Vec<String>>>) {
    let bin = fixture_root.join("bin");
    fs::create_dir_all(&bin).expect("fixture bin");
    write_executable(&bin.join("php"), "#!/bin/sh\necho 'PHP 8.3.14 (cli)'\n");
    let laravel = fixture_root.join("storefront");
    write_executable(
        &bin.join("crontab"),
        &format!(
            "#!/bin/sh\necho '* * * * * cd {} && php artisan schedule:run'\n",
            laravel.display()
        ),
    );

    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string())
    );
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
async fn detects_a_real_laravel_fixture_and_rejects_a_non_laravel_directory() {
    let root = tempfile::tempdir().expect("temporary directory");
    let project = laravel_fixture(root.path());
    let plain = non_laravel_fixture(root.path());
    let (manager, _commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let scan = scan_remote_projects(
        &manager,
        "fixture-server",
        &project.to_string_lossy(),
        false,
    )
    .await
    .expect("scan Laravel fixture");

    assert_eq!(
        scan.path,
        normalize_remote_path(&project.to_string_lossy())
            .expect("path")
            .as_str()
    );
    assert_eq!(scan.scanned_paths, 1);
    assert_eq!(scan.rejected_paths, 0);
    assert_eq!(scan.projects.len(), 1);

    let detected = &scan.projects[0];
    assert_eq!(detected.name, "storefront");
    assert_eq!(detected.remote_path, scan.path);
    assert_eq!(detected.project_type, "laravel");
    assert_eq!(detected.laravel_version.as_deref(), Some("v11.30.0"));
    assert_eq!(detected.php_version.as_deref(), Some("8.3.14"));
    assert!(
        detected
            .queue_hint
            .as_deref()
            .is_some_and(|hint| hint.contains("redis") && hint.contains("Horizon"))
    );
    assert!(
        detected
            .scheduler_hint
            .as_deref()
            .is_some_and(|hint| hint.contains("schedule:run"))
    );

    let rejected =
        scan_remote_projects(&manager, "fixture-server", &plain.to_string_lossy(), false)
            .await
            .expect("scan non-Laravel fixture");
    assert!(rejected.projects.is_empty());
    assert_eq!(rejected.scanned_paths, 1);
    assert_eq!(rejected.rejected_paths, 1);
    assert_eq!(rejected.path.as_str(), plain.to_string_lossy().as_ref());

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn browses_only_immediate_child_directories_of_the_selected_path() {
    let root = tempfile::tempdir().expect("temporary directory");
    laravel_fixture(root.path());
    non_laravel_fixture(root.path());
    write_file(&root.path().join("notes.txt"), "not a directory\n");
    fs::create_dir_all(root.path().join("empty/nested")).expect("nested fixture directory");

    let (manager, _commands) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let listing =
        list_remote_directories(&manager, "fixture-server", &root.path().to_string_lossy())
            .await
            .expect("browse fixture root");

    let names = listing
        .directories
        .iter()
        .map(|directory| directory.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["bin", "empty", "plain-php", "storefront"]);
    assert!(!listing.truncated);
    assert_eq!(
        listing.path,
        normalize_remote_path(&root.path().to_string_lossy())
            .expect("path")
            .as_str()
    );

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}

#[tokio::test]
async fn child_scans_stay_inside_the_selected_path_and_find_each_project_once() {
    let root = tempfile::tempdir().expect("temporary directory");
    laravel_fixture(root.path());
    non_laravel_fixture(root.path());
    fs::create_dir_all(root.path().join("empty")).expect("empty child directory");

    let (manager, command_log) = manager(root.path());
    manager
        .connect(request())
        .await
        .expect("connect local fixture transport");

    let selected = normalize_remote_path(&root.path().to_string_lossy()).expect("path");
    let scan = scan_remote_projects(&manager, "fixture-server", selected.as_str(), true)
        .await
        .expect("scan selected children");

    assert_eq!(scan.path, selected.as_str());
    assert_eq!(scan.scanned_paths, 5, "root plus four child directories");
    assert_eq!(scan.rejected_paths, 4);
    assert_eq!(scan.projects.len(), 1);
    assert_eq!(scan.projects[0].name, "storefront");

    let commands = command_log.lock().expect("command log").clone();
    assert!(!commands.is_empty());
    for command in &commands {
        assert!(
            command == "true" || command == "php --version" || command.contains(selected.as_str()),
            "the scan sent a command outside the selected path: {command}"
        );
    }

    manager
        .disconnect("fixture-server")
        .await
        .expect("disconnect");
}
