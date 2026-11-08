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
