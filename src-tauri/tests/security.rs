//! Security suite for issue #32.
//!
//! These tests exercise every user-controlled field that can reach a remote
//! command, SFTP request, local path, or diagnostic boundary. They fail closed:
//! rejected input must not build or send a command.

use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use serverdesk_lib::{
    core::{
        AppError,
        artisan::ArtisanAction,
        command::{
            artisan_command, browse_directories_command, laravel_package_command, log_tail_command,
            project_probe_command, queue_connection_command, scheduler_cron_command,
        },
        files::{
            DownloadRequest, FileTransferEventSink, FileTransferManager, FileTransferProgress,
            UploadRequest, browse_remote_files, create_remote_folder, delete_remote_entry,
            normalize_local_path, rename_remote_entry, validate_browse_paths,
        },
        history::record_operation,
        logs::normalize_log_path,
        projects::normalize_remote_path,
        redaction::{redact_text, sanitize_text_bounded},
        remote::RemoteCommand,
        servers::validate_connection_fields,
        services::{ServiceAction, validate_service_unit_name},
        ssh::{
            ConnectionEventSink, ConnectionManager, ConnectionRequest, ConnectionSnapshot,
            ConnectionTimeouts, SshSession, SshTransport, TransportError,
        },
        terminal::TerminalOptions,
    },
    storage::{
        Database, NewOperationRecord, OperationCategory, OperationHistoryFilter, OperationResult,
    },
};

const SECRETS: [&str; 6] = [
    "hunter2",
    "supersecret-bearer",
    "sk-live-abcdefghijklmnop",
    "ghp_1234567890abcdef",
    "private-key-body-material",
    "url-password-secret",
];

struct NoSessionTransport;

#[async_trait]
impl SshTransport for NoSessionTransport {
    async fn connect(
        &self,
        _request: &ConnectionRequest,
    ) -> Result<Arc<dyn SshSession>, TransportError> {
        Err(TransportError::Network)
    }
}

struct RecordingSink;

impl ConnectionEventSink for RecordingSink {
    fn emit(&self, _snapshot: &ConnectionSnapshot) {}
}

struct NoopTransferSink;

impl FileTransferEventSink for NoopTransferSink {
    fn emit(&self, _progress: &FileTransferProgress) {}
}

fn manager() -> Arc<ConnectionManager> {
    Arc::new(ConnectionManager::new(
        Arc::new(NoSessionTransport),
        Arc::new(RecordingSink),
        ConnectionTimeouts {
            connect: Duration::from_secs(1),
            command: Duration::from_secs(1),
            close: Duration::from_secs(1),
        },
    ))
}

fn hostile_path(marker: &Path) -> String {
    format!(
        "/tmp/serverdesk-fixture; touch {} $(touch {}) `touch {}` | cat",
        marker.display(),
        marker.display(),
        marker.display()
    )
}

#[test]
fn dynamic_command_builders_quote_every_metacharacter() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let marker = temp.path().join("injected");

    for command in [
        browse_directories_command(&hostile_path(&marker)),
        project_probe_command(&hostile_path(&marker)),
        laravel_package_command(&hostile_path(&marker)),
        queue_connection_command(&hostile_path(&marker)),
        scheduler_cron_command(&hostile_path(&marker)),
        artisan_command(&hostile_path(&marker), "about"),
        log_tail_command(&hostile_path(&marker), 1024),
    ] {
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .expect("run generated command");
    }

    assert!(
        !marker.exists(),
        "a generated command executed untrusted shell input"
    );
}

#[test]
fn path_and_unit_validators_reject_injection_payloads() {
    for payload in [
        "relative/path",
        "/srv/../etc/passwd",
        "/srv/app\nrm -rf /",
        "/srv/app\0hidden",
        "/srv/app; touch /tmp/pwn",
    ] {
        if payload.starts_with('/') && !payload.contains('\0') && !payload.contains('\n') {
            // A validated absolute path may contain shell metacharacters; the
            // command builder quotes it. Traversal and control characters must
            // still be rejected.
            if payload.contains("..") {
                assert!(normalize_remote_path(payload).is_err(), "{payload:?}");
            }
        } else {
            assert!(normalize_remote_path(payload).is_err(), "{payload:?}");
        }
    }

    assert!(normalize_remote_path("/srv/app; touch /tmp/pwn").is_ok());
    assert!(validate_browse_paths("/srv/app", "/srv/app/../etc").is_err());
    assert!(validate_browse_paths("/srv/app", "/srv/other").is_err());
    assert!(validate_browse_paths("/srv/app", "/srv/app/public").is_ok());

    for payload in [
        "nginx",
        "nginx.socket",
        "nginx.service; id",
        "nginx.service && reboot",
        "nginx.service|cat",
        "nginx.service$(id)",
        "nginx.service`id`",
        "../nginx.service",
        "/usr/lib/systemd/system/nginx.service",
        "-H.service",
        ".hidden.service",
        "nginx service.service",
        "nïginx.service",
    ] {
        assert!(
            validate_service_unit_name(payload).is_err(),
            "{payload:?} must not become a systemd command"
        );
    }
    assert!(validate_service_unit_name("nginx.service").is_ok());

    for payload in [
        "/etc/passwd",
        "/var/log/../etc/passwd",
        "/var/www/app/logs/laravel.log",
        "/var/www/app/storage/logs/laravel.txt",
        "/var/www/app/storage/logs/.hidden.log",
        "relative/storage/logs/laravel.log",
    ] {
        assert!(normalize_log_path(payload).is_err(), "{payload:?}");
    }
    assert!(
        normalize_log_path("/var/www/app/storage/logs/laravel.log").is_ok(),
        "approved Laravel log path must remain usable"
    );
}

#[test]
fn typed_actions_and_remote_commands_are_closed_sets() {
    for payload in [
        "\"migrate\"",
        "\"db:wipe\"",
        "\"about; rm -rf /\"",
        "\"route:list && id\"",
        "\"\"",
        "null",
    ] {
        assert!(serde_json::from_str::<ArtisanAction>(payload).is_err());
    }
    assert!(serde_json::from_str::<ArtisanAction>("\"about\"").is_ok());

    for payload in [
        "\"reload\"",
        "\"start; id\"",
        "\"rm -rf /\"",
        "\"\"",
        "null",
    ] {
        assert!(serde_json::from_str::<ServiceAction>(payload).is_err());
    }
    assert!(serde_json::from_str::<ServiceAction>("\"restart\"").is_ok());

    for payload in ["\"rm -rf /\"", "\"hostname; id\"", "\"\"", "null"] {
        assert!(serde_json::from_str::<RemoteCommand>(payload).is_err());
    }
    assert!(serde_json::from_str::<RemoteCommand>("\"hostname\"").is_ok());
}

#[test]
fn connection_and_terminal_fields_reject_shell_metacharacters() {
    for host in [
        "example.test; id",
        "example.test$(id)",
        "example.test`id`",
        "example.test|cat",
        "example.test\nignored",
        "user@example.test",
    ] {
        assert!(validate_connection_fields(host, 22, "deploy").is_err());
    }
    for username in [
        "deploy; id",
        "deploy$(id)",
        "deploy`id`",
        "deploy|cat",
        "deploy\nignored",
    ] {
        assert!(validate_connection_fields("198.51.100.24", 22, username).is_err());
    }
    assert!(validate_connection_fields("2001:db8::1", 22, "deploy").is_ok());

    for term in [
        "xterm; id",
        "xterm$(id)",
        "xterm`id`",
        "xterm|cat",
        "xterm\nignored",
        "xterm/../../bin/sh",
    ] {
        assert!(TerminalOptions::new(Some(term.to_string()), None).is_err());
    }
    assert!(TerminalOptions::new(Some("xterm-256color".to_string()), None).is_ok());
}

#[tokio::test]
async fn destructive_file_actions_fail_closed_before_any_sftp_session() {
    let manager = manager();

    let error = rename_remote_entry(
        manager.as_ref(),
        "server-1",
        "/srv/app",
        "/srv/app/file.txt",
        "renamed.txt",
        false,
    )
    .await
    .expect_err("rename without confirmation");
    assert_eq!(error.code, serverdesk_lib::core::ErrorCode::Validation);

    let error = delete_remote_entry(
        manager.as_ref(),
        "server-1",
        "/srv/app",
        "/srv/app/file.txt",
        false,
        true,
    )
    .await
    .expect_err("delete without confirmation");
    assert_eq!(error.code, serverdesk_lib::core::ErrorCode::Validation);

    let error = create_remote_folder(
        manager.as_ref(),
        "server-1",
        "/srv/app",
        "/srv/app",
        "../bad",
    )
    .await
    .expect_err("unsafe folder name");
    assert_eq!(error.code, serverdesk_lib::core::ErrorCode::Validation);

    let error = browse_remote_files(manager.as_ref(), "server-1", "/srv/app", "/srv/other")
        .await
        .expect_err("path outside root");
    assert_eq!(error.code, serverdesk_lib::core::ErrorCode::Validation);

    let transfers = FileTransferManager::new(Arc::clone(&manager), Arc::new(NoopTransferSink));
    let error = transfers
        .upload(UploadRequest {
            transfer_id: "bad;transfer".to_string(),
            server_id: "server-1".to_string(),
            root: "/srv/app".to_string(),
            local_path: "/tmp/fixture".to_string(),
            remote_path: "/srv/app/fixture".to_string(),
            overwrite: false,
        })
        .await
        .expect_err("invalid transfer identifier");
    assert_eq!(error.code, serverdesk_lib::core::ErrorCode::Validation);

    let error = transfers
        .download(DownloadRequest {
            transfer_id: "valid-id".to_string(),
            server_id: "server-1".to_string(),
            root: "/srv/app".to_string(),
            remote_path: "/srv/other/file".to_string(),
            local_path: "/tmp/fixture".to_string(),
            overwrite: false,
        })
        .await
        .expect_err("remote path outside root");
    assert_eq!(error.code, serverdesk_lib::core::ErrorCode::Validation);
}

#[test]
fn local_paths_and_credential_references_fail_closed() {
    for payload in ["relative/path", "/tmp/../etc/passwd", "/tmp/bad\npath", ""] {
        assert!(normalize_local_path(payload).is_err(), "{payload:?}");
    }
    assert!(normalize_local_path("/tmp/fixture").is_ok());

    for reference in [
        "",
        "serverdesk ssh test",
        "serverdesk:ssh:test;id",
        "../../secret",
    ] {
        assert!(
            serverdesk_lib::core::credentials::validate_reference(reference).is_err(),
            "{reference:?}"
        );
    }
    assert!(
        serverdesk_lib::core::credentials::validate_reference("serverdesk:ssh:fixture").is_ok()
    );
}

#[test]
fn secrets_are_redacted_from_errors_and_diagnostics() {
    let diagnostic = "DB_PASSWORD=hunter2\n\
         Authorization: Bearer supersecret-bearer\n\
         DRAWER_URL=mysql://deploy:url-password-secret@127.0.0.1/app\n\
         token ghp_1234567890abcdef\n\
         api_key=sk-live-abcdefghijklmnop\n\
         -----BEGIN OPENSSH PRIVATE KEY-----\n\
         private-key-body-material\n\
         -----END OPENSSH PRIVATE KEY-----\n";
    let redacted = redact_text(diagnostic);
    assert!(redacted.redacted);
    for secret in SECRETS {
        assert!(
            !redacted.text.contains(secret),
            "{secret:?} survived redaction:\n{}",
            redacted.text
        );
    }

    let bounded = sanitize_text_bounded(diagnostic, 80);
    assert!(bounded.chars().count() <= 80);
    for secret in SECRETS {
        assert!(!bounded.contains(secret), "{secret:?} survived bounding");
    }

    let error = AppError::remote("Remote output: DB_PASSWORD=hunter2 token=ghp_1234567890abcdef")
        .with_details(diagnostic);
    let serialized = serde_json::to_string(&error).expect("serialized error");
    let debug = format!("{error:?}");
    for secret in SECRETS {
        assert!(
            !serialized.contains(secret),
            "{secret:?} leaked in AppError"
        );
        assert!(
            !debug.contains(secret),
            "{secret:?} leaked in AppError debug"
        );
    }
}

#[test]
fn sqlite_and_sidecars_never_store_plaintext_secrets() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let path = temp.path().join("serverdesk.sqlite3");
    let database = Database::open(&path).expect("open database");

    record_operation(
        &database,
        &NewOperationRecord {
            server_id: None,
            project_id: None,
            category: OperationCategory::Command,
            operation: "remote-hostname".to_string(),
            safe_summary: format!(
                "DB_PASSWORD=hunter2 Authorization: Bearer supersecret-bearer {}",
                "ghp_1234567890abcdef"
            ),
            path: Some("/srv/sk-live-abcdefghijklmnop".to_string()),
            output_summary: Some(
                "APP_KEY=base64:supersecret-bearer\n\
                 -----BEGIN OPENSSH PRIVATE KEY-----\n\
                 private-key-body-material\n\
                 -----END OPENSSH PRIVATE KEY-----\n\
                 URL=mysql://deploy:url-password-secret@127.0.0.1/app"
                    .to_string(),
            ),
            result: OperationResult::Failure,
            exit_code: Some(1),
            started_at: 1,
            duration_ms: 1,
        },
    )
    .expect("record sanitized operation");

    let injected = database
        .list_operations_filtered(&OperationHistoryFilter {
            server_id: Some("' OR 1=1 --".to_string()),
            ..OperationHistoryFilter::default()
        })
        .expect("parameterized filter");
    assert!(
        injected.is_empty(),
        "history filters must remain parameterized"
    );

    drop(database);

    let mut raw = Vec::new();
    for candidate in [
        path.clone(),
        path.with_file_name("serverdesk.sqlite3-wal"),
        path.with_file_name("serverdesk.sqlite3-shm"),
    ] {
        if let Ok(bytes) = std::fs::read(&candidate) {
            raw.extend_from_slice(&bytes);
        }
    }
    let text = String::from_utf8_lossy(&raw);
    for secret in SECRETS {
        assert!(
            !text.contains(secret),
            "{secret:?} was stored in SQLite or its sidecars"
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("database metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "database must be private to the OS user");
    }
}

#[test]
fn audit_confirms_no_application_log_or_crash_bundle_writer_exists() {
    // The application intentionally has no file logger and no crash-report or
    // diagnostic-bundle writer. This guard fails if one is added without a
    // redaction review in this suite.
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut findings = Vec::new();
    for entry in walk(&source_root) {
        let contents = std::fs::read_to_string(&entry).unwrap_or_default();
        for needle in [
            "crash_report",
            "diagnostic_bundle",
            "log::info!",
            "log::warn!",
            "tracing::info!",
            "tracing::warn!",
        ] {
            if contents.contains(needle) {
                findings.push(format!("{}: {needle}", entry.display()));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "new diagnostics/log writers must route through redaction:\n{}",
        findings.join("\n")
    );
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files
}
