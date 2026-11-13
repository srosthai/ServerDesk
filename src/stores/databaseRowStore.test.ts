import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelTableRows,
  createDatabaseRowRequestId,
  readTableRows,
} from "@/services/databaseRows";
import { databaseRowKey, useDatabaseRowStore } from "@/stores/databaseRowStore";
import type { DatabaseRowReadResult, DatabaseRowTable } from "@/types/contracts";

vi.mock("@/services/databaseRows", () => ({
  readTableRows: vi.fn(),
  cancelTableRows: vi.fn(),
  createDatabaseRowRequestId: vi.fn(),
}));

const unavailable = { status: "unavailable" } as const;
const table: DatabaseRowTable = { database: "app", schema: "public", table: "users" };

function result(name: string): DatabaseRowReadResult {
  return {
    engine: "postgresql",
    table: { database: "app", schema: "public", name: table.table },
    columns: [],
    visibleColumns: ["name"],
    identity: { kind: "primary_key", columns: ["id"], stable: true },
    editable: true,
    readOnlyReasons: [],
    rows: [
      {
        cells: [{ value: name, truncated: false }],
        identity: [{ column: "id", value: "1" }],
      },
    ],
    page: { limit: 25, offset: 0, returned: 1, hasMore: false },
    counts: { estimatedTableRows: unavailable },
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

describe("database row store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseRowStore.setState({ entries: {} });
  });

  it("loads a bounded page and keeps the query state for the table", async () => {
    vi.mocked(createDatabaseRowRequestId).mockReturnValueOnce("req-1");
    const pending = deferred<DatabaseRowReadResult>();
    vi.mocked(readTableRows).mockReturnValueOnce(pending.promise);

    const key = databaseRowKey("profile-1", table);
    const request = useDatabaseRowStore.getState().loadRows("profile-1", table, {
      limit: 25,
      offset: 0,
      sort: [{ column: "name", direction: "asc" }],
    });
    expect(useDatabaseRowStore.getState().entries[key]?.status).toBe("loading");

    pending.resolve(result("ann"));
    await expect(request).resolves.toEqual(result("ann"));

    const entry = useDatabaseRowStore.getState().entries[key];
    expect(entry?.status).toBe("ready");
    expect(entry?.options.limit).toBe(25);
    expect(entry?.options.sort).toEqual([{ column: "name", direction: "asc" }]);
    expect(entry?.updatedAt).toEqual(expect.any(Number));
    expect(readTableRows).toHaveBeenCalledWith(
      "profile-1",
      table,
      { limit: 25, offset: 0, sort: [{ column: "name", direction: "asc" }] },
      "req-1",
    );
  });

  it("ignores a stale response that arrives after a newer request", async () => {
    vi.mocked(createDatabaseRowRequestId).mockReturnValueOnce("req-1").mockReturnValueOnce("req-2");
    const first = deferred<DatabaseRowReadResult>();
    const second = deferred<DatabaseRowReadResult>();
    vi.mocked(readTableRows).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    const firstRequest = useDatabaseRowStore.getState().loadRows("profile-1", table, { offset: 0 });
    const secondRequest = useDatabaseRowStore
      .getState()
      .loadRows("profile-1", table, { offset: 25 });

    second.resolve(result("new"));
    await expect(secondRequest).resolves.toEqual(result("new"));

    first.resolve(result("old"));
    await expect(firstRequest).resolves.toBeNull();

    const entry = useDatabaseRowStore.getState().entries[databaseRowKey("profile-1", table)];
    expect(entry?.result?.rows[0]?.cells[0]?.value).toBe("new");
    expect(entry?.options.offset).toBe(25);
  });

  it("marks a cancelled read without overwriting it with a late error", async () => {
    vi.mocked(createDatabaseRowRequestId).mockReturnValueOnce("req-1");
    const pending = deferred<DatabaseRowReadResult>();
    vi.mocked(readTableRows).mockReturnValueOnce(pending.promise);
    vi.mocked(cancelTableRows).mockResolvedValueOnce({
      requestId: "req-1",
      cancelled: true,
    });

    const request = useDatabaseRowStore.getState().loadRows("profile-1", table);
    await expect(useDatabaseRowStore.getState().cancelRows("profile-1", table)).resolves.toBe(true);
    const key = databaseRowKey("profile-1", table);
    expect(useDatabaseRowStore.getState().entries[key]?.status).toBe("cancelled");

    pending.reject({ code: "cancelled", message: "The row read was cancelled." });
    await expect(request).resolves.toBeNull();
    expect(useDatabaseRowStore.getState().entries[key]?.status).toBe("cancelled");
  });

  it("clears one table or every table for a disconnected profile", async () => {
    vi.mocked(createDatabaseRowRequestId).mockReturnValueOnce("req-1");
    vi.mocked(readTableRows).mockResolvedValueOnce(result("ann"));
    await useDatabaseRowStore.getState().loadRows("profile-1", table);

    const other = { ...table, table: "orders" };
    vi.mocked(createDatabaseRowRequestId).mockReturnValueOnce("req-2");
    vi.mocked(readTableRows).mockResolvedValueOnce(result("order"));
    await useDatabaseRowStore.getState().loadRows("profile-1", other);

    useDatabaseRowStore.getState().clearRows("profile-1", table);
    expect(
      useDatabaseRowStore.getState().entries[databaseRowKey("profile-1", table)],
    ).toBeUndefined();
    expect(
      useDatabaseRowStore.getState().entries[databaseRowKey("profile-1", other)],
    ).toBeDefined();

    useDatabaseRowStore.getState().resetProfile("profile-1");
    expect(useDatabaseRowStore.getState().entries).toEqual({});
  });
});
