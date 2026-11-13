import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  cancelTableRows,
  createDatabaseRowRequestId,
  databaseRowCommands,
  readTableRows,
} from "@/services/databaseRows";
import type { DatabaseRowReadResult } from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const unavailable = { status: "unavailable" } as const;

const result: DatabaseRowReadResult = {
  engine: "postgresql",
  table: { database: "app", schema: "public", name: "users" },
  columns: [],
  visibleColumns: [],
  identity: { kind: "primary_key", columns: ["id"], stable: true },
  editable: true,
  readOnlyReasons: [],
  rows: [],
  page: { limit: 25, offset: 50, returned: 0, hasMore: false },
  counts: { estimatedTableRows: unavailable },
};

describe("database row service", () => {
  it("sends typed row options and a caller-generated request id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(result);

    await expect(
      readTableRows(
        "profile-1",
        { database: "app", schema: "public", table: "users" },
        {
          visibleColumns: ["id", "name"],
          sort: [{ column: "name", direction: "asc" }],
          filters: [
            {
              column: "name",
              operator: "contains",
              value: { kind: "text", value: "ann" },
            },
          ],
          search: { query: "ann", columns: ["name"] },
          limit: 25,
          offset: 50,
          includeExactCount: true,
          timeoutMs: 5_000,
        },
        "request-1",
      ),
    ).resolves.toEqual(result);

    expect(invoke).toHaveBeenCalledWith(databaseRowCommands.read, {
      profileId: "profile-1",
      requestId: "request-1",
      database: "app",
      schema: "public",
      table: "users",
      visibleColumns: ["id", "name"],
      sort: [{ column: "name", direction: "asc" }],
      filters: [
        {
          column: "name",
          operator: "contains",
          value: { kind: "text", value: "ann" },
        },
      ],
      search: { query: "ann", columns: ["name"] },
      limit: 25,
      offset: 50,
      includeExactCount: true,
      timeoutMs: 5_000,
    });
  });

  it("sends null for omitted bounds and cancellation targets one id", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(result);
    await readTableRows("profile-1", { table: "users" }, {}, "request-2");
    expect(invoke).toHaveBeenNthCalledWith(1, databaseRowCommands.read, {
      profileId: "profile-1",
      requestId: "request-2",
      database: null,
      schema: null,
      table: "users",
      visibleColumns: null,
      sort: null,
      filters: null,
      search: null,
      limit: null,
      offset: null,
      includeExactCount: null,
      timeoutMs: null,
    });

    vi.mocked(invoke).mockResolvedValueOnce({
      requestId: "request-2",
      cancelled: true,
    });
    await expect(cancelTableRows("request-2")).resolves.toEqual({
      requestId: "request-2",
      cancelled: true,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, databaseRowCommands.cancel, {
      requestId: "request-2",
    });
  });

  it("generates ids the backend will accept", () => {
    expect(createDatabaseRowRequestId()).toMatch(/^[A-Za-z0-9._-]{1,128}$/);
  });
});
