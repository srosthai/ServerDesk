import { beforeEach, describe, expect, it, vi } from "vitest";
import { getSystemIdentity } from "@/services/identity";
import { useSystemIdentityStore } from "@/stores/systemIdentityStore";
import type { SystemIdentity } from "@/types/contracts";

vi.mock("@/services/identity", () => ({
  getSystemIdentity: vi.fn(),
}));

const identity: SystemIdentity = {
  serverId: "server-1",
  hostname: "production-1",
  distribution: "Ubuntu",
  distributionVersion: "24.04",
  kernel: "6.8.0-45-generic",
  architecture: "x86_64",
  uptimeSeconds: 372_063,
  sampledAt: 1_762_665_720_000,
  durationMs: 18,
  connectionState: "connected",
};

const partialIdentity: SystemIdentity = {
  ...identity,
  distribution: null,
  distributionVersion: null,
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("system identity store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useSystemIdentityStore.setState({
      identities: {},
      pending: {},
      stale: {},
      staleReason: {},
      lastAttemptAt: {},
      error: null,
    });
  });

  it("stores a typed identity sample and clears pending state", async () => {
    vi.mocked(getSystemIdentity).mockResolvedValue(identity);

    await expect(useSystemIdentityStore.getState().refresh("server-1")).resolves.toBe(true);

    expect(getSystemIdentity).toHaveBeenCalledWith("server-1");
    const state = useSystemIdentityStore.getState();
    expect(state.identities["server-1"]).toEqual(identity);
    expect(state.pending["server-1"]).toBe(false);
    expect(state.error).toBeNull();
  });

  it("keeps partial identity when a distribution is unsupported", async () => {
    vi.mocked(getSystemIdentity).mockResolvedValue(partialIdentity);

    await useSystemIdentityStore.getState().refresh("server-1");

    const stored = useSystemIdentityStore.getState().identities["server-1"];
    expect(stored?.hostname).toBe("production-1");
    expect(stored?.distribution).toBeNull();
    expect(stored?.distributionVersion).toBeNull();
    expect(stored?.kernel).toBe("6.8.0-45-generic");
  });

  it("reports command errors without storing a stale sample", async () => {
    vi.mocked(getSystemIdentity).mockRejectedValueOnce({
      code: "not_found",
      message: "The server is not connected.",
    });

    await expect(useSystemIdentityStore.getState().refresh("server-1")).resolves.toBe(false);

    const state = useSystemIdentityStore.getState();
    expect(state.error).toBe("The server is not connected.");
    expect(state.identities["server-1"]).toBeUndefined();
    expect(state.stale["server-1"]).toBeUndefined();
    expect(state.pending["server-1"]).toBe(false);
  });

  it("keeps the last successful identity and marks it stale after a failed refresh", async () => {
    vi.mocked(getSystemIdentity).mockResolvedValueOnce(identity);
    await useSystemIdentityStore.getState().refresh("server-1");

    vi.mocked(getSystemIdentity).mockRejectedValueOnce({
      code: "timeout",
      message: "The remote command timed out.",
    });
    await expect(useSystemIdentityStore.getState().refresh("server-1")).resolves.toBe(false);

    const state = useSystemIdentityStore.getState();
    expect(state.identities["server-1"]).toEqual(identity);
    expect(state.stale["server-1"]).toBe(true);
    expect(state.staleReason["server-1"]).toBe("The remote command timed out.");
    expect(state.lastAttemptAt["server-1"]).not.toBeNull();
  });

  it("clears stale state after a later successful identity refresh", async () => {
    vi.mocked(getSystemIdentity).mockResolvedValueOnce(identity);
    await useSystemIdentityStore.getState().refresh("server-1");

    vi.mocked(getSystemIdentity).mockRejectedValueOnce({
      code: "timeout",
      message: "The remote command timed out.",
    });
    await useSystemIdentityStore.getState().refresh("server-1");
    expect(useSystemIdentityStore.getState().stale["server-1"]).toBe(true);

    vi.mocked(getSystemIdentity).mockResolvedValueOnce(identity);
    await useSystemIdentityStore.getState().refresh("server-1");

    const state = useSystemIdentityStore.getState();
    expect(state.stale["server-1"]).toBe(false);
    expect(state.staleReason["server-1"]).toBeNull();
  });

  it("ignores a slow response after a newer refresh or disconnect", async () => {
    let resolveFirst: ((value: SystemIdentity) => void) | undefined;
    vi.mocked(getSystemIdentity).mockImplementationOnce(
      () =>
        new Promise<SystemIdentity>((resolve) => {
          resolveFirst = resolve;
        }),
    );

    const first = useSystemIdentityStore.getState().refresh("server-1");
    const newer = { ...identity, hostname: "production-2", sampledAt: identity.sampledAt + 1_000 };
    vi.mocked(getSystemIdentity).mockResolvedValueOnce(newer);
    await useSystemIdentityStore.getState().refresh("server-1");

    resolveFirst?.(identity);
    await first;

    expect(useSystemIdentityStore.getState().identities["server-1"]).toEqual(newer);
  });

  it("clears identity data when a session is released", async () => {
    vi.mocked(getSystemIdentity).mockResolvedValue(identity);
    await useSystemIdentityStore.getState().refresh("server-1");

    useSystemIdentityStore.getState().clearServer("server-1");

    const state = useSystemIdentityStore.getState();
    expect(state.identities["server-1"]).toBeUndefined();
    expect(state.stale["server-1"]).toBeUndefined();
    expect(state.staleReason["server-1"]).toBeUndefined();
    expect(state.lastAttemptAt["server-1"]).toBeUndefined();
    expect(state.pending["server-1"]).toBe(false);
    expect(state.error).toBeNull();
  });
});
