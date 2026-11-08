import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useMonitoringPolling } from "@/hooks/useMonitoringPolling";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { getMonitoringSnapshot } from "@/services/monitoring";
import { useMonitoringStore } from "@/stores/monitoringStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
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
    data: [],
  },
  network: {
    status: "available",
    sampledAt,
    detail: null,
    data: [],
  },
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

async function flushMicrotasks() {
  await Promise.resolve();
  await Promise.resolve();
}

describe("useMonitoringPolling", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.useFakeTimers();
    vi.mocked(getMonitoringSnapshot).mockResolvedValue(snapshot);
    useMonitoringStore.setState({ entries: {} });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES, monitoringIntervalSeconds: 5 },
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("samples immediately and then at the configured safe interval", async () => {
    renderHook(
      ({ connected }: { connected: boolean }) => useMonitoringPolling("server-1", connected),
      {
        initialProps: { connected: true },
      },
    );

    await act(async () => {
      await flushMicrotasks();
    });
    expect(getMonitoringSnapshot).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(getMonitoringSnapshot).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(getMonitoringSnapshot).toHaveBeenCalledTimes(3);
  });

  it("stops polling and marks the retained snapshot stale when disconnected", async () => {
    const { rerender } = renderHook(
      ({ connected }: { connected: boolean }) => useMonitoringPolling("server-1", connected),
      { initialProps: { connected: true } },
    );

    await act(async () => {
      await flushMicrotasks();
    });
    expect(useMonitoringStore.getState().entries["server-1"].snapshot).toEqual(snapshot);

    rerender({ connected: false });

    const entry = useMonitoringStore.getState().entries["server-1"];
    expect(entry.stale).toBe(true);
    expect(entry.staleReason).toBe("Polling is paused because the server is disconnected.");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });
    expect(getMonitoringSnapshot).toHaveBeenCalledTimes(1);
  });

  it("does not poll when no server is selected", async () => {
    renderHook(() => useMonitoringPolling(null, true));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });

    expect(getMonitoringSnapshot).not.toHaveBeenCalled();
  });
});
