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
