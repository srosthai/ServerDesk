import type {
  DatabaseConnectionState,
  DatabaseEngine,
  DatabaseProfile,
  DatabaseProfileInput,
  DatabaseSslMode,
} from "@/types/contracts";

export const MAX_DATABASE_PROFILE_NAME_LENGTH = 80;
export const MAX_DATABASE_HOST_LENGTH = 253;
export const MAX_DATABASE_SOCKET_LENGTH = 4096;
export const MAX_DATABASE_NAME_LENGTH = 128;
export const MAX_DATABASE_USERNAME_LENGTH = 128;
export const MAX_DATABASE_PASSWORD_BYTES = 64 * 1024;

export const DATABASE_ENGINE_LABELS: Record<DatabaseEngine, string> = {
  mysql: "MySQL",
  postgresql: "PostgreSQL",
};

export const DATABASE_ENGINE_DEFAULT_PORTS: Record<DatabaseEngine, number> = {
  mysql: 3306,
  postgresql: 5432,
};

export const DATABASE_SSL_MODE_LABELS: Record<DatabaseSslMode | "default", string> = {
  default: "Engine default",
  disabled: "Disabled",
  preferred: "Preferred",
  required: "Required",
  "verify-ca": "Verify CA",
  "verify-identity": "Verify identity",
};

export type DatabaseProfileFormValues = {
  name: string;
  serverId: string;
  engine: DatabaseEngine;
  host: string;
  port: string;
  socket: string;
  databaseName: string;
  username: string;
  password: string;
  readOnly: boolean;
  sslMode: DatabaseSslMode | "default";
};

export type DatabaseProfileFormField = keyof DatabaseProfileFormValues;

export type DatabaseProfileFormErrors = Partial<
  Record<Exclude<DatabaseProfileFormField, "readOnly">, string>
>;

const invalidHostCharacters = /[^a-zA-Z0-9._:%-]/;
const whitespace = /\s/;

function containsControlCharacters(value: string): boolean {
  return Array.from(value).some((character) => {
    const code = character.codePointAt(0) ?? 0;
    return code < 32 || code === 127;
  });
}

export function defaultPortForEngine(engine: DatabaseEngine): string {
  return String(DATABASE_ENGINE_DEFAULT_PORTS[engine]);
}

export function emptyDatabaseProfileForm(
  serverId: string,
  engine: DatabaseEngine = "postgresql",
): DatabaseProfileFormValues {
  return {
    name: "",
    serverId,
    engine,
    host: "127.0.0.1",
    port: defaultPortForEngine(engine),
    socket: "",
    databaseName: "",
    username: "",
    password: "",
    readOnly: false,
    sslMode: "default",
  };
}

export function databaseProfileFormFromProfile(
  profile: DatabaseProfile,
  activeServerId: string | null,
): DatabaseProfileFormValues {
  return {
    name: profile.name,
    serverId: profile.serverId || activeServerId || "",
    engine: profile.engine,
    host: profile.host,
    port: String(profile.port),
    socket: profile.socket ?? "",
    databaseName: profile.databaseName ?? "",
    username: profile.username,
    // The backend never sends the saved password. The field always starts
    // empty so editing cannot echo a secret into the DOM.
    password: "",
    readOnly: profile.readOnly,
    sslMode: profile.sslMode ?? "default",
  };
}

export function validateDatabaseProfileForm(
  values: DatabaseProfileFormValues,
): DatabaseProfileFormErrors {
  const errors: DatabaseProfileFormErrors = {};
  const name = values.name.trim();
  const host = values.host.trim();
  const socket = values.socket.trim();
  const databaseName = values.databaseName.trim();
  const username = values.username.trim();

  if (name.length === 0) {
    errors.name = "Database profile name is required.";
  } else if (name.length > MAX_DATABASE_PROFILE_NAME_LENGTH) {
    errors.name = `Database profile name must be ${MAX_DATABASE_PROFILE_NAME_LENGTH} characters or fewer.`;
  } else if (containsControlCharacters(name)) {
    errors.name = "The database profile name contains invalid characters.";
  }

  if (values.serverId.trim().length === 0) {
    errors.serverId = "Select a server profile.";
  }

  if (host.length === 0) {
    errors.host = "Database host is required.";
  } else if (host.length > MAX_DATABASE_HOST_LENGTH || invalidHostCharacters.test(host)) {
    errors.host = "Enter a valid database hostname or IP address.";
  }

  const port = Number(values.port.trim());
  if (values.port.trim().length === 0 || !Number.isInteger(port) || port < 1 || port > 65535) {
    errors.port = "Database port must be between 1 and 65535.";
  }

  if (socket.length > 0) {
    if (socket.length > MAX_DATABASE_SOCKET_LENGTH) {
      errors.socket = `Database socket must be ${MAX_DATABASE_SOCKET_LENGTH} characters or fewer.`;
    } else if (!socket.startsWith("/")) {
      errors.socket = "The database socket must be an absolute path beginning with /.";
    } else if (containsControlCharacters(socket)) {
      errors.socket = "The database socket path contains invalid characters.";
    }
  }

  if (databaseName.length > MAX_DATABASE_NAME_LENGTH) {
    errors.databaseName = `Database name must be ${MAX_DATABASE_NAME_LENGTH} characters or fewer.`;
  } else if (containsControlCharacters(databaseName)) {
    errors.databaseName = "The database name contains invalid characters.";
  }

  if (username.length === 0) {
    errors.username = "Database username is required.";
  } else if (username.length > MAX_DATABASE_USERNAME_LENGTH || whitespace.test(username)) {
    errors.username = "Enter a valid database username without spaces.";
  }

  if (values.password.length > MAX_DATABASE_PASSWORD_BYTES) {
    errors.password = "The database password is too long.";
  }

  if (values.sslMode !== "default" && !(values.sslMode in DATABASE_SSL_MODE_LABELS)) {
    errors.sslMode = "Choose a supported SSL mode.";
  }

  return errors;
}

export function firstDatabaseProfileFormError(
  errors: DatabaseProfileFormErrors,
): Exclude<DatabaseProfileFormField, "readOnly"> | null {
  const order: Exclude<DatabaseProfileFormField, "readOnly">[] = [
    "name",
    "serverId",
    "engine",
    "host",
    "port",
    "socket",
    "databaseName",
    "username",
    "password",
    "sslMode",
  ];

  return order.find((field) => Boolean(errors[field as keyof DatabaseProfileFormErrors])) ?? null;
}

export function databaseProfileInputFromForm(
  values: DatabaseProfileFormValues,
): DatabaseProfileInput {
  const port = Number(values.port.trim());
  const socket = values.socket.trim();
  const databaseName = values.databaseName.trim();
  const sslMode = values.sslMode === "default" ? null : values.sslMode;

  return {
    serverId: values.serverId.trim(),
    name: values.name.trim(),
    engine: values.engine,
    host: values.host.trim(),
    port,
    socket: socket.length > 0 ? socket : null,
    databaseName: databaseName.length > 0 ? databaseName : null,
    username: values.username.trim(),
    readOnly: values.readOnly,
    sslMode,
    // An empty field means "keep the saved credential" on update and "no
    // password" on create. The saved password is never sent to React.
    password: values.password.length > 0 ? values.password : null,
  };
}

export function formatDatabaseEndpoint(
  host: string,
  port: number | string,
  socket?: string | null,
): string {
  if (socket && socket.trim().length > 0) {
    return socket.trim();
  }

  const trimmedHost = host.trim();
  const formattedHost = trimmedHost.includes(":") ? `[${trimmedHost}]` : trimmedHost;
  return `${formattedHost}:${String(port).trim()}`;
}

export function databaseConnectionStateLabel(state: DatabaseConnectionState): string {
  switch (state) {
    case "connected":
      return "DB connected";
    case "connecting":
      return "DB connecting";
    case "reconnecting":
      return "DB reconnecting";
    case "error":
      return "DB error";
    default:
      return "DB disconnected";
  }
}

export function formatLatency(latencyMs: number | null | undefined): string {
  if (latencyMs === null || latencyMs === undefined) {
    return "Not measured";
  }
  return latencyMs < 1000 ? `${latencyMs} ms` : `${(latencyMs / 1000).toFixed(1)} s`;
}
