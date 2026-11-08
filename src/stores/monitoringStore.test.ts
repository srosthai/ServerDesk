import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMonitoringSnapshot } from "@/services/monitoring";
import { useMonitoringStore } from "@/stores/monitoringStore";
import type { MonitoringSnapshot } from "@/types/contracts";

vi.mock("@/services/monitoring", () => ({
  getMonitoringSnapshot: vi.fn(),
}));

const sampledAt = 1_762_665_720_000;

const snapshot: MonitoringSnapshot = {
  serverId: "server-1",
  sampledAt,
  durationMs: 42,
  connectionState: "connected",
  cpu: {
    status: "available",
    sampledAt,
    detail: null,
    data: { load1m: 0.42, load5m: 0.31, load15m: 0.28, logicalCpus: 8 },
  },
  memory: {
    status: "available",
    sampledAt,
    detail: null,
    data: {
      totalBytes: 8 * 1024 ** 3,
      usedBytes: 2 * 1024 ** 3,
      availableBytes: 6 * 1024 ** 3,
      usedPercent: 25,
    },
  },
  filesystems: {
    status: "available",
    sampledAt,
    detail: null,
    data: [
      {
        filesystem: "/dev/vda1",
        mount: "/",
        totalBytes: 100 * 1024 ** 3,
        usedBytes: 40 * 1024 ** 3,
        availableBytes: 60 * 1024 ** 3,
        usedPercent: 40,
      },
    ],
  },
  network: {
    status: "available",
    sampledAt,
    detail: null,
    data: [
      {
        interface: "eth0",
        receivedBytes: 1_500_000_000,
        transmittedBytes: 750_000_000,
        receivedPackets: 12_345,
        transmittedPackets: 6_789,
      },
    ],
  },
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("monitoring store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useMonitoringStore.setState({ entries: {} });
  });

  it("stores a typed snapshot and clears pending state", async () => {
    vi.mocked(getMonitoringSnapshot).mockResolvedValue(snapshot);

    await expect(useMonitoringStore.getState().refresh("server-1")).resolves.toBe(true);

    expect(getMonitoringSnapshot).toHaveBeenCalledWith("server-1");
    const entry = useMonitoringStore.getState().entries["server-1"];
    expect(entry.snapshot).toEqual(snapshot);
    expect(entry.stale).toBe(false);
    expect(entry.staleReason).toBeNull();
    expect(entry.error).toBeNull();
    expect(entry.pending).toBe(false);
    expect(entry.lastAttemptAt).not.toBeNull();
  });

  it("preserves the last snapshot and marks it stale when a refresh fails", async () => {
    vi.mocked(getMonitoringSnapshot).mockResolvedValueOnce(snapshot);
    await useMonitoringStore.getState().refresh("server-1");

    vi.mocked(getMonitoringSnapshot).mockRejectedValueOnce({
      code: "timeout",
      message: "The remote command timed out.",
    });

    await expect(useMonitoringStore.getState().refresh("server-1")).resolves.toBe(false);

    const entry = useMonitoringStore.getState().entries["server-1"];
    expect(entry.snapshot).toEqual(snapshot);
    expect(entry.snapshot?.sampledAt).toBe(sampledAt);
    expect(entry.stale).toBe(true);
    expect(entry.staleReason).toBe("The remote command timed out.");
    expect(entry.error).toBe("The remote command timed out.");
    expect(entry.pending).toBe(false);
  });

  it("reports a failed first refresh without inventing a snapshot", async () => {
    vi.mocked(getMonitoringSnapshot).mockRejectedValueOnce({
      code: "not_found",
      message: "The server is not connected.",
    });

    await expect(useMonitoringStore.getState().refresh("server-1")).resolves.toBe(false);

    const entry = useMonitoringStore.getState().entries["server-1"];
    expect(entry.snapshot).toBeNull();
    expect(entry.stale).toBe(true);
    expect(entry.error).toBe("The server is not connected.");
    expect(entry.pending).toBe(false);
  });

  it("clears stale state after a later successful refresh", async () => {
    vi.mocked(getMonitoringSnapshot).mockRejectedValueOnce({
      code: "timeout",
      message: "The remote command timed out.",
    });
    await useMonitoringStore.getState().refresh("server-1");
    expect(useMonitoringStore.getState().entries["server-1"].stale).toBe(true);

    vi.mocked(getMonitoringSnapshot).mockResolvedValueOnce(snapshot);
    await useMonitoringStore.getState().refresh("server-1");

    const entry = useMonitoringStore.getState().entries["server-1"];
    expect(entry.snapshot).toEqual(snapshot);
    expect(entry.stale).toBe(false);
    expect(entry.staleReason).toBeNull();
    expect(entry.error).toBeNull();
  });

  it("marks a retained snapshot stale when polling pauses", async () => {
    vi.mocked(getMonitoringSnapshot).mockResolvedValueOnce(snapshot);
    await useMonitoringStore.getState().refresh("server-1");

    useMonitoringStore
      .getState()
      .markStale("server-1", "Polling is paused because the server is disconnected.");

    const entry = useMonitoringStore.getState().entries["server-1"];
    expect(entry.snapshot).toEqual(snapshot);
    expect(entry.stale).toBe(true);
    expect(entry.staleReason).toBe("Polling is paused because the server is disconnected.");
  });

  it("does not mark a server without a snapshot as stale on pause", () => {
    useMonitoringStore.getState().markStale("server-1", "Polling paused.");

    expect(useMonitoringStore.getState().entries["server-1"]).toBeUndefined();
  });

  it("ignores a slow response after a newer refresh or clear", async () => {
    let resolveFirst: ((value: MonitoringSnapshot) => void) | undefined;
    vi.mocked(getMonitoringSnapshot).mockImplementationOnce(
      () =>
        new Promise<MonitoringSnapshot>((resolve) => {
          resolveFirst = resolve;
        }),
    );

    const first = useMonitoringStore.getState().refresh("server-1");
    const newer = { ...snapshot, sampledAt: sampledAt + 1_000 };
    vi.mocked(getMonitoringSnapshot).mockResolvedValueOnce(newer);
    await useMonitoringStore.getState().refresh("server-1");

    resolveFirst?.(snapshot);
    await first;

    expect(useMonitoringStore.getState().entries["server-1"].snapshot).toEqual(newer);
  });

  it("clears a server entry when it is no longer tracked", async () => {
    vi.mocked(getMonitoringSnapshot).mockResolvedValue(snapshot);
    await useMonitoringStore.getState().refresh("server-1");

    useMonitoringStore.getState().clearServer("server-1");

    expect(useMonitoringStore.getState().entries["server-1"]).toBeUndefined();
  });
});
