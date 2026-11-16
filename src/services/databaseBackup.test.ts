import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelDatabaseBackup,
  createDatabaseBackup,
  createDatabaseBackupRequestId,
  databaseBackupCommands,
  getDatabaseBackupTools,
  previewDatabaseRestore,
  restoreDatabaseBackup,
} from "@/services/databaseBackup";
import type {
  DatabaseBackupRequest,
  DatabaseBackupRestorePreview,
  DatabaseBackupRestorePreviewRequest,
  DatabaseBackupRestoreRequest,
  DatabaseDumpToolReport,
} from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const report: DatabaseDumpToolReport = {
  engine: "postgresql",
  serverVersion: "16.2.0",
  tools: [],
  exportReady: true,
  restoreReady: true,
  checkedAt: 1,
};

const backupRequest: DatabaseBackupRequest = {
  scope: { kind: "database", database: "app" },
  format: "custom",
  tool: "pg_dump",
  destination: { kind: "remote", path: "/home/deploy/backup.dump", overwrite: false },
  noOwner: false,
};

const previewRequest: DatabaseBackupRestorePreviewRequest = {
  source: { kind: "remote", path: "/home/deploy/backup.dump" },
  format: "custom",
  tool: "pg_restore",
  targetDatabase: "app",
  clean: true,
  noOwner: false,
};

const preview: DatabaseBackupRestorePreview = {
  serverId: "server-1",
  serverName: "Production",
  profileId: "profile-1",
  profileName: "Primary",
  engine: "postgresql",
  targetDatabase: "app",
  sourceKind: "remote",
  sourcePath: "/home/deploy/backup.dump",
  format: "custom",
  tool: "pg_restore",
  toolAvailable: true,
  toolCompatible: true,
  toolVersion: "16.2.0",
  replacesExistingData: true,
  replacedData: "all existing objects and data in database app may be dropped and replaced",
  clean: true,
  noOwner: false,
  canRestore: true,
  warnings: [],
  confirmation: {
    serverId: "server-1",
    serverName: "Production",
    targetDatabase: "app",
    replacedData: "all existing objects and data in database app may be dropped and replaced",
    acknowledged: false,
  },
};

const restoreRequest: DatabaseBackupRestoreRequest = {
  source: previewRequest.source,
  format: previewRequest.format,
  tool: previewRequest.tool,
  targetDatabase: previewRequest.targetDatabase,
  clean: previewRequest.clean,
  noOwner: previewRequest.noOwner,
  confirmed: true,
  confirmation: { ...preview.confirmation, acknowledged: true },
};

describe("database backup service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("uses stable Tauri command names", () => {
    expect(databaseBackupCommands).toEqual({
      tools: "get_database_backup_tools",
      previewRestore: "preview_database_restore",
      create: "create_database_backup",
      restore: "restore_database_backup",
      cancel: "cancel_database_backup",
    });
  });

  it("detects tools with the profile id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(report);
    await expect(getDatabaseBackupTools("profile-1")).resolves.toEqual(report);
    expect(invoke).toHaveBeenCalledWith("get_database_backup_tools", { profileId: "profile-1" });
  });

  it("builds a restore preview without a confirmation acknowledgement", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(preview);
    await expect(previewDatabaseRestore("profile-1", previewRequest)).resolves.toEqual(preview);
    expect(invoke).toHaveBeenCalledWith("preview_database_restore", {
      profileId: "profile-1",
      request: previewRequest,
    });
  });

  it("sends backup and restore requests with camelCase fields and null timeouts", async () => {
    vi.mocked(invoke).mockResolvedValue({});
    await createDatabaseBackup("profile-1", backupRequest, "backup-1");
    expect(invoke).toHaveBeenCalledWith("create_database_backup", {
      profileId: "profile-1",
      requestId: "backup-1",
      request: backupRequest,
      timeoutMs: null,
    });

    await restoreDatabaseBackup("profile-1", restoreRequest, "backup-2", 60_000);
    expect(invoke).toHaveBeenCalledWith("restore_database_backup", {
      profileId: "profile-1",
      requestId: "backup-2",
      request: restoreRequest,
      timeoutMs: 60_000,
    });
  });

  it("passes the explicit overwrite policy through unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue({});
    const protectedRequest: DatabaseBackupRequest = {
      ...backupRequest,
      destination: { kind: "remote", path: "/home/deploy/backup.dump", overwrite: false },
    };
    await createDatabaseBackup("profile-1", protectedRequest, "backup-3");
    expect(invoke).toHaveBeenCalledWith(
      "create_database_backup",
      expect.objectContaining({
        request: expect.objectContaining({
          destination: { kind: "remote", path: "/home/deploy/backup.dump", overwrite: false },
        }),
      }),
    );
  });

  it("cancels by request id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ requestId: "backup-1", cancelled: true });
    await expect(cancelDatabaseBackup("backup-1")).resolves.toEqual({
      requestId: "backup-1",
      cancelled: true,
    });
    expect(invoke).toHaveBeenCalledWith("cancel_database_backup", { requestId: "backup-1" });
  });

  it("creates bounded ASCII request ids", () => {
    const id = createDatabaseBackupRequestId();
    expect(id).toMatch(/^backup-[A-Za-z0-9-]+$/);
    expect(id.length).toBeLessThanOrEqual(128);
  });
});
