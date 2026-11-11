import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  databaseDiscoveryCommands,
  getObjectCounts,
  listDatabases,
  listSchemas,
  listSequences,
  listTables,
  listViews,
  loadExplorerNode,
  refreshExplorerNode,
} from "@/services/databaseDiscovery";
import type {
  DatabaseExplorerNodePayload,
  DatabaseObjectCounts,
  DatabasePage,
  DatabaseSchemaSummary,
  DatabaseSequenceSummary,
  DatabaseSummary,
  DatabaseTableSummary,
  DatabaseViewSummary,
} from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const unavailable = { status: "unavailable" } as const;
const metadata = {
  owner: unavailable,
  encoding: unavailable,
  collation: unavailable,
  sizeBytes: unavailable,
  engine: unavailable,
};

const database: DatabaseSummary = {
  name: "app",
  system: false,
  ...metadata,
  encoding: { status: "available", value: "utf8mb4" },
  sizeBytes: { status: "available", value: 4096 },
};

const schema: DatabaseSchemaSummary = {
  name: "public",
  system: false,
  ...metadata,
};

const table: DatabaseTableSummary = {
  database: "app",
  schema: "public",
  name: "users",
  kind: "table",
  estimatedRows: { status: "available", value: 2 },
  ...metadata,
  engine: { status: "available", value: "InnoDB" },
};

const view: DatabaseViewSummary = {
  database: "app",
  schema: "public",
  name: "active_users",
  kind: "view",
  ...metadata,
};

const sequence: DatabaseSequenceSummary = {
  database: "app",
  schema: "public",
  name: "users_id_seq",
  dataType: { status: "available", value: "bigint" },
  startValue: { status: "available", value: "1" },
  increment: { status: "available", value: "1" },
  ...metadata,
};

const counts: DatabaseObjectCounts = {
  tables: { status: "available", value: 3 },
  views: { status: "available", value: 1 },
  materializedViews: unavailable,
  sequences: unavailable,
};

function page<T>(items: T[]): DatabasePage<T> {
  return { items, limit: 100, offset: 0, hasMore: false, total: items.length };
}

const nodePayload: DatabaseExplorerNodePayload = {
  nodeKind: "databases",
  payload: page([database]),
};

describe("database discovery service contract", () => {
  it("uses the registered command names and sends bounded node payloads", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(page([database]))
      .mockResolvedValueOnce(page([schema]))
      .mockResolvedValueOnce(page([table]))
      .mockResolvedValueOnce(page([view]))
      .mockResolvedValueOnce(page([sequence]))
      .mockResolvedValueOnce(counts)
      .mockResolvedValueOnce(nodePayload)
      .mockResolvedValueOnce(nodePayload);

    await expect(listDatabases("profile-1", { limit: 50, offset: 10 })).resolves.toEqual(
      page([database]),
    );
    expect(invoke).toHaveBeenNthCalledWith(1, databaseDiscoveryCommands.databases, {
      profileId: "profile-1",
      limit: 50,
      offset: 10,
    });

    await expect(listSchemas("profile-1")).resolves.toEqual(page([schema]));
    expect(invoke).toHaveBeenNthCalledWith(2, databaseDiscoveryCommands.schemas, {
      profileId: "profile-1",
      limit: null,
      offset: null,
    });

    await expect(
      listTables("profile-1", { database: "app", schema: "public" }, { limit: 25, offset: 0 }),
    ).resolves.toEqual(page([table]));
    expect(invoke).toHaveBeenNthCalledWith(3, databaseDiscoveryCommands.tables, {
      profileId: "profile-1",
      database: "app",
      schema: "public",
      limit: 25,
      offset: 0,
    });

    await expect(listViews("profile-1", { database: "app", schema: "public" })).resolves.toEqual(
      page([view]),
    );
    expect(invoke).toHaveBeenNthCalledWith(4, databaseDiscoveryCommands.views, {
      profileId: "profile-1",
      database: "app",
      schema: "public",
      limit: null,
      offset: null,
    });

    await expect(
      listSequences("profile-1", { database: "app", schema: "public" }),
    ).resolves.toEqual(page([sequence]));
    expect(invoke).toHaveBeenNthCalledWith(5, databaseDiscoveryCommands.sequences, {
      profileId: "profile-1",
      database: "app",
      schema: "public",
      limit: null,
      offset: null,
    });

    await expect(
      getObjectCounts("profile-1", { database: "app", schema: "public" }),
    ).resolves.toEqual(counts);
    expect(invoke).toHaveBeenNthCalledWith(6, databaseDiscoveryCommands.objectCounts, {
      profileId: "profile-1",
      database: "app",
      schema: "public",
    });

    const node = { kind: "databases" } as const;
    await expect(loadExplorerNode("profile-1", node, { limit: 100 })).resolves.toEqual(nodePayload);
    expect(invoke).toHaveBeenNthCalledWith(7, databaseDiscoveryCommands.loadNode, {
      profileId: "profile-1",
      node,
      limit: 100,
      offset: null,
    });

    await expect(refreshExplorerNode("profile-1", node)).resolves.toEqual(nodePayload);
    expect(invoke).toHaveBeenNthCalledWith(8, databaseDiscoveryCommands.refreshNode, {
      profileId: "profile-1",
      node,
      limit: null,
      offset: null,
    });
  });

  it("keeps unsupported metadata explicit in the serialized contract", () => {
    expect(database.owner).toEqual({ status: "unavailable" });
    expect(database.encoding).toEqual({ status: "available", value: "utf8mb4" });
    expect(table.engine).toEqual({ status: "available", value: "InnoDB" });
    expect(sequence.dataType).toEqual({ status: "available", value: "bigint" });
    expect(counts.sequences).toEqual({ status: "unavailable" });
  });
});
