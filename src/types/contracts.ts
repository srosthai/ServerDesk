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
  /** Engine-native code (MySQL error number or PostgreSQL SQLSTATE). */
  databaseCode?: string;
  /** One-based statement position when the engine reports one. */
  position?: number;
  /** Time spent before the failure, when the backend measured it. */
  durationMs?: number;
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
  | "browse_rows"
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
  | "replace_view"
  | "drop_view"
  | "create_routine"
  | "edit_routine"
  | "drop_routine"
  | "create_trigger"
  | "edit_trigger"
  | "drop_trigger"
  | "create_sequence"
  | "alter_sequence"
  | "drop_sequence"
  | "drop_materialized_view"
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
  routineEdit: boolean;
  triggers: boolean;
  triggerEdit: boolean;
  viewManagement: boolean;
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

export type DatabaseTableSize = {
  database: string | null;
  schema: string | null;
  name: string;
  kind: DatabaseTableKind;
  sizeBytes: DatabaseMetadataValue<number>;
  estimatedRows: DatabaseMetadataValue<number>;
  metadataRefreshedAt: DatabaseMetadataValue<string>;
};

export type DatabaseHealthSnapshot = {
  profileId: string;
  engine: DatabaseEngine;
  database: string | null;
  version: DatabaseVersionInfo;
  sampledAt: number;
  latencyMs: DatabaseMetadataValue<number>;
  databaseSizeBytes: DatabaseMetadataValue<number>;
  tableSizes: DatabaseMetadataValue<DatabaseBounded<DatabaseTableSize>>;
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

export type DatabaseRoutineKind = "function" | "procedure";

export type DatabaseRoutineSummary = {
  database: string | null;
  schema: string | null;
  name: string;
  kind: DatabaseRoutineKind;
  language: DatabaseMetadataValue<string>;
  returnType: DatabaseMetadataValue<string>;
  arguments: DatabaseMetadataValue<string>;
  owner: DatabaseMetadataValue<string>;
};

export type DatabaseTriggerTiming = "before" | "after" | "instead_of";
export type DatabaseTriggerEvent = "insert" | "update" | "delete" | "truncate";

export type DatabaseTriggerSummary = {
  database: string | null;
  schema: string | null;
  name: string;
  table: string;
  timing: DatabaseTriggerTiming;
  events: DatabaseTriggerEvent[];
  owner: DatabaseMetadataValue<string>;
};

export type DatabaseObjectCounts = {
  tables: DatabaseMetadataValue<number>;
  views: DatabaseMetadataValue<number>;
  materializedViews: DatabaseMetadataValue<number>;
  sequences: DatabaseMetadataValue<number>;
  routines: DatabaseMetadataValue<number>;
  triggers: DatabaseMetadataValue<number>;
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

export type DatabaseRowColumnKind =
  | "text"
  | "integer"
  | "float"
  | "decimal"
  | "boolean"
  | "temporal"
  | "uuid"
  | "json"
  | "binary"
  | "other";

export type DatabaseRowFilterOperator =
  | "equals"
  | "not_equals"
  | "contains"
  | "starts_with"
  | "ends_with"
  | "greater_than"
  | "greater_than_or_equal"
  | "less_than"
  | "less_than_or_equal"
  | "in"
  | "is_null"
  | "is_not_null";

export type DatabaseRowFilterValue =
  | { kind: "text"; value: string }
  | { kind: "integer"; value: number }
  | { kind: "float"; value: number }
  | { kind: "boolean"; value: boolean };

export type DatabaseRowFilter = {
  column: string;
  operator: DatabaseRowFilterOperator;
  value?: DatabaseRowFilterValue;
  values?: DatabaseRowFilterValue[];
};

export type DatabaseRowSortDirection = "asc" | "desc";

export type DatabaseRowSort = {
  column: string;
  direction: DatabaseRowSortDirection;
};

export type DatabaseRowSearch = {
  query: string;
  columns?: string[];
};

export type DatabaseRowTable = DatabaseDiscoveryScope & {
  table: string;
};

export type DatabaseRowOptions = {
  visibleColumns?: string[];
  sort?: DatabaseRowSort[];
  filters?: DatabaseRowFilter[];
  search?: DatabaseRowSearch | null;
  limit?: number;
  offset?: number;
  includeExactCount?: boolean;
  timeoutMs?: number;
};

export type DatabaseRowIdentityKind = "primary_key" | "unique_index" | "none";

export type DatabaseRowIdentity = {
  kind: DatabaseRowIdentityKind;
  columns: string[];
  index?: string;
  stable: boolean;
};

export type DatabaseRowReadOnlyReason = "no_stable_identity" | "session_read_only";

export type DatabaseRowColumn = {
  name: string;
  dataType: string;
  kind: DatabaseRowColumnKind;
  nullable: boolean;
  ordinalPosition: number;
  primaryKey: boolean;
  identity: DatabaseColumnIdentity;
  generated: boolean;
  sortable: boolean;
  filterable: boolean;
  searchable: boolean;
};

export type DatabaseRowCell = {
  value: string | null;
  truncated: boolean;
};

export type DatabaseRowIdentityValue = {
  column: string;
  value: string | null;
};

export type DatabaseRowRecord = {
  cells: DatabaseRowCell[];
  identity: DatabaseRowIdentityValue[];
};

export type DatabaseRowExactCount = {
  value: number;
  filtered: boolean;
};

export type DatabaseRowCounts = {
  estimatedTableRows: DatabaseMetadataValue<number>;
  exactRows?: DatabaseRowExactCount;
};

export type DatabaseRowPageInfo = {
  limit: number;
  offset: number;
  returned: number;
  hasMore: boolean;
};

export type DatabaseRowReadResult = {
  engine: DatabaseEngine;
  table: DatabaseDiscoveryScope & { name: string };
  columns: DatabaseRowColumn[];
  visibleColumns: string[];
  identity: DatabaseRowIdentity;
  editable: boolean;
  readOnlyReasons: DatabaseRowReadOnlyReason[];
  rows: DatabaseRowRecord[];
  page: DatabaseRowPageInfo;
  counts: DatabaseRowCounts;
};

export type DatabaseRowCancellationResult = {
  requestId: string;
  cancelled: boolean;
};

export type DatabaseRowValueMode = "value" | "null" | "default";

export type DatabaseRowWriteValue = {
  column: string;
  mode: DatabaseRowValueMode;
  value?: string | null;
};

export type DatabaseRowWriteTable = {
  database?: string | null;
  schema?: string | null;
  name: string;
};

export type DatabaseRowInsertRequest = {
  table: DatabaseRowWriteTable;
  values: DatabaseRowWriteValue[];
};

export type DatabaseRowUpdateRequest = {
  table: DatabaseRowWriteTable;
  identity: DatabaseRowIdentityValue[];
  values: DatabaseRowWriteValue[];
  expected: DatabaseRowIdentityValue[];
};

export type DatabaseRowDeleteRequest = {
  table: DatabaseRowWriteTable;
  targets: DatabaseRowIdentityValue[][];
};

export type DatabaseRowWriteOperation = "insert" | "update" | "delete";

export type DatabaseRowTransactionState = "committed" | "rolled_back";

export type DatabaseRowWriteResult = {
  operation: DatabaseRowWriteOperation;
  table: DatabaseRowWriteTable;
  affectedRows: number;
  verified: boolean;
  verification: string;
  transaction: DatabaseRowTransactionState;
  row?: DatabaseRowRecord;
};

export type DatabaseCsvTable = {
  database?: string | null;
  schema?: string | null;
  name: string;
};

export type DatabaseCsvDelimiter = "comma" | "semicolon" | "tab" | "pipe";

export type DatabaseCsvEncoding = "utf8" | "utf8_bom" | "utf16_le" | "utf16_be" | "latin1";

export type DatabaseCsvNullMode =
  { kind: "unquotedEmpty" } | { kind: "literal"; value: string } | { kind: "never" };

export type DatabaseCsvImportOptions = {
  delimiter: DatabaseCsvDelimiter;
  encoding: DatabaseCsvEncoding;
  hasHeader: boolean;
  nullMode: DatabaseCsvNullMode;
};

export type DatabaseCsvColumnMapping = {
  sourceIndex: number;
  targetColumn: string;
};

export type DatabaseCsvExportSource =
  { kind: "table"; table: DatabaseCsvTable } | { kind: "query"; sql: string };

export type DatabaseCsvExportRequest = {
  source: DatabaseCsvExportSource;
  destinationPath: string;
  delimiter: DatabaseCsvDelimiter;
  encoding: DatabaseCsvEncoding;
  includeHeader: boolean;
  overwrite: boolean;
};

export type DatabaseCsvImportRequest = {
  table: DatabaseCsvTable;
  sourcePath: string;
  options: DatabaseCsvImportOptions;
  mappings: DatabaseCsvColumnMapping[];
};

export type DatabaseCsvTransferStatus = "running" | "completed" | "cancelled" | "failed";

export type DatabaseCsvTransferDirection = "export" | "import";

export type DatabaseCsvTransferPhase = "preparing" | "transferring" | "finalizing";

export type DatabaseCsvTransferProgress = {
  transferId: string;
  direction: DatabaseCsvTransferDirection;
  status: DatabaseCsvTransferStatus;
  phase: DatabaseCsvTransferPhase;
  rowsProcessed: number;
  rowsWritten: number;
  rowsSkipped: number;
  totalRows: number | null;
  bytesProcessed: number | null;
  totalBytes: number | null;
  warnings: string[];
  mayBeIncomplete: boolean;
  emittedAt: number;
};

export type DatabaseCsvWarning = {
  rowNumber: number;
  columnNumber?: number;
  columnName?: string;
  message: string;
};

export type DatabaseCsvExportResult = {
  status: DatabaseCsvTransferStatus;
  engine: DatabaseEngine;
  table?: DatabaseCsvTable;
  destinationPath: string;
  rowsExported: number;
  columns: string[];
  bytesWritten: number;
  encoding: DatabaseCsvEncoding;
  delimiter: DatabaseCsvDelimiter;
  includeHeader: boolean;
  warnings: string[];
  mayBeIncomplete: boolean;
  valuesTruncated: boolean;
  durationMs: number;
};

export type DatabaseCsvImportResult = {
  status: DatabaseCsvTransferStatus;
  engine: DatabaseEngine;
  table: DatabaseCsvTable;
  rowsRead: number;
  rowsImported: number;
  rowsSkipped: number;
  warnings: DatabaseCsvWarning[];
  warningsTruncated: boolean;
  transaction: DatabaseRowTransactionState;
  mayBeIncomplete: boolean;
  durationMs: number;
};

export type DatabaseCsvPreviewField = {
  value: string;
  quoted: boolean;
  isNull: boolean;
};

export type DatabaseCsvPreviewRow = {
  rowNumber: number;
  fields: DatabaseCsvPreviewField[];
};

export type DatabaseCsvImportPreview = {
  headers: string[];
  rows: DatabaseCsvPreviewRow[];
  warnings: DatabaseCsvWarning[];
  warningsTruncated: boolean;
  sampledRows: number;
  hasMore: boolean;
  delimiter: DatabaseCsvDelimiter;
  encoding: DatabaseCsvEncoding;
  hasHeader: boolean;
};

export type DatabaseCsvCancellationResult = {
  requestId: string;
  cancelled: boolean;
};

export type DatabaseDumpTool =
  "mysqldump" | "mysql" | "pg_dump" | "pg_dumpall" | "pg_restore" | "psql";

export type DatabaseDumpToolVendor = "unknown" | "mysql" | "mariadb" | "postgresql";

export type DatabaseDumpFormat = "plain_sql" | "custom" | "tar";

export type DatabaseDumpScope =
  | { kind: "cluster" }
  | { kind: "database"; database?: string | null }
  | { kind: "schema"; database?: string | null; schema: string }
  | {
      kind: "tables";
      database?: string | null;
      schema?: string | null;
      tables: string[];
    };

export type DatabaseDumpDestination =
  | { kind: "remote"; path: string; overwrite: boolean }
  | { kind: "local"; path: string; overwrite: boolean };

export type DatabaseDumpSource = { kind: "remote"; path: string } | { kind: "local"; path: string };

export type DatabaseDumpExportRequest = {
  scope: DatabaseDumpScope;
  format: DatabaseDumpFormat;
  /** Optional explicit vendor tool. Omit to let the backend select one. */
  tool?: DatabaseDumpTool | null;
  destination: DatabaseDumpDestination;
  noOwner: boolean;
};

export type DatabaseDumpRestoreConfirmation = {
  serverId: string;
  serverName: string;
  targetDatabase: string;
  replacedData: string;
  acknowledged: boolean;
};

export type DatabaseDumpRestoreRequest = {
  source: DatabaseDumpSource;
  format: DatabaseDumpFormat;
  /** Optional explicit vendor tool. Omit to let the backend select one. */
  tool?: DatabaseDumpTool | null;
  targetDatabase: string;
  clean: boolean;
  noOwner: boolean;
  confirmed: boolean;
  confirmation?: DatabaseDumpRestoreConfirmation | null;
};

export type DatabaseDumpRestorePreviewRequest = {
  source: DatabaseDumpSource;
  format: DatabaseDumpFormat;
  tool?: DatabaseDumpTool | null;
  targetDatabase: string;
  clean: boolean;
  noOwner: boolean;
};

export type DatabaseDumpRestorePreview = {
  serverId: string;
  serverName: string;
  profileId: string;
  profileName: string;
  engine: DatabaseEngine;
  targetDatabase: string;
  sourceKind: DatabaseDumpLocationKind;
  sourcePath: string;
  format: DatabaseDumpFormat;
  tool: DatabaseDumpTool;
  toolAvailable: boolean;
  toolCompatible: boolean;
  toolVersion?: string;
  replacesExistingData: boolean;
  replacedData: string;
  clean: boolean;
  noOwner: boolean;
  canRestore: boolean;
  warnings: string[];
  confirmation: DatabaseDumpRestoreConfirmation;
};

export type DatabaseDumpStatus = "running" | "completed" | "cancelled" | "failed";

export type DatabaseDumpDirection = "export" | "restore";

export type DatabaseDumpPhase =
  | "preparing"
  | "dumping"
  | "uploading"
  | "transferring"
  | "restoring"
  | "verifying"
  | "cleaning_up"
  | "finalizing";

export type DatabaseDumpProgress = {
  transferId: string;
  direction: DatabaseDumpDirection;
  status: DatabaseDumpStatus;
  phase: DatabaseDumpPhase;
  bytesProcessed: number;
  totalBytes: number | null;
  percent: number | null;
  message: string;
  mayBeIncomplete: boolean;
  emittedAt: number;
};

export type DatabaseDumpToolStatus = {
  tool: DatabaseDumpTool;
  binary: string;
  available: boolean;
  path?: string;
  version?: string;
  versionMajor?: number;
  vendor: DatabaseDumpToolVendor;
  compatible: boolean;
  detail?: string;
};

export type DatabaseDumpToolReport = {
  engine: DatabaseEngine;
  serverVersion: string;
  tools: DatabaseDumpToolStatus[];
  exportReady: boolean;
  restoreReady: boolean;
  checkedAt: number;
};

export type DatabaseDumpLocationKind = "remote" | "local";

export type DatabaseDumpExportResult = {
  status: DatabaseDumpStatus;
  engine: DatabaseEngine;
  scope: DatabaseDumpScope;
  format: DatabaseDumpFormat;
  destinationKind: DatabaseDumpLocationKind;
  destinationPath: string;
  bytesWritten: number;
  verified: boolean;
  tool: DatabaseDumpTool;
  toolVersion?: string;
  exitCode?: number;
  warnings: string[];
  failureSummary?: string;
  partialRemoved: boolean;
  tempCleaned: boolean;
  mayBeIncomplete: boolean;
  durationMs: number;
};

export type DatabaseDumpRestoreResult = {
  status: DatabaseDumpStatus;
  engine: DatabaseEngine;
  format: DatabaseDumpFormat;
  sourceKind: DatabaseDumpLocationKind;
  sourcePath: string;
  targetDatabase: string;
  bytesRead: number;
  tool: DatabaseDumpTool;
  toolVersion?: string;
  exitCode?: number;
  warnings: string[];
  failureSummary?: string;
  partialRemoved: boolean;
  tempCleaned: boolean;
  mayBeIncomplete: boolean;
  durationMs: number;
};

export type DatabaseDumpCancellationResult = {
  requestId: string;
  cancelled: boolean;
};

/** Backup workflow aliases over the verified SQL dump transfer types. */
export type DatabaseBackupRequest = DatabaseDumpExportRequest;
export type DatabaseBackupResult = DatabaseDumpExportResult;
export type DatabaseBackupRestoreRequest = DatabaseDumpRestoreRequest;
export type DatabaseBackupRestoreResult = DatabaseDumpRestoreResult;
export type DatabaseBackupRestorePreviewRequest = DatabaseDumpRestorePreviewRequest;
export type DatabaseBackupRestorePreview = DatabaseDumpRestorePreview;
export type DatabaseBackupRestoreConfirmation = DatabaseDumpRestoreConfirmation;
export type DatabaseBackupCancellationResult = DatabaseDumpCancellationResult;

export type DatabaseQueryWarningCode =
  | "drop"
  | "truncate"
  | "delete_without_where"
  | "update_without_where"
  | "delete_broad_predicate"
  | "update_broad_predicate";

export type DatabaseQueryWarning = {
  code: DatabaseQueryWarningCode;
  /** One-based statement position inside the submitted SQL. */
  statementIndex: number;
  message: string;
};

export type DatabaseSqlAnalysis = {
  engine: DatabaseEngine;
  statementCount: number;
  hasWrites: boolean;
  destructive: boolean;
  requiresConfirmation: boolean;
  warnings: DatabaseQueryWarning[];
};

export type DatabaseQueryColumn = {
  name: string;
  dataType: string;
};

export type DatabaseQueryCell = {
  value: string | null;
  truncated: boolean;
};

export type DatabaseQueryResultSet = {
  index: number;
  columns: DatabaseQueryColumn[];
  rows: DatabaseQueryCell[][];
  returnedRows: number;
  affectedRows: number | null;
  lastInsertId: number | null;
  truncated: boolean;
  columnsTruncated: boolean;
};

export type DatabaseQueryExecutionResult = {
  engine: DatabaseEngine;
  readOnly: boolean;
  results: DatabaseQueryResultSet[];
  warnings: DatabaseQueryWarning[];
  destructive: boolean;
  requiresConfirmation: boolean;
  truncated: boolean;
  maxRows: number;
  durationMs: number;
};

export type DatabaseQueryCancellationResult = {
  requestId: string;
  cancelled: boolean;
};

export type DatabaseQueryOptions = {
  maxRows?: number;
  timeoutMs?: number;
  confirmed?: boolean;
};

export type DatabaseQueryHistoryResult = "success" | "failure" | "cancelled";

/** One bounded, sanitized SQL history entry stored in the local database. */
export type DatabaseQueryHistoryEntry = {
  id: string;
  serverId: string;
  connectionId: string;
  engine: DatabaseEngine;
  databaseName: string | null;
  sqlText: string;
  result: DatabaseQueryHistoryResult;
  durationMs: number;
  rowCount: number | null;
  errorSummary: string | null;
  startedAt: number;
};

export type DatabaseQueryHistoryFilter = {
  serverId?: string | null;
  connectionId?: string | null;
  result?: DatabaseQueryHistoryResult | null;
  startedAfter?: number | null;
  startedBefore?: number | null;
  limit?: number | null;
};

/** Retention and size policy for SQL history and saved snippets. */
export type DatabaseQueryHistoryPolicy = {
  maxEntries: number;
  maxSqlChars: number;
  errorSummaryMaxChars: number;
  snippetNameMaxChars: number;
  snippetSqlMaxChars: number;
  retention: string;
};

/** A reusable SQL snippet. Snippets are local and not tied to a server. */
export type DatabaseSavedQuery = {
  id: string;
  name: string;
  engine: DatabaseEngine;
  sqlText: string;
  createdAt: number;
  updatedAt: number;
};

export type DatabaseSavedQueryInput = {
  name: string;
  engine: DatabaseEngine;
  sql: string;
};

export type DatabaseExplorerNode =
  | { kind: "databases" }
  | { kind: "schemas" }
  | ({ kind: "tables" } & DatabaseDiscoveryScope)
  | ({ kind: "views" } & DatabaseDiscoveryScope)
  | ({ kind: "sequences" } & DatabaseDiscoveryScope)
  | ({ kind: "routines" } & DatabaseDiscoveryScope)
  | ({ kind: "triggers" } & DatabaseDiscoveryScope)
  | ({ kind: "objectCounts" } & DatabaseDiscoveryScope);

export type DatabaseExplorerNodeKind = DatabaseExplorerNode["kind"];

export type DatabaseExplorerNodePayload =
  | { nodeKind: "databases"; payload: DatabasePage<DatabaseSummary> }
  | { nodeKind: "schemas"; payload: DatabasePage<DatabaseSchemaSummary> }
  | { nodeKind: "tables"; payload: DatabasePage<DatabaseTableSummary> }
  | { nodeKind: "views"; payload: DatabasePage<DatabaseViewSummary> }
  | { nodeKind: "sequences"; payload: DatabasePage<DatabaseSequenceSummary> }
  | { nodeKind: "routines"; payload: DatabasePage<DatabaseRoutineSummary> }
  | { nodeKind: "triggers"; payload: DatabasePage<DatabaseTriggerSummary> }
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

export type DatabaseObjectKind = "view" | "materialized_view" | "routine" | "trigger" | "sequence";

export type DatabaseSequenceDataType = "small_int" | "integer" | "big_int";

export type DatabaseObjectRef = {
  kind: DatabaseObjectKind;
  database?: string | null;
  schema?: string | null;
  name: string;
  routineKind?: DatabaseRoutineKind | null;
  arguments?: string | null;
  table?: string | null;
};

export type DatabaseTriggerBodyInput =
  | { kind: "statement"; body: string }
  | {
      kind: "function";
      schema?: string | null;
      name: string;
      arguments?: string | null;
    };

export type DatabaseObjectActionRequest =
  | ({
      kind: "create_view";
      name: string;
      query: string;
      orReplace?: boolean;
    } & DatabaseStructureTableScope)
  | ({ kind: "drop_view"; name: string; materialized?: boolean } & DatabaseStructureTableScope)
  | ({
      kind: "create_routine";
      name: string;
      routineKind: DatabaseRoutineKind;
      arguments?: string | null;
      returnType?: string | null;
      language?: string | null;
      body: string;
      orReplace?: boolean;
    } & DatabaseStructureTableScope)
  | ({
      kind: "drop_routine";
      name: string;
      routineKind: DatabaseRoutineKind;
      arguments?: string | null;
    } & DatabaseStructureTableScope)
  | ({
      kind: "create_trigger";
      name: string;
      table: string;
      timing: DatabaseTriggerTiming;
      events: DatabaseTriggerEvent[];
      body: DatabaseTriggerBodyInput;
      orReplace?: boolean;
    } & DatabaseStructureTableScope)
  | ({ kind: "drop_trigger"; name: string; table: string } & DatabaseStructureTableScope)
  | ({
      kind: "create_sequence";
      name: string;
      dataType?: DatabaseSequenceDataType | null;
      start?: number | null;
      increment?: number | null;
      minimum?: number | null;
      maximum?: number | null;
      cache?: number | null;
      cycle?: boolean;
    } & DatabaseStructureTableScope)
  | ({
      kind: "alter_sequence";
      name: string;
      increment?: number | null;
      minimum?: number | null;
      maximum?: number | null;
      restart?: number | null;
      cache?: number | null;
      cycle?: boolean | null;
    } & DatabaseStructureTableScope)
  | ({ kind: "drop_sequence"; name: string } & DatabaseStructureTableScope)
  | ({ kind: "refresh_materialized_view"; name: string } & DatabaseStructureTableScope)
  | ({ kind: "drop_materialized_view"; name: string } & DatabaseStructureTableScope);

export type DatabaseObjectActionKind =
  | "create_view"
  | "replace_view"
  | "drop_view"
  | "create_routine"
  | "replace_routine"
  | "drop_routine"
  | "create_trigger"
  | "replace_trigger"
  | "drop_trigger"
  | "create_sequence"
  | "alter_sequence"
  | "drop_sequence"
  | "refresh_materialized_view"
  | "drop_materialized_view";

export type DatabaseObjectDefinition = {
  kind: DatabaseObjectKind;
  database: string | null;
  schema: string | null;
  name: string;
  owner: DatabaseMetadataValue<string>;
  language: DatabaseMetadataValue<string>;
  routineKind: DatabaseMetadataValue<DatabaseRoutineKind>;
  arguments: DatabaseMetadataValue<string>;
  returnType: DatabaseMetadataValue<string>;
  viewQuery: DatabaseMetadataValue<string>;
  routineBody: DatabaseMetadataValue<string>;
  definition: DatabaseMetadataValue<string>;
  trigger: {
    table: string;
    timing: DatabaseTriggerTiming;
    events: DatabaseTriggerEvent[];
    statement: DatabaseMetadataValue<string>;
    functionName: DatabaseMetadataValue<string>;
    function: DatabaseMetadataValue<string>;
  } | null;
  sequence: {
    dataType: DatabaseMetadataValue<string>;
    startValue: DatabaseMetadataValue<string>;
    increment: DatabaseMetadataValue<string>;
    minimum: DatabaseMetadataValue<string>;
    maximum: DatabaseMetadataValue<string>;
    currentValue: DatabaseMetadataValue<string>;
    cache: DatabaseMetadataValue<string>;
    cycle: DatabaseMetadataValue<boolean>;
  } | null;
  truncated: boolean;
};

export type DatabaseObjectDdlPreview = {
  action: DatabaseObjectActionKind;
  engine: DatabaseEngine;
  object: DatabaseObjectRef;
  statements: string[];
  ddl: string;
  warnings: string[];
  destructive: boolean;
  requiresConfirmation: boolean;
};

export type DatabaseObjectActionResult = {
  action: DatabaseObjectActionKind;
  engine: DatabaseEngine;
  object: DatabaseObjectRef;
  statements: string[];
  warnings: string[];
  verified: boolean;
  verification: string;
  verificationError?: string | null;
  definition?: DatabaseObjectDefinition | null;
  durationMs: number;
};

export type DatabaseRoleKind = "user" | "role";

export type DatabaseRoleRef = {
  name: string;
  host?: string | null;
};

export type DatabaseRoleSummary = {
  name: string;
  host?: string | null;
  kind: DatabaseRoleKind;
  canLogin: DatabaseMetadataValue<boolean>;
  superuser: DatabaseMetadataValue<boolean>;
  createDatabase: DatabaseMetadataValue<boolean>;
  createRole: DatabaseMetadataValue<boolean>;
  inherit: DatabaseMetadataValue<boolean>;
};

export type DatabasePrivilegeTargetKind = "global" | "database" | "schema" | "table" | "sequence";

export type DatabasePrivilegeTarget = {
  kind: DatabasePrivilegeTargetKind;
  database?: string | null;
  schema?: string | null;
  object?: string | null;
};

export type DatabasePrivilege =
  | "select"
  | "insert"
  | "update"
  | "delete"
  | "truncate"
  | "references"
  | "trigger"
  | "create"
  | "drop"
  | "alter"
  | "index"
  | "execute"
  | "usage"
  | "connect"
  | "temporary"
  | "create_temporary_tables"
  | "lock_tables"
  | "create_view"
  | "show_view"
  | "create_routine"
  | "alter_routine"
  | "event";

export type DatabasePrivilegeGrant = {
  privilege: string;
  target: DatabasePrivilegeTarget;
  grantable: boolean;
};

export type DatabaseEffectivePrivilege = DatabasePrivilegeGrant & {
  sources: string[];
};

export type DatabaseRolePrivilegeSnapshot = {
  role: DatabaseRoleRef;
  direct: DatabasePrivilegeGrant[];
  memberOf: DatabaseRoleRef[];
  effective: DatabaseEffectivePrivilege[];
};

export type DatabasePrivilegeDiff = {
  before: DatabaseEffectivePrivilege[];
  after: DatabaseEffectivePrivilege[];
  added: DatabaseEffectivePrivilege[];
  removed: DatabaseEffectivePrivilege[];
};

export type DatabaseRoleAttributeChange = {
  attribute: string;
  before: string;
  after: string;
};

export type DatabaseRoleActionRequest =
  | {
      kind: "create_role";
      name: string;
      host?: string | null;
      roleKind: DatabaseRoleKind;
      canLogin?: boolean | null;
      superuser?: boolean | null;
      createDatabase?: boolean | null;
      createRole?: boolean | null;
      inherit?: boolean | null;
    }
  | {
      kind: "alter_role";
      name: string;
      host?: string | null;
      canLogin?: boolean | null;
      superuser?: boolean | null;
      createDatabase?: boolean | null;
      createRole?: boolean | null;
      inherit?: boolean | null;
    }
  | {
      kind: "grant_privileges";
      grantee: DatabaseRoleRef;
      privileges: DatabasePrivilege[];
      target: DatabasePrivilegeTarget;
      withGrantOption?: boolean;
    }
  | {
      kind: "revoke_privileges";
      grantee: DatabaseRoleRef;
      privileges: DatabasePrivilege[];
      target: DatabasePrivilegeTarget;
      revokeGrantOption?: boolean;
    }
  | {
      kind: "grant_role";
      role: DatabaseRoleRef;
      member: DatabaseRoleRef;
    }
  | {
      kind: "revoke_role";
      role: DatabaseRoleRef;
      member: DatabaseRoleRef;
    };

export type DatabaseRoleActionKind =
  | "create_role"
  | "alter_role"
  | "grant_privileges"
  | "revoke_privileges"
  | "grant_role"
  | "revoke_role";

export type DatabaseRoleDdlPreview = {
  action: DatabaseRoleActionKind;
  engine: DatabaseEngine;
  role: DatabaseRoleRef;
  target?: DatabasePrivilegeTarget | null;
  statements: string[];
  ddl: string;
  warnings: string[];
  destructive: boolean;
  requiresConfirmation: boolean;
  privilegeDiff: DatabasePrivilegeDiff;
  roleChanges: DatabaseRoleAttributeChange[];
};

export type DatabaseRoleActionResult = {
  action: DatabaseRoleActionKind;
  engine: DatabaseEngine;
  role: DatabaseRoleRef;
  target?: DatabasePrivilegeTarget | null;
  statements: string[];
  warnings: string[];
  verified: boolean;
  verification: string;
  verificationError?: string | null;
  privilegeDiff: DatabasePrivilegeDiff;
  roleChanges: DatabaseRoleAttributeChange[];
  durationMs: number;
};

export type DatabaseSessionQuery = {
  text: string;
  redacted: boolean;
  truncated: boolean;
  startedAt: DatabaseMetadataValue<string>;
  elapsedMs: DatabaseMetadataValue<number>;
};

export type DatabaseSessionSummary = {
  id: number;
  user: string;
  host: DatabaseMetadataValue<string>;
  database: DatabaseMetadataValue<string>;
  applicationName: DatabaseMetadataValue<string>;
  command: DatabaseMetadataValue<string>;
  state: DatabaseMetadataValue<string>;
  waitEvent: DatabaseMetadataValue<string>;
  backendType: DatabaseMetadataValue<string>;
  connectedAt: DatabaseMetadataValue<string>;
  isCurrent: boolean;
  query: DatabaseMetadataValue<DatabaseSessionQuery>;
};

export type DatabaseSessionActionKind = "cancel_query" | "terminate_session";

export type DatabaseSessionActionRequest = {
  sessionId: number;
  action: DatabaseSessionActionKind;
  expectedUser: string;
  expectedDatabase?: string | null;
  confirmation: string;
};

export type DatabaseSessionActionResult = {
  action: DatabaseSessionActionKind;
  engine: DatabaseEngine;
  sessionId: number;
  user: string;
  database: DatabaseMetadataValue<string>;
  requestAccepted: boolean;
  verified: boolean;
  verification: string;
  verificationError?: string | null;
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
