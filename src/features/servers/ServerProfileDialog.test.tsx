import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServerProfileDialog } from "@/features/servers/ServerProfileDialog";
import { createPrivateKeyCredential, storePrivateKeyCredential } from "@/services/credentials";
import { updateServer } from "@/services/servers";
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
