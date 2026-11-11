import type { CommandError, ErrorCode } from "@/types/contracts";

type ConnectionErrorPresentation = {
  title: string;
  action: string;
};

const presentations: Record<ErrorCode, ConnectionErrorPresentation> = {
  validation: {
    title: "Check the connection details",
    action: "Correct the highlighted values and test the connection again.",
  },
  connection: {
    title: "Connection failed",
    action: "Check the host, port, and SSH server configuration, then try again.",
  },
  authentication: {
    title: "Authentication failed",
    action:
      "Check the username and credential. Update the password or private key, then test again.",
  },
  host_key: {
    title: "Host key mismatch",
    action:
      "The server key did not match the trusted fingerprint. Verify the key with the server owner before trusting it.",
  },
  timeout: {
    title: "Connection timed out",
    action:
      "Check that the host is reachable, the SSH port is correct, and a firewall is not blocking the connection.",
  },
  dns: {
    title: "Hostname could not be resolved",
    action: "Check the hostname for typos or use the server IP address instead.",
  },
  network: {
    title: "Network connection lost",
    action: "Check your network connection and that the SSH service is running, then try again.",
  },
  permission: {
    title: "Permission denied",
    action: "Check that the SSH account has permission to perform this operation.",
  },
  remote: {
    title: "Remote check failed",
    action: "The SSH server rejected the check. Review the server logs and SSH configuration.",
  },
  tunnel: {
    title: "Database tunnel failed",
    action:
      "Check that the SSH session is connected and the database endpoint is reachable from the server.",
  },
  database_auth: {
    title: "Database authentication failed",
    action:
      "Check the database username and password, and confirm the account can connect from the server.",
  },
  query: {
    title: "Database query failed",
    action: "Review the SQL and database permissions, then run the statement again.",
  },
  capability: {
    title: "Action not supported",
    action:
      "The selected database engine or version does not support this action. Choose a supported operation.",
  },
  cancelled: {
    title: "Connection cancelled",
    action: "Start the connection test again when you are ready.",
  },
  conflict: {
    title: "The destination already exists",
    action: "Confirm overwrite explicitly before replacing an existing file.",
  },
  not_found: {
    title: "Server or credential not found",
    action: "Refresh the server list and check that the saved credential still exists.",
  },
  storage: {
    title: "Local storage error",
    action: "Check that the application data directory is writable, then try again.",
  },
  credential: {
    title: "Credential unavailable",
    action: "Re-enter the password or private key, then test the connection again.",
  },
  credential_unavailable: {
    title: "Credential store unavailable",
    action:
      "Start or unlock your operating system credential provider, then save the credential again.",
  },
  credential_locked: {
    title: "Credential store locked",
    action: "Unlock your operating system credential store, then try again.",
  },
  credential_missing: {
    title: "Saved credential missing",
    action: "Re-enter the saved password or private key and save it again.",
  },
  credential_invalid: {
    title: "Saved credential invalid",
    action: "Re-enter the saved password or private key, save it again, and retry.",
  },
  internal: {
    title: "Unexpected connection error",
    action: "Try again. If the problem continues, restart ServerDesk.",
  },
};

export function describeConnectionError(
  error: Pick<CommandError, "code">,
): ConnectionErrorPresentation {
  return presentations[error.code] ?? presentations.connection;
}
