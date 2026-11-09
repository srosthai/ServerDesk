import { beforeEach, describe, expect, it, vi } from "vitest";
import { getQueueSupervisorStatus } from "@/services/queue";
import { useQueueStore } from "@/stores/queueStore";
import type { QueueSupervisorStatus } from "@/types/contracts";

vi.mock("@/services/queue", () => ({
  getQueueSupervisorStatus: vi.fn(),
}));

const status: QueueSupervisorStatus = {
  serverId: "server-1",
  supervisor: {
    status: "available",
    installed: true,
    programs: [
      {
        name: "horizon",
        state: "running",
        stateText: "RUNNING",
        queueRelated: true,
        detail: "pid 2102, uptime 0:10:10",
      },
    ],
    detail: null,
  },
  queueWorkers: {
    status: "available",
    workers: [
      {
        pid: 2102,
        elapsedSeconds: 610,
        kind: "horizon",
        queue: null,
        connection: null,
      },
    ],
    truncated: false,
    detail: null,
  },
  sampledAt: 1_762_665_720_000,
  durationMs: 31,
  connectionState: "connected",
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("queue store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useQueueStore.setState({ entries: {} });
    vi.mocked(getQueueSupervisorStatus).mockReset();
  });

  it("stores the typed queue and Supervisor snapshot for one server", async () => {
    vi.mocked(getQueueSupervisorStatus).mockResolvedValue(status);

    await expect(useQueueStore.getState().refresh("server-1")).resolves.toBe(true);

    expect(getQueueSupervisorStatus).toHaveBeenCalledWith("server-1");
    const entry = useQueueStore.getState().entries["server-1"];
    expect(entry?.status).toEqual(status);
    expect(entry?.pending).toBe(false);
    expect(entry?.error).toBeNull();
    expect(entry?.lastAttemptAt).not.toBeNull();
  });

  it("keeps missing data as a typed status and not as an error", async () => {
    const missing: QueueSupervisorStatus = {
      ...status,
      supervisor: {
        status: "missing",
        installed: false,
        programs: [],
        detail: "Supervisor is not installed: supervisorctl and supervisord were not found.",
      },
      queueWorkers: {
        status: "unavailable",
        workers: [],
        truncated: false,
        detail: "Queue worker processes could not be read (exit code 1).",
      },
    };
    vi.mocked(getQueueSupervisorStatus).mockResolvedValue(missing);

    await expect(useQueueStore.getState().refresh("server-1")).resolves.toBe(true);

    const entry = useQueueStore.getState().entries["server-1"];
    expect(entry?.status?.supervisor.status).toBe("missing");
    expect(entry?.status?.supervisor.programs).toEqual([]);
    expect(entry?.status?.queueWorkers.status).toBe("unavailable");
    expect(entry?.error).toBeNull();
  });

  it("stores a command error and can clear it without losing the last snapshot", async () => {
    vi.mocked(getQueueSupervisorStatus).mockResolvedValueOnce(status);
    await useQueueStore.getState().refresh("server-1");

    vi.mocked(getQueueSupervisorStatus).mockRejectedValueOnce({
      code: "timeout",
      message: "Queue detection timed out.",
      details: "Fixed detection commands exceeded the duration limit.",
    });
    await expect(useQueueStore.getState().refresh("server-1")).resolves.toBe(false);

    let entry = useQueueStore.getState().entries["server-1"];
    expect(entry?.error).toBe("Queue detection timed out.");
    expect(entry?.errorDetails).toBe("Fixed detection commands exceeded the duration limit.");
    expect(entry?.status).toEqual(status);

    useQueueStore.getState().clearError("server-1");
    entry = useQueueStore.getState().entries["server-1"];
    expect(entry?.error).toBeNull();
    expect(entry?.errorDetails).toBeNull();
    expect(entry?.status).toEqual(status);
  });

  it("clears one server without touching another", async () => {
    vi.mocked(getQueueSupervisorStatus).mockResolvedValue(status);
    await useQueueStore.getState().refresh("server-1");
    await useQueueStore.getState().refresh("server-2");

    useQueueStore.getState().clearServer("server-1");

    expect(useQueueStore.getState().entries["server-1"]).toBeUndefined();
    expect(useQueueStore.getState().entries["server-2"]?.status).toEqual(status);
  });
});
