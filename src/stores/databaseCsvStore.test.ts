import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelDatabaseCsvTransfer,
  createDatabaseCsvRequestId,
  exportDatabaseCsv,
  importDatabaseCsv,
  previewDatabaseCsvImport,
} from "@/services/databaseCsv";
import { useDatabaseCsvStore } from "@/stores/databaseCsvStore";
import type {
  DatabaseCsvExportResult,
  DatabaseCsvImportOptions,
  DatabaseCsvImportPreview,
  DatabaseCsvImportResult,
  DatabaseCsvTransferProgress,
} from "@/types/contracts";

vi.mock("@/services/databaseCsv", () => ({
  cancelDatabaseCsvTransfer: vi.fn(),
  createDatabaseCsvRequestId: vi.fn(),
  exportDatabaseCsv: vi.fn(),
  importDatabaseCsv: vi.fn(),
  previewDatabaseCsvImport: vi.fn(),
}));

const table = { database: "app", schema: "public", name: "users" };
const options: DatabaseCsvImportOptions = {
  delimiter: "comma",
  encoding: "utf8",
  hasHeader: true,
  nullMode: { kind: "unquotedEmpty" },
};

function preview(): DatabaseCsvImportPreview {
  return {
    headers: ["name"],
    rows: [{ rowNumber: 2, fields: [{ value: "ann", quoted: false, isNull: false }] }],
    warnings: [],
    warningsTruncated: false,
    sampledRows: 1,
    hasMore: false,
    delimiter: "comma",
    encoding: "utf8",
    hasHeader: true,
  };
}

function exportResult(): DatabaseCsvExportResult {
  return {
    status: "completed",
    engine: "postgresql",
    table,
    destinationPath: "/tmp/users.csv",
    rowsExported: 2,
    columns: ["name"],
    bytesWritten: 12,
    encoding: "utf8",
    delimiter: "comma",
    includeHeader: true,
    warnings: [],
    mayBeIncomplete: false,
    valuesTruncated: false,
    durationMs: 3,
  };
}

function importResult(): DatabaseCsvImportResult {
  return {
    status: "cancelled",
    engine: "postgresql",
    table,
    rowsRead: 2,
    rowsImported: 0,
    rowsSkipped: 0,
    warnings: [],
    warningsTruncated: false,
    transaction: "rolled_back",
    mayBeIncomplete: false,
    durationMs: 1,
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

describe("database CSV store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseCsvStore.setState({
      preview: { status: "idle" },
      transfer: { direction: "export", status: "idle", cancelling: false },
    });
  });

  it("previews parsed rows without starting a transfer", async () => {
    vi.mocked(previewDatabaseCsvImport).mockResolvedValueOnce(preview());

    await expect(
      useDatabaseCsvStore.getState().previewImport("/tmp/users.csv", options, 10),
    ).resolves.toEqual(preview());
    expect(previewDatabaseCsvImport).toHaveBeenCalledWith("/tmp/users.csv", options, 10);
    expect(useDatabaseCsvStore.getState().preview.status).toBe("ready");
    expect(useDatabaseCsvStore.getState().transfer.status).toBe("idle");
  });

  it("tracks an export and applies only matching progress", async () => {
    vi.mocked(createDatabaseCsvRequestId).mockReturnValueOnce("csv-1");
    const pending = deferred<DatabaseCsvExportResult>();
    vi.mocked(exportDatabaseCsv).mockReturnValueOnce(pending.promise);

    const request = useDatabaseCsvStore.getState().startExport("profile-1", {
      source: { kind: "table", table },
      destinationPath: "/tmp/users.csv",
      delimiter: "comma",
      encoding: "utf8",
      includeHeader: true,
      overwrite: false,
    });
    expect(useDatabaseCsvStore.getState().transfer.status).toBe("running");

    const progress: DatabaseCsvTransferProgress = {
      transferId: "csv-1",
      direction: "export",
      status: "running",
      phase: "transferring",
      rowsProcessed: 1,
      rowsWritten: 1,
      rowsSkipped: 0,
      totalRows: null,
      bytesProcessed: 4,
      totalBytes: null,
      warnings: [],
      mayBeIncomplete: false,
      emittedAt: 1,
    };
    useDatabaseCsvStore.getState().applyTransferProgress(progress);
    expect(useDatabaseCsvStore.getState().transfer.progress?.rowsWritten).toBe(1);
    useDatabaseCsvStore.getState().applyTransferProgress({ ...progress, transferId: "other" });
    expect(useDatabaseCsvStore.getState().transfer.progress?.rowsWritten).toBe(1);

    pending.resolve(exportResult());
    await expect(request).resolves.toEqual(exportResult());
    expect(useDatabaseCsvStore.getState().transfer.status).toBe("completed");
    expect(useDatabaseCsvStore.getState().transfer.exportResult?.rowsExported).toBe(2);
  });

  it("keeps a cancelled import as a rolled-back result, not a success", async () => {
    vi.mocked(createDatabaseCsvRequestId).mockReturnValueOnce("csv-2");
    vi.mocked(importDatabaseCsv).mockResolvedValueOnce(importResult());

    await expect(
      useDatabaseCsvStore.getState().startImport("profile-1", {
        table,
        sourcePath: "/tmp/users.csv",
        options,
        mappings: [{ sourceIndex: 0, targetColumn: "name" }],
      }),
    ).resolves.toEqual(importResult());
    expect(useDatabaseCsvStore.getState().transfer.status).toBe("cancelled");
    expect(useDatabaseCsvStore.getState().transfer.importResult?.transaction).toBe("rolled_back");
  });

  it("cancels the active transfer id", async () => {
    useDatabaseCsvStore.setState({
      transfer: {
        transferId: "csv-3",
        direction: "import",
        status: "running",
        cancelling: false,
      },
    });
    vi.mocked(cancelDatabaseCsvTransfer).mockResolvedValueOnce({
      requestId: "csv-3",
      cancelled: true,
    });

    await expect(useDatabaseCsvStore.getState().cancelTransfer()).resolves.toBe(true);
    expect(cancelDatabaseCsvTransfer).toHaveBeenCalledWith("csv-3");
    expect(useDatabaseCsvStore.getState().transfer.cancelling).toBe(true);
  });

  it("surfaces preview and transfer errors without a result", async () => {
    vi.mocked(previewDatabaseCsvImport).mockRejectedValueOnce({
      code: "validation",
      message: "The CSV file is empty.",
    });
    await useDatabaseCsvStore.getState().previewImport("/tmp/empty.csv", options);
    expect(useDatabaseCsvStore.getState().preview.status).toBe("error");
    expect(useDatabaseCsvStore.getState().preview.errorMessage).toContain("empty");

    vi.mocked(createDatabaseCsvRequestId).mockReturnValueOnce("csv-4");
    vi.mocked(exportDatabaseCsv).mockRejectedValueOnce({
      code: "conflict",
      message: "The CSV destination already exists.",
    });
    await useDatabaseCsvStore.getState().startExport("profile-1", {
      source: { kind: "table", table },
      destinationPath: "/tmp/users.csv",
      delimiter: "comma",
      encoding: "utf8",
      includeHeader: true,
      overwrite: false,
    });
    expect(useDatabaseCsvStore.getState().transfer.status).toBe("error");
    expect(useDatabaseCsvStore.getState().transfer.errorMessage).toContain("already exists");
  });
});
