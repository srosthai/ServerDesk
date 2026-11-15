import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  analyzeDatabaseSql,
  cancelDatabaseSql,
  createDatabaseQueryRequestId,
  executeDatabaseSql,
} from "@/services/databaseQueries";
import { useDatabaseQueryStore } from "@/stores/databaseQueryStore";
import type { DatabaseQueryExecutionResult, DatabaseSqlAnalysis } from "@/types/contracts";

vi.mock("@/services/databaseQueries", () => ({
  analyzeDatabaseSql: vi.fn(),
  cancelDatabaseSql: vi.fn(),
  createDatabaseQueryRequestId: vi.fn(),
  executeDatabaseSql: vi.fn(),
}));

const analysis: DatabaseSqlAnalysis = {
  engine: "postgresql",
  statementCount: 1,
  hasWrites: false,
  destructive: false,
  requiresConfirmation: false,
  warnings: [],
};

function result(value: string): DatabaseQueryExecutionResult {
  return {
    engine: "postgresql",
    readOnly: false,
    results: [
      {
        index: 0,
        columns: [{ name: "value", dataType: "text" }],
        rows: [[{ value, truncated: false }]],
        returnedRows: 1,
        affectedRows: null,
        lastInsertId: null,
        truncated: false,
        columnsTruncated: false,
      },
    ],
    warnings: [],
    destructive: false,
    requiresConfirmation: false,
    truncated: false,
    maxRows: 500,
    durationMs: 2,
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

describe("database query store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseQueryStore.setState({ entries: {} });
  });

  it("analyzes SQL and keeps destructive warnings in the query-tab entry", async () => {
    vi.mocked(createDatabaseQueryRequestId).mockReturnValueOnce("analysis-1");
    vi.mocked(analyzeDatabaseSql).mockResolvedValueOnce(analysis);

    await expect(
      useDatabaseQueryStore.getState().analyze("query-1", "postgresql", "SELECT 1"),
    ).resolves.toEqual(analysis);

    const entry = useDatabaseQueryStore.getState().entries["query-1"];
    expect(entry?.status).toBe("ready");
    expect(entry?.analysis).toEqual(analysis);
    expect(entry?.analysisToken).toBeUndefined();
    expect(entry?.updatedAt).toEqual(expect.any(Number));
  });

  it("executes with the profile id, bounded options, and a request id", async () => {
    vi.mocked(createDatabaseQueryRequestId).mockReturnValueOnce("request-1");
    vi.mocked(executeDatabaseSql).mockResolvedValueOnce(result("ann"));

    const request = useDatabaseQueryStore.getState().execute("query-1", "profile-1", "SELECT 1", {
      maxRows: 25,
      timeoutMs: 5_000,
      confirmed: true,
    });
    expect(useDatabaseQueryStore.getState().entries["query-1"]?.status).toBe("running");

    await expect(request).resolves.toEqual(result("ann"));
    const entry = useDatabaseQueryStore.getState().entries["query-1"];
    expect(entry?.status).toBe("succeeded");
    expect(entry?.result?.results[0].rows[0][0].value).toBe("ann");
    expect(entry?.executedSql).toBe("SELECT 1");
    expect(executeDatabaseSql).toHaveBeenCalledWith(
      "profile-1",
      "SELECT 1",
      { maxRows: 25, timeoutMs: 5_000, confirmed: true },
      "request-1",
    );
  });

  it("keeps results separate for two query tabs on one connection", async () => {
    vi.mocked(createDatabaseQueryRequestId)
      .mockReturnValueOnce("request-1")
      .mockReturnValueOnce("request-2");
    vi.mocked(executeDatabaseSql)
      .mockResolvedValueOnce(result("one"))
      .mockResolvedValueOnce(result("two"));

    await useDatabaseQueryStore.getState().execute("query-1", "profile-1", "SELECT 1");
    await useDatabaseQueryStore.getState().execute("query-2", "profile-1", "SELECT 2");

    expect(
      useDatabaseQueryStore.getState().entries["query-1"]?.result?.results[0].rows[0][0].value,
    ).toBe("one");
    expect(
      useDatabaseQueryStore.getState().entries["query-2"]?.result?.results[0].rows[0][0].value,
    ).toBe("two");
  });

  it("keeps the newest analysis when an older response arrives late", async () => {
    vi.mocked(createDatabaseQueryRequestId)
      .mockReturnValueOnce("analysis-1")
      .mockReturnValueOnce("analysis-2");
    const first = deferred<DatabaseSqlAnalysis>();
    const second = deferred<DatabaseSqlAnalysis>();
    vi.mocked(analyzeDatabaseSql)
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const firstRequest = useDatabaseQueryStore
      .getState()
      .analyze("query-1", "postgresql", "DROP TABLE users");
    const secondRequest = useDatabaseQueryStore
      .getState()
      .analyze("query-1", "postgresql", "SELECT 1");

    second.resolve(analysis);
    await expect(secondRequest).resolves.toEqual(analysis);

    first.resolve({ ...analysis, destructive: true, requiresConfirmation: true });
    await expect(firstRequest).resolves.toBeNull();
    expect(useDatabaseQueryStore.getState().entries["query-1"]?.analysis?.destructive).toBe(false);
  });

  it("marks a cancelled execution and ignores its late error", async () => {
    vi.mocked(createDatabaseQueryRequestId).mockReturnValueOnce("request-1");
    const pending = deferred<DatabaseQueryExecutionResult>();
    vi.mocked(executeDatabaseSql).mockReturnValueOnce(pending.promise);
    vi.mocked(cancelDatabaseSql).mockResolvedValueOnce({
      requestId: "request-1",
      cancelled: true,
    });

    const request = useDatabaseQueryStore
      .getState()
      .execute("query-1", "profile-1", "SELECT pg_sleep(5)");
    await expect(useDatabaseQueryStore.getState().cancel("query-1")).resolves.toBe(true);
    expect(useDatabaseQueryStore.getState().entries["query-1"]?.status).toBe("cancelled");

    pending.reject({ code: "cancelled", message: "The SQL execution was cancelled." });
    await expect(request).resolves.toBeNull();
    expect(useDatabaseQueryStore.getState().entries["query-1"]?.status).toBe("cancelled");
  });

  it("records read-only blocks and validation failures without executing", async () => {
    useDatabaseQueryStore
      .getState()
      .block(
        "query-1",
        { ...analysis, hasWrites: true, statementCount: 2 },
        "Read-only mode blocked the SQL.",
      );
    expect(useDatabaseQueryStore.getState().entries["query-1"]).toMatchObject({
      status: "blocked",
      error: { code: "permission", message: "Read-only mode blocked the SQL." },
    });
    expect(executeDatabaseSql).not.toHaveBeenCalled();

    useDatabaseQueryStore.getState().fail("query-2", {
      code: "validation",
      message: "Enter a SQL statement before running it.",
    });
    expect(useDatabaseQueryStore.getState().entries["query-2"]).toMatchObject({
      status: "error",
      errorMessage: "Enter a SQL statement before running it.",
    });
  });

  it("keeps structured error metadata and clears it explicitly", async () => {
    vi.mocked(createDatabaseQueryRequestId).mockReturnValueOnce("request-1");
    vi.mocked(executeDatabaseSql).mockRejectedValueOnce({
      code: "query",
      message: "The SQL statement was rejected.",
      databaseCode: "42601",
      position: 8,
      durationMs: 12,
    });

    await expect(
      useDatabaseQueryStore.getState().execute("query-1", "profile-1", "SELEC 1"),
    ).resolves.toBeNull();
    const entry = useDatabaseQueryStore.getState().entries["query-1"];
    expect(entry?.status).toBe("error");
    expect(entry?.error?.databaseCode).toBe("42601");
    expect(entry?.error?.position).toBe(8);
    expect(entry?.executedSql).toBe("SELEC 1");

    useDatabaseQueryStore.getState().clearError("query-1");
    expect(useDatabaseQueryStore.getState().entries["query-1"]?.status).toBe("idle");
    expect(useDatabaseQueryStore.getState().entries["query-1"]?.error).toBeUndefined();
  });

  it("removes one entry or many on clear", async () => {
    vi.mocked(createDatabaseQueryRequestId)
      .mockReturnValueOnce("request-1")
      .mockReturnValueOnce("request-2");
    vi.mocked(executeDatabaseSql)
      .mockResolvedValueOnce(result("ann"))
      .mockResolvedValueOnce(result("bob"));
    await useDatabaseQueryStore.getState().execute("query-1", "profile-1", "SELECT 1");
    await useDatabaseQueryStore.getState().execute("query-2", "profile-1", "SELECT 2");

    useDatabaseQueryStore.getState().clear("query-1");
    expect(useDatabaseQueryStore.getState().entries["query-1"]).toBeUndefined();
    expect(useDatabaseQueryStore.getState().entries["query-2"]).toBeDefined();

    useDatabaseQueryStore.getState().clearMany(["query-2"]);
    expect(useDatabaseQueryStore.getState().entries).toEqual({});
  });
});
