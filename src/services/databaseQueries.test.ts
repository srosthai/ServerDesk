import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  analyzeDatabaseSql,
  cancelDatabaseSql,
  createDatabaseQueryRequestId,
  databaseQueryCommands,
  executeDatabaseSql,
} from "@/services/databaseQueries";
import type { DatabaseQueryExecutionResult, DatabaseSqlAnalysis } from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const analysis: DatabaseSqlAnalysis = {
  engine: "postgresql",
  statementCount: 1,
  hasWrites: true,
  destructive: true,
  requiresConfirmation: true,
  warnings: [
    {
      code: "drop",
      statementIndex: 1,
      message: "Statement 1 contains DROP.",
    },
  ],
};

const result: DatabaseQueryExecutionResult = {
  engine: "postgresql",
  readOnly: false,
  results: [],
  warnings: [],
  destructive: false,
  requiresConfirmation: false,
  truncated: false,
  maxRows: 500,
  durationMs: 4,
};

describe("database query service", () => {
  it("analyzes SQL with the selected engine without a request id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(analysis);

    await expect(analyzeDatabaseSql("postgresql", "DROP TABLE users")).resolves.toEqual(analysis);

    expect(invoke).toHaveBeenCalledWith(databaseQueryCommands.analyze, {
      engine: "postgresql",
      sql: "DROP TABLE users",
    });
  });

  it("sends bounded execution options and a caller-generated request id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(result);

    await expect(
      executeDatabaseSql(
        "profile-1",
        "SELECT 1",
        { maxRows: 25, timeoutMs: 5_000, confirmed: true },
        "request-1",
      ),
    ).resolves.toEqual(result);

    expect(invoke).toHaveBeenCalledWith(databaseQueryCommands.execute, {
      profileId: "profile-1",
      requestId: "request-1",
      sql: "SELECT 1",
      maxRows: 25,
      timeoutMs: 5_000,
      confirmed: true,
    });
  });

  it("sends null bounds and defaults confirmation to false", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(result);
    await executeDatabaseSql("profile-1", "SELECT 1", {}, "request-2");

    expect(invoke).toHaveBeenCalledWith(databaseQueryCommands.execute, {
      profileId: "profile-1",
      requestId: "request-2",
      sql: "SELECT 1",
      maxRows: null,
      timeoutMs: null,
      confirmed: false,
    });
  });

  it("cancels exactly one request id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      requestId: "request-3",
      cancelled: true,
    });

    await expect(cancelDatabaseSql("request-3")).resolves.toEqual({
      requestId: "request-3",
      cancelled: true,
    });
    expect(invoke).toHaveBeenCalledWith(databaseQueryCommands.cancel, {
      requestId: "request-3",
    });
  });

  it("generates ids the backend will accept", () => {
    expect(createDatabaseQueryRequestId()).toMatch(/^[A-Za-z0-9._-]{1,128}$/);
  });
});
