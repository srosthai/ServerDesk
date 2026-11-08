import { beforeEach, describe, expect, it, vi } from "vitest";
import { deleteKnownHost, listKnownHosts } from "@/services/knownHosts";
import { useHostKeyStore } from "@/stores/hostKeyStore";
import type { TrustedHostKey } from "@/types/contracts";

vi.mock("@/services/knownHosts", () => ({
  listKnownHosts: vi.fn(),
  trustHostKey: vi.fn(),
  rejectHostKey: vi.fn(),
  deleteKnownHost: vi.fn(),
}));

const knownHosts: TrustedHostKey[] = [
  {
    id: "host-1",
    serverId: "server-1",
    keyType: "ssh-ed25519",
    fingerprint: "SHA256:trusted",
    firstSeenAt: 1,
    verifiedAt: 2,
  },
];

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("host key store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useHostKeyStore.setState({ knownHosts: {}, status: {}, error: null });
  });

  it("loads trusted fingerprints from the backend", async () => {
    vi.mocked(listKnownHosts).mockResolvedValue(knownHosts);

    await useHostKeyStore.getState().fetchKnownHosts("server-1");

    expect(listKnownHosts).toHaveBeenCalledWith("server-1");
    expect(useHostKeyStore.getState().knownHosts["server-1"]).toEqual(knownHosts);
    expect(useHostKeyStore.getState().status["server-1"]).toBe("ready");
  });

  it("removes a trusted fingerprint after confirmation reaches the store", async () => {
    vi.mocked(deleteKnownHost).mockResolvedValue(undefined);
    useHostKeyStore.setState({
      knownHosts: { "server-1": knownHosts },
      status: { "server-1": "ready" },
    });

    await expect(
      useHostKeyStore.getState().removeKnownHost("server-1", "ssh-ed25519"),
    ).resolves.toBe(true);

    expect(deleteKnownHost).toHaveBeenCalledWith("server-1", "ssh-ed25519");
    expect(useHostKeyStore.getState().knownHosts["server-1"]).toEqual([]);
  });

  it("keeps the fingerprint and surfaces a typed removal error", async () => {
    vi.mocked(deleteKnownHost).mockRejectedValue({
      code: "not_found",
      message: "That trusted host key was not found.",
    });
    useHostKeyStore.setState({
      knownHosts: { "server-1": knownHosts },
      status: { "server-1": "ready" },
    });

    await expect(
      useHostKeyStore.getState().removeKnownHost("server-1", "ssh-ed25519"),
    ).resolves.toBe(false);

    expect(useHostKeyStore.getState().knownHosts["server-1"]).toEqual(knownHosts);
    expect(useHostKeyStore.getState().error).toBe("That trusted host key was not found.");
  });
});
