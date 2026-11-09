import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { KnownHostsDialog } from "@/features/servers/KnownHostsDialog";
import { deleteKnownHost, listKnownHosts } from "@/services/knownHosts";
import { useHostKeyStore } from "@/stores/hostKeyStore";
import type { ServerProfile } from "@/types/contracts";

vi.mock("@/services/knownHosts", () => ({
  listKnownHosts: vi.fn(),
  trustHostKey: vi.fn(),
  rejectHostKey: vi.fn(),
  deleteKnownHost: vi.fn(),
}));

const server: ServerProfile = {
  id: "server-1",
  name: "Production",
  host: "198.51.100.24",
  port: 22,
  username: "deploy",
  authType: "password",
  credentialRef: null,
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

describe("KnownHostsDialog", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useHostKeyStore.setState({ knownHosts: {}, status: {}, error: null });
  });

  it("lists trusted fingerprints and removes one with explicit confirmation", async () => {
    const user = userEvent.setup();
    vi.mocked(listKnownHosts).mockResolvedValue([
      {
        id: "host-1",
        serverId: server.id,
        keyType: "ssh-ed25519",
        fingerprint: "SHA256:trusted",
        firstSeenAt: 1,
        verifiedAt: 2,
      },
    ]);
    vi.mocked(deleteKnownHost).mockResolvedValue(undefined);

    render(<KnownHostsDialog server={server} onOpenChange={vi.fn()} />);

    expect(await screen.findByText("SHA256:trusted")).toBeInTheDocument();
    expect(screen.getByText("ssh-ed25519")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Remove ssh-ed25519 fingerprint" }));
    expect(screen.getByText("Remove trusted fingerprint?")).toBeInTheDocument();
    expect(screen.getByText("This only changes local trust.")).toBeInTheDocument();
    expect(screen.getByText(/The remote server is not modified/)).toBeInTheDocument();
    expect(screen.getAllByText("SHA256:trusted").length).toBeGreaterThan(1);

    await user.click(screen.getByRole("button", { name: "Remove fingerprint" }));

    await waitFor(() => {
      expect(deleteKnownHost).toHaveBeenCalledWith("server-1", "ssh-ed25519");
    });
    expect(await screen.findByText("No trusted host keys")).toBeInTheDocument();
    expect(screen.queryByText("Remove trusted fingerprint?")).not.toBeInTheDocument();
  });

  it("shows an empty state before the first connection is trusted", async () => {
    vi.mocked(listKnownHosts).mockResolvedValue([]);

    render(<KnownHostsDialog server={server} onOpenChange={vi.fn()} />);

    expect(await screen.findByText("No trusted host keys")).toBeInTheDocument();
  });
});
