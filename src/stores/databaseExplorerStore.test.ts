import { beforeEach, describe, expect, it, vi } from "vitest";
import { loadExplorerNode, refreshExplorerNode } from "@/services/databaseDiscovery";
import { databaseExplorerNodeKey, useDatabaseExplorerStore } from "@/stores/databaseExplorerStore";
import type {
  DatabaseExplorerNode,
  DatabaseExplorerNodePayload,
  DatabasePage,
  DatabaseSummary,
  DatabaseTableSummary,
} from "@/types/contracts";

vi.mock("@/services/databaseDiscovery", () => ({
  loadExplorerNode: vi.fn(),
  refreshExplorerNode: vi.fn(),
}));

const unavailable = { status: "unavailable" } as const;
const metadata = {
  owner: unavailable,
  encoding: unavailable,
  collation: unavailable,
  sizeBytes: unavailable,
  engine: unavailable,
};

function page<T>(items: T[]): DatabasePage<T> {
  return { items, limit: 100, offset: 0, hasMore: false, total: items.length };
}

const database: DatabaseSummary = {
  name: "app",
  system: false,
  ...metadata,
};

const table: DatabaseTableSummary = {
  database: "app",
  schema: "public",
  name: "users",
  kind: "table",
  estimatedRows: unavailable,
  metadataRefreshedAt: unavailable,
  ...metadata,
};

const databasesNode: DatabaseExplorerNode = { kind: "databases" };
const tablesNode: DatabaseExplorerNode = {
  kind: "tables",
  database: "app",
  schema: "public",
};

const databasesPayload: DatabaseExplorerNodePayload = {
  nodeKind: "databases",
  payload: page([database]),
};
const tablesPayload: DatabaseExplorerNodePayload = {
  nodeKind: "tables",
  payload: page([table]),
};

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

describe("database explorer store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseExplorerStore.setState({
      nodes: {},
      error: null,
      commandError: null,
    });
  });

  it("lazily loads one node and serves the cached payload on repeat", async () => {
    const pending = deferred<DatabaseExplorerNodePayload>();
    vi.mocked(loadExplorerNode).mockReturnValue(pending.promise);

    const key = databaseExplorerNodeKey("profile-1", databasesNode);
    const request = useDatabaseExplorerStore.getState().loadNode("profile-1", databasesNode);
    expect(useDatabaseExplorerStore.getState().nodes[key]?.status).toBe("loading");

    pending.resolve(databasesPayload);
    await expect(request).resolves.toEqual(databasesPayload);

    const entry = useDatabaseExplorerStore.getState().nodes[key];
    expect(entry?.status).toBe("ready");
    expect(entry?.payload).toEqual(databasesPayload);
    expect(entry?.updatedAt).toEqual(expect.any(Number));

    await useDatabaseExplorerStore.getState().loadNode("profile-1", databasesNode);
    expect(loadExplorerNode).toHaveBeenCalledTimes(1);
  });

  it("appends the next bounded page without replacing loaded rows", async () => {
    const firstPage: DatabaseExplorerNodePayload = {
      nodeKind: "databases",
      payload: {
        items: [database],
        limit: 1,
        offset: 0,
        hasMore: true,
        total: 2,
      },
    };
    const secondPage: DatabaseExplorerNodePayload = {
      nodeKind: "databases",
      payload: {
        items: [{ ...database, name: "analytics" }],
        limit: 1,
        offset: 1,
        hasMore: false,
        total: 2,
      },
    };
    vi.mocked(loadExplorerNode).mockResolvedValueOnce(firstPage).mockResolvedValueOnce(secondPage);

    await useDatabaseExplorerStore.getState().loadNode("profile-1", databasesNode);
    const merged = await useDatabaseExplorerStore
      .getState()
      .loadMoreNode("profile-1", databasesNode);

    expect(loadExplorerNode).toHaveBeenNthCalledWith(2, "profile-1", databasesNode, {
      limit: 1,
      offset: 1,
    });
    expect(merged?.nodeKind).toBe("databases");
    if (merged?.nodeKind === "databases") {
      expect(merged.payload.items.map((item) => item.name)).toEqual(["app", "analytics"]);
      expect(merged.payload.hasMore).toBe(false);
    }
    const entry =
      useDatabaseExplorerStore.getState().nodes[
        databaseExplorerNodeKey("profile-1", databasesNode)
      ];
    expect(entry?.loadingMore).toBe(false);
  });

  it("refreshes one node without replacing unrelated node state", async () => {
    const databasesKey = databaseExplorerNodeKey("profile-1", databasesNode);
    const tablesKey = databaseExplorerNodeKey("profile-1", tablesNode);

    vi.mocked(loadExplorerNode).mockResolvedValueOnce(databasesPayload);
    vi.mocked(loadExplorerNode).mockResolvedValueOnce(tablesPayload);
    await useDatabaseExplorerStore.getState().loadNode("profile-1", databasesNode);
    await useDatabaseExplorerStore.getState().loadNode("profile-1", tablesNode);

    const before = useDatabaseExplorerStore.getState().nodes;
    const refreshedTables: DatabaseExplorerNodePayload = {
      nodeKind: "tables",
      payload: page([{ ...table, name: "orders" }]),
    };
    vi.mocked(refreshExplorerNode).mockResolvedValue(refreshedTables);

    await useDatabaseExplorerStore.getState().refreshNode("profile-1", tablesNode);

    const after = useDatabaseExplorerStore.getState().nodes;
    expect(refreshExplorerNode).toHaveBeenCalledWith("profile-1", tablesNode, undefined);
    expect(after[databasesKey]).toBe(before[databasesKey]);
    expect(after[tablesKey]?.payload).toEqual(refreshedTables);
  });

  it("stores typed permission errors on the affected node only", async () => {
    const databasesKey = databaseExplorerNodeKey("profile-1", databasesNode);
    const tablesKey = databaseExplorerNodeKey("profile-1", tablesNode);
    vi.mocked(loadExplorerNode).mockResolvedValueOnce(databasesPayload);
    await useDatabaseExplorerStore.getState().loadNode("profile-1", databasesNode);

    vi.mocked(loadExplorerNode).mockRejectedValueOnce({
      code: "permission",
      message: "The database account does not have permission for this operation.",
      details: "PostgreSQL SQLSTATE 42501.",
    });

    await expect(
      useDatabaseExplorerStore.getState().loadNode("profile-1", tablesNode),
    ).resolves.toBeNull();

    const state = useDatabaseExplorerStore.getState();
    expect(state.nodes[databasesKey]?.status).toBe("ready");
    expect(state.nodes[tablesKey]?.status).toBe("error");
    expect(state.nodes[tablesKey]?.error?.code).toBe("permission");
    expect(state.nodes[tablesKey]?.errorMessage).toContain("permission");
    expect(state.nodes[databasesKey]?.error).toBeUndefined();
  });

  it("rejects a payload tagged for another node and resets one profile", async () => {
    vi.mocked(loadExplorerNode)
      .mockResolvedValueOnce(databasesPayload)
      .mockResolvedValueOnce(databasesPayload);
    await useDatabaseExplorerStore.getState().loadNode("profile-1", databasesNode);
    await useDatabaseExplorerStore.getState().loadNode("profile-2", databasesNode, {
      limit: 25,
    });
    expect(
      useDatabaseExplorerStore.getState().nodes[databaseExplorerNodeKey("profile-2", databasesNode)]
        ?.status,
    ).toBe("ready");

    const mismatched = {
      ...databasesPayload,
      nodeKind: "schemas",
    } as unknown as DatabaseExplorerNodePayload;
    vi.mocked(refreshExplorerNode).mockResolvedValueOnce(mismatched);
    await expect(
      useDatabaseExplorerStore.getState().refreshNode("profile-1", databasesNode),
    ).resolves.toBeNull();
    expect(useDatabaseExplorerStore.getState().commandError?.code).toBe("internal");

    useDatabaseExplorerStore.getState().resetProfile("profile-1");
    const keys = Object.keys(useDatabaseExplorerStore.getState().nodes);
    expect(keys).toHaveLength(1);
    expect(keys[0]).toContain("profile-2::");
  });
});
