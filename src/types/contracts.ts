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
