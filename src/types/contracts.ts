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
