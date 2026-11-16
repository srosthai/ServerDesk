import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelDatabaseDump,
  createDatabaseDumpRequestId,
  databaseDumpCommands,
  exportDatabaseDump,
  getDatabaseDumpTools,
  importDatabaseDump,
} from "@/services/databaseDump";
import type {
  DatabaseDumpExportRequest,
  DatabaseDumpRestoreRequest,
  DatabaseDumpToolReport,
} from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const report: DatabaseDumpToolReport = {
  engine: "mysql",
  serverVersion: "8.0.36",
  tools: [],
  exportReady: true,
  restoreReady: true,
  checkedAt: 1,
};

const exportRequest: DatabaseDumpExportRequest = {
  scope: { kind: "database", database: "app" },
  format: "plain_sql",
  destination: { kind: "remote", path: "/home/deploy/backup.sql", overwrite: false },
  noOwner: false,
};

const restoreRequest: DatabaseDumpRestoreRequest = {
  source: { kind: "remote", path: "/home/deploy/backup.sql" },
  format: "plain_sql",
  targetDatabase: "app",
  clean: false,
  noOwner: false,
  confirmed: true,
};

describe("database dump service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("uses stable Tauri command names", () => {
    expect(databaseDumpCommands).toEqual({
      tools: "get_database_dump_tools",
      export: "export_database_dump",
      restore: "import_database_dump",
      cancel: "cancel_database_dump",
    });
  });

  it("detects tools with the profile id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(report);
    await expect(getDatabaseDumpTools("profile-1")).resolves.toEqual(report);
    expect(invoke).toHaveBeenCalledWith("get_database_dump_tools", { profileId: "profile-1" });
  });

  it("sends export and restore requests with camelCase fields and null timeouts", async () => {
    vi.mocked(invoke).mockResolvedValue({});
    await exportDatabaseDump("profile-1", exportRequest, "dump-1");
    expect(invoke).toHaveBeenCalledWith("export_database_dump", {
      profileId: "profile-1",
      requestId: "dump-1",
      request: exportRequest,
      timeoutMs: null,
    });

    await importDatabaseDump("profile-1", restoreRequest, "dump-2", 60_000);
    expect(invoke).toHaveBeenCalledWith("import_database_dump", {
      profileId: "profile-1",
      requestId: "dump-2",
      request: restoreRequest,
      timeoutMs: 60_000,
    });
  });

  it("cancels by request id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ requestId: "dump-1", cancelled: true });
    await expect(cancelDatabaseDump("dump-1")).resolves.toEqual({
      requestId: "dump-1",
      cancelled: true,
    });
    expect(invoke).toHaveBeenCalledWith("cancel_database_dump", { requestId: "dump-1" });
  });

  it("creates bounded ASCII request ids", () => {
    const id = createDatabaseDumpRequestId();
    expect(id).toMatch(/^dump-[A-Za-z0-9-]+$/);
    expect(id.length).toBeLessThanOrEqual(128);
  });
});
