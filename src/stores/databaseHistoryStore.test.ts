import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearDatabaseQueryHistory,
  createDatabaseSavedQuery,
  deleteDatabaseSavedQuery,
  getDatabaseQueryHistoryPolicy,
  listDatabaseQueryHistory,
  listDatabaseSavedQueries,
  updateDatabaseSavedQuery,
} from "@/services/databaseHistory";
import { useDatabaseHistoryStore } from "@/stores/databaseHistoryStore";
import type {
  DatabaseQueryHistoryEntry,
  DatabaseQueryHistoryPolicy,
  DatabaseSavedQuery,
} from "@/types/contracts";

vi.mock("@/services/databaseHistory", () => ({
  listDatabaseQueryHistory: vi.fn(),
  getDatabaseQueryHistoryPolicy: vi.fn(),
  clearDatabaseQueryHistory: vi.fn(),
  listDatabaseSavedQueries: vi.fn(),
  createDatabaseSavedQuery: vi.fn(),
  updateDatabaseSavedQuery: vi.fn(),
  deleteDatabaseSavedQuery: vi.fn(),
}));

const policy: DatabaseQueryHistoryPolicy = {
  maxEntries: 1_000,
  maxSqlChars: 8_192,
  errorSummaryMaxChars: 512,
  snippetNameMaxChars: 120,
  snippetSqlMaxChars: 65_536,
  retention: "Keeps the newest 1,000 queries.",
};

const history: DatabaseQueryHistoryEntry[] = [
  {
    id: "history-1",
    serverId: "server-1",
    connectionId: "connection-1",
    engine: "postgresql",
    databaseName: "app",
    sqlText: "SELECT 1",
    result: "success",
    durationMs: 4,
    rowCount: 1,
    errorSummary: null,
    startedAt: 1_700_000_100,
  },
];

const snippet: DatabaseSavedQuery = {
  id: "snippet-1",
  name: "Active users",
  engine: "postgresql",
  sqlText: "SELECT * FROM users WHERE active = true",
  createdAt: 1,
  updatedAt: 1,
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("database history store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseHistoryStore.getState().reset();
    vi.mocked(getDatabaseQueryHistoryPolicy).mockResolvedValue(policy);
    vi.mocked(listDatabaseQueryHistory).mockResolvedValue(history);
    vi.mocked(listDatabaseSavedQueries).mockResolvedValue([snippet]);
    vi.mocked(clearDatabaseQueryHistory).mockResolvedValue(undefined);
    vi.mocked(createDatabaseSavedQuery).mockResolvedValue(snippet);
    vi.mocked(updateDatabaseSavedQuery).mockResolvedValue({ ...snippet, name: "Updated" });
    vi.mocked(deleteDatabaseSavedQuery).mockResolvedValue(undefined);
  });

  it("loads sanitized history with the supplied filter", async () => {
    await useDatabaseHistoryStore.getState().loadHistory({
      serverId: "server-1",
      connectionId: "connection-1",
      result: "success",
      startedAfter: 100,
      startedBefore: 200,
      limit: 50,
    });

    expect(listDatabaseQueryHistory).toHaveBeenCalledWith({
      serverId: "server-1",
      connectionId: "connection-1",
      result: "success",
      startedAfter: 100,
      startedBefore: 200,
      limit: 50,
    });
    expect(useDatabaseHistoryStore.getState().history).toEqual(history);
    expect(useDatabaseHistoryStore.getState().historyStatus).toBe("ready");
  });

  it("loads the retention policy and saved snippets", async () => {
    await useDatabaseHistoryStore.getState().loadPolicy();
    await useDatabaseHistoryStore.getState().loadSnippets();

    expect(useDatabaseHistoryStore.getState().policy).toEqual(policy);
    expect(useDatabaseHistoryStore.getState().snippets).toEqual([snippet]);
    expect(useDatabaseHistoryStore.getState().snippetStatus).toBe("ready");
  });

  it("requires confirmation before clearing history", async () => {
    await useDatabaseHistoryStore.getState().loadHistory({});

    await expect(useDatabaseHistoryStore.getState().clearHistory(false)).resolves.toBe(false);
    expect(clearDatabaseQueryHistory).not.toHaveBeenCalled();
    expect(useDatabaseHistoryStore.getState().history).toEqual(history);

    await expect(useDatabaseHistoryStore.getState().clearHistory(true)).resolves.toBe(true);
    expect(clearDatabaseQueryHistory).toHaveBeenCalledWith(true);
    expect(useDatabaseHistoryStore.getState().history).toEqual([]);
  });

  it("creates and updates snippets and keeps the list sorted", async () => {
    await useDatabaseHistoryStore.getState().loadSnippets();
    vi.mocked(createDatabaseSavedQuery).mockResolvedValueOnce({
      ...snippet,
      id: "snippet-2",
      name: "Zeta",
    });

    const created = await useDatabaseHistoryStore.getState().createSnippet({
      name: "Zeta",
      engine: "postgresql",
      sql: "SELECT 2",
    });
    expect(created?.id).toBe("snippet-2");
    expect(createDatabaseSavedQuery).toHaveBeenCalledWith({
      name: "Zeta",
      engine: "postgresql",
      sql: "SELECT 2",
    });
    expect(useDatabaseHistoryStore.getState().snippets.map((entry) => entry.name)).toEqual([
      "Active users",
      "Zeta",
    ]);

    const updated = await useDatabaseHistoryStore.getState().updateSnippet("snippet-1", {
      name: "Updated",
      engine: "postgresql",
      sql: "SELECT 3",
    });
    expect(updated?.name).toBe("Updated");
    expect(updateDatabaseSavedQuery).toHaveBeenCalledWith("snippet-1", {
      name: "Updated",
      engine: "postgresql",
      sql: "SELECT 3",
    });
  });

  it("requires confirmation before deleting a snippet", async () => {
    await useDatabaseHistoryStore.getState().loadSnippets();

    await expect(
      useDatabaseHistoryStore.getState().deleteSnippet("snippet-1", false),
    ).resolves.toBe(false);
    expect(deleteDatabaseSavedQuery).not.toHaveBeenCalled();
    expect(useDatabaseHistoryStore.getState().snippets).toEqual([snippet]);

    await expect(useDatabaseHistoryStore.getState().deleteSnippet("snippet-1", true)).resolves.toBe(
      true,
    );
    expect(deleteDatabaseSavedQuery).toHaveBeenCalledWith("snippet-1", true);
    expect(useDatabaseHistoryStore.getState().snippets).toEqual([]);
  });

  it("surfaces command errors without dropping the last good list", async () => {
    await useDatabaseHistoryStore.getState().loadHistory({});
    vi.mocked(listDatabaseQueryHistory).mockRejectedValueOnce({
      code: "storage",
      message: "History could not be read.",
    });

    await useDatabaseHistoryStore.getState().loadHistory({ result: "failure" });

    expect(useDatabaseHistoryStore.getState().historyStatus).toBe("error");
    expect(useDatabaseHistoryStore.getState().historyError).toBe("History could not be read.");
    expect(useDatabaseHistoryStore.getState().history).toEqual(history);
  });
});
