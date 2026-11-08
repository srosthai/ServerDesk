import { beforeEach, describe, expect, it, vi } from "vitest";
import { listLogSources, readLogTail } from "@/services/logs";
import { logEntryKey, useLogStore } from "@/stores/logStore";
import type { LogTail, SystemLogSource } from "@/types/contracts";

vi.mock("@/services/logs", () => ({
  listLogSources: vi.fn(),
  readLogTail: vi.fn(),
}));

const source: SystemLogSource = {
  path: "/var/log/syslog",
  label: "System log (syslog)",
  description: "General system messages.",
};

function tail(overrides: Partial<LogTail> = {}): LogTail {
  return {
    serverId: "server-1",
    requestedPath: "/var/log/syslog",
    resolvedPath: "/var/log/syslog",
    sourceKind: "system",
    status: "available",
    detail: null,
    content: "line one\nline two\n",
    returnedLines: 2,
    returnedBytes: 18,
    fileSizeBytes: 18,
    lineLimit: 200,
    byteLimit: 64 * 1024,
    truncated: false,
    truncatedBytes: false,
    truncatedLines: false,
    partialFirstLine: false,
    rotated: false,
    rotationDetail: null,
    retrievedAt: 1_762_665_720_000,
    durationMs: 12,
    connectionState: "connected",
    ...overrides,
  };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("log store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useLogStore.setState({
      sources: [],
      sourcesPending: false,
      sourcesError: null,
      sourcesLoaded: false,
      entries: {},
    });
    vi.mocked(listLogSources).mockReset();
    vi.mocked(readLogTail).mockReset();
  });

  it("loads and caches the fixed system log source list", async () => {
    vi.mocked(listLogSources).mockResolvedValue([source]);

    await expect(useLogStore.getState().fetchSources()).resolves.toBe(true);
    await expect(useLogStore.getState().fetchSources()).resolves.toBe(true);

    expect(listLogSources).toHaveBeenCalledTimes(1);
    expect(useLogStore.getState().sources).toEqual([source]);
    expect(useLogStore.getState().sourcesLoaded).toBe(true);
    expect(useLogStore.getState().sourcesError).toBeNull();
  });

  it("reads a bounded tail and stores it by server and path", async () => {
    vi.mocked(readLogTail).mockResolvedValue(tail());

    await expect(useLogStore.getState().read("server-1", "/var/log/syslog", 200)).resolves.toBe(
      true,
    );

    expect(readLogTail).toHaveBeenCalledWith("server-1", "/var/log/syslog", 200);
    const entry = useLogStore.getState().entries[logEntryKey("server-1", "/var/log/syslog")];
    expect(entry?.tail).toEqual(tail());
    expect(entry?.pending).toBe(false);
    expect(entry?.error).toBeNull();
  });

  it("keeps the last successful tail visible when a refresh fails", async () => {
    const key = logEntryKey("server-1", "/var/log/syslog");
    useLogStore.setState({
      entries: {
        [key]: {
          tail: tail(),
          pending: false,
          error: null,
          errorDetails: null,
          lastAttemptAt: 1,
        },
      },
    });
    vi.mocked(readLogTail).mockRejectedValueOnce({
      code: "permission",
      message: "The SSH account cannot read this log file.",
      details: "Try another approved log.",
    });

    await expect(useLogStore.getState().read("server-1", "/var/log/syslog", 200)).resolves.toBe(
      false,
    );

    const entry = useLogStore.getState().entries[key];
    expect(entry?.tail).toEqual(tail());
    expect(entry?.pending).toBe(false);
    expect(entry?.error).toBe("The SSH account cannot read this log file.");
    expect(entry?.errorDetails).toBe("Try another approved log.");
  });

  it("stores missing and unreadable results as typed successful reads", async () => {
    vi.mocked(readLogTail)
      .mockResolvedValueOnce(
        tail({
          status: "missing",
          detail: "The log file does not exist on the server.",
          content: "",
          returnedLines: 0,
          returnedBytes: 0,
          fileSizeBytes: null,
        }),
      )
      .mockResolvedValueOnce(
        tail({
          status: "unreadable",
          detail: "The SSH account cannot read this log file.",
          content: "",
          returnedLines: 0,
          returnedBytes: 0,
        }),
      );

    await useLogStore.getState().read("server-1", "/var/log/syslog", 200);
    await useLogStore.getState().read("server-1", "/var/log/auth.log", 200);

    expect(
      useLogStore.getState().entries[logEntryKey("server-1", "/var/log/syslog")]?.tail?.status,
    ).toBe("missing");
    expect(
      useLogStore.getState().entries[logEntryKey("server-1", "/var/log/auth.log")]?.tail?.status,
    ).toBe("unreadable");
  });

  it("clears server entries and errors without touching other servers", () => {
    useLogStore.setState({
      entries: {
        [logEntryKey("server-1", "/var/log/syslog")]: {
          tail: tail(),
          pending: false,
          error: "failed",
          errorDetails: null,
          lastAttemptAt: 1,
        },
        [logEntryKey("server-2", "/var/log/syslog")]: {
          tail: tail({ serverId: "server-2" }),
          pending: false,
          error: null,
          errorDetails: null,
          lastAttemptAt: 1,
        },
      },
    });

    useLogStore.getState().clearError("server-1", "/var/log/syslog");
    expect(
      useLogStore.getState().entries[logEntryKey("server-1", "/var/log/syslog")]?.error,
    ).toBeNull();

    useLogStore.getState().clearServer("server-1");
    expect(
      useLogStore.getState().entries[logEntryKey("server-1", "/var/log/syslog")],
    ).toBeUndefined();
    expect(
      useLogStore.getState().entries[logEntryKey("server-2", "/var/log/syslog")]?.tail,
    ).toEqual(tail({ serverId: "server-2" }));
  });
});
