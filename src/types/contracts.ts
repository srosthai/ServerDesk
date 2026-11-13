export type ErrorCode =
  | "validation"
  | "connection"
  | "authentication"
  | "host_key"
  | "timeout"
  | "dns"
  | "network"
  | "permission"
  | "remote"
  | "tunnel"
  | "database_auth"
  | "query"
  | "capability"
  | "cancelled"
  | "conflict"
  | "not_found"
  | "storage"
  | "credential"
  | "credential_unavailable"
  | "credential_locked"
  | "credential_missing"
  | "credential_invalid"
  | "internal";

export type ErrorScope = {
  object: string;
  name: string;
};

export type CommandError = {
  code: ErrorCode;
  message: string;
  details?: string;
  scope?: ErrorScope;
};

export type AppInfo = {
  appVersion: string;
  databaseSchemaVersion: number;
};

export type ThemePreference = "system" | "light" | "dark";

export type UserPreferences = {
  theme: ThemePreference;
  monitoringIntervalSeconds: number;
  lastActiveServerId: string | null;
};

export type AuthType = "password" | "private_key";

export type ConnectionState =
  "disconnected" | "connecting" | "host-key-verification" | "connected" | "reconnecting" | "error";

export type HostKeyDetails = {
  keyType: string;
  fingerprint: string;
  expectedFingerprint?: string;
};

export type TrustedHostKey = {
  id: string;
  serverId: string;
  keyType: string;
  fingerprint: string;
  firstSeenAt: number;
  verifiedAt: number | null;
};

export type ConnectionSnapshot = {
  serverId: string;
  state: ConnectionState;
  changedAt: number;
  error?: CommandError;
  hostKey?: HostKeyDetails;
};

export type TrustedHostKeyInput = {
  keyType: string;
  fingerprint: string;
};

export type ConnectionTestInput = {
  serverId?: string | null;
  host: string;
  port: number;
  username: string;
  authType: AuthType;
  credentialRef?: string | null;
  secret?: string | null;
  passphrase?: string | null;
  acceptedHostKey?: TrustedHostKeyInput | null;
};

export type ConnectionTestOutcome =
  | { status: "connected"; durationMs: number }
  | { status: "hostKeyVerification"; hostKey: HostKeyDetails };

export type RemoteCommandName = "hostname" | "uptime";

export type RemoteCommandResult = {
  serverId: string;
  command: RemoteCommandName;
  template: string;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  succeeded: boolean;
  durationMs: number;
  connectionState: ConnectionState;
};

export type SystemIdentity = {
  serverId: string;
  hostname: string | null;
  distribution: string | null;
  distributionVersion: string | null;
  kernel: string | null;
  architecture: string | null;
  uptimeSeconds: number | null;
  sampledAt: number;
  durationMs: number;
  connectionState: ConnectionState;
};

export type MetricStatus = "available" | "unsupported" | "unavailable";

export type MetricGroup<T> = {
  status: MetricStatus;
  sampledAt: number;
  data: T | null;
  detail: string | null;
};

export type CpuMetrics = {
  load1m: number;
  load5m: number;
  load15m: number;
  logicalCpus: number | null;
};

export type MemoryMetrics = {
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  usedPercent: number;
};

export type FilesystemMetrics = {
  filesystem: string | null;
  mount: string;
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  usedPercent: number;
};

export type NetworkInterfaceMetrics = {
  interface: string;
  receivedBytes: number;
  transmittedBytes: number;
  receivedPackets: number;
  transmittedPackets: number;
};

export type MonitoringSnapshot = {
  serverId: string;
  sampledAt: number;
  durationMs: number;
  connectionState: ConnectionState;
  cpu: MetricGroup<CpuMetrics>;
  memory: MetricGroup<MemoryMetrics>;
  filesystems: MetricGroup<FilesystemMetrics[]>;
  network: MetricGroup<NetworkInterfaceMetrics[]>;
};

export type ServiceState = "active" | "inactive" | "failed" | "unknown";

export type ServiceDiscoveryStatus = "available" | "unsupported" | "unavailable";

export type ServiceUnit = {
  name: string;
  state: ServiceState;
  activeState: string;
  subState: string | null;
  loadState: string | null;
  description: string | null;
};

export type ServiceDiscovery = {
  serverId: string;
  status: ServiceDiscoveryStatus;
  services: ServiceUnit[];
  detail: string | null;
  truncated: boolean;
  sampledAt: number;
  durationMs: number;
  connectionState: ConnectionState;
};

export type ServiceActionName = "status" | "start" | "stop" | "restart";

export type ServiceStatus = {
  serverId: string;
  unit: string;
  state: ServiceState;
  activeState: string;
  subState: string | null;
  loadState: string | null;
  unitFileState: string | null;
  description: string | null;
  sampledAt: number;
  durationMs: number;
};

export type ServiceActionResult = {
  serverId: string;
  unit: string;
  action: ServiceActionName;
  command: string;
  succeeded: boolean;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  durationMs: number;
  permissionDenied: boolean;
  sudoGuidance: string | null;
  verifiedStatus: ServiceStatus | null;
  stateVerified: boolean | null;
  verificationError: string | null;
  error: CommandError | null;
  connectionState: ConnectionState;
};

export type DatabaseEngine = "mysql" | "postgresql";

export type DatabaseSslMode =
  "disabled" | "preferred" | "required" | "verify-ca" | "verify-identity";

export type DatabaseProfile = {
  id: string;
  serverId: string;
  name: string;
  engine: DatabaseEngine;
  host: string;
  port: number;
  socket: string | null;
  databaseName: string | null;
  username: string;
  hasCredential: boolean;
  readOnly: boolean;
  sslMode: DatabaseSslMode | null;
  createdAt: number;
  updatedAt: number;
  lastUsedAt: number | null;
};

export type DatabaseProfileInput = {
  serverId: string;
  name: string;
  engine: DatabaseEngine;
  host: string;
  port: number;
  socket?: string | null;
  databaseName?: string | null;
  username: string;
  readOnly: boolean;
  sslMode?: DatabaseSslMode | null;
  /** Omit or set null to keep the saved password on update. */
  password?: string | null;
};

export type DatabaseAction =
  | "browse_databases"
  | "browse_schemas"
  | "browse_tables"
  | "browse_columns"
  | "browse_indexes"
  | "browse_constraints"
  | "browse_views"
  | "browse_materialized_views"
  | "browse_sequences"
  | "browse_routines"
  | "browse_triggers"
  | "browse_roles"
  | "browse_sessions"
  | "show_object_size"
  | "create_database"
  | "rename_database"
  | "drop_database"
  | "set_active_database"
  | "create_schema"
  | "drop_schema"
  | "create_table"
  | "rename_table"
  | "duplicate_table"
  | "truncate_table"
  | "drop_table"
  | "add_column"
  | "edit_column"
  | "rename_column"
  | "drop_column"
  | "reorder_columns"
  | "create_index"
  | "rename_index"
  | "drop_index"
  | "create_check_constraint"
  | "create_view"
  | "create_routine"
  | "create_trigger"
  | "manage_roles"
  | "kill_session"
  | "cancel_query"
  | "explain_query"
  | "run_query"
  | "create_generated_column"
  | "create_identity_column"
  | "create_auto_increment_column"
  | "create_partial_index"
  | "refresh_materialized_view"
  | "insert_row"
  | "edit_row"
  | "delete_row";

export type CapabilityFlags = {
  schemas: boolean;
  sequences: boolean;
  materializedViews: boolean;
  routines: boolean;
  triggers: boolean;
  roles: boolean;
  sessions: boolean;
  transactions: boolean;
  savepoints: boolean;
  cancelQuery: boolean;
  explain: boolean;
  checkConstraints: boolean;
  generatedColumns: boolean;
  identityColumns: boolean;
  autoIncrementColumns: boolean;
  columnTypeChange: boolean;
  columnRename: boolean;
  tableRename: boolean;
  tableDuplicate: boolean;
  columnReorder: boolean;
  databaseRename: boolean;
  indexRename: boolean;
  partialIndexes: boolean;
  queryTimeout: boolean;
};

export type DatabaseCapabilities = {
  engine: DatabaseEngine;
  version: string | null;
  flags: CapabilityFlags;
  actions: DatabaseAction[];
};

export type DatabaseVersionInfo = {
  version: string;
  raw: string;
};

export type DatabaseTunnelState = "opening" | "active" | "closed";

export type DatabaseTunnelSnapshot = {
  profileId: string;
  serverId: string;
  state: DatabaseTunnelState;
  localHost: string;
  localPort?: number;
  remoteHost: string;
  remotePort: number;
  socket?: string;
  openedAt: number;
};

export type DatabaseConnectionState =
  "disconnected" | "connecting" | "reconnecting" | "connected" | "error";

export type DatabaseConnectionSnapshot = {
  profileId: string;
  serverId: string;
  name: string;
  engine: DatabaseEngine;
  state: DatabaseConnectionState;
  readOnly: boolean;
  databaseName: string | null;
  version?: DatabaseVersionInfo;
  capabilities?: DatabaseCapabilities;
  tunnel?: DatabaseTunnelSnapshot;
  error?: CommandError;
  changedAt: number;
};

export type DatabaseTestResult = {
  profileId: string;
  serverId: string;
  engine: DatabaseEngine;
  version: DatabaseVersionInfo;
  databaseName: string | null;
  readOnly: boolean;
  capabilities: DatabaseCapabilities;
  latencyMs: number;
  testedAt: number;
};

export type DatabaseMetadataValue<T> =
  { status: "available"; value: T } | { status: "unavailable" };

export type DatabaseObjectMetadata = {
  owner: DatabaseMetadataValue<string>;
  encoding: DatabaseMetadataValue<string>;
  collation: DatabaseMetadataValue<string>;
  sizeBytes: DatabaseMetadataValue<number>;
  engine: DatabaseMetadataValue<string>;
};

export type DatabaseSummary = {
  name: string;
  system: boolean;
} & DatabaseObjectMetadata;

export type DatabaseSchemaSummary = {
  name: string;
  system: boolean;
} & DatabaseObjectMetadata;

export type DatabaseTableKind = "table" | "partitioned_table" | "view" | "foreign_table" | "other";

export type DatabaseViewKind = "view" | "materialized_view";

export type DatabaseTableSummary = {
  database: string | null;
  schema: string | null;
  name: string;
  kind: DatabaseTableKind;
  estimatedRows: DatabaseMetadataValue<number>;
  metadataRefreshedAt: DatabaseMetadataValue<string>;
} & DatabaseObjectMetadata;

export type DatabaseColumnIdentity = "none" | "auto_increment" | "always" | "by_default";

export type DatabaseColumnSummary = {
  name: string;
  dataType: string;
  length: DatabaseMetadataValue<number | null>;
  precision: DatabaseMetadataValue<number | null>;
  scale: DatabaseMetadataValue<number | null>;
  nullable: boolean;
  defaultValue: DatabaseMetadataValue<string | null>;
  ordinalPosition: number;
  primaryKey: boolean;
  identity: DatabaseMetadataValue<DatabaseColumnIdentity>;
  generated: DatabaseMetadataValue<boolean>;
  generationExpression: DatabaseMetadataValue<string | null>;
  comment: DatabaseMetadataValue<string | null>;
  collation: DatabaseMetadataValue<string | null>;
};

export type DatabaseIndexSummary = {
  name: string;
  columns: string[];
  unique: boolean;
  primary: boolean;
  method: DatabaseMetadataValue<string>;
  predicate: DatabaseMetadataValue<string | null>;
};

export type DatabaseConstraintKind = "primary_key" | "unique" | "foreign_key" | "check" | "other";

export type DatabaseConstraintSummary = {
  name: string;
  kind: DatabaseConstraintKind;
  columns: string[];
  referencedSchema: DatabaseMetadataValue<string | null>;
  referencedTable: DatabaseMetadataValue<string | null>;
  referencedColumns: string[];
  onUpdate: DatabaseMetadataValue<string | null>;
  onDelete: DatabaseMetadataValue<string | null>;
  checkExpression: DatabaseMetadataValue<string | null>;
  deferrable: DatabaseMetadataValue<boolean>;
  initiallyDeferred: DatabaseMetadataValue<boolean>;
};

export type DatabaseDependencyKind =
  | "incoming_foreign_key"
  | "dependent_view"
  | "dependent_materialized_view"
  | "owned_sequence"
  | "trigger";

export type DatabaseDependencySummary = {
  kind: DatabaseDependencyKind;
  name: string;
  database: string | null;
  schema: string | null;
  table: string | null;
  columns: string[];
  referencedColumns: string[];
  onUpdate: DatabaseMetadataValue<string | null>;
  onDelete: DatabaseMetadataValue<string | null>;
  details: DatabaseMetadataValue<string>;
};

export type DatabaseBounded<T> = {
  items: T[];
  limit: number;
  truncated: boolean;
};

export type DatabaseTableDependencies = {
  incomingForeignKeys: DatabaseMetadataValue<DatabaseBounded<DatabaseDependencySummary>>;
  dependentViews: DatabaseMetadataValue<DatabaseBounded<DatabaseDependencySummary>>;
  ownedSequences: DatabaseMetadataValue<DatabaseBounded<DatabaseDependencySummary>>;
  triggers: DatabaseMetadataValue<DatabaseBounded<DatabaseDependencySummary>>;
};

export type DatabaseViewSummary = {
  database: string | null;
  schema: string | null;
  name: string;
  kind: DatabaseViewKind;
} & DatabaseObjectMetadata;

export type DatabaseSequenceSummary = {
  database: string | null;
  schema: string | null;
  name: string;
  dataType: DatabaseMetadataValue<string>;
  startValue: DatabaseMetadataValue<string>;
  increment: DatabaseMetadataValue<string>;
} & DatabaseObjectMetadata;

export type DatabaseObjectCounts = {
  tables: DatabaseMetadataValue<number>;
  views: DatabaseMetadataValue<number>;
  materializedViews: DatabaseMetadataValue<number>;
  sequences: DatabaseMetadataValue<number>;
};

export type DatabasePage<T> = {
  items: T[];
  limit: number;
  offset: number;
  hasMore: boolean;
  total?: number;
};

export type DatabaseDiscoveryScope = {
  database?: string | null;
  schema?: string | null;
};

export type DatabaseDiscoveryPage = {
  limit?: number;
  offset?: number;
};

export type DatabaseExplorerNode =
  | { kind: "databases" }
  | { kind: "schemas" }
  | ({ kind: "tables" } & DatabaseDiscoveryScope)
  | ({ kind: "views" } & DatabaseDiscoveryScope)
  | ({ kind: "sequences" } & DatabaseDiscoveryScope)
  | ({ kind: "objectCounts" } & DatabaseDiscoveryScope);

export type DatabaseExplorerNodeKind = DatabaseExplorerNode["kind"];

export type DatabaseExplorerNodePayload =
  | { nodeKind: "databases"; payload: DatabasePage<DatabaseSummary> }
  | { nodeKind: "schemas"; payload: DatabasePage<DatabaseSchemaSummary> }
  | { nodeKind: "tables"; payload: DatabasePage<DatabaseTableSummary> }
  | { nodeKind: "views"; payload: DatabasePage<DatabaseViewSummary> }
  | { nodeKind: "sequences"; payload: DatabasePage<DatabaseSequenceSummary> }
  | { nodeKind: "objectCounts"; payload: DatabaseObjectCounts };

export type DatabaseActionKind = "create" | "rename" | "drop" | "set_active";

export type DatabaseActionRequest =
  | {
      kind: "create";
      name: string;
      encoding?: string | null;
      collation?: string | null;
      owner?: string | null;
      template?: string | null;
    }
  | { kind: "rename"; name: string; newName: string }
  | { kind: "drop"; name: string }
  | { kind: "set_active"; name: string };

export type DatabaseDdlPreview = {
  action: DatabaseActionKind;
  engine: DatabaseEngine;
  database: string;
  targetDatabase?: string | null;
  statements: string[];
  ddl: string;
  warnings: string[];
  destructive: boolean;
  requiresConfirmation: boolean;
};

export type DatabaseActionResult = {
  action: DatabaseActionKind;
  engine: DatabaseEngine;
  database: string;
  previousName?: string | null;
  statements: string[];
  verified: boolean;
  verification: string;
  verificationError?: string | null;
  metadata?: DatabaseSummary | null;
  activeDatabase?: string | null;
  durationMs: number;
};

export type DatabaseColumnDefaultInput =
  | { kind: "null" }
  | { kind: "string"; value: string }
  | { kind: "number"; value: string }
  | { kind: "boolean"; value: boolean }
  | { kind: "current_timestamp" };

export type DatabaseGeneratedColumnInput = {
  expression: string;
  stored: boolean;
};

export type DatabaseColumnDefinitionInput = {
  name: string;
  dataType: string;
  nullable: boolean;
  default?: DatabaseColumnDefaultInput | null;
  comment?: string | null;
  generated?: DatabaseGeneratedColumnInput | null;
  identity?: DatabaseColumnIdentity | null;
};

export type DatabaseColumnPositionInput = { kind: "first" } | { kind: "after"; column: string };

export type DatabaseUniqueKeyInput = {
  name: string;
  columns: string[];
};

export type DatabaseTableOptionsInput = {
  engine?: string | null;
  charset?: string | null;
  collation?: string | null;
  tablespace?: string | null;
  unlogged?: boolean;
  comment?: string | null;
};

export type DatabaseReferentialAction =
  "no_action" | "restrict" | "cascade" | "set_null" | "set_default";

export type DatabaseConstraintInput =
  | { kind: "primary_key"; name: string; columns: string[] }
  | { kind: "unique"; name: string; columns: string[] }
  | {
      kind: "foreign_key";
      name: string;
      columns: string[];
      referencedSchema?: string | null;
      referencedTable: string;
      referencedColumns: string[];
      onDelete?: DatabaseReferentialAction | null;
      onUpdate?: DatabaseReferentialAction | null;
    }
  | { kind: "check"; name: string; expression: string };

type DatabaseStructureTableScope = {
  database?: string | null;
  schema?: string | null;
};

export type DatabaseStructureActionRequest =
  | ({
      kind: "create_table";
      name: string;
      columns: DatabaseColumnDefinitionInput[];
      primaryKey?: string[];
      uniqueKeys?: DatabaseUniqueKeyInput[];
      options?: DatabaseTableOptionsInput | null;
    } & DatabaseStructureTableScope)
  | ({ kind: "rename_table"; name: string; newName: string } & DatabaseStructureTableScope)
  | ({ kind: "duplicate_table"; name: string; newName: string } & DatabaseStructureTableScope)
  | ({ kind: "drop_table"; name: string } & DatabaseStructureTableScope)
  | ({ kind: "truncate_table"; name: string } & DatabaseStructureTableScope)
  | ({
      kind: "add_column";
      table: string;
      column: DatabaseColumnDefinitionInput;
      position?: DatabaseColumnPositionInput | null;
    } & DatabaseStructureTableScope)
  | ({
      kind: "alter_column";
      table: string;
      name: string;
      dataType?: string | null;
      nullable?: boolean | null;
      default?: DatabaseColumnDefaultInput | null;
      dropDefault?: boolean;
      comment?: string | null;
      dropComment?: boolean;
    } & DatabaseStructureTableScope)
  | ({
      kind: "rename_column";
      table: string;
      name: string;
      newName: string;
    } & DatabaseStructureTableScope)
  | ({ kind: "drop_column"; table: string; name: string } & DatabaseStructureTableScope)
  | ({
      kind: "reorder_column";
      table: string;
      column: DatabaseColumnDefinitionInput;
      position: DatabaseColumnPositionInput;
    } & DatabaseStructureTableScope)
  | ({
      kind: "create_index";
      table: string;
      name: string;
      columns: string[];
      unique?: boolean;
      method?: string | null;
      predicate?: string | null;
    } & DatabaseStructureTableScope)
  | ({
      kind: "rename_index";
      table: string;
      name: string;
      newName: string;
    } & DatabaseStructureTableScope)
  | ({ kind: "drop_index"; table: string; name: string } & DatabaseStructureTableScope)
  | ({
      kind: "add_constraint";
      table: string;
      constraint: DatabaseConstraintInput;
    } & DatabaseStructureTableScope)
  | ({
      kind: "drop_constraint";
      table: string;
      name: string;
      constraintKind: DatabaseConstraintKind;
    } & DatabaseStructureTableScope);

export type DatabaseStructureActionKind =
  | "create_table"
  | "rename_table"
  | "duplicate_table"
  | "drop_table"
  | "truncate_table"
  | "add_column"
  | "alter_column"
  | "rename_column"
  | "drop_column"
  | "reorder_column"
  | "create_index"
  | "rename_index"
  | "drop_index"
  | "add_constraint"
  | "drop_constraint";

export type DatabaseStructureObjectKind = "table" | "column" | "index" | "constraint";

export type DatabaseAffectedObject = {
  kind: DatabaseStructureObjectKind;
  database?: string | null;
  schema?: string | null;
  table: string;
  name?: string | null;
};

export type DatabaseStructureDdlPreview = {
  action: DatabaseStructureActionKind;
  engine: DatabaseEngine;
  object: DatabaseAffectedObject;
  statements: string[];
  ddl: string;
  warnings: string[];
  destructive: boolean;
  requiresConfirmation: boolean;
};

export type DatabaseStructureActionResult = {
  action: DatabaseStructureActionKind;
  engine: DatabaseEngine;
  object: DatabaseAffectedObject;
  targetTable?: string | null;
  statements: string[];
  warnings: string[];
  verified: boolean;
  verification: string;
  verificationError?: string | null;
  metadata?: DatabaseTableSummary | null;
  durationMs: number;
};

export type ProjectType = "laravel";

export type OperationCategory = "command" | "service" | "artisan" | "sftp" | "queue" | "database";

export type OperationResult = "success" | "failure" | "cancelled" | "unknown";

export type OperationHistoryEntry = {
  id: string;
  serverId: string | null;
  projectId: string | null;
  category: OperationCategory;
  operation: string;
  safeSummary: string;
  path: string | null;
  outputSummary: string | null;
  result: OperationResult;
  exitCode: number | null;
  startedAt: number;
  durationMs: number;
};

export type OperationHistoryFilter = {
  serverId?: string | null;
  category?: OperationCategory | null;
  result?: OperationResult | null;
  startedAfter?: number | null;
  startedBefore?: number | null;
  limit?: number;
};

export type OperationHistoryPolicy = {
  maxEntries: number;
  outputSummaryMaxChars: number;
  retention: string;
};

export type ProjectOperationStatus = "succeeded" | "failed" | "unknown";

export type ProjectOperationSummary = {
  id: string;
  operation: string;
  safeSummary: string;
  status: ProjectOperationStatus;
  exitCode: number | null;
  startedAt: number;
  durationMs: number;
};

export type ProjectRecord = {
  id: string;
  serverId: string;
  name: string;
  remotePath: string;
  projectType: ProjectType;
  laravelVersion: string | null;
  phpVersion: string | null;
  queueHint: string | null;
  schedulerHint: string | null;
  detectedAt: number;
  logPath: string;
  recentOperations: ProjectOperationSummary[];
};

export type RemoteEntryType = "file" | "directory" | "symlink" | "other";

export type RemoteFileEntry = {
  name: string;
  path: string;
  entryType: RemoteEntryType;
  size: number | null;
  permissions: string | null;
  mode: number | null;
  modifiedAt: number | null;
  symlinkTarget?: string;
  resolvedPath?: string;
  symlinkEscapesRoot?: boolean;
  brokenSymlink: boolean;
};

export type RemoteFileListing = {
  serverId: string;
  root: string;
  path: string;
  resolvedPath: string;
  entries: RemoteFileEntry[];
  truncated: boolean;
  sampledAt: number;
  durationMs: number;
  connectionState: ConnectionState;
};

export type FileTransferDirection = "upload" | "download";

export type FileTransferStatus = "running" | "completed" | "cancelled" | "failed";

export type FileTransferKind = "file" | "directory";

export type FileTransferProgress = {
  transferId: string;
  serverId: string;
  direction: FileTransferDirection;
  status: FileTransferStatus;
  kind: FileTransferKind;
  bytesTransferred: number;
  totalBytes: number | null;
  filesTransferred: number;
  filesTotal: number | null;
  currentPath: string | null;
  mayBeIncomplete: boolean;
  emittedAt: number;
};

export type FileTransferResult = {
  transferId: string;
  serverId: string;
  direction: FileTransferDirection;
  kind: FileTransferKind;
  status: FileTransferStatus;
  sourcePath: string;
  destinationPath: string;
  bytesTransferred: number;
  totalBytes: number | null;
  filesTransferred: number;
  filesTotal: number | null;
  directoriesCreated: number;
  skippedEntries: number;
  warnings: string[];
  partialPaths: string[];
  mayBeIncomplete: boolean;
  overwrite: boolean;
  startedAt: number;
  finishedAt: number;
  durationMs: number;
  error?: CommandError;
  connectionState: ConnectionState;
};

export type RemoteMutationAction = "create_folder" | "rename" | "delete";

export type RemoteMutationResult = {
  serverId: string;
  action: RemoteMutationAction;
  root: string;
  path: string;
  targetPath?: string;
  recursive: boolean;
  deletedEntries: number;
  verified: boolean;
  verificationDetail?: string;
  completedAt: number;
  connectionState: ConnectionState;
};

export type RemoteDirectory = {
  name: string;
  path: string;
};

export type RemoteDirectoryListing = {
  serverId: string;
  path: string;
  directories: RemoteDirectory[];
  truncated: boolean;
  sampledAt: number;
  durationMs: number;
  connectionState: ConnectionState;
};

export type ProjectScanResult = {
  serverId: string;
  path: string;
  includeChildren: boolean;
  scannedPaths: number;
  rejectedPaths: number;
  truncated: boolean;
  projects: ProjectRecord[];
  durationMs: number;
  connectionState: ConnectionState;
};

export type ArtisanActionName =
  | "about"
  | "route-list"
  | "config-cache"
  | "config-clear"
  | "cache-clear"
  | "optimize-clear"
  | "queue-restart"
  | "migrate-status";

export type ArtisanActionResult = {
  serverId: string;
  projectId: string;
  action: ArtisanActionName;
  command: string;
  targetPath: string;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  succeeded: boolean;
  startedAt: number;
  durationMs: number;
  secretsRedacted: boolean;
  outputTruncated: boolean;
  connectionState: ConnectionState;
};

export type ProjectArtisanRun = {
  result: ArtisanActionResult;
  historyRecorded: boolean;
  historyError: string | null;
};

export type SupervisorStatus = "available" | "missing" | "unavailable";

export type SupervisorProgramState =
  "running" | "stopped" | "starting" | "backoff" | "fatal" | "exited" | "unknown";

export type SupervisorProgram = {
  name: string;
  state: SupervisorProgramState;
  stateText: string;
  queueRelated: boolean;
  detail: string | null;
};

/**
 * Supervisor detection from fixed read-only commands. `programs` is empty
 * whenever `status` is not `available`; missing data is never guessed.
 */
export type SupervisorDetection = {
  status: SupervisorStatus;
  installed: boolean | null;
  programs: SupervisorProgram[];
  detail: string | null;
};

export type QueueWorkerStatus = "available" | "unavailable";

export type QueueWorkerKind = "queue_work" | "queue_listen" | "horizon";

export type QueueWorkerProcess = {
  pid: number;
  elapsedSeconds: number;
  kind: QueueWorkerKind;
  queue: string | null;
  connection: string | null;
};

export type QueueWorkerDetection = {
  status: QueueWorkerStatus;
  workers: QueueWorkerProcess[];
  truncated: boolean;
  detail: string | null;
};

export type QueueSupervisorStatus = {
  serverId: string;
  supervisor: SupervisorDetection;
  queueWorkers: QueueWorkerDetection;
  sampledAt: number;
  durationMs: number;
  connectionState: ConnectionState;
};

export type SystemLogSource = {
  path: string;
  label: string;
  description: string;
};

export type LogSourceKind = "system" | "laravel";

export type LogTailStatus = "available" | "missing" | "not_file" | "unreadable" | "unavailable";

export type LogTail = {
  serverId: string;
  requestedPath: string;
  resolvedPath: string | null;
  sourceKind: LogSourceKind;
  status: LogTailStatus;
  detail: string | null;
  content: string;
  returnedLines: number;
  returnedBytes: number;
  fileSizeBytes: number | null;
  lineLimit: number;
  byteLimit: number;
  truncated: boolean;
  truncatedBytes: boolean;
  truncatedLines: boolean;
  partialFirstLine: boolean;
  secretsRedacted: boolean;
  rotated: boolean;
  rotationDetail: string | null;
  retrievedAt: number;
  durationMs: number;
  connectionState: ConnectionState;
};

export type TerminalSize = {
  cols: number;
  rows: number;
  pixelWidth: number;
  pixelHeight: number;
};

export type TerminalSessionInfo = {
  sessionId: string;
  serverId: string;
  term: string;
  cols: number;
  rows: number;
  openedAt: number;
};

export type TerminalExitReason = "remote-exit" | "closed" | "disconnected" | "error";

export type TerminalEvent =
  | {
      type: "output";
      sessionId: string;
      sequence: number;
      data: string;
    }
  | {
      type: "exited";
      sessionId: string;
      sequence: number;
      reason: TerminalExitReason;
      exitCode: number | null;
      message: string | null;
    };

export type ServerProfile = {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  authType: AuthType;
  credentialRef: string | null;
  groupName: string | null;
  tags: string[];
  notes: string | null;
  createdAt: number;
  updatedAt: number;
};

export type ServerProfileInput = Omit<ServerProfile, "id" | "createdAt" | "updatedAt">;
