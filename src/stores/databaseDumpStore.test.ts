import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelDatabaseDump,
  createDatabaseDumpRequestId,
  exportDatabaseDump,
  getDatabaseDumpTools,
  importDatabaseDump,
} from "@/services/databaseDump";
import { useDatabaseDumpStore } from "@/stores/databaseDumpStore";
import type {
  DatabaseDumpExportRequest,
  DatabaseDumpExportResult,
  DatabaseDumpProgress,
  DatabaseDumpRestoreRequest,
  DatabaseDumpRestoreResult,
  DatabaseDumpToolReport,
} from "@/types/contracts";

vi.mock("@/services/databaseDump", () => ({
  cancelDatabaseDump: vi.fn(),
  createDatabaseDumpRequestId: vi.fn(),
  exportDatabaseDump: vi.fn(),
  getDatabaseDumpTools: vi.fn(),
  importDatabaseDump: vi.fn(),
}));

const exportRequest: DatabaseDumpExportRequest = {
  scope: { kind: "database", database: "app" },
  format: "plain_sql",
  destination: { kind: "remote", path: "/home/deploy/backup.sql", overwrite: false },
  noOwner: false,
};

const restoreRequest: DatabaseDumpRestoreRequest = {
  source: { kind: "local", path: "/tmp/backup.sql" },
  format: "plain_sql",
  targetDatabase: "app",
  clean: false,
  noOwner: false,
  confirmed: true,
};

function toolReport(): DatabaseDumpToolReport {
  return {
    engine: "postgresql",
    serverVersion: "16.2.0",
    tools: [
      {
        tool: "pg_dump",
        binary: "pg_dump",
        available: true,
        path: "/usr/bin/pg_dump",
        version: "16.2.0",
        versionMajor: 16,
        vendor: "postgresql",
        compatible: true,
      },
      {
        tool: "pg_restore",
        binary: "pg_restore",
        available: false,
        vendor: "unknown",
        compatible: false,
        detail: "Not installed or not on PATH.",
      },
    ],
    exportReady: true,
    restoreReady: false,
    checkedAt: 1,
  };
}

function exportResult(): DatabaseDumpExportResult {
  return {
    status: "completed",
    engine: "postgresql",
    scope: { kind: "database", database: "app" },
    format: "plain_sql",
    destinationKind: "remote",
    destinationPath: "/home/deploy/backup.sql",
    bytesWritten: 2048,
    verified: true,
    tool: "pg_dump",
    toolVersion: "16.2.0",
    exitCode: 0,
    warnings: [],
    partialRemoved: false,
    tempCleaned: true,
    mayBeIncomplete: false,
    durationMs: 5,
  };
}

function restoreResult(): DatabaseDumpRestoreResult {
  return {
    status: "cancelled",
    engine: "postgresql",
    format: "plain_sql",
    sourceKind: "local",
    sourcePath: "/tmp/backup.sql",
    targetDatabase: "app",
    bytesRead: 12,
    tool: "psql",
    toolVersion: "16.2.0",
    exitCode: 143,
    warnings: [],
    failureSummary: "Cancellation was requested.",
    partialRemoved: true,
    tempCleaned: true,
    mayBeIncomplete: true,
    durationMs: 8,
  };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("database dump store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseDumpStore.setState({
      tools: { status: "idle" },
      transfer: { direction: "export", status: "idle", cancelling: false },
    });
  });

  it("detects tools without starting a transfer", async () => {
    vi.mocked(getDatabaseDumpTools).mockResolvedValueOnce(toolReport());

    await expect(useDatabaseDumpStore.getState().detectTools("profile-1")).resolves.toEqual(
      toolReport(),
    );
    expect(getDatabaseDumpTools).toHaveBeenCalledWith("profile-1");
    expect(useDatabaseDumpStore.getState().tools.status).toBe("ready");
    expect(useDatabaseDumpStore.getState().tools.report?.restoreReady).toBe(false);
    expect(useDatabaseDumpStore.getState().transfer.status).toBe("idle");
  });

  it("reports tool detection failures", async () => {
    vi.mocked(getDatabaseDumpTools).mockRejectedValueOnce({
      code: "connection",
      message: "The server is not connected.",
    });

    await expect(useDatabaseDumpStore.getState().detectTools("profile-1")).resolves.toBeNull();
    expect(useDatabaseDumpStore.getState().tools.status).toBe("error");
    expect(useDatabaseDumpStore.getState().tools.errorMessage).toContain("not connected");
  });

  it("tracks an export and applies only matching progress", async () => {
    vi.mocked(createDatabaseDumpRequestId).mockReturnValueOnce("dump-1");
    const pending = deferred<DatabaseDumpExportResult>();
    vi.mocked(exportDatabaseDump).mockReturnValueOnce(pending.promise);

    const request = useDatabaseDumpStore.getState().startExport("profile-1", exportRequest);
    expect(useDatabaseDumpStore.getState().transfer.status).toBe("running");

    const progress: DatabaseDumpProgress = {
      transferId: "dump-1",
      direction: "export",
      status: "running",
      phase: "dumping",
      bytesProcessed: 1024,
      totalBytes: null,
      percent: null,
      message: "Writing the dump on the server.",
      mayBeIncomplete: false,
      emittedAt: 1,
    };
    useDatabaseDumpStore.getState().applyTransferProgress(progress);
    expect(useDatabaseDumpStore.getState().transfer.progress?.bytesProcessed).toBe(1024);
    useDatabaseDumpStore.getState().applyTransferProgress({
      ...progress,
      transferId: "other",
      bytesProcessed: 999,
    });
    expect(useDatabaseDumpStore.getState().transfer.progress?.bytesProcessed).toBe(1024);

    pending.resolve(exportResult());
    await expect(request).resolves.toEqual(exportResult());
    expect(useDatabaseDumpStore.getState().transfer.status).toBe("completed");
    expect(useDatabaseDumpStore.getState().transfer.exportResult?.bytesWritten).toBe(2048);
  });

  it("keeps a cancelled restore visible as cancelled and incomplete", async () => {
    vi.mocked(createDatabaseDumpRequestId).mockReturnValueOnce("dump-2");
    vi.mocked(importDatabaseDump).mockResolvedValueOnce(restoreResult());

    await expect(
      useDatabaseDumpStore.getState().startRestore("profile-1", restoreRequest),
    ).resolves.toEqual(restoreResult());
    expect(useDatabaseDumpStore.getState().transfer.status).toBe("cancelled");
    expect(useDatabaseDumpStore.getState().transfer.restoreResult?.mayBeIncomplete).toBe(true);
  });

  it("cancels the active transfer id", async () => {
    useDatabaseDumpStore.setState({
      transfer: {
        transferId: "dump-3",
        direction: "export",
        status: "running",
        cancelling: false,
      },
    });
    vi.mocked(cancelDatabaseDump).mockResolvedValueOnce({
      requestId: "dump-3",
      cancelled: true,
    });

    await expect(useDatabaseDumpStore.getState().cancelTransfer()).resolves.toBe(true);
    expect(cancelDatabaseDump).toHaveBeenCalledWith("dump-3");
    expect(useDatabaseDumpStore.getState().transfer.cancelling).toBe(true);
  });

  it("surfaces export errors without a result", async () => {
    vi.mocked(createDatabaseDumpRequestId).mockReturnValueOnce("dump-4");
    vi.mocked(exportDatabaseDump).mockRejectedValueOnce({
      code: "capability",
      message: "mysqldump is not installed on the remote server.",
    });

    await useDatabaseDumpStore.getState().startExport("profile-1", exportRequest);
    expect(useDatabaseDumpStore.getState().transfer.status).toBe("error");
    expect(useDatabaseDumpStore.getState().transfer.errorMessage).toContain("not installed");
  });

  it("does not start a second transfer while one is running", async () => {
    useDatabaseDumpStore.setState({
      transfer: {
        transferId: "dump-5",
        direction: "export",
        status: "running",
        cancelling: false,
      },
    });

    await expect(
      useDatabaseDumpStore.getState().startExport("profile-1", exportRequest),
    ).resolves.toBeNull();
    expect(exportDatabaseDump).not.toHaveBeenCalled();
  });
});
