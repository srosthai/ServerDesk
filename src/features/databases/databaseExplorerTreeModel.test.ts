import { describe, expect, it } from "vitest";
import { objectKindFor } from "@/features/databases/databaseExplorerTreeModel";
import type {
  DatabaseRoutineSummary,
  DatabaseTableSummary,
  DatabaseViewSummary,
} from "@/types/contracts";

const unavailable = { status: "unavailable" } as const;
const metadata = {
  owner: unavailable,
  encoding: unavailable,
  collation: unavailable,
  sizeBytes: unavailable,
  engine: unavailable,
};

describe("database explorer object kinds", () => {
  it("does not classify a table as a view", () => {
    const table = {
      database: "app",
      schema: "public",
      name: "users",
      kind: "table",
      estimatedRows: unavailable,
      metadataRefreshedAt: unavailable,
      ...metadata,
    } as DatabaseTableSummary;

    expect(objectKindFor("tables", table)).toBeNull();
  });

  it("keeps the typed kind for views and routines", () => {
    const view = {
      database: "app",
      schema: "public",
      name: "active_users",
      kind: "view",
      ...metadata,
    } as DatabaseViewSummary;
    const routine = {
      database: "app",
      schema: "public",
      name: "refresh_users",
      kind: "function",
      language: unavailable,
      returnType: unavailable,
      arguments: unavailable,
      ...metadata,
    } as unknown as DatabaseRoutineSummary;

    expect(objectKindFor("views", view)).toBe("view");
    expect(objectKindFor("routines", routine)).toBe("routine");
  });
});
