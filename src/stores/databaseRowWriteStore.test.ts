import { beforeEach, describe, expect, it, vi } from "vitest";
import { deleteTableRows, insertTableRow, updateTableRow } from "@/services/databaseRowWrites";
import { databaseRowKey } from "@/stores/databaseRowStore";
import { useDatabaseRowWriteStore } from "@/stores/databaseRowWriteStore";
import type { DatabaseRowTable, DatabaseRowWriteResult } from "@/types/contracts";

vi.mock("@/services/databaseRowWrites", () => ({
  insertTableRow: vi.fn(),
  updateTableRow: vi.fn(),
  deleteTableRows: vi.fn(),
}));

const table: DatabaseRowTable = { database: "app", schema: "public", table: "users" };

function result(operation: DatabaseRowWriteResult["operation"]): DatabaseRowWriteResult {
  return {
    operation,
    table: { database: "app", schema: "public", name: "users" },
    affectedRows: 1,
    verified: true,
    verification: "Committed and verified.",
    transaction: "committed",
  };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("database row write store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseRowWriteStore.setState({ entries: {} });
  });

  it("stores a committed insert result for the table", async () => {
    vi.mocked(insertTableRow).mockResolvedValueOnce(result("insert"));
    const written = await useDatabaseRowWriteStore
      .getState()
      .insertRow("profile-1", table, [{ column: "name", mode: "value", value: "ann" }]);

    expect(written).toEqual(result("insert"));
    const entry = useDatabaseRowWriteStore.getState().entries[databaseRowKey("profile-1", table)];
    expect(entry?.status).toBe("succeeded");
    expect(entry?.result?.transaction).toBe("committed");
  });

  it("keeps a typed conflict failure and never marks it as committed", async () => {
    vi.mocked(updateTableRow).mockRejectedValueOnce({
      code: "conflict",
      message: "The row changed in another session.",
      details: "The transaction was rolled back; no rows were changed.",
    });

    const written = await useDatabaseRowWriteStore
      .getState()
      .updateRow(
        "profile-1",
        table,
        [{ column: "id", value: "1" }],
        [{ column: "name", mode: "value", value: "stale" }],
        [{ column: "name", value: "ann" }],
      );

    expect(written).toBeNull();
    const entry = useDatabaseRowWriteStore.getState().entries[databaseRowKey("profile-1", table)];
    expect(entry?.status).toBe("failed");
    expect(entry?.error?.code).toBe("conflict");
    expect(entry?.errorMessage).toContain("changed in another session");
  });

  it("passes the explicit confirmation flag through a scoped bulk delete", async () => {
    vi.mocked(deleteTableRows).mockResolvedValueOnce(result("delete"));
    await useDatabaseRowWriteStore
      .getState()
      .deleteRows(
        "profile-1",
        table,
        [[{ column: "id", value: "1" }], [{ column: "id", value: "2" }]],
        true,
      );

    expect(deleteTableRows).toHaveBeenCalledWith(
      "profile-1",
      table,
      [[{ column: "id", value: "1" }], [{ column: "id", value: "2" }]],
      true,
    );
  });

  it("clears one table entry and every entry for a profile", async () => {
    vi.mocked(insertTableRow).mockResolvedValue(result("insert"));
    await useDatabaseRowWriteStore
      .getState()
      .insertRow("profile-1", table, [{ column: "name", mode: "default" }]);

    useDatabaseRowWriteStore.getState().clearEntry("profile-1", table);
    expect(
      useDatabaseRowWriteStore.getState().entries[databaseRowKey("profile-1", table)],
    ).toBeUndefined();

    await useDatabaseRowWriteStore
      .getState()
      .insertRow("profile-1", table, [{ column: "name", mode: "default" }]);
    useDatabaseRowWriteStore.getState().resetProfile("profile-1");
    expect(useDatabaseRowWriteStore.getState().entries).toEqual({});
  });
});
