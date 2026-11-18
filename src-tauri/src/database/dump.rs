//! SQL dump and restore workflows for MySQL and PostgreSQL.
//!
//! Dump tools run on the remote server over the existing authenticated SSH
//! session. Their output is written to a temporary remote file, verified, and
//! only then moved to the user-selected remote path or downloaded to a local
//! partial file and renamed. A cancelled or failed run removes every temporary
//! file it created, so a destination is never replaced by an incomplete dump.
//!
//! Credentials are written to a mode-0700 remote directory and read by the
//! tool through an environment variable. They never appear in a command line,
//! a progress event, a result, or a log. Tool arguments are built from closed
//! enums and validated identifiers; every path and identifier is quoted as one
//! POSIX shell argument.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    core::{
        AppError, ErrorCode, command::shell_quote, redaction::sanitize_text_bounded,
        ssh::SftpMetadata,
    },
    storage::{DatabaseConnection, DatabaseEngine},
};

use super::{
    capabilities::DatabaseVersion,
    dialect::Identifier,
    errors::DatabaseError,
    rows::RowQueryCancellation,
    traits::{DatabaseConnectionInfo, DatabasePassword},
};

/// Event emitted while a SQL dump or restore runs.
pub const DATABASE_DUMP_EVENT: &str = "database-dump-event";
/// Default bound for one dump or restore.
pub const DEFAULT_DUMP_TIMEOUT: Duration = Duration::from_secs(3_600);
/// Hard maximum bound for one dump or restore.
pub const MAX_DUMP_TIMEOUT: Duration = Duration::from_secs(86_400);
/// Maximum number of tables accepted in one table-scope dump.
pub const MAX_DUMP_TABLES: usize = 100;
/// Maximum length of a caller-generated transfer id.
pub const MAX_DUMP_REQUEST_ID_LENGTH: usize = 128;
/// Maximum number of warnings returned with one result.
pub const MAX_DUMP_REPORTED_WARNINGS: usize = 20;
/// Maximum characters in one warning.
pub const MAX_DUMP_WARNING_CHARS: usize = 500;
/// Maximum characters in one failure summary.
pub const MAX_DUMP_FAILURE_CHARS: usize = 2_000;
/// Maximum bytes read from a remote tool error log.
pub const MAX_DUMP_ERROR_BYTES: usize = 64 * 1024;
/// Default interval between progress samples.
pub const DUMP_PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

const PROGRESS_THROTTLE: Duration = Duration::from_millis(120);
const DEFAULT_CANCEL_GRACE: Duration = Duration::from_secs(3);
const DEFAULT_KILL_GRACE: Duration = Duration::from_secs(3);
const MAX_PID: u32 = 4_194_304;
const MIN_DUMP_BYTES: u64 = 16;

/// One remote tool used by a dump or restore workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpTool {
    Mysqldump,
    Mysql,
    PgDump,
    PgDumpall,
    PgRestore,
    Psql,
}

impl DumpTool {
    pub const ALL: [Self; 6] = [
        Self::Mysqldump,
        Self::Mysql,
        Self::PgDump,
        Self::PgDumpall,
        Self::PgRestore,
        Self::Psql,
    ];

    /// The exact binary name. It is a closed literal, never user input.
    pub const fn binary(self) -> &'static str {
        match self {
            Self::Mysqldump => "mysqldump",
            Self::Mysql => "mysql",
            Self::PgDump => "pg_dump",
            Self::PgDumpall => "pg_dumpall",
            Self::PgRestore => "pg_restore",
            Self::Psql => "psql",
        }
    }

    pub const fn engine(self) -> DatabaseEngine {
        match self {
            Self::Mysqldump | Self::Mysql => DatabaseEngine::Mysql,
            Self::PgDump | Self::PgDumpall | Self::PgRestore | Self::Psql => {
                DatabaseEngine::Postgresql
            }
        }
    }

    pub const fn purpose(self) -> &'static str {
        match self {
            Self::Mysqldump | Self::PgDump | Self::PgDumpall => "export",
            Self::Mysql | Self::PgRestore | Self::Psql => "restore",
        }
    }
}

/// Vendor reported by a tool's `--version` output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpToolVendor {
    #[default]
    Unknown,
    Mysql,
    Mariadb,
    Postgresql,
}

/// Archive format selected for an export or restore.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpFormat {
    #[default]
    PlainSql,
    Custom,
    Tar,
}

impl DumpFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PlainSql => "plain SQL",
            Self::Custom => "custom archive",
            Self::Tar => "tar archive",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::PlainSql => "sql",
            Self::Custom => "dump",
            Self::Tar => "tar",
        }
    }
}

/// What part of the server a dump covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DumpScope {
    /// Every database and role in a PostgreSQL cluster (`pg_dumpall`).
    Cluster,
    /// One database. `None` uses the profile's database.
    Database {
        #[serde(default)]
        database: Option<Identifier>,
    },
    /// One schema inside a PostgreSQL database.
    Schema {
        #[serde(default)]
        database: Option<Identifier>,
        schema: Identifier,
    },
    /// One or more tables. A MySQL schema must equal its database.
    Tables {
        #[serde(default)]
        database: Option<Identifier>,
        #[serde(default)]
        schema: Option<Identifier>,
        tables: Vec<Identifier>,
    },
}

impl DumpScope {
    /// Resolves the database for this scope, falling back to the profile.
    pub fn database<'a>(&'a self, fallback: Option<&'a str>) -> Option<&'a str> {
        match self {
            Self::Cluster => None,
            Self::Database { database }
            | Self::Schema { database, .. }
            | Self::Tables { database, .. } => {
                database.as_ref().map(Identifier::as_str).or(fallback)
            }
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Cluster => "the PostgreSQL cluster".to_string(),
            Self::Database { database } => match database {
                Some(database) => format!("database {}", database.as_str()),
                None => "the profile database".to_string(),
            },
            Self::Schema { schema, .. } => format!("schema {}", schema.as_str()),
            Self::Tables { tables, .. } => {
                if tables.len() == 1 {
                    format!("table {}", tables[0].as_str())
                } else {
                    format!("{} tables", tables.len())
                }
            }
        }
    }
}

/// A user-selected destination for a dump.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DumpDestination {
    Remote {
        path: String,
        #[serde(default)]
        overwrite: bool,
    },
    Local {
        path: String,
        #[serde(default)]
        overwrite: bool,
    },
}

/// A user-selected source for a restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DumpSource {
    Remote { path: String },
    Local { path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpLocationKind {
    Remote,
    Local,
}

/// One export request. `tool` optionally pins the vendor tool; omitting it
/// selects the engine-appropriate default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpExportRequest {
    pub scope: DumpScope,
    #[serde(default)]
    pub format: DumpFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<DumpTool>,
    pub destination: DumpDestination,
    #[serde(default)]
    pub no_owner: bool,
}

/// Explicit restore acknowledgement. It must name the server, the target
/// database, and the data the restore may replace. The confirmation is
/// generated by a preview and echoed back by the caller; a bare boolean is not
/// enough to start a restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRestoreConfirmation {
    pub server_id: String,
    pub server_name: String,
    pub target_database: String,
    pub replaced_data: String,
    #[serde(default)]
    pub acknowledged: bool,
}

/// One restore request. It carries an explicit [`DumpRestoreConfirmation`]
/// naming the server, database, and replaced data before any remote command
/// runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRestoreRequest {
    pub source: DumpSource,
    #[serde(default)]
    pub format: DumpFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<DumpTool>,
    pub target_database: Identifier,
    #[serde(default)]
    pub clean: bool,
    #[serde(default)]
    pub no_owner: bool,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<DumpRestoreConfirmation>,
}

/// Request for a restore preview. It has no confirmation because it only reads
/// metadata and never changes the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRestorePreviewRequest {
    pub source: DumpSource,
    #[serde(default)]
    pub format: DumpFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<DumpTool>,
    pub target_database: Identifier,
    #[serde(default)]
    pub clean: bool,
    #[serde(default)]
    pub no_owner: bool,
}

/// What a restore will replace. The preview returns this exact text and the
/// restore request must echo it back, so a confirmation cannot silently name a
/// different target.
pub fn restore_replaced_data(target_database: &str, clean: bool) -> String {
    if clean {
        format!(
            "all existing objects and data in database {target_database} may be dropped and replaced"
        )
    } else {
        format!(
            "existing objects and rows that conflict with the dump in database {target_database} may be replaced"
        )
    }
}

/// Read-only restore plan shown before the user confirms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRestorePreview {
    pub server_id: String,
    pub server_name: String,
    pub profile_id: String,
    pub profile_name: String,
    pub engine: DatabaseEngine,
    pub target_database: String,
    pub source_kind: DumpLocationKind,
    pub source_path: String,
    pub format: DumpFormat,
    pub tool: DumpTool,
    pub tool_available: bool,
    pub tool_compatible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    pub replaces_existing_data: bool,
    pub replaced_data: String,
    pub clean: bool,
    pub no_owner: bool,
    pub can_restore: bool,
    pub warnings: Vec<String>,
    pub confirmation: DumpRestoreConfirmation,
}

/// Shared terminal status for dump and restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpDirection {
    Export,
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpPhase {
    Preparing,
    Dumping,
    Uploading,
    Transferring,
    Restoring,
    Verifying,
    CleaningUp,
    Finalizing,
}

/// Progress emitted while a dump or restore runs. It never contains
/// credentials, SQL text, or tool stderr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpProgress {
    pub transfer_id: String,
    pub direction: DumpDirection,
    pub status: DumpStatus,
    pub phase: DumpPhase,
    pub bytes_processed: u64,
    pub total_bytes: Option<u64>,
    pub percent: Option<u8>,
    pub message: String,
    pub may_be_incomplete: bool,
    pub emitted_at: i64,
}

/// Sink used by the command layer to forward progress to the UI.
pub trait DumpEventSink: Send + Sync {
    fn emit(&self, progress: &DumpProgress);
}

/// Availability and version compatibility for one remote tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpToolStatus {
    pub tool: DumpTool,
    pub binary: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_major: Option<u16>,
    pub vendor: DumpToolVendor,
    pub compatible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Tool inventory for one server. It is checked before every transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpToolReport {
    pub engine: DatabaseEngine,
    pub server_version: String,
    pub tools: Vec<DumpToolStatus>,
    pub export_ready: bool,
    pub restore_ready: bool,
    pub checked_at: i64,
}

impl DumpToolReport {
    pub fn status(&self, tool: DumpTool) -> Option<&DumpToolStatus> {
        self.tools.iter().find(|status| status.tool == tool)
    }
}

/// Engine and version of the live database session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DumpServerContext {
    pub engine: DatabaseEngine,
    pub version: DatabaseVersion,
}

impl DumpServerContext {
    pub fn from_info(info: &DatabaseConnectionInfo) -> Self {
        Self {
            engine: info.engine,
            version: info.version.version,
        }
    }
}

/// Connection values needed by the remote tools. It never holds a password.
#[derive(Debug, Clone)]
pub struct DumpConnection {
    pub engine: DatabaseEngine,
    pub host: String,
    pub port: u16,
    pub socket: Option<String>,
    pub database: Option<String>,
    pub username: String,
    pub ssl_mode: Option<String>,
    /// Restores are refused when the saved profile is read-only.
    pub read_only: bool,
}

impl From<&DatabaseConnection> for DumpConnection {
    fn from(profile: &DatabaseConnection) -> Self {
        Self {
            engine: profile.engine,
            host: profile.host.clone(),
            port: profile.port,
            socket: profile.socket.clone(),
            database: profile.database_name.clone(),
            username: profile.username.clone(),
            ssl_mode: profile.ssl_mode.clone(),
            read_only: profile.read_only,
        }
    }
}

/// Per-request timeout and optional cancellation.
#[derive(Debug, Clone)]
pub struct DumpControl {
    pub timeout: Duration,
    pub cancellation: Option<RowQueryCancellation>,
}

impl DumpControl {
    pub fn new(timeout: Duration, cancellation: Option<RowQueryCancellation>) -> Self {
        Self {
            timeout: timeout.clamp(Duration::from_secs(10), MAX_DUMP_TIMEOUT),
            cancellation,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(RowQueryCancellation::is_cancelled)
    }
}

/// Resolves an optional timeout in milliseconds with the dump hard bound.
pub fn dump_timeout(timeout_ms: Option<u64>) -> Duration {
    let milliseconds = timeout_ms
        .unwrap_or(DEFAULT_DUMP_TIMEOUT.as_millis() as u64)
        .clamp(10_000, MAX_DUMP_TIMEOUT.as_millis() as u64);
    Duration::from_millis(milliseconds)
}

/// Validates a caller-generated transfer id. It is only an in-memory
/// cancellation key and is never sent to a database or written to a file.
pub fn validate_dump_request_id(value: &str) -> Result<(), DatabaseError> {
    if value.is_empty() || value.len() > MAX_DUMP_REQUEST_ID_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A dump transfer id must contain between 1 and {MAX_DUMP_REQUEST_ID_LENGTH} characters."
        )));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(DatabaseError::validation(
            "A dump transfer id may contain only letters, digits, hyphens, underscores, and dots.",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Remote transport contract
// ---------------------------------------------------------------------------

/// One completed remote command. Exit status is separate from transport
/// success: a non-zero status is a tool failure, not an SSH failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
}

/// Metadata for one remote file or directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteFileInfo {
    pub size: u64,
    pub is_file: bool,
}

impl RemoteFileInfo {
    pub fn from_metadata(metadata: SftpMetadata) -> Self {
        Self {
            size: metadata.size.unwrap_or(0),
            is_file: metadata.kind == crate::core::ssh::SftpEntryKind::File,
        }
    }
}

/// Opens one remote session for a dump or restore.
#[async_trait]
pub trait DumpRemote: Send + Sync {
    async fn open(&self, server_id: &str) -> Result<Box<dyn DumpRemoteSession>, AppError>;
}

/// One remote session. Implementations reuse a single SFTP channel for
/// temporary files, progress samples, downloads, and uploads.
#[async_trait]
pub trait DumpRemoteSession: Send + Sync {
    async fn execute(&self, command: &str) -> Result<RemoteCommandOutput, AppError>;

    async fn execute_long(
        &self,
        command: &str,
        timeout: Duration,
    ) -> Result<RemoteCommandOutput, AppError>;

    async fn write_private_file(&self, path: &str, bytes: &[u8]) -> Result<(), AppError>;

    async fn stat(&self, path: &str) -> Result<Option<RemoteFileInfo>, AppError>;

    async fn read_bounded(&self, path: &str, max_bytes: usize)
    -> Result<Option<Vec<u8>>, AppError>;

    async fn download(
        &self,
        remote_path: &str,
        local_path: &Path,
        total_bytes: u64,
        cancel: &RowQueryCancellation,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<u64, AppError>;

    async fn upload(
        &self,
        local_path: &Path,
        remote_path: &str,
        total_bytes: u64,
        cancel: &RowQueryCancellation,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<u64, AppError>;
}

// ---------------------------------------------------------------------------
// Result documents
// ---------------------------------------------------------------------------

/// Result of one completed, cancelled, or failed export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpExportResult {
    pub status: DumpStatus,
    pub engine: DatabaseEngine,
    pub scope: DumpScope,
    pub format: DumpFormat,
    pub destination_kind: DumpLocationKind,
    pub destination_path: String,
    pub bytes_written: u64,
    pub verified: bool,
    pub tool: DumpTool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u32>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_summary: Option<String>,
    pub partial_removed: bool,
    pub temp_cleaned: bool,
    pub may_be_incomplete: bool,
    pub duration_ms: u64,
}

/// Result of one completed, cancelled, or failed restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRestoreResult {
    pub status: DumpStatus,
    pub engine: DatabaseEngine,
    pub format: DumpFormat,
    pub source_kind: DumpLocationKind,
    pub source_path: String,
    pub target_database: String,
    pub bytes_read: u64,
    pub tool: DumpTool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u32>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_summary: Option<String>,
    pub partial_removed: bool,
    pub temp_cleaned: bool,
    pub may_be_incomplete: bool,
    pub duration_ms: u64,
}

/// Cancellation response returned to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpCancellationResult {
    pub request_id: String,
    pub cancelled: bool,
}

// ---------------------------------------------------------------------------
// Cancellation registry
// ---------------------------------------------------------------------------

type CancellationEntries = Arc<Mutex<HashMap<String, Arc<RowQueryCancellation>>>>;

/// Tracks live dump and restore cancellations by caller-generated id.
#[derive(Default, Clone)]
pub struct DumpTransferRegistry {
    entries: CancellationEntries,
}

impl DumpTransferRegistry {
    pub fn begin(&self, request_id: &str) -> Result<DumpTransferGuard, DatabaseError> {
        validate_dump_request_id(request_id)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.contains_key(request_id) {
            return Err(DatabaseError::validation(
                "A dump transfer with this request id is already running.",
            ));
        }
        let cancellation = Arc::new(RowQueryCancellation::default());
        entries.insert(request_id.to_string(), Arc::clone(&cancellation));
        Ok(DumpTransferGuard {
            request_id: request_id.to_string(),
            cancellation,
            entries: Arc::clone(&self.entries),
        })
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        if validate_dump_request_id(request_id).is_err() {
            return false;
        }
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match entries.get(request_id) {
            Some(cancellation) => {
                cancellation.cancel();
                true
            }
            None => false,
        }
    }

    pub fn cancel_all(&self) {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for cancellation in entries.values() {
            cancellation.cancel();
        }
    }

    pub fn active_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// Removes a cancellation entry when the command finishes.
#[derive(Debug)]
pub struct DumpTransferGuard {
    request_id: String,
    cancellation: Arc<RowQueryCancellation>,
    entries: CancellationEntries,
}

impl DumpTransferGuard {
    pub fn cancellation(&self) -> RowQueryCancellation {
        RowQueryCancellation::clone(&self.cancellation)
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

impl Drop for DumpTransferGuard {
    fn drop(&mut self) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(current) = entries.get(&self.request_id)
            && Arc::ptr_eq(current, &self.cancellation)
        {
            entries.remove(&self.request_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Request validation
// ---------------------------------------------------------------------------

/// Validates the engine, scope, and format of one export request.
pub fn validate_export_request(
    context: &DumpServerContext,
    connection: &DumpConnection,
    request: &DumpExportRequest,
) -> Result<(), DatabaseError> {
    validate_scope(context.engine, connection, &request.scope, request.format)?;
    if request.no_owner && context.engine == DatabaseEngine::Mysql {
        return Err(DatabaseError::capability(
            "MySQL dumps do not support the no-owner option.",
        ));
    }
    if matches!(request.scope, DumpScope::Cluster) && request.no_owner {
        return Err(DatabaseError::validation(
            "A PostgreSQL cluster dump does not support the no-owner option.",
        ));
    }
    export_tool_for(context.engine, &request.scope, request.format, request.tool)?;
    Ok(())
}

/// Validates one restore request. A restore always requires explicit
/// confirmation before any tool is detected or any remote file is written.
pub fn validate_restore_request(
    context: &DumpServerContext,
    request: &DumpRestoreRequest,
) -> Result<(), DatabaseError> {
    if !request.confirmed {
        return Err(DatabaseError::validation(
            "Restoring a SQL dump requires explicit confirmation.",
        ));
    }
    let confirmation = request.confirmation.as_ref().ok_or_else(|| {
        DatabaseError::validation(
            "The restore confirmation must name the server, target database, and replaced data.",
        )
    })?;
    if !confirmation.acknowledged {
        return Err(DatabaseError::validation(
            "Acknowledge that the restore may replace existing data before starting it.",
        ));
    }
    if confirmation.server_id.trim().is_empty() || confirmation.server_name.trim().is_empty() {
        return Err(DatabaseError::validation(
            "The restore confirmation must name the server.",
        ));
    }
    if confirmation.target_database != request.target_database.as_str() {
        return Err(DatabaseError::validation(
            "The restore confirmation does not name the target database.",
        ));
    }
    let replaced_data = restore_replaced_data(request.target_database.as_str(), request.clean);
    if confirmation.replaced_data != replaced_data {
        return Err(DatabaseError::validation(
            "The restore confirmation does not describe the data that will be replaced.",
        ));
    }
    if request.target_database.as_str().trim().is_empty() {
        return Err(DatabaseError::validation(
            "Choose the database that the dump should be restored into.",
        ));
    }
    validate_tool_database_name(request.target_database.as_str())?;
    restore_tool_for(context.engine, request.format, request.tool)?;
    match context.engine {
        DatabaseEngine::Mysql => {
            if request.format != DumpFormat::PlainSql {
                return Err(DatabaseError::capability(
                    "The mysql client can restore plain SQL dumps only.",
                ));
            }
            if request.clean || request.no_owner {
                return Err(DatabaseError::capability(
                    "MySQL restores do not support clean or no-owner options.",
                ));
            }
        }
        DatabaseEngine::Postgresql => {
            if request.format == DumpFormat::PlainSql && (request.clean || request.no_owner) {
                return Err(DatabaseError::validation(
                    "Plain SQL restores do not support clean or no-owner options. Use a custom or tar archive instead.",
                ));
            }
        }
    }
    Ok(())
}

/// Validates the parts of a restore preview that do not require a remote
/// session. The preview itself only reads metadata and never changes data.
pub fn validate_restore_preview_request(
    context: &DumpServerContext,
    request: &DumpRestorePreviewRequest,
) -> Result<(), DatabaseError> {
    if request.target_database.as_str().trim().is_empty() {
        return Err(DatabaseError::validation(
            "Choose the database that the dump should be restored into.",
        ));
    }
    validate_tool_database_name(request.target_database.as_str())?;
    restore_tool_for(context.engine, request.format, request.tool)?;
    match context.engine {
        DatabaseEngine::Mysql => {
            if request.format != DumpFormat::PlainSql {
                return Err(DatabaseError::capability(
                    "The mysql client can restore plain SQL dumps only.",
                ));
            }
            if request.clean || request.no_owner {
                return Err(DatabaseError::capability(
                    "MySQL restores do not support clean or no-owner options.",
                ));
            }
        }
        DatabaseEngine::Postgresql => {
            if request.format == DumpFormat::PlainSql && (request.clean || request.no_owner) {
                return Err(DatabaseError::validation(
                    "Plain SQL restores do not support clean or no-owner options. Use a custom or tar archive instead.",
                ));
            }
        }
    }
    Ok(())
}

/// Validates the server identity in a restore confirmation. The command layer
/// also checks the stored server name before connecting.
pub fn validate_restore_confirmation(
    server_id: &str,
    request: &DumpRestoreRequest,
) -> Result<(), DatabaseError> {
    let confirmation = request.confirmation.as_ref().ok_or_else(|| {
        DatabaseError::validation(
            "The restore confirmation must name the server, target database, and replaced data.",
        )
    })?;
    if confirmation.server_id != server_id {
        return Err(DatabaseError::validation(
            "The restore confirmation names a different server.",
        ));
    }
    Ok(())
}

/// Rejects database names that a dump tool would interpret as a connection
/// string instead of a literal database name.
///
/// `pg_dump`, `pg_dumpall`, `psql`, and `pg_restore` accept a libpq conninfo
/// string in `--dbname`. A document value such as `host=attacker` would make
/// the tool connect elsewhere and send the process `PGPASSWORD` there. The
/// same characters are not valid in a MySQL database name on any supported
/// platform, so both engines share this fail-closed check.
fn validate_tool_database_name(name: &str) -> Result<(), DatabaseError> {
    if name.trim().is_empty() || name.trim() != name {
        return Err(DatabaseError::validation(
            "The dump database name must not be empty or surrounded by whitespace.",
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(DatabaseError::validation(
            "The dump database name cannot contain control characters.",
        ));
    }
    if name.starts_with('-') {
        return Err(DatabaseError::validation(
            "The dump database name cannot start with '-'; a database tool could treat it as an option.",
        )
        .with_details("Rename the database or choose one that does not begin with a dash."));
    }
    if let Some(character) = name
        .chars()
        .find(|character| matches!(character, '=' | ':' | '/' | '@' | '?' | '&' | '%' | '#'))
    {
        return Err(DatabaseError::validation(format!(
            "The dump database name contains {character:?}, which a database tool can interpret as a connection string."
        ))
        .with_details(
            "Choose a database name without '=', ':', '/', '@', '?', '&', '%', or '#'. The tool receives the name as a literal database, never as a connection string.",
        ));
    }
    Ok(())
}

fn validate_scope(
    engine: DatabaseEngine,
    connection: &DumpConnection,
    scope: &DumpScope,
    format: DumpFormat,
) -> Result<(), DatabaseError> {
    match engine {
        DatabaseEngine::Mysql => {
            if format != DumpFormat::PlainSql {
                return Err(DatabaseError::capability(
                    "mysqldump exports plain SQL only. Choose the plain SQL format.",
                ));
            }
            match scope {
                DumpScope::Cluster => {
                    return Err(DatabaseError::capability(
                        "MySQL has no cluster-wide dump scope. Choose a database or table scope.",
                    ));
                }
                DumpScope::Schema { .. } => {
                    return Err(DatabaseError::capability(
                        "MySQL schemas are databases. Choose a database scope instead.",
                    ));
                }
                DumpScope::Database { .. } | DumpScope::Tables { .. } => {}
            }
            let database = scope
                .database(connection.database.as_deref())
                .ok_or_else(|| {
                    DatabaseError::validation(
                        "Choose the MySQL database to dump; the profile has no default database.",
                    )
                })?;
            validate_tool_database_name(database)?;
            if let DumpScope::Tables {
                schema: Some(schema),
                tables,
                ..
            } = scope
            {
                if schema.as_str() != database {
                    return Err(DatabaseError::validation(
                        "A MySQL table scope must use the same schema and database name.",
                    ));
                }
                validate_tables(tables, engine)?;
            } else if let DumpScope::Tables { tables, .. } = scope {
                validate_tables(tables, engine)?;
            }
        }
        DatabaseEngine::Postgresql => {
            if matches!(scope, DumpScope::Cluster) {
                if format != DumpFormat::PlainSql {
                    return Err(DatabaseError::capability(
                        "pg_dumpall exports plain SQL only. Choose the plain SQL format.",
                    ));
                }
                let database = connection.database.as_deref().unwrap_or("postgres");
                validate_tool_database_name(database)?;
                return Ok(());
            }
            let database = scope
                .database(connection.database.as_deref())
                .ok_or_else(|| {
                    DatabaseError::validation(
                        "Choose the PostgreSQL database to dump; the profile has no default database.",
                    )
                })?;
            validate_tool_database_name(database)?;
            if let DumpScope::Schema { schema, .. } = scope {
                validate_pg_pattern_name(schema, "schema")?;
            }
            if let DumpScope::Tables {
                schema: Some(schema),
                tables,
                ..
            } = scope
            {
                validate_pg_pattern_name(schema, "schema")?;
                validate_tables(tables, engine)?;
            } else if let DumpScope::Tables { tables, .. } = scope {
                validate_tables(tables, engine)?;
            }
        }
    }
    Ok(())
}

fn validate_pg_pattern_name(
    identifier: &Identifier,
    kind: &'static str,
) -> Result<(), DatabaseError> {
    if identifier
        .as_str()
        .chars()
        .any(|character| matches!(character, '*' | '?' | '[' | ']'))
    {
        return Err(DatabaseError::validation(format!(
            "PostgreSQL {kind} names containing pattern characters cannot be selected individually."
        )));
    }
    Ok(())
}

fn validate_tables(tables: &[Identifier], engine: DatabaseEngine) -> Result<(), DatabaseError> {
    if tables.is_empty() {
        return Err(DatabaseError::validation(
            "Choose at least one table for a table-scope dump.",
        ));
    }
    if tables.len() > MAX_DUMP_TABLES {
        return Err(DatabaseError::validation(format!(
            "A table-scope dump supports at most {MAX_DUMP_TABLES} tables."
        )));
    }
    let mut seen = HashSet::with_capacity(tables.len());
    for table in tables {
        if !seen.insert(table.as_str()) {
            return Err(DatabaseError::validation(format!(
                "The table {} was selected more than once.",
                table.as_str()
            )));
        }
        if table.as_str().starts_with('-') {
            return Err(DatabaseError::validation(format!(
                "The table {} cannot start with '-'; a database tool could treat it as an option.",
                table.as_str()
            )));
        }
        if engine == DatabaseEngine::Postgresql
            && table
                .as_str()
                .chars()
                .any(|character| matches!(character, '*' | '?' | '[' | ']'))
        {
            return Err(DatabaseError::validation(
                "PostgreSQL table names containing pattern characters cannot be selected individually.",
            ));
        }
    }
    Ok(())
}

/// Selects the export tool for one scope and format.
pub fn export_tool(
    engine: DatabaseEngine,
    scope: &DumpScope,
    format: DumpFormat,
) -> Result<DumpTool, DatabaseError> {
    match engine {
        DatabaseEngine::Mysql => {
            if format != DumpFormat::PlainSql {
                return Err(DatabaseError::capability(
                    "mysqldump exports plain SQL only.",
                ));
            }
            Ok(DumpTool::Mysqldump)
        }
        DatabaseEngine::Postgresql => {
            if matches!(scope, DumpScope::Cluster) {
                if format != DumpFormat::PlainSql {
                    return Err(DatabaseError::capability(
                        "pg_dumpall exports plain SQL only.",
                    ));
                }
                Ok(DumpTool::PgDumpall)
            } else {
                Ok(DumpTool::PgDump)
            }
        }
    }
}

/// Selects the export tool, honoring an explicit user choice only when that
/// tool is valid for the engine, scope, and format.
pub fn export_tool_for(
    engine: DatabaseEngine,
    scope: &DumpScope,
    format: DumpFormat,
    requested: Option<DumpTool>,
) -> Result<DumpTool, DatabaseError> {
    let expected = export_tool(engine, scope, format)?;
    let Some(requested) = requested else {
        return Ok(expected);
    };
    if requested != expected {
        return Err(DatabaseError::capability(format!(
            "{} cannot export this scope and format. Use {} instead.",
            requested.binary(),
            expected.binary()
        )));
    }
    Ok(requested)
}

/// Selects the restore tool for one engine and format.
pub fn restore_tool(engine: DatabaseEngine, format: DumpFormat) -> DumpTool {
    match (engine, format) {
        (DatabaseEngine::Mysql, _) => DumpTool::Mysql,
        (DatabaseEngine::Postgresql, DumpFormat::PlainSql) => DumpTool::Psql,
        (DatabaseEngine::Postgresql, DumpFormat::Custom | DumpFormat::Tar) => DumpTool::PgRestore,
    }
}

/// Selects the restore tool, honoring an explicit user choice only when that
/// tool is valid for the engine and source format.
pub fn restore_tool_for(
    engine: DatabaseEngine,
    format: DumpFormat,
    requested: Option<DumpTool>,
) -> Result<DumpTool, DatabaseError> {
    let expected = restore_tool(engine, format);
    let Some(requested) = requested else {
        return Ok(expected);
    };
    if requested != expected {
        return Err(DatabaseError::capability(format!(
            "{} cannot restore this format. Use {} instead.",
            requested.binary(),
            expected.binary()
        )));
    }
    Ok(requested)
}

/// Requires one tool to be installed and version-compatible.
pub fn require_tool(
    report: &DumpToolReport,
    tool: DumpTool,
) -> Result<&DumpToolStatus, DatabaseError> {
    let status = report.status(tool).ok_or_else(|| {
        DatabaseError::internal("The remote tool report is missing a required entry.")
    })?;
    if !status.available {
        return Err(DatabaseError::capability(format!(
            "{} is not installed on the remote server.",
            tool.binary()
        ))
        .with_details(format!(
            "Install the database client package that provides {} and reconnect the server. No dump or restore was started.",
            tool.binary()
        )));
    }
    if !status.compatible {
        let version = status.version.as_deref().unwrap_or("an unknown version");
        return Err(DatabaseError::capability(format!(
            "{} {version} is not compatible with the database server version {}.",
            tool.binary(),
            report.server_version
        ))
        .with_details(
            "Install a client version whose major version is at least the server major version, then retry.",
        ));
    }
    Ok(status)
}

// ---------------------------------------------------------------------------
// Tool detection
// ---------------------------------------------------------------------------

/// Detects every tool for the engine and compares its major version with the
/// server. Tool absence is returned as a typed status, never as a panic.
pub async fn detect_tools(
    session: &dyn DumpRemoteSession,
    context: &DumpServerContext,
) -> Result<DumpToolReport, AppError> {
    let mut tools = Vec::new();
    for tool in DumpTool::ALL
        .into_iter()
        .filter(|tool| tool.engine() == context.engine)
    {
        tools.push(detect_tool(session, tool, context).await?);
    }
    let ready = |tool| {
        tools
            .iter()
            .any(|status| status.tool == tool && status.available && status.compatible)
    };
    let export_ready = match context.engine {
        DatabaseEngine::Mysql => ready(DumpTool::Mysqldump),
        DatabaseEngine::Postgresql => ready(DumpTool::PgDump),
    };
    let restore_ready = match context.engine {
        DatabaseEngine::Mysql => ready(DumpTool::Mysql),
        DatabaseEngine::Postgresql => ready(DumpTool::PgRestore),
    };
    Ok(DumpToolReport {
        engine: context.engine,
        server_version: context.version.to_string(),
        tools,
        export_ready,
        restore_ready,
        checked_at: now_unix_seconds(),
    })
}

async fn detect_tool(
    session: &dyn DumpRemoteSession,
    tool: DumpTool,
    context: &DumpServerContext,
) -> Result<DumpToolStatus, AppError> {
    let lookup = session
        .execute(&format!("command -v {}", tool.binary()))
        .await?;
    let path = first_line(&lookup.stdout).filter(|line| !line.is_empty());
    let available = lookup.exit_code == Some(0) && path.is_some();
    if !available {
        return Ok(DumpToolStatus {
            tool,
            binary: tool.binary().to_string(),
            available: false,
            path: None,
            version: None,
            version_major: None,
            vendor: DumpToolVendor::Unknown,
            compatible: false,
            detail: Some("Not installed or not on PATH.".to_string()),
        });
    }

    let version_output = session
        .execute(&format!("{} --version", tool.binary()))
        .await?;
    let version_text = format!("{} {}", version_output.stdout, version_output.stderr);
    let version = DatabaseVersion::parse(&version_text);
    let vendor = tool_vendor(tool, &version_text);
    let compatible = version.is_some_and(|version| version.major >= context.version.major);
    let detail = if version.is_none() {
        Some("The installed version could not be determined.".to_string())
    } else if !compatible {
        Some(format!(
            "The installed version is older than the server version {}.",
            context.version
        ))
    } else {
        None
    };

    Ok(DumpToolStatus {
        tool,
        binary: tool.binary().to_string(),
        available: true,
        path,
        version: version.map(|version| version.to_string()),
        version_major: version.map(|version| version.major),
        vendor,
        compatible,
        detail,
    })
}

fn tool_vendor(tool: DumpTool, version_text: &str) -> DumpToolVendor {
    let lower = version_text.to_ascii_lowercase();
    if lower.contains("mariadb") {
        return DumpToolVendor::Mariadb;
    }
    match tool.engine() {
        DatabaseEngine::Mysql => DumpToolVendor::Mysql,
        DatabaseEngine::Postgresql => DumpToolVendor::Postgresql,
    }
}

fn first_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| sanitize_text_bounded(line, MAX_DUMP_WARNING_CHARS))
}

// ---------------------------------------------------------------------------
// Command paths and builders
// ---------------------------------------------------------------------------

/// Temporary remote paths used by one workflow.
#[derive(Debug, Clone)]
pub struct DumpPaths {
    pub work_dir: String,
    pub output: Option<String>,
    pub error_log: String,
    pub pid: String,
    pub credential: String,
    pub input: Option<String>,
}

impl DumpPaths {
    fn new(work_dir: String) -> Self {
        Self {
            output: None,
            error_log: format!("{work_dir}/stderr.log"),
            pid: format!("{work_dir}/dump.pid"),
            credential: format!("{work_dir}/password"),
            input: None,
            work_dir,
        }
    }
}

/// Builds the fixed shell wrapper around one dump tool. The password is read
/// from a mode-0600 file into an environment variable; it is never an argument.
pub fn wrap_tool_command(
    binary: &'static str,
    password_variable: &'static str,
    arguments: &[String],
    paths: &DumpPaths,
    stdout: &str,
    stdin: Option<&str>,
) -> String {
    wrap_tool_command_with_environment(
        binary,
        password_variable,
        arguments,
        &[],
        paths,
        stdout,
        stdin,
    )
}

/// Same wrapper with fixed `NAME=value` environment assignments. Callers use
/// this for libpq options such as `PGSSLMODE`, which `pg_dump`, `pg_restore`,
/// and `psql` read from the environment rather than a command-line flag.
pub fn wrap_tool_command_with_environment(
    binary: &'static str,
    password_variable: &'static str,
    arguments: &[String],
    environment: &[String],
    paths: &DumpPaths,
    stdout: &str,
    stdin: Option<&str>,
) -> String {
    let args = arguments.join(" ");
    let input = stdin
        .map(|path| format!(" < {}", shell_quote(path)))
        .unwrap_or_default();
    let environment = environment
        .iter()
        .map(|entry| format!("{entry} "))
        .collect::<String>();
    format!(
        "umask 077; set -C; printf '%s' \"$$\" > {}; {password_variable}=$(cat {}); export \
         {password_variable}; {environment}exec {binary} {args}{input} > {} 2> {}",
        shell_quote(&paths.pid),
        shell_quote(&paths.credential),
        shell_quote(stdout),
        shell_quote(&paths.error_log),
    )
}

pub fn mkdir_command(path: &str) -> String {
    format!("mkdir -m 700 -- {}", shell_quote(path))
}

pub fn rm_rf_command(path: &str) -> String {
    format!("rm -rf -- {}", shell_quote(path))
}

pub fn rm_file_command(path: &str) -> String {
    format!("rm -f -- {}", shell_quote(path))
}

pub fn kill_command(signal: &'static str, pid: u32) -> String {
    format!("kill -{signal} -- {pid}")
}

fn move_command(source: &str, destination: &str, overwrite: bool) -> String {
    let source = shell_quote(source);
    let destination = shell_quote(destination);
    if overwrite {
        format!("mv -f -- {source} {destination}")
    } else {
        format!(
            "if [ -e {destination} ]; then printf '%s' '__exists__' >&2; exit 17; fi; mv -- \
             {source} {destination}"
        )
    }
}

fn probe_remote_destination_command(parent: &str, destination: &str, partial: &str) -> String {
    let parent = shell_quote(parent);
    let destination = shell_quote(destination);
    let partial = shell_quote(partial);
    format!(
        "if [ ! -d {parent} ]; then printf '%s' '__parent_missing__'; exit 0; fi; \
         if [ ! -w {parent} ]; then printf '%s' '__parent_unwritable__'; exit 0; fi; \
         if [ -e {partial} ]; then printf '%s' '__partial_exists__'; exit 0; fi; \
         if [ -d {destination} ]; then printf '%s' '__dest_directory__'; exit 0; fi; \
         if [ -e {destination} ]; then printf '%s' '__dest_exists__'; else printf '%s' \
         '__dest_missing__'; fi"
    )
}

pub(crate) fn short_transfer_id(transfer_id: &str) -> String {
    let suffix = transfer_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(16)
        .collect::<String>();
    if suffix.is_empty() {
        "partial".to_string()
    } else {
        suffix
    }
}

fn remote_work_dir() -> String {
    format!(
        "/tmp/serverdesk-dump-{}",
        Uuid::new_v4().to_string().replace('-', "")
    )
}

fn remote_partial_path(destination: &str, transfer_id: &str) -> String {
    format!(
        "{destination}.serverdesk-{}.part",
        short_transfer_id(transfer_id)
    )
}

fn local_partial_path(destination: &Path, transfer_id: &str) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(format!(
        ".serverdesk-{}.part",
        short_transfer_id(transfer_id)
    ));
    destination.with_file_name(name)
}

fn remote_parent(path: &str) -> Option<&str> {
    if path == "/" {
        return None;
    }
    let index = path.rfind('/')?;
    Some(if index == 0 { "/" } else { &path[..index] })
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

struct DumpProgressTracker {
    transfer_id: String,
    direction: DumpDirection,
    events: Arc<dyn DumpEventSink>,
    bytes: Mutex<u64>,
    total: Mutex<Option<u64>>,
    may_be_incomplete: Mutex<bool>,
    last_emit: Mutex<Option<Instant>>,
}

impl DumpProgressTracker {
    fn new(transfer_id: &str, direction: DumpDirection, events: Arc<dyn DumpEventSink>) -> Self {
        Self {
            transfer_id: transfer_id.to_string(),
            direction,
            events,
            bytes: Mutex::new(0),
            total: Mutex::new(None),
            may_be_incomplete: Mutex::new(false),
            last_emit: Mutex::new(None),
        }
    }

    fn set_bytes(&self, bytes: u64) {
        *self
            .bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = bytes;
    }

    fn bytes(&self) -> u64 {
        *self
            .bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_total(&self, total: Option<u64>) {
        *self
            .total
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = total;
    }

    fn set_incomplete(&self, may_be_incomplete: bool) {
        *self
            .may_be_incomplete
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = may_be_incomplete;
    }

    fn emit(&self, status: DumpStatus, phase: DumpPhase, message: &'static str, force: bool) {
        let now = Instant::now();
        let mut last = self
            .last_emit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !force
            && let Some(last) = *last
            && now.duration_since(last) < PROGRESS_THROTTLE
        {
            return;
        }
        *last = Some(now);
        drop(last);

        let bytes = self.bytes();
        let total = *self
            .total
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let may_be_incomplete = *self
            .may_be_incomplete
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let percent = total.map(|total| {
            u8::try_from(
                bytes
                    .saturating_mul(100)
                    .checked_div(total)
                    .unwrap_or(100)
                    .min(100),
            )
            .unwrap_or(100)
        });
        self.events.emit(&DumpProgress {
            transfer_id: self.transfer_id.clone(),
            direction: self.direction,
            status,
            phase,
            bytes_processed: bytes,
            total_bytes: total,
            percent,
            message: message.to_string(),
            may_be_incomplete,
            emitted_at: now_unix_seconds(),
        });
    }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

struct PreparedExport {
    paths: DumpPaths,
    destination: PreparedDestination,
}

#[derive(Debug, Clone)]
enum PreparedDestination {
    Remote {
        path: String,
        overwrite: bool,
        partial: String,
    },
    Local {
        path: PathBuf,
        overwrite: bool,
        partial: PathBuf,
    },
}

impl PreparedDestination {
    fn kind(&self) -> DumpLocationKind {
        match self {
            Self::Remote { .. } => DumpLocationKind::Remote,
            Self::Local { .. } => DumpLocationKind::Local,
        }
    }

    fn path(&self) -> String {
        match self {
            Self::Remote { path, .. } => path.clone(),
            Self::Local { path, .. } => path.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Debug, Clone)]
enum RestoreSource {
    Remote { path: String, size: u64 },
    Local { path: PathBuf, size: u64 },
}

impl RestoreSource {
    fn kind(&self) -> DumpLocationKind {
        match self {
            Self::Remote { .. } => DumpLocationKind::Remote,
            Self::Local { .. } => DumpLocationKind::Local,
        }
    }

    fn path(&self) -> String {
        match self {
            Self::Remote { path, .. } => path.clone(),
            Self::Local { path, .. } => path.to_string_lossy().into_owned(),
        }
    }

    fn size(&self) -> u64 {
        match self {
            Self::Remote { size, .. } | Self::Local { size, .. } => *size,
        }
    }
}

struct CommandOutcome {
    exit_code: Option<u32>,
    cancelled: bool,
    timed_out: bool,
    transport_error: Option<String>,
    stderr: String,
}

struct CleanupReport {
    partial_removed: bool,
    work_dir_removed: bool,
    warnings: Vec<String>,
}

struct DumpCleanup<'a> {
    session: &'a dyn DumpRemoteSession,
    work_dir: String,
    remote_partial: Option<String>,
    local_partial: Option<PathBuf>,
}

impl<'a> DumpCleanup<'a> {
    fn new(
        session: &'a dyn DumpRemoteSession,
        work_dir: String,
        remote_partial: Option<String>,
        local_partial: Option<PathBuf>,
    ) -> Self {
        Self {
            session,
            work_dir,
            remote_partial,
            local_partial,
        }
    }

    async fn run(&self) -> CleanupReport {
        let mut report = CleanupReport {
            partial_removed: false,
            work_dir_removed: false,
            warnings: Vec::new(),
        };

        if let Some(partial) = &self.remote_partial {
            match self.session.stat(partial).await {
                Ok(Some(_)) => match self.session.execute(&rm_file_command(partial)).await {
                    Ok(output) if output.exit_code == Some(0) => report.partial_removed = true,
                    _ => report
                        .warnings
                        .push("The partial dump file could not be removed.".to_string()),
                },
                Ok(None) => {}
                Err(_) => report
                    .warnings
                    .push("The partial dump file could not be checked.".to_string()),
            }
        }

        if let Some(partial) = &self.local_partial
            && tokio::fs::metadata(partial).await.is_ok()
        {
            match tokio::fs::remove_file(partial).await {
                Ok(()) => report.partial_removed = true,
                Err(_) => report
                    .warnings
                    .push("The local partial dump file could not be removed.".to_string()),
            }
        }

        match self.session.execute(&rm_rf_command(&self.work_dir)).await {
            Ok(output) if output.exit_code == Some(0) => report.work_dir_removed = true,
            _ => report
                .warnings
                .push("The remote temporary directory could not be removed.".to_string()),
        }

        report
    }
}

/// Stateless dump and restore service over one remote transport.
pub struct DatabaseDumpService<'a> {
    remote: &'a dyn DumpRemote,
    poll_interval: Duration,
    cancel_grace: Duration,
    kill_grace: Duration,
}

impl<'a> DatabaseDumpService<'a> {
    pub fn new(remote: &'a dyn DumpRemote) -> Self {
        Self {
            remote,
            poll_interval: DUMP_PROGRESS_INTERVAL,
            cancel_grace: DEFAULT_CANCEL_GRACE,
            kill_grace: DEFAULT_KILL_GRACE,
        }
    }

    #[cfg(test)]
    fn with_timings(
        remote: &'a dyn DumpRemote,
        poll_interval: Duration,
        cancel_grace: Duration,
        kill_grace: Duration,
    ) -> Self {
        Self {
            remote,
            poll_interval,
            cancel_grace,
            kill_grace,
        }
    }

    /// Detects tool availability without starting a transfer.
    pub async fn detect(
        &self,
        server_id: &str,
        context: &DumpServerContext,
    ) -> Result<DumpToolReport, AppError> {
        let session = self.remote.open(server_id).await?;
        detect_tools(session.as_ref(), context).await
    }

    /// Builds a read-only restore plan. It detects the selected tool and checks
    /// the source file, but it never starts a restore or changes data.
    #[allow(clippy::too_many_arguments)]
    pub async fn preview_restore(
        &self,
        server_id: &str,
        server_name: &str,
        profile_id: &str,
        profile_name: &str,
        context: &DumpServerContext,
        request: &DumpRestorePreviewRequest,
    ) -> Result<DumpRestorePreview, AppError> {
        validate_restore_preview_request(context, request).map_err(AppError::from)?;
        let tool = restore_tool_for(context.engine, request.format, request.tool)
            .map_err(AppError::from)?;
        let session = self.remote.open(server_id).await?;
        let report = detect_tools(session.as_ref(), context).await?;
        let status = report.status(tool).cloned();
        let tool_available = status.as_ref().is_some_and(|status| status.available);
        let tool_compatible = status.as_ref().is_some_and(|status| status.compatible);
        let tool_version = status.as_ref().and_then(|status| status.version.clone());
        let mut warnings = Vec::new();
        if !tool_available {
            warnings.push(format!(
                "{} is not installed on the remote server.",
                tool.binary()
            ));
        } else if !tool_compatible {
            warnings.push(format!(
                "{} {} is not compatible with the database server version {}.",
                tool.binary(),
                tool_version.as_deref().unwrap_or("(unknown)"),
                report.server_version
            ));
        } else if let Some(detail) = status.as_ref().and_then(|status| status.detail.clone()) {
            warnings.push(detail);
        }

        let (source_kind, source_path, source_ready) = match &request.source {
            DumpSource::Remote { path } => {
                let normalized = crate::core::projects::normalize_remote_path(path)?;
                let path = normalized.as_str().to_string();
                let ready = match session.stat(&path).await? {
                    Some(info) if info.is_file && info.size >= MIN_DUMP_BYTES => true,
                    Some(info) if !info.is_file => {
                        warnings.push("The remote dump source is not a regular file.".to_string());
                        false
                    }
                    Some(_) => {
                        warnings.push(
                            "The remote dump source is empty or too small to be a valid dump."
                                .to_string(),
                        );
                        false
                    }
                    None => {
                        warnings.push("The remote dump source does not exist.".to_string());
                        false
                    }
                };
                (DumpLocationKind::Remote, path, ready)
            }
            DumpSource::Local { path } => {
                let normalized = crate::core::files::normalize_local_path(path)?;
                let path = normalized.to_string_lossy().into_owned();
                let ready = match tokio::fs::metadata(&normalized).await {
                    Ok(metadata) if metadata.is_file() && metadata.len() >= MIN_DUMP_BYTES => true,
                    Ok(metadata) if !metadata.is_file() => {
                        warnings.push("The local dump source is not a regular file.".to_string());
                        false
                    }
                    Ok(_) => {
                        warnings.push(
                            "The local dump source is empty or too small to be a valid dump."
                                .to_string(),
                        );
                        false
                    }
                    Err(_) => {
                        warnings.push("The local dump source does not exist.".to_string());
                        false
                    }
                };
                (DumpLocationKind::Local, path, ready)
            }
        };

        let replaced_data = restore_replaced_data(request.target_database.as_str(), request.clean);
        let confirmation = DumpRestoreConfirmation {
            server_id: server_id.to_string(),
            server_name: server_name.to_string(),
            target_database: request.target_database.as_str().to_string(),
            replaced_data: replaced_data.clone(),
            acknowledged: false,
        };
        Ok(DumpRestorePreview {
            server_id: server_id.to_string(),
            server_name: server_name.to_string(),
            profile_id: profile_id.to_string(),
            profile_name: profile_name.to_string(),
            engine: context.engine,
            target_database: request.target_database.as_str().to_string(),
            source_kind,
            source_path,
            format: request.format,
            tool,
            tool_available,
            tool_compatible,
            tool_version,
            replaces_existing_data: true,
            replaced_data,
            clean: request.clean,
            no_owner: request.no_owner,
            can_restore: tool_available && tool_compatible && source_ready,
            warnings,
            confirmation,
        })
    }

    /// Runs one export and returns a typed result for every post-start
    /// failure. Preflight failures are returned as errors before any file is
    /// created.
    #[allow(clippy::too_many_arguments)]
    pub async fn export(
        &self,
        server_id: &str,
        context: &DumpServerContext,
        connection: &DumpConnection,
        password: &DatabasePassword,
        transfer_id: &str,
        request: &DumpExportRequest,
        control: &DumpControl,
        events: Arc<dyn DumpEventSink>,
    ) -> Result<DumpExportResult, AppError> {
        validate_export_request(context, connection, request).map_err(AppError::from)?;
        let started = Instant::now();
        let tracker =
            DumpProgressTracker::new(transfer_id, DumpDirection::Export, Arc::clone(&events));
        tracker.emit(
            DumpStatus::Running,
            DumpPhase::Preparing,
            "Checking the remote database tools.",
            true,
        );

        let session = self.remote.open(server_id).await?;
        let report = detect_tools(session.as_ref(), context).await?;
        let tool = export_tool_for(context.engine, &request.scope, request.format, request.tool)
            .map_err(AppError::from)?;
        let tool_status = require_tool(&report, tool).map_err(AppError::from)?.clone();
        let prepared = self
            .prepare_export(session.as_ref(), transfer_id, request)
            .await?;
        session
            .execute(&mkdir_command(&prepared.paths.work_dir))
            .await?;

        let (local_partial, remote_partial) = match &prepared.destination {
            PreparedDestination::Remote { partial, .. } => (None, Some(partial.clone())),
            PreparedDestination::Local { partial, .. } => (Some(partial.clone()), None),
        };
        let cleanup = DumpCleanup::new(
            session.as_ref(),
            prepared.paths.work_dir.clone(),
            remote_partial,
            local_partial,
        );

        let operation = async {
            if let Err(error) = session
                .write_private_file(&prepared.paths.credential, password.expose_secret().as_bytes())
                .await
            {
                return Ok(failed_export(
                    context.engine,
                    request,
                    &prepared.destination,
                    tool,
                    &tool_status,
                    None,
                    format!(
                        "The temporary credential file could not be written. {}",
                        error.message
                    ),
                    started,
                ));
            }

            let command = match build_export_command(
                context.engine,
                connection,
                request,
                &prepared.paths,
                tool_status.vendor,
            ) {
                Ok(command) => command,
                Err(error) => {
                    return Ok(failed_export(
                        context.engine,
                        request,
                        &prepared.destination,
                        tool,
                        &tool_status,
                        None,
                        error.message,
                        started,
                    ));
                }
            };

            tracker.emit(
                DumpStatus::Running,
                DumpPhase::Dumping,
                "Writing the dump on the server.",
                true,
            );
            let output_path = prepared.paths.output.as_deref();
            let outcome = self
                .run_command(
                    session.as_ref(),
                    &command,
                    &prepared.paths,
                    output_path,
                    control,
                    &tracker,
                    DumpPhase::Dumping,
                )
                .await;

            let warnings = safe_warnings(&outcome.stderr, password.expose_secret());
            if outcome.cancelled {
                tracker.set_incomplete(false);
                tracker.emit(
                    DumpStatus::Cancelled,
                    DumpPhase::CleaningUp,
                    "Stopping the dump and removing temporary files.",
                    true,
                );
                return Ok(DumpExportResult {
                    status: DumpStatus::Cancelled,
                    engine: context.engine,
                    scope: request.scope.clone(),
                    format: request.format,
                    destination_kind: prepared.destination.kind(),
                    destination_path: prepared.destination.path(),
                    bytes_written: tracker.bytes(),
                    verified: false,
                    tool,
                    tool_version: tool_status.version.clone(),
                    exit_code: outcome.exit_code,
                    warnings,
                    failure_summary: Some(
                        "Cancellation was requested. The dump process was stopped and temporary files were removed; the destination was not changed."
                            .to_string(),
                    ),
                    partial_removed: false,
                    temp_cleaned: false,
                    may_be_incomplete: false,
                    duration_ms: elapsed_ms(started),
                });
            }

            if outcome.timed_out || outcome.transport_error.is_some() || outcome.exit_code != Some(0)
            {
                tracker.emit(
                    DumpStatus::Failed,
                    DumpPhase::CleaningUp,
                    "Removing the partial dump.",
                    true,
                );
                let summary = failure_summary(tool, &outcome, password.expose_secret());
                return Ok(DumpExportResult {
                    status: DumpStatus::Failed,
                    engine: context.engine,
                    scope: request.scope.clone(),
                    format: request.format,
                    destination_kind: prepared.destination.kind(),
                    destination_path: prepared.destination.path(),
                    bytes_written: tracker.bytes(),
                    verified: false,
                    tool,
                    tool_version: tool_status.version.clone(),
                    exit_code: outcome.exit_code,
                    warnings,
                    failure_summary: Some(summary),
                    partial_removed: false,
                    temp_cleaned: false,
                    may_be_incomplete: false,
                    duration_ms: elapsed_ms(started),
                });
            }

            let Some(output_path) = prepared.paths.output.as_deref() else {
                return Ok(failed_export(
                    context.engine,
                    request,
                    &prepared.destination,
                    tool,
                    &tool_status,
                    outcome.exit_code,
                    "The dump tool did not produce an output path.".to_string(),
                    started,
                ));
            };
            let info = match session.stat(output_path).await {
                Ok(Some(info)) if info.is_file && info.size >= MIN_DUMP_BYTES => info,
                Ok(_) => {
                    return Ok(failed_export(
                        context.engine,
                        request,
                        &prepared.destination,
                        tool,
                        &tool_status,
                        outcome.exit_code,
                        "The dump tool finished but the output file is missing, too small, or not a regular file."
                            .to_string(),
                        started,
                    ));
                }
                Err(error) => {
                    return Ok(failed_export(
                        context.engine,
                        request,
                        &prepared.destination,
                        tool,
                        &tool_status,
                        outcome.exit_code,
                        format!("The dump file could not be verified. {}", error.message),
                        started,
                    ));
                }
            };

            tracker.set_bytes(info.size);
            tracker.set_total(Some(info.size));
            tracker.emit(
                DumpStatus::Running,
                DumpPhase::Verifying,
                "Verifying the dump file.",
                true,
            );

            match &prepared.destination {
                PreparedDestination::Remote {
                    path,
                    overwrite,
                    partial: _,
                } => {
                    let moved = match session
                        .execute(&move_command(output_path, path, *overwrite))
                        .await
                    {
                        Ok(output) => output,
                        Err(error) => {
                            return Ok(failed_export(
                                context.engine,
                                request,
                                &prepared.destination,
                                tool,
                                &tool_status,
                                None,
                                format!("The completed dump could not be moved to its destination. {}", error.message),
                                started,
                            ));
                        }
                    };
                    if moved.exit_code != Some(0) {
                        let detail = sanitize_text_bounded(moved.stderr.trim(), MAX_DUMP_FAILURE_CHARS);
                        return Ok(failed_export(
                            context.engine,
                            request,
                            &prepared.destination,
                            tool,
                            &tool_status,
                            moved.exit_code,
                            if detail.is_empty() {
                                "The completed dump could not be moved to its destination.".to_string()
                            } else {
                                format!("The completed dump could not be moved to its destination. {detail}")
                            },
                            started,
                        ));
                    }
                    let verified = session
                        .stat(path)
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|final_info| final_info.is_file && final_info.size == info.size);
                    if !verified {
                        return Ok(DumpExportResult {
                            status: DumpStatus::Failed,
                            engine: context.engine,
                            scope: request.scope.clone(),
                            format: request.format,
                            destination_kind: prepared.destination.kind(),
                            destination_path: prepared.destination.path(),
                            bytes_written: info.size,
                            verified: false,
                            tool,
                            tool_version: tool_status.version.clone(),
                            exit_code: moved.exit_code,
                            warnings,
                            failure_summary: Some(
                                "The dump was moved but its final size could not be verified."
                                    .to_string(),
                            ),
                            partial_removed: false,
                            temp_cleaned: false,
                            may_be_incomplete: true,
                            duration_ms: elapsed_ms(started),
                        });
                    }
                    tracker.emit(
                        DumpStatus::Completed,
                        DumpPhase::Finalizing,
                        "The dump completed.",
                        true,
                    );
                    Ok(DumpExportResult {
                        status: DumpStatus::Completed,
                        engine: context.engine,
                        scope: request.scope.clone(),
                        format: request.format,
                        destination_kind: prepared.destination.kind(),
                        destination_path: prepared.destination.path(),
                        bytes_written: info.size,
                        verified: true,
                        tool,
                        tool_version: tool_status.version.clone(),
                        exit_code: outcome.exit_code,
                        warnings,
                        failure_summary: None,
                        partial_removed: false,
                        temp_cleaned: false,
                        may_be_incomplete: false,
                        duration_ms: elapsed_ms(started),
                    })
                }
                PreparedDestination::Local {
                    path,
                    overwrite,
                    partial,
                } => {
                    tracker.set_total(Some(info.size));
                    tracker.emit(
                        DumpStatus::Running,
                        DumpPhase::Transferring,
                        "Downloading the dump to the local destination.",
                        true,
                    );
                    let cancellation = control
                        .cancellation
                        .clone()
                        .unwrap_or_default();
                    let downloaded = match session
                        .download(
                            output_path,
                            partial,
                            info.size,
                            &cancellation,
                            &|bytes| {
                                tracker.set_bytes(bytes);
                                tracker.emit(
                                    DumpStatus::Running,
                                    DumpPhase::Transferring,
                                    "Downloading the dump to the local destination.",
                                    false,
                                );
                            },
                        )
                        .await
                    {
                        Ok(downloaded) => downloaded,
                        Err(error) if error.code == ErrorCode::Cancelled => {
                            tracker.emit(
                                DumpStatus::Cancelled,
                                DumpPhase::CleaningUp,
                                "Stopping the download and removing temporary files.",
                                true,
                            );
                            return Ok(DumpExportResult {
                                status: DumpStatus::Cancelled,
                                engine: context.engine,
                                scope: request.scope.clone(),
                                format: request.format,
                                destination_kind: prepared.destination.kind(),
                                destination_path: prepared.destination.path(),
                                bytes_written: tracker.bytes(),
                                verified: false,
                                tool,
                                tool_version: tool_status.version.clone(),
                                exit_code: outcome.exit_code,
                                warnings,
                                failure_summary: Some(
                                    "Cancellation was requested. The partial download was removed and the destination was not changed."
                                        .to_string(),
                                ),
                                partial_removed: false,
                                temp_cleaned: false,
                                may_be_incomplete: false,
                                duration_ms: elapsed_ms(started),
                            });
                        }
                        Err(error) => {
                            return Ok(failed_export(
                                context.engine,
                                request,
                                &prepared.destination,
                                tool,
                                &tool_status,
                                outcome.exit_code,
                                format!("The dump could not be downloaded. {}", error.message),
                                started,
                            ));
                        }
                    };
                    if downloaded != info.size {
                        return Ok(failed_export(
                            context.engine,
                            request,
                            &prepared.destination,
                            tool,
                            &tool_status,
                            outcome.exit_code,
                            format!(
                                "The downloaded dump has {downloaded} bytes but the remote dump has {} bytes.",
                                info.size
                            ),
                            started,
                        ));
                    }
                    if !overwrite && tokio::fs::symlink_metadata(path).await.is_ok() {
                        return Ok(failed_export(
                            context.engine,
                            request,
                            &prepared.destination,
                            tool,
                            &tool_status,
                            outcome.exit_code,
                            "The local destination already exists. Choose another path or allow overwrite."
                                .to_string(),
                            started,
                        ));
                    }
                    if let Err(error) = tokio::fs::rename(partial, path).await {
                        return Ok(failed_export(
                            context.engine,
                            request,
                            &prepared.destination,
                            tool,
                            &tool_status,
                            outcome.exit_code,
                            format!(
                                "The completed dump could not be moved to its local destination. {}",
                                error
                            ),
                            started,
                        ));
                    }
                    let verified = tokio::fs::metadata(path)
                        .await
                        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == info.size);
                    if !verified {
                        return Ok(DumpExportResult {
                            status: DumpStatus::Failed,
                            engine: context.engine,
                            scope: request.scope.clone(),
                            format: request.format,
                            destination_kind: prepared.destination.kind(),
                            destination_path: prepared.destination.path(),
                            bytes_written: downloaded,
                            verified: false,
                            tool,
                            tool_version: tool_status.version.clone(),
                            exit_code: outcome.exit_code,
                            warnings,
                            failure_summary: Some(
                                "The local dump file was written but its final size could not be verified."
                                    .to_string(),
                            ),
                            partial_removed: false,
                            temp_cleaned: false,
                            may_be_incomplete: true,
                            duration_ms: elapsed_ms(started),
                        });
                    }
                    tracker.emit(
                        DumpStatus::Completed,
                        DumpPhase::Finalizing,
                        "The dump completed.",
                        true,
                    );
                    Ok(DumpExportResult {
                        status: DumpStatus::Completed,
                        engine: context.engine,
                        scope: request.scope.clone(),
                        format: request.format,
                        destination_kind: prepared.destination.kind(),
                        destination_path: prepared.destination.path(),
                        bytes_written: downloaded,
                        verified: true,
                        tool,
                        tool_version: tool_status.version.clone(),
                        exit_code: outcome.exit_code,
                        warnings,
                        failure_summary: None,
                        partial_removed: false,
                        temp_cleaned: false,
                        may_be_incomplete: false,
                        duration_ms: elapsed_ms(started),
                    })
                }
            }
        }
        .await;

        let cleanup_report = cleanup.run().await;
        match operation {
            Ok(mut result) => {
                result.temp_cleaned = cleanup_report.work_dir_removed;
                result.partial_removed |= cleanup_report.partial_removed;
                result.warnings.extend(cleanup_report.warnings);
                result.warnings.truncate(MAX_DUMP_REPORTED_WARNINGS);
                Ok(result)
            }
            Err(error) => {
                tracker.emit(
                    DumpStatus::Failed,
                    DumpPhase::Finalizing,
                    "The dump failed before it produced a file.",
                    true,
                );
                Err(error)
            }
        }
    }

    /// Runs one restore. Every post-start failure is returned as a typed
    /// result because a cancelled or failed restore can leave the target
    /// database partially changed.
    #[allow(clippy::too_many_arguments)]
    pub async fn restore(
        &self,
        server_id: &str,
        context: &DumpServerContext,
        connection: &DumpConnection,
        password: &DatabasePassword,
        transfer_id: &str,
        request: &DumpRestoreRequest,
        control: &DumpControl,
        events: Arc<dyn DumpEventSink>,
    ) -> Result<DumpRestoreResult, AppError> {
        validate_restore_request(context, request).map_err(AppError::from)?;
        validate_restore_confirmation(server_id, request).map_err(AppError::from)?;
        if connection.read_only {
            return Err(AppError::permission(
                "The database connection profile is read-only; restoring a dump requires a writable connection profile.",
            )
            .with_details(
                "No statement was executed and no dump file was read. Use a writable database profile to restore data.",
            ));
        }
        let started = Instant::now();
        let tracker =
            DumpProgressTracker::new(transfer_id, DumpDirection::Restore, Arc::clone(&events));
        tracker.emit(
            DumpStatus::Running,
            DumpPhase::Preparing,
            "Checking the remote database tools.",
            true,
        );

        let session = self.remote.open(server_id).await?;
        let report = detect_tools(session.as_ref(), context).await?;
        let tool = restore_tool_for(context.engine, request.format, request.tool)
            .map_err(AppError::from)?;
        let tool_status = require_tool(&report, tool).map_err(AppError::from)?.clone();
        let (paths, source) = self.prepare_restore(session.as_ref(), request).await?;
        session.execute(&mkdir_command(&paths.work_dir)).await?;
        let cleanup = DumpCleanup::new(session.as_ref(), paths.work_dir.clone(), None, None);

        let operation = async {
            if let Err(error) = session
                .write_private_file(&paths.credential, password.expose_secret().as_bytes())
                .await
            {
                return Ok(failed_restore(
                    context.engine,
                    request,
                    &source,
                    tool,
                    &tool_status,
                    None,
                    format!(
                        "The temporary credential file could not be written. {}",
                        error.message
                    ),
                    started,
                ));
            }

            tracker.set_total(Some(source.size()));
            if matches!(source, RestoreSource::Remote { .. }) {
                tracker.set_bytes(source.size());
            }
            if let RestoreSource::Local { path, size } = &source {
                tracker.emit(
                    DumpStatus::Running,
                    DumpPhase::Uploading,
                    "Uploading the dump to the server.",
                    true,
                );
                let cancellation = control.cancellation.clone().unwrap_or_default();
                let uploaded = match session
                    .upload(
                        path,
                        paths.input.as_deref().unwrap_or_default(),
                        *size,
                        &cancellation,
                        &|bytes| {
                            tracker.set_bytes(bytes);
                            tracker.emit(
                                DumpStatus::Running,
                                DumpPhase::Uploading,
                                "Uploading the dump to the server.",
                                false,
                            );
                        },
                    )
                    .await
                {
                    Ok(uploaded) => uploaded,
                    Err(error) if error.code == ErrorCode::Cancelled => {
                        tracker.emit(
                            DumpStatus::Cancelled,
                            DumpPhase::CleaningUp,
                            "Stopping the upload and removing temporary files.",
                            true,
                        );
                        return Ok(DumpRestoreResult {
                            status: DumpStatus::Cancelled,
                            engine: context.engine,
                            format: request.format,
                            source_kind: source.kind(),
                            source_path: source.path(),
                            target_database: request.target_database.as_str().to_string(),
                            bytes_read: tracker.bytes(),
                            tool,
                            tool_version: tool_status.version.clone(),
                            exit_code: None,
                            warnings: Vec::new(),
                            failure_summary: Some(
                                "Cancellation was requested before the restore started. The temporary upload was removed."
                                    .to_string(),
                            ),
                            partial_removed: false,
                            temp_cleaned: false,
                            may_be_incomplete: false,
                            duration_ms: elapsed_ms(started),
                        });
                    }
                    Err(error) => {
                        return Ok(failed_restore(
                            context.engine,
                            request,
                            &source,
                            tool,
                            &tool_status,
                            None,
                            format!("The dump could not be uploaded. {}", error.message),
                            started,
                        ));
                    }
                };
                if uploaded != *size {
                    return Ok(failed_restore(
                        context.engine,
                        request,
                        &source,
                        tool,
                        &tool_status,
                        None,
                        format!(
                            "The uploaded dump has {uploaded} bytes but the local file has {size} bytes."
                        ),
                        started,
                    ));
                }
                let input_path = paths.input.as_deref().unwrap_or_default();
                match session.stat(input_path).await {
                    Ok(Some(info)) if info.is_file && info.size == *size => {}
                    Ok(_) => {
                        return Ok(failed_restore(
                            context.engine,
                            request,
                            &source,
                            tool,
                            &tool_status,
                            None,
                            "The uploaded dump could not be verified on the remote server."
                                .to_string(),
                            started,
                        ));
                    }
                    Err(error) => {
                        return Ok(failed_restore(
                            context.engine,
                            request,
                            &source,
                            tool,
                            &tool_status,
                            None,
                            format!(
                                "The uploaded dump could not be verified on the remote server. {}",
                                error.message
                            ),
                            started,
                        ));
                    }
                }
            }

            let command = match build_restore_command(
                context.engine,
                connection,
                request,
                &paths,
                tool_status.vendor,
            ) {
                Ok(command) => command,
                Err(error) => {
                    return Ok(failed_restore(
                        context.engine,
                        request,
                        &source,
                        tool,
                        &tool_status,
                        None,
                        error.message,
                        started,
                    ));
                }
            };

            tracker.emit(
                DumpStatus::Running,
                DumpPhase::Restoring,
                "Applying the dump to the database.",
                true,
            );
            let outcome = self
                .run_command(
                    session.as_ref(),
                    &command,
                    &paths,
                    None,
                    control,
                    &tracker,
                    DumpPhase::Restoring,
                )
                .await;
            let warnings = safe_warnings(&outcome.stderr, password.expose_secret());

            if outcome.cancelled {
                tracker.set_incomplete(true);
                tracker.emit(
                    DumpStatus::Cancelled,
                    DumpPhase::CleaningUp,
                    "Stopping the restore and removing temporary files.",
                    true,
                );
                return Ok(DumpRestoreResult {
                    status: DumpStatus::Cancelled,
                    engine: context.engine,
                    format: request.format,
                    source_kind: source.kind(),
                    source_path: source.path(),
                    target_database: request.target_database.as_str().to_string(),
                    bytes_read: tracker.bytes(),
                    tool,
                    tool_version: tool_status.version.clone(),
                    exit_code: outcome.exit_code,
                    warnings,
                    failure_summary: Some(
                        "Cancellation was requested. The restore process was stopped, but the target database may contain partially restored objects."
                            .to_string(),
                    ),
                    partial_removed: false,
                    temp_cleaned: false,
                    may_be_incomplete: true,
                    duration_ms: elapsed_ms(started),
                });
            }

            if outcome.timed_out || outcome.transport_error.is_some() || outcome.exit_code != Some(0)
            {
                tracker.set_incomplete(true);
                tracker.emit(
                    DumpStatus::Failed,
                    DumpPhase::CleaningUp,
                    "Cleaning up the failed restore.",
                    true,
                );
                return Ok(DumpRestoreResult {
                    status: DumpStatus::Failed,
                    engine: context.engine,
                    format: request.format,
                    source_kind: source.kind(),
                    source_path: source.path(),
                    target_database: request.target_database.as_str().to_string(),
                    bytes_read: tracker.bytes(),
                    tool,
                    tool_version: tool_status.version.clone(),
                    exit_code: outcome.exit_code,
                    warnings,
                    failure_summary: Some(failure_summary(
                        tool,
                        &outcome,
                        password.expose_secret(),
                    )),
                    partial_removed: false,
                    temp_cleaned: false,
                    may_be_incomplete: true,
                    duration_ms: elapsed_ms(started),
                });
            }

            tracker.emit(
                DumpStatus::Completed,
                DumpPhase::Finalizing,
                "The restore completed.",
                true,
            );
            Ok(DumpRestoreResult {
                status: DumpStatus::Completed,
                engine: context.engine,
                format: request.format,
                source_kind: source.kind(),
                source_path: source.path(),
                target_database: request.target_database.as_str().to_string(),
                bytes_read: tracker.bytes().max(source.size()),
                tool,
                tool_version: tool_status.version.clone(),
                exit_code: outcome.exit_code,
                warnings,
                failure_summary: None,
                partial_removed: false,
                temp_cleaned: false,
                may_be_incomplete: false,
                duration_ms: elapsed_ms(started),
            })
        }
        .await;

        let cleanup_report = cleanup.run().await;
        match operation {
            Ok(mut result) => {
                result.temp_cleaned = cleanup_report.work_dir_removed;
                result.partial_removed |= cleanup_report.partial_removed;
                result.warnings.extend(cleanup_report.warnings);
                result.warnings.truncate(MAX_DUMP_REPORTED_WARNINGS);
                Ok(result)
            }
            Err(error) => {
                tracker.emit(
                    DumpStatus::Failed,
                    DumpPhase::Finalizing,
                    "The restore failed before it started.",
                    true,
                );
                Err(error)
            }
        }
    }

    async fn prepare_export(
        &self,
        session: &dyn DumpRemoteSession,
        transfer_id: &str,
        request: &DumpExportRequest,
    ) -> Result<PreparedExport, AppError> {
        let work_dir = remote_work_dir();
        let mut paths = DumpPaths::new(work_dir.clone());
        let destination = match &request.destination {
            DumpDestination::Remote { path, overwrite } => {
                let normalized = crate::core::projects::normalize_remote_path(path)?;
                if normalized.as_str() == "/" {
                    return Err(AppError::validation(
                        "The remote dump destination must be a file path, not the root directory.",
                    ));
                }
                let parent = remote_parent(normalized.as_str()).ok_or_else(|| {
                    AppError::validation(
                        "The remote dump destination must have a parent directory.",
                    )
                })?;
                let partial = remote_partial_path(normalized.as_str(), transfer_id);
                let probe = session
                    .execute(&probe_remote_destination_command(
                        parent,
                        normalized.as_str(),
                        &partial,
                    ))
                    .await?;
                match probe.stdout.trim() {
                    "__parent_missing__" => {
                        return Err(AppError::validation(
                            "The remote dump destination directory does not exist.",
                        ));
                    }
                    "__parent_unwritable__" => {
                        return Err(AppError::permission(
                            "The SSH account cannot write to the remote dump destination directory.",
                        ));
                    }
                    "__partial_exists__" => {
                        return Err(AppError::conflict(
                            "A partial dump from an earlier attempt already exists at the destination.",
                        ));
                    }
                    "__dest_directory__" => {
                        return Err(AppError::validation(
                            "The remote dump destination is a directory, not a file.",
                        ));
                    }
                    "__dest_exists__" if !overwrite => {
                        return Err(AppError::conflict(
                            "The remote dump destination already exists. Choose another path or allow overwrite.",
                        ));
                    }
                    "__dest_exists__" | "__dest_missing__" => {}
                    other => {
                        return Err(AppError::internal(format!(
                            "The remote dump destination could not be checked: {other}"
                        )));
                    }
                }
                paths.output = Some(partial.clone());
                PreparedDestination::Remote {
                    path: normalized.as_str().to_string(),
                    overwrite: *overwrite,
                    partial,
                }
            }
            DumpDestination::Local { path, overwrite } => {
                let normalized = crate::core::files::normalize_local_path(path)?;
                let parent = normalized.parent().ok_or_else(|| {
                    AppError::validation("The local dump destination must have a parent directory.")
                })?;
                if !parent.is_dir() {
                    return Err(AppError::validation(
                        "The local dump destination directory does not exist.",
                    ));
                }
                if let Ok(metadata) = tokio::fs::symlink_metadata(&normalized).await {
                    if metadata.file_type().is_symlink() {
                        return Err(AppError::validation(
                            "The local dump destination cannot be a symbolic link.",
                        ));
                    }
                    if metadata.is_dir() {
                        return Err(AppError::validation(
                            "The local dump destination is a directory, not a file.",
                        ));
                    }
                    if !overwrite {
                        return Err(AppError::conflict(
                            "The local dump destination already exists. Choose another path or allow overwrite.",
                        ));
                    }
                }
                let partial = local_partial_path(&normalized, transfer_id);
                if tokio::fs::symlink_metadata(&partial).await.is_ok() {
                    return Err(AppError::conflict(
                        "A partial dump from an earlier attempt already exists next to the destination.",
                    ));
                }
                paths.output = Some(format!("{work_dir}/dump.partial"));
                PreparedDestination::Local {
                    path: normalized,
                    overwrite: *overwrite,
                    partial,
                }
            }
        };
        Ok(PreparedExport { paths, destination })
    }

    async fn prepare_restore(
        &self,
        session: &dyn DumpRemoteSession,
        request: &DumpRestoreRequest,
    ) -> Result<(DumpPaths, RestoreSource), AppError> {
        let work_dir = remote_work_dir();
        let mut paths = DumpPaths::new(work_dir.clone());
        let source = match &request.source {
            DumpSource::Remote { path } => {
                let normalized = crate::core::projects::normalize_remote_path(path)?;
                let info = session
                    .stat(normalized.as_str())
                    .await?
                    .ok_or_else(|| AppError::not_found("The remote dump source does not exist."))?;
                if !info.is_file {
                    return Err(AppError::validation(
                        "The remote dump source is not a regular file.",
                    ));
                }
                if info.size < MIN_DUMP_BYTES {
                    return Err(AppError::validation(
                        "The remote dump source is empty or too small to be a valid dump.",
                    ));
                }
                paths.input = Some(normalized.as_str().to_string());
                RestoreSource::Remote {
                    path: normalized.as_str().to_string(),
                    size: info.size,
                }
            }
            DumpSource::Local { path } => {
                let normalized = crate::core::files::normalize_local_path(path)?;
                let metadata = tokio::fs::metadata(&normalized)
                    .await
                    .map_err(|_| AppError::not_found("The local dump source does not exist."))?;
                if !metadata.is_file() {
                    return Err(AppError::validation(
                        "The local dump source is not a regular file.",
                    ));
                }
                if metadata.len() < MIN_DUMP_BYTES {
                    return Err(AppError::validation(
                        "The local dump source is empty or too small to be a valid dump.",
                    ));
                }
                let input = format!("{work_dir}/restore.input");
                paths.input = Some(input);
                RestoreSource::Local {
                    path: normalized,
                    size: metadata.len(),
                }
            }
        };
        Ok((paths, source))
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_command(
        &self,
        session: &dyn DumpRemoteSession,
        command: &str,
        paths: &DumpPaths,
        output_path: Option<&str>,
        control: &DumpControl,
        tracker: &DumpProgressTracker,
        phase: DumpPhase,
    ) -> CommandOutcome {
        let mut run = Box::pin(session.execute_long(command, control.timeout));
        let mut ticker = tokio::time::interval(self.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut pid: Option<u32> = None;
        let mut term_sent = false;
        let mut kill_sent = false;
        let mut kill_at: Option<Instant> = None;
        let mut stop_at: Option<Instant> = None;

        loop {
            tokio::select! {
                result = &mut run => {
                    let error_log = read_error_log(session, &paths.error_log).await;
                    return match result {
                        Ok(output) => CommandOutcome {
                            exit_code: output.exit_code,
                            cancelled: control.is_cancelled(),
                            timed_out: false,
                            transport_error: None,
                            stderr: combine_stderr(&output.stderr, &error_log),
                        },
                        Err(error) => {
                            let timed_out = error.code == ErrorCode::Timeout;
                            if timed_out
                                && let Some(pid) = read_pid(session, &paths.pid).await
                            {
                                let _ = session.execute(&kill_command("KILL", pid)).await;
                            }
                            CommandOutcome {
                                exit_code: None,
                                cancelled: control.is_cancelled(),
                                timed_out,
                                transport_error: Some(error.message),
                                stderr: error_log,
                            }
                        }
                    };
                }
                _ = ticker.tick() => {
                    if let Some(path) = output_path
                        && let Ok(Some(info)) = session.stat(path).await
                        && info.is_file
                    {
                        tracker.emit(
                            DumpStatus::Running,
                            phase,
                            match phase {
                                DumpPhase::Dumping => "Writing the dump on the server.",
                                _ => "Applying the dump to the database.",
                            },
                            false,
                        );
                        tracker.set_bytes(info.size);
                    }

                    if control.is_cancelled() {
                        if !term_sent {
                            term_sent = true;
                            pid = read_pid(session, &paths.pid).await;
                            if let Some(pid) = pid {
                                let _ = session.execute(&kill_command("TERM", pid)).await;
                            }
                            kill_at = Some(Instant::now() + self.cancel_grace);
                            stop_at = Some(Instant::now() + self.cancel_grace + self.kill_grace);
                            tracker.emit(
                                DumpStatus::Running,
                                DumpPhase::CleaningUp,
                                "Stopping the remote process.",
                                true,
                            );
                        } else {
                            if pid.is_none() {
                                pid = read_pid(session, &paths.pid).await;
                            }
                            if let Some(deadline) = kill_at
                                && !kill_sent
                                && Instant::now() >= deadline
                            {
                                if let Some(pid) = pid {
                                    let _ = session.execute(&kill_command("KILL", pid)).await;
                                }
                                kill_sent = true;
                            }
                            if let Some(deadline) = stop_at
                                && Instant::now() >= deadline
                            {
                                let stderr = read_error_log(session, &paths.error_log).await;
                                return CommandOutcome {
                                    exit_code: None,
                                    cancelled: true,
                                    timed_out: false,
                                    transport_error: Some(
                                        "The remote process did not stop after cancellation."
                                            .to_string(),
                                    ),
                                    stderr,
                                };
                            }
                        }
                    }
                }
            }
        }
    }
}

fn build_export_command(
    engine: DatabaseEngine,
    connection: &DumpConnection,
    request: &DumpExportRequest,
    paths: &DumpPaths,
    vendor: DumpToolVendor,
) -> Result<String, DatabaseError> {
    match engine {
        DatabaseEngine::Mysql => super::mysql::dump::export_command(
            connection,
            &request.scope,
            request.format,
            paths,
            vendor,
        ),
        DatabaseEngine::Postgresql => super::postgres::dump::export_command(
            connection,
            &request.scope,
            request.format,
            request.no_owner,
            paths,
            vendor,
        ),
    }
}

fn build_restore_command(
    engine: DatabaseEngine,
    connection: &DumpConnection,
    request: &DumpRestoreRequest,
    paths: &DumpPaths,
    vendor: DumpToolVendor,
) -> Result<String, DatabaseError> {
    match engine {
        DatabaseEngine::Mysql => super::mysql::dump::restore_command(
            connection,
            request.format,
            &request.target_database,
            paths,
            vendor,
        ),
        DatabaseEngine::Postgresql => super::postgres::dump::restore_command(
            connection,
            request.format,
            &request.target_database,
            request.clean,
            request.no_owner,
            paths,
            vendor,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn failed_export(
    engine: DatabaseEngine,
    request: &DumpExportRequest,
    destination: &PreparedDestination,
    tool: DumpTool,
    tool_status: &DumpToolStatus,
    exit_code: Option<u32>,
    summary: String,
    started: Instant,
) -> DumpExportResult {
    DumpExportResult {
        status: DumpStatus::Failed,
        engine,
        scope: request.scope.clone(),
        format: request.format,
        destination_kind: destination.kind(),
        destination_path: destination.path(),
        bytes_written: 0,
        verified: false,
        tool,
        tool_version: tool_status.version.clone(),
        exit_code,
        warnings: Vec::new(),
        failure_summary: Some(sanitize_text_bounded(&summary, MAX_DUMP_FAILURE_CHARS)),
        partial_removed: false,
        temp_cleaned: false,
        may_be_incomplete: false,
        duration_ms: elapsed_ms(started),
    }
}

#[allow(clippy::too_many_arguments)]
fn failed_restore(
    engine: DatabaseEngine,
    request: &DumpRestoreRequest,
    source: &RestoreSource,
    tool: DumpTool,
    tool_status: &DumpToolStatus,
    exit_code: Option<u32>,
    summary: String,
    started: Instant,
) -> DumpRestoreResult {
    DumpRestoreResult {
        status: DumpStatus::Failed,
        engine,
        format: request.format,
        source_kind: source.kind(),
        source_path: source.path(),
        target_database: request.target_database.as_str().to_string(),
        bytes_read: 0,
        tool,
        tool_version: tool_status.version.clone(),
        exit_code,
        warnings: Vec::new(),
        failure_summary: Some(sanitize_text_bounded(&summary, MAX_DUMP_FAILURE_CHARS)),
        partial_removed: false,
        temp_cleaned: false,
        may_be_incomplete: true,
        duration_ms: elapsed_ms(started),
    }
}

async fn read_error_log(session: &dyn DumpRemoteSession, path: &str) -> String {
    match session.read_bounded(path, MAX_DUMP_ERROR_BYTES).await {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        _ => String::new(),
    }
}

fn combine_stderr(channel_stderr: &str, error_log: &str) -> String {
    match (
        channel_stderr.trim().is_empty(),
        error_log.trim().is_empty(),
    ) {
        (true, true) => String::new(),
        (true, false) => error_log.to_string(),
        (false, true) => channel_stderr.to_string(),
        (false, false) => format!("{channel_stderr}\n{error_log}"),
    }
}

async fn read_pid(session: &dyn DumpRemoteSession, path: &str) -> Option<u32> {
    let bytes = session.read_bounded(path, 64).await.ok().flatten()?;
    let text = String::from_utf8_lossy(&bytes);
    let digits = text
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    if digits.is_empty() || digits.len() > 10 {
        return None;
    }
    let pid = digits.parse::<u32>().ok()?;
    (pid > 1 && pid <= MAX_PID).then_some(pid)
}

fn failure_summary(tool: DumpTool, outcome: &CommandOutcome, secret: &str) -> String {
    let mut parts = Vec::new();
    if outcome.timed_out {
        parts.push(format!(
            "{} did not finish within the configured timeout and was stopped.",
            tool.binary()
        ));
    }
    if let Some(error) = &outcome.transport_error {
        parts.push(error.clone());
    }
    if let Some(code) = outcome.exit_code {
        parts.push(format!("{} exited with status {code}.", tool.binary()));
    } else if !outcome.timed_out && outcome.transport_error.is_none() {
        parts.push(format!("{} did not report an exit status.", tool.binary()));
    }
    let stderr = sanitize_dump_text(&outcome.stderr, secret);
    let detail = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    if !detail.is_empty() {
        parts.push(detail);
    }
    sanitize_text_bounded(&parts.join(" "), MAX_DUMP_FAILURE_CHARS)
}

fn safe_warnings(stderr: &str, secret: &str) -> Vec<String> {
    sanitize_dump_text(stderr, secret)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MAX_DUMP_REPORTED_WARNINGS)
        .map(|line| sanitize_text_bounded(line, MAX_DUMP_WARNING_CHARS))
        .collect()
}

fn sanitize_dump_text(text: &str, secret: &str) -> String {
    let replaced = if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "<redacted>")
    };
    sanitize_text_bounded(&replaced, MAX_DUMP_ERROR_BYTES)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// Builds a remote path for tests and command construction. It is not a
/// general-purpose path helper; callers pass validated segments.
pub fn join_remote(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::errors::DatabaseErrorKind;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct FakeState {
        tool_versions: HashMap<&'static str, Option<String>>,
        commands: Vec<String>,
        writes: Vec<(String, Vec<u8>)>,
        output_size: u64,
        exit_code: Option<u32>,
        stderr: String,
        probe_result: String,
        wait_for_kill: bool,
        killed: AtomicBool,
        download_fails: bool,
        uploaded: Option<Vec<u8>>,
        delay: Duration,
        removed: Vec<String>,
    }

    struct FakeRemote {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeRemote {
        fn new() -> Self {
            let mut tool_versions = HashMap::new();
            for tool in DumpTool::ALL {
                let version = match tool {
                    DumpTool::Mysqldump => "mysqldump  Ver 8.0.36 for Linux on x86_64",
                    DumpTool::Mysql => "mysql  Ver 8.0.36 for Linux on x86_64",
                    DumpTool::PgDump => "pg_dump (PostgreSQL) 16.2",
                    DumpTool::PgDumpall => "pg_dumpall (PostgreSQL) 16.2",
                    DumpTool::PgRestore => "pg_restore (PostgreSQL) 16.2",
                    DumpTool::Psql => "psql (PostgreSQL) 16.2",
                };
                tool_versions.insert(tool.binary(), Some(version.to_string()));
            }
            Self {
                state: Arc::new(Mutex::new(FakeState {
                    tool_versions,
                    probe_result: "__dest_missing__".to_string(),
                    ..FakeState::default()
                })),
            }
        }

        fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }

        fn set_missing(&self, binary: &'static str) {
            self.state().tool_versions.insert(binary, None);
        }

        fn set_version(&self, binary: &'static str, version: &str) {
            self.state()
                .tool_versions
                .insert(binary, Some(version.to_string()));
        }

        fn set_output_size(&self, size: u64) {
            self.state().output_size = size;
        }

        fn set_exit(&self, code: Option<u32>, stderr: &str) {
            let mut state = self.state();
            state.exit_code = code;
            state.stderr = stderr.to_string();
        }

        fn set_wait_for_kill(&self) {
            self.state().wait_for_kill = true;
        }

        fn commands(&self) -> Vec<String> {
            self.state().commands.clone()
        }

        fn writes(&self) -> Vec<(String, Vec<u8>)> {
            self.state().writes.clone()
        }

        fn killed(&self) -> bool {
            self.state().killed.load(Ordering::SeqCst)
        }

        fn set_download_fails(&self) {
            self.state().download_fails = true;
        }

        fn uploaded(&self) -> Option<Vec<u8>> {
            self.state().uploaded.clone()
        }
    }

    #[async_trait]
    impl DumpRemote for FakeRemote {
        async fn open(&self, _server_id: &str) -> Result<Box<dyn DumpRemoteSession>, AppError> {
            Ok(Box::new(FakeSession {
                state: Arc::clone(&self.state),
            }))
        }
    }

    struct FakeSession {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeSession {
        fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }
    }

    #[async_trait]
    impl DumpRemoteSession for FakeSession {
        async fn execute(&self, command: &str) -> Result<RemoteCommandOutput, AppError> {
            let mut state = self.state();
            state.commands.push(command.to_string());

            if let Some(binary) = command.strip_prefix("command -v ") {
                let available = state.tool_versions.get(binary).is_some_and(Option::is_some);
                return Ok(RemoteCommandOutput {
                    stdout: if available {
                        format!("/usr/bin/{binary}")
                    } else {
                        String::new()
                    },
                    stderr: String::new(),
                    exit_code: Some(if available { 0 } else { 1 }),
                });
            }
            if let Some(binary) = command.strip_suffix(" --version") {
                let version = state
                    .tool_versions
                    .get(binary)
                    .cloned()
                    .flatten()
                    .unwrap_or_default();
                return Ok(RemoteCommandOutput {
                    stdout: version,
                    stderr: String::new(),
                    exit_code: Some(0),
                });
            }
            if command.starts_with("if [ ! -d ") {
                return Ok(RemoteCommandOutput {
                    stdout: state.probe_result.clone(),
                    stderr: String::new(),
                    exit_code: Some(0),
                });
            }
            if command.starts_with("kill ") {
                state.killed.store(true, Ordering::SeqCst);
                return Ok(RemoteCommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: Some(0),
                });
            }
            if command.starts_with("rm -rf ") || command.starts_with("rm -f ") {
                state.removed.push(command.to_string());
                return Ok(RemoteCommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: Some(0),
                });
            }
            Ok(RemoteCommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(0),
            })
        }

        async fn execute_long(
            &self,
            command: &str,
            _timeout: Duration,
        ) -> Result<RemoteCommandOutput, AppError> {
            self.state().commands.push(command.to_string());
            let delay = self.state().delay;
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if self.state().wait_for_kill {
                loop {
                    if self.state().killed.load(Ordering::SeqCst) {
                        return Ok(RemoteCommandOutput {
                            stdout: String::new(),
                            stderr: String::new(),
                            exit_code: Some(143),
                        });
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            }
            let state = self.state();
            let exit_code = state.exit_code.unwrap_or(0);
            Ok(RemoteCommandOutput {
                stdout: String::new(),
                stderr: state.stderr.clone(),
                exit_code: Some(exit_code),
            })
        }

        async fn write_private_file(&self, path: &str, bytes: &[u8]) -> Result<(), AppError> {
            self.state().writes.push((path.to_string(), bytes.to_vec()));
            Ok(())
        }

        async fn stat(&self, path: &str) -> Result<Option<RemoteFileInfo>, AppError> {
            let state = self.state();
            if path.ends_with("restore.input") {
                return Ok(state.uploaded.as_ref().map(|bytes| RemoteFileInfo {
                    size: bytes.len() as u64,
                    is_file: true,
                }));
            }
            let looks_like_output = path.ends_with(".part")
                || path.contains("dump.partial")
                || path.ends_with(".sql")
                || path.ends_with(".dump")
                || path.ends_with(".tar")
                || path.contains("backup");
            if looks_like_output {
                Ok(Some(RemoteFileInfo {
                    size: state.output_size,
                    is_file: true,
                }))
            } else {
                Ok(None)
            }
        }

        async fn read_bounded(
            &self,
            path: &str,
            _max_bytes: usize,
        ) -> Result<Option<Vec<u8>>, AppError> {
            let state = self.state();
            if path.ends_with("dump.pid") {
                Ok(Some(b"12345".to_vec()))
            } else if path.ends_with("stderr.log") {
                Ok(Some(state.stderr.as_bytes().to_vec()))
            } else {
                Ok(None)
            }
        }

        async fn download(
            &self,
            _remote_path: &str,
            local_path: &Path,
            total_bytes: u64,
            cancel: &RowQueryCancellation,
            progress: &(dyn Fn(u64) + Send + Sync),
        ) -> Result<u64, AppError> {
            if self.state().download_fails {
                return Err(AppError::internal("fixture download failed"));
            }
            if cancel.is_cancelled() {
                return Err(DatabaseError::cancelled("cancelled").into());
            }
            let bytes = vec![b'x'; usize::try_from(total_bytes).unwrap_or(0)];
            if let Some(parent) = local_path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|_| {
                    AppError::internal("fixture could not create the local partial directory")
                })?;
            }
            tokio::fs::write(local_path, &bytes)
                .await
                .map_err(|_| AppError::internal("fixture could not write the local file"))?;
            progress(total_bytes);
            Ok(total_bytes)
        }

        async fn upload(
            &self,
            local_path: &Path,
            _remote_path: &str,
            _total_bytes: u64,
            cancel: &RowQueryCancellation,
            progress: &(dyn Fn(u64) + Send + Sync),
        ) -> Result<u64, AppError> {
            if cancel.is_cancelled() {
                return Err(DatabaseError::cancelled("cancelled").into());
            }
            let bytes = tokio::fs::read(local_path)
                .await
                .map_err(|_| AppError::internal("fixture could not read the local file"))?;
            self.state().uploaded = Some(bytes.clone());
            progress(bytes.len() as u64);
            Ok(bytes.len() as u64)
        }
    }

    struct RecordingSink {
        progress: Mutex<Vec<DumpProgress>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                progress: Mutex::new(Vec::new()),
            }
        }

        fn progress(&self) -> Vec<DumpProgress> {
            self.progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    impl DumpEventSink for RecordingSink {
        fn emit(&self, progress: &DumpProgress) {
            self.progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(progress.clone());
        }
    }

    fn context(engine: DatabaseEngine) -> DumpServerContext {
        DumpServerContext {
            engine,
            version: match engine {
                DatabaseEngine::Mysql => DatabaseVersion::new(8, 0, 36),
                DatabaseEngine::Postgresql => DatabaseVersion::new(16, 2, 0),
            },
        }
    }

    fn connection(engine: DatabaseEngine) -> DumpConnection {
        DumpConnection {
            engine,
            host: "127.0.0.1".to_string(),
            port: match engine {
                DatabaseEngine::Mysql => 3306,
                DatabaseEngine::Postgresql => 5432,
            },
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            ssl_mode: None,
            read_only: false,
        }
    }

    fn password() -> DatabasePassword {
        DatabasePassword::new("hunter2")
    }

    fn export_request(destination: DumpDestination) -> DumpExportRequest {
        DumpExportRequest {
            scope: DumpScope::Database { database: None },
            format: DumpFormat::PlainSql,
            tool: None,
            destination,
            no_owner: false,
        }
    }

    fn restore_request(source: DumpSource) -> DumpRestoreRequest {
        DumpRestoreRequest {
            source,
            format: DumpFormat::PlainSql,
            tool: None,
            target_database: Identifier::new("app").expect("database"),
            clean: false,
            no_owner: false,
            confirmed: true,
            confirmation: Some(DumpRestoreConfirmation {
                server_id: "server-1".to_string(),
                server_name: "Fixture".to_string(),
                target_database: "app".to_string(),
                replaced_data: restore_replaced_data("app", false),
                acknowledged: true,
            }),
        }
    }

    fn remote_destination() -> DumpDestination {
        DumpDestination::Remote {
            path: "/home/deploy/backup.sql".to_string(),
            overwrite: false,
        }
    }

    fn control() -> DumpControl {
        DumpControl::new(Duration::from_secs(30), None)
    }

    fn fast_service<'a>(remote: &'a dyn DumpRemote) -> DatabaseDumpService<'a> {
        DatabaseDumpService::with_timings(
            remote,
            Duration::from_millis(5),
            Duration::from_millis(20),
            Duration::from_millis(20),
        )
    }

    #[tokio::test]
    async fn detection_reports_missing_and_incompatible_tools() {
        let remote = FakeRemote::new();
        remote.set_missing("mysqldump");
        remote.set_version("mysql", "mysql  Ver 5.7.44 for Linux on x86_64");
        let service = DatabaseDumpService::new(&remote);
        let report = service
            .detect("server-1", &context(DatabaseEngine::Mysql))
            .await
            .expect("report");

        assert!(!report.export_ready);
        assert!(!report.restore_ready);
        let dump = report.status(DumpTool::Mysqldump).expect("dump status");
        assert!(!dump.available);
        assert!(
            dump.detail
                .as_deref()
                .unwrap_or_default()
                .contains("Not installed")
        );
        let client = report.status(DumpTool::Mysql).expect("client status");
        assert!(client.available);
        assert!(!client.compatible);
        assert!(
            client
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("older")
        );
    }

    #[tokio::test]
    async fn missing_export_tool_is_rejected_before_any_transfer() {
        let remote = FakeRemote::new();
        remote.set_missing("mysqldump");
        let service = DatabaseDumpService::new(&remote);
        let sink = Arc::new(RecordingSink::new());
        let error = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(remote_destination()),
                &control(),
                sink,
            )
            .await
            .expect_err("missing tool");

        assert_eq!(error.code, ErrorCode::Capability);
        assert!(error.message.contains("mysqldump"));
        assert!(
            !remote
                .commands()
                .iter()
                .any(|command| command.contains("exec "))
        );
    }

    #[tokio::test]
    async fn exports_mysql_dump_to_remote_path_without_secret_in_commands() {
        let remote = FakeRemote::new();
        remote.set_output_size(1024);
        let service = fast_service(&remote);
        let sink = Arc::new(RecordingSink::new());
        let result = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(remote_destination()),
                &control(),
                Arc::clone(&sink) as Arc<dyn DumpEventSink>,
            )
            .await
            .expect("export");

        assert_eq!(result.status, DumpStatus::Completed);
        assert_eq!(result.bytes_written, 1024);
        assert!(result.verified);
        assert!(result.temp_cleaned);
        assert!(result.failure_summary.is_none());
        assert_eq!(result.destination_kind, DumpLocationKind::Remote);

        let commands = remote.commands();
        assert!(
            commands
                .iter()
                .any(|command| command.contains("exec mysqldump"))
        );
        assert!(commands.iter().any(|command| command.contains("mv ")));
        for command in &commands {
            assert!(!command.contains("hunter2"));
        }
        let writes = remote.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].1, b"hunter2");
        assert!(
            sink.progress()
                .iter()
                .any(|progress| progress.status == DumpStatus::Completed)
        );
    }

    #[tokio::test]
    async fn implausibly_small_dump_is_rejected_without_moving_it() {
        let remote = FakeRemote::new();
        remote.set_output_size(4);
        let service = fast_service(&remote);
        let result = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(remote_destination()),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect("export result");

        assert_eq!(result.status, DumpStatus::Failed);
        assert!(!result.verified);
        assert!(result.partial_removed);
        assert!(
            result
                .failure_summary
                .unwrap_or_default()
                .contains("too small")
        );
    }

    #[tokio::test]
    async fn failed_dump_removes_partial_and_redacts_secret() {
        let remote = FakeRemote::new();
        remote.set_output_size(64);
        remote.set_exit(Some(2), "Access denied for password hunter2");
        let service = fast_service(&remote);
        let result = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(remote_destination()),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect("export result");

        assert_eq!(result.status, DumpStatus::Failed);
        assert!(result.partial_removed);
        assert!(result.temp_cleaned);
        let summary = result.failure_summary.expect("summary");
        assert!(summary.contains("exited with status 2"));
        assert!(!summary.contains("hunter2"));
        assert!(
            result
                .warnings
                .iter()
                .all(|warning| !warning.contains("hunter2"))
        );
    }

    #[tokio::test]
    async fn cancelled_dump_stops_process_and_cleans_up() {
        let remote = Arc::new(FakeRemote::new());
        remote.set_output_size(128);
        remote.set_wait_for_kill();
        let cancellation = RowQueryCancellation::default();
        let control = DumpControl::new(Duration::from_secs(30), Some(cancellation.clone()));
        let sink = Arc::new(RecordingSink::new());
        let task = {
            let remote = Arc::clone(&remote);
            tokio::spawn(async move {
                let service = fast_service(remote.as_ref());
                service
                    .export(
                        "server-1",
                        &context(DatabaseEngine::Mysql),
                        &connection(DatabaseEngine::Mysql),
                        &password(),
                        "dump-1",
                        &export_request(remote_destination()),
                        &control,
                        sink,
                    )
                    .await
            })
        };

        tokio::time::sleep(Duration::from_millis(30)).await;
        cancellation.cancel();
        let result = task.await.expect("task").expect("export result");

        assert_eq!(result.status, DumpStatus::Cancelled);
        assert!(remote.killed());
        assert!(result.partial_removed);
        assert!(result.temp_cleaned);
        assert!(
            result
                .failure_summary
                .unwrap_or_default()
                .contains("Cancellation")
        );
    }

    #[tokio::test]
    async fn local_destination_downloads_and_verifies() {
        let remote = FakeRemote::new();
        remote.set_output_size(512);
        let directory = tempfile::tempdir().expect("temp dir");
        let destination = directory.path().join("backup.sql");
        let service = fast_service(&remote);
        let result = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(DumpDestination::Local {
                    path: destination.to_string_lossy().into_owned(),
                    overwrite: false,
                }),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect("export");

        assert_eq!(result.status, DumpStatus::Completed);
        assert!(result.verified);
        assert_eq!(result.bytes_written, 512);
        assert_eq!(
            tokio::fs::metadata(&destination)
                .await
                .expect("destination")
                .len(),
            512
        );
        assert!(result.temp_cleaned);
    }

    #[tokio::test]
    async fn local_download_failure_removes_partial() {
        let remote = FakeRemote::new();
        remote.set_output_size(512);
        remote.set_download_fails();
        let directory = tempfile::tempdir().expect("temp dir");
        let destination = directory.path().join("backup.sql");
        let service = fast_service(&remote);
        let result = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(DumpDestination::Local {
                    path: destination.to_string_lossy().into_owned(),
                    overwrite: false,
                }),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect("export");

        assert_eq!(result.status, DumpStatus::Failed);
        assert!(!destination.exists());
        assert!(result.temp_cleaned);
    }

    #[tokio::test]
    async fn restore_requires_explicit_confirmation_before_any_command() {
        let remote = FakeRemote::new();
        let service = DatabaseDumpService::new(&remote);
        let request = DumpRestoreRequest {
            source: DumpSource::Remote {
                path: "/home/deploy/backup.sql".to_string(),
            },
            format: DumpFormat::PlainSql,
            tool: None,
            target_database: Identifier::new("app").expect("database"),
            clean: false,
            no_owner: false,
            confirmed: false,
            confirmation: Some(DumpRestoreConfirmation {
                server_id: "server-1".to_string(),
                server_name: "Fixture".to_string(),
                target_database: "app".to_string(),
                replaced_data: restore_replaced_data("app", false),
                acknowledged: true,
            }),
        };
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-1",
                &request,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("confirmation required");

        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.message.contains("confirmation"));
        assert!(remote.commands().is_empty());
    }

    #[tokio::test]
    async fn read_only_profile_cannot_restore_before_any_command() {
        let remote = FakeRemote::new();
        let service = DatabaseDumpService::new(&remote);
        let mut connection = connection(DatabaseEngine::Postgresql);
        connection.read_only = true;
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection,
                &password(),
                "restore-read-only",
                &restore_request(DumpSource::Remote {
                    path: "/home/deploy/backup.sql".to_string(),
                }),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("read-only restore must fail");

        assert_eq!(error.code, ErrorCode::Permission);
        assert!(error.message.contains("read-only"));
        assert!(remote.commands().is_empty());
    }

    #[test]
    fn dump_database_names_cannot_smuggle_a_connection_string() {
        let hostile = "host=attacker port=5432 dbname=postgres";
        let database = Identifier::new(hostile).expect("identifier accepts the payload");
        let connection = DumpConnection {
            database: Some(hostile.to_string()),
            ..connection(DatabaseEngine::Postgresql)
        };
        let context = context(DatabaseEngine::Postgresql);

        let export = DumpExportRequest {
            scope: DumpScope::Database {
                database: Some(database.clone()),
            },
            format: DumpFormat::PlainSql,
            tool: None,
            destination: remote_destination(),
            no_owner: false,
        };
        let error = validate_export_request(&context, &connection, &export)
            .expect_err("conninfo database must fail closed");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);
        assert!(error.message.contains("connection string"));

        let cluster = DumpExportRequest {
            scope: DumpScope::Cluster,
            ..export
        };
        assert!(
            validate_export_request(&context, &connection, &cluster).is_err(),
            "cluster dump uses the profile database too"
        );

        let mut restore = restore_request(DumpSource::Remote {
            path: "/home/deploy/backup.sql".to_string(),
        });
        restore.target_database = database;
        restore
            .confirmation
            .as_mut()
            .expect("confirmation")
            .target_database = hostile.to_string();
        restore
            .confirmation
            .as_mut()
            .expect("confirmation")
            .replaced_data = restore_replaced_data(hostile, false);
        let error = validate_restore_request(&context, &restore)
            .expect_err("conninfo restore target must fail closed");
        assert_eq!(error.kind, DatabaseErrorKind::Validation);
        assert!(error.message.contains("connection string"));
    }

    #[tokio::test]
    async fn restores_a_local_plain_sql_dump_and_cleans_up() {
        let remote = FakeRemote::new();
        let directory = tempfile::tempdir().expect("temp dir");
        let source = directory.path().join("backup.sql");
        tokio::fs::write(&source, b"CREATE TABLE t (id int);\n")
            .await
            .expect("source");
        let service = fast_service(&remote);
        let request = DumpRestoreRequest {
            source: DumpSource::Local {
                path: source.to_string_lossy().into_owned(),
            },
            format: DumpFormat::PlainSql,
            tool: None,
            target_database: Identifier::new("app").expect("database"),
            clean: false,
            no_owner: false,
            confirmed: true,
            confirmation: Some(DumpRestoreConfirmation {
                server_id: "server-1".to_string(),
                server_name: "Fixture".to_string(),
                target_database: "app".to_string(),
                replaced_data: restore_replaced_data("app", false),
                acknowledged: true,
            }),
        };
        let result = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-1",
                &request,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect("restore");

        assert_eq!(result.status, DumpStatus::Completed);
        assert_eq!(result.tool, DumpTool::Psql);
        assert_eq!(
            remote.uploaded().expect("uploaded"),
            b"CREATE TABLE t (id int);\n"
        );
        assert!(result.temp_cleaned);
        let commands = remote.commands();
        assert!(commands.iter().any(|command| command.contains("exec psql")));
        for command in &commands {
            assert!(!command.contains("hunter2"));
        }
    }

    #[tokio::test]
    async fn failed_restore_reports_partial_database_state() {
        let remote = FakeRemote::new();
        remote.set_output_size(2048);
        remote.set_exit(Some(1), "relation already exists");
        let service = fast_service(&remote);
        let request = DumpRestoreRequest {
            source: DumpSource::Remote {
                path: "/home/deploy/backup.sql".to_string(),
            },
            format: DumpFormat::PlainSql,
            tool: None,
            target_database: Identifier::new("app").expect("database"),
            clean: false,
            no_owner: false,
            confirmed: true,
            confirmation: Some(DumpRestoreConfirmation {
                server_id: "server-1".to_string(),
                server_name: "Fixture".to_string(),
                target_database: "app".to_string(),
                replaced_data: restore_replaced_data("app", false),
                acknowledged: true,
            }),
        };
        let result = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-1",
                &request,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect("restore result");

        assert_eq!(result.status, DumpStatus::Failed);
        assert!(result.may_be_incomplete);
        assert!(result.temp_cleaned);
        assert!(
            result
                .failure_summary
                .unwrap_or_default()
                .contains("status 1")
        );
    }

    #[tokio::test]
    async fn restore_confirmation_must_name_the_server_database_and_replaced_data() {
        let remote = FakeRemote::new();
        let service = DatabaseDumpService::new(&remote);
        let source = DumpSource::Remote {
            path: "/home/deploy/backup.sql".to_string(),
        };

        let mut wrong_server = restore_request(source.clone());
        if let Some(confirmation) = wrong_server.confirmation.as_mut() {
            confirmation.server_id = "server-2".to_string();
        }
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-1",
                &wrong_server,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("wrong server");
        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.message.contains("different server"));

        let mut wrong_database = restore_request(source.clone());
        if let Some(confirmation) = wrong_database.confirmation.as_mut() {
            confirmation.target_database = "other".to_string();
        }
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-2",
                &wrong_database,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("wrong database");
        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.message.contains("target database"));

        let mut wrong_replaced_data = restore_request(source.clone());
        if let Some(confirmation) = wrong_replaced_data.confirmation.as_mut() {
            confirmation.replaced_data = "some other data".to_string();
        }
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-3",
                &wrong_replaced_data,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("wrong replaced data");
        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.message.contains("replaced"));

        let mut unacknowledged = restore_request(source);
        if let Some(confirmation) = unacknowledged.confirmation.as_mut() {
            confirmation.acknowledged = false;
        }
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-4",
                &unacknowledged,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("not acknowledged");
        assert_eq!(error.code, ErrorCode::Validation);
        assert!(remote.commands().is_empty());
    }

    #[tokio::test]
    async fn restore_preview_names_server_database_tool_and_replaced_data() {
        let remote = FakeRemote::new();
        remote.set_output_size(2048);
        let service = fast_service(&remote);
        let request = DumpRestorePreviewRequest {
            source: DumpSource::Remote {
                path: "/home/deploy/backup.sql".to_string(),
            },
            format: DumpFormat::PlainSql,
            tool: None,
            target_database: Identifier::new("app").expect("database"),
            clean: false,
            no_owner: false,
        };
        let preview = service
            .preview_restore(
                "server-1",
                "Production",
                "profile-1",
                "Primary",
                &context(DatabaseEngine::Postgresql),
                &request,
            )
            .await
            .expect("preview");

        assert_eq!(preview.server_id, "server-1");
        assert_eq!(preview.server_name, "Production");
        assert_eq!(preview.profile_name, "Primary");
        assert_eq!(preview.target_database, "app");
        assert_eq!(preview.tool, DumpTool::Psql);
        assert!(preview.tool_available);
        assert!(preview.tool_compatible);
        assert!(preview.can_restore);
        assert!(preview.replaces_existing_data);
        assert_eq!(preview.replaced_data, restore_replaced_data("app", false));
        assert_eq!(preview.confirmation.server_name, "Production");
        assert_eq!(preview.confirmation.target_database, "app");
        assert!(!preview.confirmation.acknowledged);
        assert!(preview.warnings.is_empty());
    }

    #[tokio::test]
    async fn restore_preview_reports_missing_tools_without_running_them() {
        let remote = FakeRemote::new();
        remote.set_output_size(2048);
        remote.set_missing("psql");
        let service = fast_service(&remote);
        let request = DumpRestorePreviewRequest {
            source: DumpSource::Remote {
                path: "/home/deploy/backup.sql".to_string(),
            },
            format: DumpFormat::PlainSql,
            tool: None,
            target_database: Identifier::new("app").expect("database"),
            clean: false,
            no_owner: false,
        };
        let preview = service
            .preview_restore(
                "server-1",
                "Production",
                "profile-1",
                "Primary",
                &context(DatabaseEngine::Postgresql),
                &request,
            )
            .await
            .expect("preview");

        assert!(!preview.tool_available);
        assert!(!preview.can_restore);
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("not installed"))
        );
        assert!(
            !remote
                .commands()
                .iter()
                .any(|command| command.contains("exec psql"))
        );
    }

    #[test]
    fn export_destination_defaults_to_no_overwrite() {
        let request: DumpExportRequest = serde_json::from_value(serde_json::json!({
            "scope": { "kind": "database", "database": "app" },
            "destination": { "kind": "remote", "path": "/home/deploy/backup.sql" }
        }))
        .expect("request");
        match request.destination {
            DumpDestination::Remote { overwrite, .. } => assert!(!overwrite),
            DumpDestination::Local { .. } => panic!("expected remote destination"),
        }
        assert_eq!(request.tool, None);
    }

    #[test]
    fn explicit_tool_choice_accepts_matching_tools() {
        assert_eq!(
            export_tool_for(
                DatabaseEngine::Postgresql,
                &DumpScope::Cluster,
                DumpFormat::PlainSql,
                Some(DumpTool::PgDumpall)
            )
            .expect("cluster tool"),
            DumpTool::PgDumpall
        );
        assert_eq!(
            export_tool_for(
                DatabaseEngine::Postgresql,
                &DumpScope::Database { database: None },
                DumpFormat::Custom,
                Some(DumpTool::PgDump)
            )
            .expect("database tool"),
            DumpTool::PgDump
        );
        assert_eq!(
            restore_tool_for(
                DatabaseEngine::Postgresql,
                DumpFormat::Custom,
                Some(DumpTool::PgRestore)
            )
            .expect("restore tool"),
            DumpTool::PgRestore
        );
        assert_eq!(
            restore_tool_for(
                DatabaseEngine::Mysql,
                DumpFormat::PlainSql,
                Some(DumpTool::Mysql)
            )
            .expect("mysql restore tool"),
            DumpTool::Mysql
        );
    }

    #[tokio::test]
    async fn explicit_tool_choice_rejects_incompatible_tools_before_execution() {
        let remote = FakeRemote::new();
        remote.set_output_size(1024);
        let service = fast_service(&remote);
        let mut request = export_request(remote_destination());
        request.tool = Some(DumpTool::PgDump);
        let error = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &request,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("wrong export tool");
        assert_eq!(error.code, ErrorCode::Capability);
        assert!(error.message.contains("mysqldump"));
        assert!(
            !remote
                .commands()
                .iter()
                .any(|command| command.contains("exec "))
        );

        let mut restore = restore_request(DumpSource::Remote {
            path: "/home/deploy/backup.sql".to_string(),
        });
        restore.tool = Some(DumpTool::PgDump);
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-1",
                &restore,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("wrong restore tool");
        assert_eq!(error.code, ErrorCode::Capability);
        assert!(error.message.contains("psql"));
    }

    #[tokio::test]
    async fn existing_remote_destination_is_not_overwritten_without_explicit_overwrite() {
        let remote = FakeRemote::new();
        remote.set_output_size(1024);
        remote.state().probe_result = "__dest_exists__".to_string();
        let service = fast_service(&remote);
        let error = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(remote_destination()),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("destination exists");
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(error.message.contains("already exists"));
        assert!(
            !remote
                .commands()
                .iter()
                .any(|command| command.contains("exec mysqldump"))
        );

        let mut request = export_request(DumpDestination::Remote {
            path: "/home/deploy/backup.sql".to_string(),
            overwrite: true,
        });
        request.tool = Some(DumpTool::Mysqldump);
        let result = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-2",
                &request,
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect("overwrite export");
        assert_eq!(result.status, DumpStatus::Completed);
        assert!(result.verified);
    }

    #[tokio::test]
    async fn tiny_restore_source_is_rejected_before_upload() {
        let remote = FakeRemote::new();
        remote.set_output_size(4);
        let service = fast_service(&remote);
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-1",
                &restore_request(DumpSource::Remote {
                    path: "/home/deploy/backup.sql".to_string(),
                }),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("too small");

        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.message.contains("too small"));
        assert!(
            !remote
                .commands()
                .iter()
                .any(|command| command.contains("exec "))
        );
    }

    #[tokio::test]
    async fn existing_local_destination_is_not_overwritten_without_explicit_overwrite() {
        let remote = FakeRemote::new();
        remote.set_output_size(512);
        let directory = tempfile::tempdir().expect("temp dir");
        let destination = directory.path().join("backup.sql");
        tokio::fs::write(&destination, b"existing backup")
            .await
            .expect("existing file");
        let service = fast_service(&remote);
        let error = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(DumpDestination::Local {
                    path: destination.to_string_lossy().into_owned(),
                    overwrite: false,
                }),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("destination exists");

        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(
            tokio::fs::read(&destination).await.expect("existing file"),
            b"existing backup"
        );
        assert!(
            !remote
                .commands()
                .iter()
                .any(|command| command.contains("exec "))
        );
    }

    #[tokio::test]
    async fn empty_dump_is_rejected_as_unverified() {
        let remote = FakeRemote::new();
        remote.set_output_size(0);
        let service = fast_service(&remote);
        let result = service
            .export(
                "server-1",
                &context(DatabaseEngine::Mysql),
                &connection(DatabaseEngine::Mysql),
                &password(),
                "dump-1",
                &export_request(remote_destination()),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect("export result");

        assert_eq!(result.status, DumpStatus::Failed);
        assert!(!result.verified);
        assert!(result.partial_removed);
        assert!(
            result
                .failure_summary
                .unwrap_or_default()
                .contains("too small")
        );
    }

    #[tokio::test]
    async fn cancelled_restore_preserves_partial_database_state_information() {
        let remote = Arc::new(FakeRemote::new());
        remote.set_output_size(128);
        remote.set_wait_for_kill();
        let cancellation = RowQueryCancellation::default();
        let control = DumpControl::new(Duration::from_secs(30), Some(cancellation.clone()));
        let request = restore_request(DumpSource::Remote {
            path: "/home/deploy/backup.sql".to_string(),
        });
        let task = {
            let remote = Arc::clone(&remote);
            tokio::spawn(async move {
                let service = fast_service(remote.as_ref());
                service
                    .restore(
                        "server-1",
                        &context(DatabaseEngine::Postgresql),
                        &connection(DatabaseEngine::Postgresql),
                        &password(),
                        "restore-1",
                        &request,
                        &control,
                        Arc::new(RecordingSink::new()),
                    )
                    .await
            })
        };

        tokio::time::sleep(Duration::from_millis(30)).await;
        cancellation.cancel();
        let result = task.await.expect("task").expect("restore result");

        assert_eq!(result.status, DumpStatus::Cancelled);
        assert!(result.may_be_incomplete);
        assert!(result.temp_cleaned);
        assert!(remote.killed());
        assert!(
            result
                .failure_summary
                .unwrap_or_default()
                .contains("partially restored")
        );
    }

    #[tokio::test]
    async fn incompatible_restore_tool_is_rejected_before_execution() {
        let remote = FakeRemote::new();
        remote.set_output_size(2048);
        remote.set_version("psql", "psql (PostgreSQL) 9.6");
        let service = fast_service(&remote);
        let error = service
            .restore(
                "server-1",
                &context(DatabaseEngine::Postgresql),
                &connection(DatabaseEngine::Postgresql),
                &password(),
                "restore-1",
                &restore_request(DumpSource::Remote {
                    path: "/home/deploy/backup.sql".to_string(),
                }),
                &control(),
                Arc::new(RecordingSink::new()),
            )
            .await
            .expect_err("incompatible tool");

        assert_eq!(error.code, ErrorCode::Capability);
        assert!(error.message.contains("psql"));
        assert!(
            !remote
                .commands()
                .iter()
                .any(|command| command.contains("exec "))
        );
    }

    #[test]
    fn transfer_ids_are_bounded_ascii_keys() {
        assert!(validate_dump_request_id("dump-123").is_ok());
        assert!(validate_dump_request_id("").is_err());
        assert!(validate_dump_request_id("dump 1").is_err());
        assert!(validate_dump_request_id("dump;rm").is_err());
        assert!(validate_dump_request_id(&"a".repeat(MAX_DUMP_REQUEST_ID_LENGTH + 1)).is_err());
    }

    #[test]
    fn progress_percent_is_bounded() {
        let sink = Arc::new(RecordingSink::new());
        let tracker = DumpProgressTracker::new("dump-1", DumpDirection::Export, sink.clone());
        tracker.set_total(Some(200));
        tracker.set_bytes(100);
        tracker.emit(DumpStatus::Running, DumpPhase::Dumping, "working", true);
        let progress = sink.progress();
        assert_eq!(progress.last().expect("progress").percent, Some(50));
    }

    #[test]
    fn postgres_pattern_names_are_rejected_for_schema_and_table_scope() {
        let postgres = connection(DatabaseEngine::Postgresql);
        let postgres_context = context(DatabaseEngine::Postgresql);
        for scope in [
            DumpScope::Schema {
                database: None,
                schema: Identifier::new("pub*").expect("schema"),
            },
            DumpScope::Tables {
                database: None,
                schema: Some(Identifier::new("public").expect("schema")),
                tables: vec![Identifier::new("user?").expect("table")],
            },
        ] {
            assert!(
                validate_export_request(
                    &postgres_context,
                    &postgres,
                    &DumpExportRequest {
                        scope,
                        format: DumpFormat::PlainSql,
                        tool: None,
                        destination: remote_destination(),
                        no_owner: false,
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn scope_validation_rejects_unsupported_combinations() {
        let mysql = connection(DatabaseEngine::Mysql);
        let mysql_context = context(DatabaseEngine::Mysql);
        assert!(
            validate_export_request(
                &mysql_context,
                &mysql,
                &DumpExportRequest {
                    scope: DumpScope::Cluster,
                    format: DumpFormat::PlainSql,
                    tool: None,
                    destination: remote_destination(),
                    no_owner: false,
                }
            )
            .is_err()
        );
        assert!(
            validate_export_request(
                &mysql_context,
                &mysql,
                &DumpExportRequest {
                    scope: DumpScope::Database { database: None },
                    format: DumpFormat::Custom,
                    tool: None,
                    destination: remote_destination(),
                    no_owner: false,
                }
            )
            .is_err()
        );
    }
}
