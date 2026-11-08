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
  cancelled: {
    title: "Connection cancelled",
    action: "Start the connection test again when you are ready.",
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
