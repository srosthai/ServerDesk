import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelDatabaseBackup,
  createDatabaseBackup,
  createDatabaseBackupRequestId,
  getDatabaseBackupTools,
  previewDatabaseRestore,
  restoreDatabaseBackup,
} from "@/services/databaseBackup";
import { useDatabaseBackupStore } from "@/stores/databaseBackupStore";
import type {
  DatabaseBackupRequest,
  DatabaseBackupRestorePreview,
  DatabaseBackupRestorePreviewRequest,
  DatabaseBackupRestoreRequest,
  DatabaseBackupResult,
  DatabaseBackupRestoreResult,
  DatabaseDumpProgress,
  DatabaseDumpToolReport,
} from "@/types/contracts";

vi.mock("@/services/databaseBackup", () => ({
  cancelDatabaseBackup: vi.fn(),
  createDatabaseBackup: vi.fn(),
  createDatabaseBackupRequestId: vi.fn(),
  getDatabaseBackupTools: vi.fn(),
  previewDatabaseRestore: vi.fn(),
  restoreDatabaseBackup: vi.fn(),
}));

const backupRequest: DatabaseBackupRequest = {
  scope: { kind: "database", database: "app" },
  format: "plain_sql",
  tool: "pg_dump",
  destination: { kind: "remote", path: "/home/deploy/backup.sql", overwrite: false },
  noOwner: false,
};

const previewRequest: DatabaseBackupRestorePreviewRequest = {
  source: { kind: "remote", path: "/home/deploy/backup.sql" },
  format: "plain_sql",
  tool: "psql",
  targetDatabase: "app",
  clean: false,
  noOwner: false,
};

function toolReport(): DatabaseDumpToolReport {
  return {
    engine: "postgresql",
    serverVersion: "16.2.0",
    tools: [],
    exportReady: true,
    restoreReady: true,
    checkedAt: 1,
  };
}

function preview(): DatabaseBackupRestorePreview {
  return {
    serverId: "server-1",
    serverName: "Production",
    profileId: "profile-1",
    profileName: "Primary",
    engine: "postgresql",
    targetDatabase: "app",
    sourceKind: "remote",
    sourcePath: "/home/deploy/backup.sql",
    format: "plain_sql",
    tool: "psql",
    toolAvailable: true,
    toolCompatible: true,
    replacesExistingData: true,
    replacedData:
      "existing objects and rows that conflict with the dump in database app may be replaced",
    clean: false,
    noOwner: false,
    canRestore: true,
    warnings: [],
    confirmation: {
      serverId: "server-1",
      serverName: "Production",
      targetDatabase: "app",
      replacedData:
        "existing objects and rows that conflict with the dump in database app may be replaced",
      acknowledged: false,
    },
  };
}

function restoreRequest(acknowledged = true): DatabaseBackupRestoreRequest {
  const plan = preview();
  return {
    source: previewRequest.source,
    format: previewRequest.format,
    tool: previewRequest.tool,
    targetDatabase: previewRequest.targetDatabase,
    clean: previewRequest.clean,
    noOwner: previewRequest.noOwner,
    confirmed: true,
    confirmation: { ...plan.confirmation, acknowledged },
  };
}

function backupResult(): DatabaseBackupResult {
  return {
    status: "completed",
    engine: "postgresql",
    scope: { kind: "database", database: "app" },
    format: "plain_sql",
    destinationKind: "remote",
    destinationPath: "/home/deploy/backup.sql",
    bytesWritten: 4096,
    verified: true,
    tool: "pg_dump",
    toolVersion: "16.2.0",
    exitCode: 0,
    warnings: [],
    partialRemoved: false,
    tempCleaned: true,
    mayBeIncomplete: false,
    durationMs: 8,
  };
}

function restoreResult(): DatabaseBackupRestoreResult {
  return {
    status: "cancelled",
    engine: "postgresql",
    format: "plain_sql",
    sourceKind: "remote",
    sourcePath: "/home/deploy/backup.sql",
    targetDatabase: "app",
    bytesRead: 4096,
    tool: "psql",
    toolVersion: "16.2.0",
    exitCode: 143,
    warnings: [],
    failureSummary: "Cancellation was requested.",
    partialRemoved: false,
    tempCleaned: true,
    mayBeIncomplete: true,
    durationMs: 9,
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

describe("database backup store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseBackupStore.setState({
      tools: { status: "idle" },
      preview: { status: "idle" },
      transfer: { direction: "backup", status: "idle", cancelling: false },
    });
  });

  it("detects tools without starting a transfer", async () => {
    vi.mocked(getDatabaseBackupTools).mockResolvedValueOnce(toolReport());

    await expect(useDatabaseBackupStore.getState().detectTools("profile-1")).resolves.toEqual(
      toolReport(),
    );
    expect(useDatabaseBackupStore.getState().tools.status).toBe("ready");
    expect(useDatabaseBackupStore.getState().transfer.status).toBe("idle");
  });

  it("previews a restore with server, database, and replaced data", async () => {
    vi.mocked(previewDatabaseRestore).mockResolvedValueOnce(preview());

    await expect(
      useDatabaseBackupStore.getState().previewRestore("profile-1", previewRequest),
    ).resolves.toEqual(preview());
    const state = useDatabaseBackupStore.getState().preview;
    expect(state.status).toBe("ready");
    expect(state.preview?.serverName).toBe("Production");
    expect(state.preview?.targetDatabase).toBe("app");
    expect(state.preview?.replacedData).toContain("may be replaced");
    expect(state.preview?.confirmation.acknowledged).toBe(false);
  });

  it("tracks a backup and applies only matching progress", async () => {
    vi.mocked(createDatabaseBackupRequestId).mockReturnValueOnce("backup-1");
    const pending = deferred<DatabaseBackupResult>();
    vi.mocked(createDatabaseBackup).mockReturnValueOnce(pending.promise);

    const request = useDatabaseBackupStore.getState().startBackup("profile-1", backupRequest);
    expect(useDatabaseBackupStore.getState().transfer.status).toBe("running");

    const progress: DatabaseDumpProgress = {
      transferId: "backup-1",
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
    useDatabaseBackupStore.getState().applyTransferProgress(progress);
    expect(useDatabaseBackupStore.getState().transfer.progress?.bytesProcessed).toBe(1024);
    useDatabaseBackupStore.getState().applyTransferProgress({
      ...progress,
      transferId: "other",
      bytesProcessed: 999,
    });
    expect(useDatabaseBackupStore.getState().transfer.progress?.bytesProcessed).toBe(1024);

    pending.resolve(backupResult());
    await expect(request).resolves.toEqual(backupResult());
    expect(useDatabaseBackupStore.getState().transfer.status).toBe("completed");
    expect(useDatabaseBackupStore.getState().transfer.backupResult?.verified).toBe(true);
  });

  it("surfaces overwrite protection without claiming a backup succeeded", async () => {
    vi.mocked(createDatabaseBackupRequestId).mockReturnValueOnce("backup-2");
    vi.mocked(createDatabaseBackup).mockRejectedValueOnce({
      code: "conflict",
      message:
        "The remote dump destination already exists. Choose another path or allow overwrite.",
    });

    await expect(
      useDatabaseBackupStore.getState().startBackup("profile-1", backupRequest),
    ).resolves.toBeNull();
    expect(useDatabaseBackupStore.getState().transfer.status).toBe("error");
    expect(useDatabaseBackupStore.getState().transfer.errorMessage).toContain("already exists");
    expect(useDatabaseBackupStore.getState().transfer.backupResult).toBeUndefined();
  });

  it("refuses to restore without an acknowledged confirmation", async () => {
    await expect(
      useDatabaseBackupStore.getState().startRestore("profile-1", restoreRequest(false)),
    ).resolves.toBeNull();
    expect(restoreDatabaseBackup).not.toHaveBeenCalled();
    expect(useDatabaseBackupStore.getState().transfer.status).toBe("error");
    expect(useDatabaseBackupStore.getState().transfer.errorMessage).toContain("confirmation");
  });

  it("keeps a cancelled restore visible as cancelled and incomplete", async () => {
    vi.mocked(createDatabaseBackupRequestId).mockReturnValueOnce("backup-3");
    vi.mocked(restoreDatabaseBackup).mockResolvedValueOnce(restoreResult());

    await expect(
      useDatabaseBackupStore.getState().startRestore("profile-1", restoreRequest()),
    ).resolves.toEqual(restoreResult());
    expect(useDatabaseBackupStore.getState().transfer.status).toBe("cancelled");
    expect(useDatabaseBackupStore.getState().transfer.restoreResult?.mayBeIncomplete).toBe(true);
    expect(useDatabaseBackupStore.getState().transfer.restoreResult?.tempCleaned).toBe(true);
  });

  it("starts an acknowledged restore and stores the completed result", async () => {
    vi.mocked(createDatabaseBackupRequestId).mockReturnValueOnce("backup-8");
    vi.mocked(restoreDatabaseBackup).mockResolvedValueOnce({
      ...restoreResult(),
      status: "completed",
      exitCode: 0,
      failureSummary: undefined,
      mayBeIncomplete: false,
    });

    await useDatabaseBackupStore.getState().startRestore("profile-1", restoreRequest());
    const transfer = useDatabaseBackupStore.getState().transfer;
    expect(transfer.status).toBe("completed");
    expect(transfer.restoreResult?.mayBeIncomplete).toBe(false);
    expect(restoreDatabaseBackup).toHaveBeenCalledWith(
      "profile-1",
      expect.objectContaining({
        confirmed: true,
        confirmation: expect.objectContaining({
          serverName: "Production",
          targetDatabase: "app",
          acknowledged: true,
        }),
      }),
      "backup-8",
    );
  });

  it("keeps a failed restore marked incomplete instead of successful", async () => {
    vi.mocked(createDatabaseBackupRequestId).mockReturnValueOnce("backup-7");
    vi.mocked(restoreDatabaseBackup).mockResolvedValueOnce({
      ...restoreResult(),
      status: "failed",
      exitCode: 1,
      failureSummary: "pg_restore exited with status 1.",
      mayBeIncomplete: true,
    });

    await useDatabaseBackupStore.getState().startRestore("profile-1", restoreRequest());
    const transfer = useDatabaseBackupStore.getState().transfer;
    expect(transfer.status).toBe("failed");
    expect(transfer.restoreResult?.mayBeIncomplete).toBe(true);
    expect(transfer.restoreResult?.failureSummary).toContain("status 1");
  });

  it("cancels the active transfer id", async () => {
    useDatabaseBackupStore.setState({
      transfer: {
        transferId: "backup-4",
        direction: "backup",
        status: "running",
        cancelling: false,
      },
    });
    vi.mocked(cancelDatabaseBackup).mockResolvedValueOnce({
      requestId: "backup-4",
      cancelled: true,
    });

    await expect(useDatabaseBackupStore.getState().cancelTransfer()).resolves.toBe(true);
    expect(cancelDatabaseBackup).toHaveBeenCalledWith("backup-4");
    expect(useDatabaseBackupStore.getState().transfer.cancelling).toBe(true);
  });

  it("surfaces missing-tool errors without a result", async () => {
    vi.mocked(createDatabaseBackupRequestId).mockReturnValueOnce("backup-5");
    vi.mocked(createDatabaseBackup).mockRejectedValueOnce({
      code: "capability",
      message: "pg_dump is not installed on the remote server.",
    });

    await useDatabaseBackupStore.getState().startBackup("profile-1", backupRequest);
    expect(useDatabaseBackupStore.getState().transfer.status).toBe("error");
    expect(useDatabaseBackupStore.getState().transfer.errorMessage).toContain("not installed");
  });

  it("does not start a second transfer while one is running", async () => {
    useDatabaseBackupStore.setState({
      transfer: {
        transferId: "backup-6",
        direction: "backup",
        status: "running",
        cancelling: false,
      },
    });

    await expect(
      useDatabaseBackupStore.getState().startBackup("profile-1", backupRequest),
    ).resolves.toBeNull();
    expect(createDatabaseBackup).not.toHaveBeenCalled();
  });
});
