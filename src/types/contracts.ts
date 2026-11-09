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
  | "cancelled"
  | "conflict"
  | "not_found"
  | "storage"
  | "credential"
  | "internal";

export type CommandError = {
  code: ErrorCode;
  message: string;
  details?: string;
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

export type ProjectType = "laravel";

export type OperationCategory = "command" | "service" | "artisan" | "sftp" | "queue";

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
