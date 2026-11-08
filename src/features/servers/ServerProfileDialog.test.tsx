import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServerProfileDialog } from "@/features/servers/ServerProfileDialog";
import { testServerConnection } from "@/services/connections";
import {
  createCredential,
  createPrivateKeyCredential,
  storePrivateKeyCredential,
} from "@/services/credentials";
import { createServer, updateServer } from "@/services/servers";
import { useServerStore } from "@/stores/serverStore";
import type { ServerProfile } from "@/types/contracts";

vi.mock("@/services/credentials", () => ({
  createCredential: vi.fn(),
  storeCredential: vi.fn(),
  deleteCredential: vi.fn(),
  createPrivateKeyCredential: vi.fn(),
  storePrivateKeyCredential: vi.fn(),
}));

vi.mock("@/services/servers", () => ({
  listServers: vi.fn(),
  getServer: vi.fn(),
  createServer: vi.fn(),
  updateServer: vi.fn(),
  deleteServer: vi.fn(),
}));

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    testServerConnection: vi.fn(),
  };
});

const privateKeyServer: ServerProfile = {
  id: "server-1",
  name: "Production",
  host: "198.51.100.24",
  port: 2222,
  username: "deploy",
  authType: "private_key",
  credentialRef: "serverdesk:ssh:existing",
  groupName: null,
  tags: [],
  notes: null,
  createdAt: 1,
  updatedAt: 1,
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("ServerProfileDialog", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServerStore.setState({ servers: [privateKeyServer], status: "ready", error: null });
    vi.mocked(updateServer).mockResolvedValue({ ...privateKeyServer, port: 2222 });
  });

  it("tests and saves a password-authenticated server with all profile fields", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    vi.mocked(testServerConnection).mockResolvedValue({ status: "connected", durationMs: 120 });
    vi.mocked(createCredential).mockResolvedValue("serverdesk:ssh:new");
    vi.mocked(createServer).mockImplementation(async (input) => ({
      ...input,
      id: "server-new",
      createdAt: 1,
      updatedAt: 1,
    }));

    render(<ServerProfileDialog server={null} onOpenChange={onOpenChange} />);

    await user.type(screen.getByLabelText("Name"), "Staging");
    await user.type(screen.getByLabelText("Host"), "staging.example.com");
    await user.clear(screen.getByLabelText("SSH port"));
    await user.type(screen.getByLabelText("SSH port"), "2222");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.type(screen.getByLabelText("Group"), "Production");
    await user.type(screen.getByLabelText("Tags"), "web, laravel");
    await user.type(screen.getByLabelText("Notes"), "Primary staging host");
    await user.type(screen.getByLabelText("SSH password"), "fixture-password");
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("Connection test passed")).toBeInTheDocument();
    expect(testServerConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        serverId: null,
        host: "staging.example.com",
        port: 2222,
        username: "deploy",
        authType: "password",
        secret: "fixture-password",
        passphrase: null,
        acceptedHostKey: null,
      }),
    );

    await user.click(screen.getByRole("button", { name: "Save server" }));

    await waitFor(() => {
      expect(createCredential).toHaveBeenCalledWith("fixture-password");
    });
    expect(createServer).toHaveBeenCalledWith({
      name: "Staging",
      host: "staging.example.com",
      port: 2222,
      username: "deploy",
      authType: "password",
      credentialRef: "serverdesk:ssh:new",
      groupName: "Production",
      tags: ["web", "laravel"],
      notes: "Primary staging host",
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("preserves non-secret fields and classifies an authentication failure", async () => {
    const user = userEvent.setup();
    const secret = "fixture-secret-password";
    vi.mocked(testServerConnection).mockRejectedValue({
      code: "authentication",
      message: "SSH authentication failed. Check the username and credentials.",
    });

    render(<ServerProfileDialog server={null} onOpenChange={vi.fn()} />);

    await user.type(screen.getByLabelText("Name"), "Staging");
    await user.type(screen.getByLabelText("Host"), "staging.example.com");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.type(screen.getByLabelText("Group"), "Production");
    await user.type(screen.getByLabelText("Tags"), "web, laravel");
    await user.type(screen.getByLabelText("Notes"), "Do not lose this");
    await user.type(screen.getByLabelText("SSH password"), secret);
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("Authentication failed")).toBeInTheDocument();
    expect(
      screen.getByText("SSH authentication failed. Check the username and credentials."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Check the username and credential. Update the password or private key, then test again.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Name")).toHaveValue("Staging");
    expect(screen.getByLabelText("Host")).toHaveValue("staging.example.com");
    expect(screen.getByLabelText("Username")).toHaveValue("deploy");
    expect(screen.getByLabelText("Group")).toHaveValue("Production");
    expect(screen.getByLabelText("Tags")).toHaveValue("web, laravel");
    expect(screen.getByLabelText("Notes")).toHaveValue("Do not lose this");
    expect(document.body.textContent).not.toContain(secret);
  });

  it("distinguishes DNS, timeout, and network failures", async () => {
    const user = userEvent.setup();
    render(<ServerProfileDialog server={null} onOpenChange={vi.fn()} />);
    await user.type(screen.getByLabelText("Host"), "missing.example.com");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.type(screen.getByLabelText("SSH password"), "fixture-password");

    for (const [code, title, message] of [
      ["dns", "Hostname could not be resolved", "Could not resolve the server hostname."],
      ["timeout", "Connection timed out", "Connection timed out after 15 seconds."],
      ["network", "Network connection lost", "Could not reach the SSH server."],
    ] as const) {
      vi.mocked(testServerConnection).mockRejectedValue({ code, message });
      await user.click(screen.getByRole("button", { name: "Test connection" }));

      expect(await screen.findByText(title)).toBeInTheDocument();
      expect(screen.getByText(message)).toBeInTheDocument();
    }
  });

  it("waits for explicit host-key acceptance before retrying a draft test", async () => {
    const user = userEvent.setup();
    const hostKey = {
      keyType: "ssh-ed25519",
      fingerprint: "SHA256:firstseenfingerprint",
    };
    vi.mocked(testServerConnection)
      .mockResolvedValueOnce({ status: "hostKeyVerification", hostKey })
      .mockResolvedValueOnce({ status: "connected", durationMs: 25 });

    render(<ServerProfileDialog server={null} onOpenChange={vi.fn()} />);

    await user.type(screen.getByLabelText("Name"), "Staging");
    await user.type(screen.getByLabelText("Host"), "staging.example.com");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.type(screen.getByLabelText("SSH password"), "fixture-password");
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByRole("heading", { name: "Verify SSH host key" })).toBeInTheDocument();
    expect(screen.getByText("ssh-ed25519")).toBeInTheDocument();
    expect(screen.getByText("SHA256:firstseenfingerprint")).toBeInTheDocument();
    expect(testServerConnection).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Trust and connect" }));

    await waitFor(() => {
      expect(testServerConnection).toHaveBeenCalledTimes(2);
    });
    expect(vi.mocked(testServerConnection).mock.calls[1]?.[0]).toEqual(
      expect.objectContaining({ acceptedHostKey: hostKey }),
    );
    expect(await screen.findByText("Connection test passed")).toBeInTheDocument();
  });

  it("aborts a draft test when the host key is rejected", async () => {
    const user = userEvent.setup();
    vi.mocked(testServerConnection).mockResolvedValue({
      status: "hostKeyVerification",
      hostKey: {
        keyType: "ssh-ed25519",
        fingerprint: "SHA256:firstseenfingerprint",
      },
    });

    render(<ServerProfileDialog server={null} onOpenChange={vi.fn()} />);

    await user.type(screen.getByLabelText("Host"), "staging.example.com");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.type(screen.getByLabelText("SSH password"), "fixture-password");
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    await screen.findByRole("heading", { name: "Verify SSH host key" });
    await user.click(screen.getByRole("button", { name: "Reject key" }));

    expect(await screen.findByText("Connection test cancelled")).toBeInTheDocument();
    expect(
      screen.getByText("The host key was rejected. No credentials were sent to the server."),
    ).toBeInTheDocument();
    expect(testServerConnection).toHaveBeenCalledTimes(1);
  });

  it("tests a new passphrase-protected private key without saving it", async () => {
    const user = userEvent.setup();
    vi.mocked(testServerConnection).mockResolvedValue({ status: "connected", durationMs: 42 });
    render(<ServerProfileDialog server={privateKeyServer} onOpenChange={vi.fn()} />);

    await user.type(screen.getByLabelText("Private key"), "-----BEGIN OPENSSH PRIVATE KEY-----");
    await user.type(screen.getByLabelText("Key passphrase (optional)"), "fixture-passphrase");
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    await waitFor(() => {
      expect(testServerConnection).toHaveBeenCalledWith(
        expect.objectContaining({
          serverId: "server-1",
          authType: "private_key",
          credentialRef: null,
          secret: "-----BEGIN OPENSSH PRIVATE KEY-----",
          passphrase: "fixture-passphrase",
        }),
      );
    });
    expect(storePrivateKeyCredential).not.toHaveBeenCalled();
    expect(createPrivateKeyCredential).not.toHaveBeenCalled();
  });

  it("tests a saved server with its stored credential when the secret is blank", async () => {
    const user = userEvent.setup();
    const passwordServer: ServerProfile = {
      ...privateKeyServer,
      authType: "password",
    };
    vi.mocked(testServerConnection).mockResolvedValue({ status: "connected", durationMs: 18 });
    render(<ServerProfileDialog server={passwordServer} onOpenChange={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Test connection" }));

    await waitFor(() => {
      expect(testServerConnection).toHaveBeenCalledWith(
        expect.objectContaining({
          serverId: "server-1",
          credentialRef: "serverdesk:ssh:existing",
          secret: null,
          passphrase: null,
        }),
      );
    });
    expect(await screen.findByText("Connection test passed")).toBeInTheDocument();
  });

  it("requires a credential before testing a new server", async () => {
    const user = userEvent.setup();
    render(<ServerProfileDialog server={null} onOpenChange={vi.fn()} />);

    await user.type(screen.getByLabelText("Host"), "staging.example.com");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(
      await screen.findByText("Enter the SSH password before testing the connection."),
    ).toBeInTheDocument();
    expect(testServerConnection).not.toHaveBeenCalled();
  });

  it("stores a passphrase-protected private key in the credential store", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<ServerProfileDialog server={privateKeyServer} onOpenChange={onOpenChange} />);

    await user.type(screen.getByLabelText("Private key"), "-----BEGIN OPENSSH PRIVATE KEY-----");
    await user.type(screen.getByLabelText("Key passphrase (optional)"), "fixture-passphrase");
    await user.click(screen.getByRole("button", { name: "Save server" }));

    await waitFor(() => {
      expect(storePrivateKeyCredential).toHaveBeenCalledWith(
        "serverdesk:ssh:existing",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "fixture-passphrase",
      );
    });
    expect(updateServer).toHaveBeenCalledWith(
      "server-1",
      expect.objectContaining({
        authType: "private_key",
        port: 2222,
        username: "deploy",
        credentialRef: "serverdesk:ssh:existing",
      }),
    );
    expect(createPrivateKeyCredential).not.toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("requires the key again before changing its passphrase", async () => {
    const user = userEvent.setup();
    render(<ServerProfileDialog server={privateKeyServer} onOpenChange={vi.fn()} />);

    await user.type(screen.getByLabelText("Key passphrase (optional)"), "new-passphrase");
    await user.click(screen.getByRole("button", { name: "Save server" }));

    expect(
      await screen.findByText("Enter the private key again to change its passphrase."),
    ).toBeInTheDocument();
    expect(storePrivateKeyCredential).not.toHaveBeenCalled();
    expect(updateServer).not.toHaveBeenCalled();
  });
});
