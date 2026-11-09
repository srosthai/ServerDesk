//! Safe, read-only remote file browsing over the authenticated SSH session.
//!
//! Browsing uses the SFTP subsystem on the existing SSH connection. No
//! separate file-transfer listener or public port is opened. Every request is
//! bounded to one selected root: the requested path must be lexically inside
//! that root, and its canonical SFTP path must still be inside the canonical
//! root. That second check blocks symlinks that point outside the selected
//! directory even when a crafted path bypasses the UI.
//!
//! Directory listings return only metadata. ServerDesk does not read file
//! contents in this layer, and a symlink is never followed silently: its target
//! and whether it escapes the root are reported explicitly.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::{
    AppError,
    projects::{RemotePath, normalize_remote_path},
    ssh::{ConnectionManager, ConnectionState, SftpBrowser, SftpEntryKind, SftpError},
};

/// Maximum number of directory entries returned by one browse request.
pub const MAX_FILE_ENTRIES: usize = 2_000;
/// Hard duration bound for one complete browse request.
pub const FILE_BROWSE_TIMEOUT: Duration = Duration::from_secs(15);

const MAX_REMOTE_NAME_CHARS: usize = 255;
const MAX_SYMLINK_TARGET_CHARS: usize = 1_024;
const MAX_DETAIL_CHARS: usize = 300;

/// The type shown for one remote directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteEntryType {
    File,
    Directory,
    Symlink,
    Other,
}

/// One file or directory in a browsed remote directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileEntry {
    pub name: String,
    /// Normalized path built from the browsed path and this entry name.
    pub path: String,
    pub entry_type: RemoteEntryType,
    pub size: Option<u64>,
    /// Symbolic permissions in `ls -l` form, for example `-rw-r--r--`.
    pub permissions: Option<String>,
    /// Raw Unix mode bits, including the file-type bits.
    pub mode: Option<u32>,
    /// Modification time in seconds since the Unix epoch.
    pub modified_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
    /// `None` means the symlink target could not be verified safely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink_escapes_root: Option<bool>,
    pub broken_symlink: bool,
}

/// The result of browsing one directory inside a selected root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileListing {
    pub server_id: String,
    /// The selected root that bounds every navigation request.
    pub root: String,
    pub path: String,
    /// The canonical SFTP path of the browsed directory.
    pub resolved_path: String,
    pub entries: Vec<RemoteFileEntry>,
    pub truncated: bool,
    pub sampled_at: i64,
    pub duration_ms: u64,
    pub connection_state: ConnectionState,
}

/// Normalizes a selected root and requested path, and rejects any path that is
/// not lexically inside the root.
pub fn validate_browse_paths(root: &str, path: &str) -> Result<(RemotePath, RemotePath), AppError> {
    let root = normalize_remote_path(root)?;
    let path = normalize_remote_path(path)?;

    if !is_within_remote_root(root.as_str(), path.as_str()) {
        return Err(outside_root_error());
    }

    Ok((root, path))
}

/// Returns true when `path` is the root or a descendant of it.
pub fn is_within_remote_root(root: &str, path: &str) -> bool {
    if root == "/" {
        return path.starts_with('/');
    }

    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

fn outside_root_error() -> AppError {
    AppError::validation("The requested path is outside the selected root directory.").with_details(
        "ServerDesk only browses the selected root and its descendants. Parent segments and \
         absolute paths outside the root are rejected before any SFTP request is sent.",
    )
}

fn resolved_outside_root_error() -> AppError {
    AppError::permission("The requested path resolves outside the selected root directory.")
        .with_details(
            "The path is a symlink that leaves the selected root. ServerDesk blocked the browse \
             instead of following it. Choose a directory inside the root.",
        )
}

/// Browses one directory inside `root` over the managed authenticated session.
pub async fn browse_remote_files(
    manager: &ConnectionManager,
    server_id: &str,
    root: &str,
    path: &str,
) -> Result<RemoteFileListing, AppError> {
    if server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }

    let (root, path) = validate_browse_paths(root, path)?;
    let started = Instant::now();
    let browser = manager.open_sftp(server_id).await?;

    let result = tokio::time::timeout(
        FILE_BROWSE_TIMEOUT,
        browse_with_browser(manager, server_id, &root, &path, browser.as_ref(), started),
    )
    .await;
    let _ = browser.close().await;

    match result {
        Err(_) => Err(AppError::timeout(format!(
            "Remote file browsing timed out after {} seconds.",
            FILE_BROWSE_TIMEOUT.as_secs()
        ))),
        Ok(result) => result,
    }
}

async fn browse_with_browser(
    manager: &ConnectionManager,
    server_id: &str,
    root: &RemotePath,
    path: &RemotePath,
    browser: &dyn SftpBrowser,
    started: Instant,
) -> Result<RemoteFileListing, AppError> {
    let root_real = browser
        .canonicalize(root.as_str())
        .await
        .map_err(sftp_error)?;
    let path_real = browser
        .canonicalize(path.as_str())
        .await
        .map_err(sftp_error)?;

    if !is_within_remote_root(&root_real, &path_real) {
        return Err(resolved_outside_root_error());
    }

    let metadata = browser.stat(&path_real).await.map_err(sftp_error)?;
    if metadata.kind != SftpEntryKind::Directory {
        return Err(AppError::validation(
            "The selected remote path is not a directory, so it cannot be browsed.",
        ));
    }

    let (raw_entries, truncated) = browser
        .read_dir(&path_real, MAX_FILE_ENTRIES)
        .await
        .map_err(sftp_error)?;

    let mut entries = Vec::with_capacity(raw_entries.len());
    for raw in raw_entries {
        let Some(entry) = describe_entry(browser, &root_real, path.as_str(), &raw).await else {
            continue;
        };
        entries.push(entry);
    }

    entries.sort_by(|left, right| {
        entry_sort_rank(left.entry_type)
            .cmp(&entry_sort_rank(right.entry_type))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });

    let connection_state = manager.connection_state(server_id)?.state;

    Ok(RemoteFileListing {
        server_id: server_id.to_string(),
        root: root.as_str().to_string(),
        path: path.as_str().to_string(),
        resolved_path: path_real,
        entries,
        truncated,
        sampled_at: now_millis(),
        duration_ms: duration_millis(started),
        connection_state,
    })
}

async fn describe_entry(
    browser: &dyn SftpBrowser,
    root_real: &str,
    parent_path: &str,
    raw: &crate::core::ssh::SftpDirEntry,
) -> Option<RemoteFileEntry> {
    let name = safe_entry_name(&raw.name)?;
    let entry_path = join_remote_path(parent_path, &name);
    let mut entry = RemoteFileEntry {
        name,
        path: entry_path,
        entry_type: entry_type(raw.metadata.kind),
        size: raw.metadata.size,
        permissions: raw.metadata.mode.map(format_permissions),
        mode: raw.metadata.mode,
        modified_at: raw.metadata.modified_at,
        symlink_target: None,
        resolved_path: None,
        symlink_escapes_root: None,
        broken_symlink: false,
    };

    if raw.metadata.kind == SftpEntryKind::Symlink {
        if let Ok(target) = browser.read_link(&entry.path).await {
            entry.symlink_target = safe_symlink_target(&target);
        }

        match browser.canonicalize(&entry.path).await {
            Ok(resolved) => {
                let escapes = !is_within_remote_root(root_real, &resolved);
                entry.symlink_escapes_root = Some(escapes);
                if !escapes {
                    entry.resolved_path = Some(resolved);
                }
            }
            Err(SftpError::NotFound) => entry.broken_symlink = true,
            Err(_) => {
                // Leaving the target unverified is safer than guessing. The UI
                // blocks navigation until the link can be resolved.
            }
        }
    }

    Some(entry)
}

fn entry_type(kind: SftpEntryKind) -> RemoteEntryType {
    match kind {
        SftpEntryKind::File => RemoteEntryType::File,
        SftpEntryKind::Directory => RemoteEntryType::Directory,
        SftpEntryKind::Symlink => RemoteEntryType::Symlink,
        SftpEntryKind::Other => RemoteEntryType::Other,
    }
}

fn entry_sort_rank(kind: RemoteEntryType) -> u8 {
    match kind {
        RemoteEntryType::Directory => 0,
        RemoteEntryType::Symlink => 1,
        RemoteEntryType::File => 2,
        RemoteEntryType::Other => 3,
    }
}

fn safe_entry_name(value: &str) -> Option<String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\0')
        || value.chars().count() > MAX_REMOTE_NAME_CHARS
        || value.chars().any(char::is_control)
    {
        return None;
    }

    Some(value.to_string())
}

fn safe_symlink_target(value: &str) -> Option<String> {
    let bounded = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_SYMLINK_TARGET_CHARS)
        .collect::<String>();

    if bounded.is_empty() {
        None
    } else {
        Some(bounded)
    }
}

fn join_remote_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// Formats Unix mode bits as an `ls -l` style permission string.
pub fn format_permissions(mode: u32) -> String {
    let mut result = String::with_capacity(10);
    result.push(match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '-',
    });

    let owner = (mode >> 6) & 0o7;
    let group = (mode >> 3) & 0o7;
    let other = mode & 0o7;
    push_permission_triplet(&mut result, owner, mode & 0o4000 != 0, 's');
    push_permission_triplet(&mut result, group, mode & 0o2000 != 0, 's');
    push_permission_triplet(&mut result, other, mode & 0o1000 != 0, 't');

    result
}

fn push_permission_triplet(result: &mut String, bits: u32, special: bool, special_char: char) {
    result.push(if bits & 0o4 != 0 { 'r' } else { '-' });
    result.push(if bits & 0o2 != 0 { 'w' } else { '-' });
    if special {
        result.push(if bits & 0o1 != 0 {
            special_char
        } else {
            special_char.to_ascii_uppercase()
        });
    } else {
        result.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
}

fn sftp_error(error: SftpError) -> AppError {
    let mut error = error.to_app_error();
    if let Some(details) = error.details.take() {
        let details = details
            .chars()
            .filter(|character| !character.is_control() || *character == '\t')
            .take(MAX_DETAIL_CHARS)
            .collect::<String>();
        if !details.trim().is_empty() {
            error.details = Some(details);
        }
    }
    error
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn duration_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths_and_rejects_traversal_and_paths_outside_the_root() {
        let (root, path) = validate_browse_paths(" /var//www/./app/ ", "/var/www/app/public")
            .expect("valid paths");
        assert_eq!(root.as_str(), "/var/www/app");
        assert_eq!(path.as_str(), "/var/www/app/public");

        for (root, path) in [
            ("/var/www/app", "/var/www/application"),
            ("/var/www/app", "/var/www"),
            ("/var/www/app", "/etc/passwd"),
            ("/var/www/app", "/var/www/app/../etc"),
            ("/var/www/app", "relative/path"),
            ("", "/var/www"),
            ("/var/www", ""),
        ] {
            assert!(
                validate_browse_paths(root, path).is_err(),
                "{root:?} {path:?} must be rejected"
            );
        }
    }

    #[test]
    fn root_containment_handles_the_filesystem_root_and_boundaries() {
        assert!(is_within_remote_root("/", "/etc/passwd"));
        assert!(is_within_remote_root("/var/www", "/var/www"));
        assert!(is_within_remote_root("/var/www", "/var/www/app"));
        assert!(!is_within_remote_root("/var/www", "/var/wwwroot"));
        assert!(!is_within_remote_root("/var/www", "/var"));
        assert!(!is_within_remote_root("/var/www/", "/var/www"));
    }

    #[test]
    fn formats_ls_style_permissions_and_special_bits() {
        assert_eq!(format_permissions(0o040755), "drwxr-xr-x");
        assert_eq!(format_permissions(0o100644), "-rw-r--r--");
        assert_eq!(format_permissions(0o120777), "lrwxrwxrwx");
        assert_eq!(format_permissions(0o104755), "-rwsr-xr-x");
        assert_eq!(format_permissions(0o102755), "-rwxr-sr-x");
        assert_eq!(format_permissions(0o041777), "drwxrwxrwt");
        assert_eq!(format_permissions(0o041770), "drwxrwx--T");
        assert_eq!(format_permissions(0o041776), "drwxrwxrwT");
    }

    #[test]
    fn rejects_unsafe_entry_names_and_bounds_symlink_targets() {
        for valid in ["file.txt", "dir with spaces", ".hidden", "a-b_c"] {
            assert_eq!(safe_entry_name(valid).as_deref(), Some(valid));
        }
        for invalid in ["", ".", "..", "a/b", "a\0b", "line\nbreak"] {
            assert!(safe_entry_name(invalid).is_none(), "{invalid:?}");
        }

        let long = "a".repeat(MAX_SYMLINK_TARGET_CHARS + 20);
        assert_eq!(
            safe_symlink_target(&long)
                .expect("bounded target")
                .chars()
                .count(),
            MAX_SYMLINK_TARGET_CHARS
        );
        assert!(safe_symlink_target("\n").is_none());
    }

    #[test]
    fn joins_entry_paths_without_double_slashes() {
        assert_eq!(join_remote_path("/", "etc"), "/etc");
        assert_eq!(join_remote_path("/var/www", "app"), "/var/www/app");
    }

    #[test]
    fn serializes_metadata_and_symlink_flags_for_the_frontend() {
        let listing = RemoteFileListing {
            server_id: "server-1".to_string(),
            root: "/var/www/app".to_string(),
            path: "/var/www/app".to_string(),
            resolved_path: "/var/www/app".to_string(),
            entries: vec![
                RemoteFileEntry {
                    name: "public".to_string(),
                    path: "/var/www/app/public".to_string(),
                    entry_type: RemoteEntryType::Directory,
                    size: Some(4_096),
                    permissions: Some("drwxr-xr-x".to_string()),
                    mode: Some(0o040755),
                    modified_at: Some(1_762_665_720),
                    symlink_target: None,
                    resolved_path: None,
                    symlink_escapes_root: None,
                    broken_symlink: false,
                },
                RemoteFileEntry {
                    name: "latest".to_string(),
                    path: "/var/www/app/latest".to_string(),
                    entry_type: RemoteEntryType::Symlink,
                    size: Some(12),
                    permissions: Some("lrwxrwxrwx".to_string()),
                    mode: Some(0o120777),
                    modified_at: Some(1_762_665_720),
                    symlink_target: Some("/etc".to_string()),
                    resolved_path: None,
                    symlink_escapes_root: Some(true),
                    broken_symlink: false,
                },
            ],
            truncated: false,
            sampled_at: 1_762_665_720_000,
            duration_ms: 12,
            connection_state: ConnectionState::Connected,
        };

        let value = serde_json::to_value(listing).expect("serialized listing");
        assert_eq!(value["root"], "/var/www/app");
        assert_eq!(value["entries"][0]["entryType"], "directory");
        assert_eq!(value["entries"][0]["permissions"], "drwxr-xr-x");
        assert_eq!(value["entries"][0]["modifiedAt"], 1_762_665_720_i64);
        assert_eq!(value["entries"][1]["entryType"], "symlink");
        assert_eq!(value["entries"][1]["symlinkEscapesRoot"], true);
        assert!(value["entries"][0].get("symlinkTarget").is_none());
    }

    #[test]
    fn sftp_errors_map_to_typed_application_errors() {
        assert_eq!(
            sftp_error(SftpError::PermissionDenied).code,
            crate::core::ErrorCode::Permission
        );
        assert_eq!(
            sftp_error(SftpError::NotFound).code,
            crate::core::ErrorCode::NotFound
        );
        assert_eq!(
            sftp_error(SftpError::NotDirectory).code,
            crate::core::ErrorCode::Validation
        );
        assert_eq!(
            sftp_error(SftpError::Timeout).code,
            crate::core::ErrorCode::Timeout
        );
        assert_eq!(
            sftp_error(SftpError::Disconnected).code,
            crate::core::ErrorCode::Connection
        );
    }
}
