import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  deleteKnownHost,
  listKnownHosts,
  rejectHostKey,
  trustHostKey,
} from "@/services/knownHosts";
import type { ConnectionSnapshot, TrustedHostKey } from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const knownHost: TrustedHostKey = {
  id: "host-1",
  serverId: "server-1",
  keyType: "ssh-ed25519",
  fingerprint: "SHA256:trusted",
  firstSeenAt: 1,
  verifiedAt: 2,
};

const disconnected: ConnectionSnapshot = {
  serverId: "server-1",
  state: "disconnected",
  changedAt: 3,
};

describe("known host service", () => {
  it("uses the registered command names and argument shapes", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce([knownHost])
      .mockResolvedValueOnce(knownHost)
      .mockResolvedValueOnce(disconnected)
      .mockResolvedValueOnce(undefined);

    await expect(listKnownHosts("server-1")).resolves.toEqual([knownHost]);
    expect(invoke).toHaveBeenNthCalledWith(1, "list_known_hosts", { id: "server-1" });

    await expect(trustHostKey("server-1", "ssh-ed25519", "SHA256:trusted")).resolves.toEqual(
      knownHost,
    );
    expect(invoke).toHaveBeenNthCalledWith(2, "trust_host_key", {
      id: "server-1",
      keyType: "ssh-ed25519",
      fingerprint: "SHA256:trusted",
    });

    await expect(rejectHostKey("server-1", "ssh-ed25519", "SHA256:trusted")).resolves.toEqual(
      disconnected,
    );
    expect(invoke).toHaveBeenNthCalledWith(3, "reject_host_key", {
      id: "server-1",
      keyType: "ssh-ed25519",
      fingerprint: "SHA256:trusted",
    });

    await expect(deleteKnownHost("server-1", "ssh-ed25519")).resolves.toBeUndefined();
    expect(invoke).toHaveBeenNthCalledWith(4, "delete_known_host", {
      id: "server-1",
      keyType: "ssh-ed25519",
      confirmed: true,
    });
  });
});
