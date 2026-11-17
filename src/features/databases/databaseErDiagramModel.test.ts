import { describe, expect, it } from "vitest";
import {
  ER_DIAGRAM_MAX_RELATIONSHIPS,
  buildErDiagram,
  erDiagramCardinalityDescription,
  erDiagramCardinalityLabel,
  erDiagramQualifiedName,
  erDiagramRelationshipColumns,
  erDiagramSelectionForTable,
  erDiagramTableCountLabel,
  erDiagramTableKey,
  normalizeErDiagramTables,
  relationshipCardinality,
  selectErDiagramTables,
} from "@/features/databases/databaseErDiagramModel";
import { objectRowId } from "@/features/databases/databaseExplorerTreeModel";
import type {
  DatabaseConstraintSummary,
  DatabaseMetadataValue,
  DatabaseProfile,
  DatabaseTableSummary,
} from "@/types/contracts";

const unavailable = { status: "unavailable" } as const;

function available<T>(value: T): DatabaseMetadataValue<T> {
  return { status: "available", value };
}

function table(name: string, overrides: Partial<DatabaseTableSummary> = {}): DatabaseTableSummary {
  return {
    database: "app",
    schema: "public",
    name,
    kind: "table",
    estimatedRows: available(42),
    metadataRefreshedAt: available("2026-11-17T08:00:00Z"),
    owner: unavailable,
    encoding: unavailable,
    collation: unavailable,
    sizeBytes: unavailable,
    engine: unavailable,
    ...overrides,
  };
}

function constraint(
  kind: DatabaseConstraintSummary["kind"],
  name: string,
  columns: string[],
  overrides: Partial<DatabaseConstraintSummary> = {},
): DatabaseConstraintSummary {
  return {
    name,
    kind,
    columns,
    referencedSchema: unavailable,
    referencedTable: unavailable,
    referencedColumns: [],
    onUpdate: unavailable,
    onDelete: unavailable,
    checkExpression: unavailable,
    deferrable: unavailable,
    initiallyDeferred: unavailable,
    ...overrides,
  };
}

function foreignKey(
  name: string,
  columns: string[],
  referencedTable: string,
  referencedColumns: string[],
  overrides: Partial<DatabaseConstraintSummary> = {},
): DatabaseConstraintSummary {
  return constraint("foreign_key", name, columns, {
    referencedSchema: available("public"),
    referencedTable: available(referencedTable),
    referencedColumns,
    onUpdate: available("CASCADE"),
    onDelete: available("SET NULL"),
    ...overrides,
  });
}

const users = table("users", { estimatedRows: available(42) });
const orders = table("orders", { estimatedRows: available(1200) });
const orderItems = table("order_items");
const categories = table("categories");

const tablesByKey = {
  [erDiagramTableKey("app", "public", "users")]: [constraint("primary_key", "users_pkey", ["id"])],
  [erDiagramTableKey("app", "public", "orders")]: [
    constraint("primary_key", "orders_pkey", ["id"]),
    constraint("unique", "orders_user_key", ["user_id"]),
    foreignKey("orders_user_fkey", ["user_id"], "users", ["id"]),
  ],
  [erDiagramTableKey("app", "public", "order_items")]: [
    constraint("primary_key", "order_items_pkey", ["order_id", "line_no"]),
    foreignKey("order_items_order_fkey", ["order_id"], "orders", ["id"]),
    foreignKey("order_items_product_fkey", ["product_id"], "products", ["id"]),
  ],
  [erDiagramTableKey("app", "public", "categories")]: [
    constraint("primary_key", "categories_pkey", ["id"]),
    foreignKey("categories_parent_fkey", ["parent_id"], "categories", ["id"], {
      referencedSchema: unavailable,
    }),
  ],
};

describe("database ER diagram model", () => {
  it("builds accurate foreign-key relationships with cardinality", () => {
    const graph = buildErDiagram({
      tables: [users, orders, orderItems, categories],
      constraintsByTable: tablesByKey,
      limit: 50,
    });

    expect(graph.tables.map((entry) => entry.qualifiedName)).toEqual([
      "public.categories",
      "public.order_items",
      "public.orders",
      "public.users",
    ]);
    expect(graph.relationships).toHaveLength(3);
    const orderToUser = graph.relationships.find(
      (relationship) => relationship.name === "orders_user_fkey",
    );
    expect(orderToUser).toMatchObject({
      sourceTable: "public.orders",
      targetTable: "public.users",
      sourceColumns: ["user_id"],
      targetColumns: ["id"],
      cardinality: "one-to-one",
      onUpdate: "CASCADE",
      onDelete: "SET NULL",
    });
    const itemToOrder = graph.relationships.find(
      (relationship) => relationship.name === "order_items_order_fkey",
    );
    expect(itemToOrder?.cardinality).toBe("many-to-one");
    const selfReference = graph.relationships.find(
      (relationship) => relationship.name === "categories_parent_fkey",
    );
    expect(selfReference).toMatchObject({
      sourceTable: "public.categories",
      targetTable: "public.categories",
      cardinality: "many-to-one",
    });
    expect(graph.stats).toMatchObject({
      loadedTables: 4,
      matchedTables: 4,
      visibleTables: 4,
      totalRelationships: 3,
      visibleRelationships: 3,
      unresolvedForeignKeys: 1,
      duplicateTables: 0,
    });
    const orderNode = graph.tables.find((entry) => entry.name === "orders");
    expect(orderNode).toMatchObject({ incoming: 1, outgoing: 1, estimatedRows: 1200 });
    const userNode = graph.tables.find((entry) => entry.name === "users");
    expect(userNode).toMatchObject({ incoming: 1, outgoing: 0 });
  });

  it("recognizes one-to-one cardinality from a matching unique key", () => {
    const constraints = [
      constraint("unique", "profile_user_key", ["user_id"]),
      foreignKey("profile_user_fkey", ["user_id"], "users", ["id"]),
    ];
    expect(relationshipCardinality(constraints, constraints[1])).toBe("one-to-one");
    expect(
      relationshipCardinality(
        [constraint("unique", "composite_key", ["tenant_id", "user_id"])],
        foreignKey("composite_fk", ["user_id", "tenant_id"], "users", ["id", "tenant_id"]),
      ),
    ).toBe("one-to-one");
    expect(
      relationshipCardinality(
        [constraint("unique", "partial_key", ["tenant_id"])],
        foreignKey("partial_fk", ["tenant_id", "user_id"], "users", ["id", "tenant_id"]),
      ),
    ).toBe("many-to-one");
    expect(erDiagramCardinalityLabel("one-to-one")).toBe("1:1");
    expect(erDiagramCardinalityLabel("many-to-one")).toBe("N:1");
    expect(erDiagramCardinalityDescription("many-to-one")).toContain("many child rows");
    expect(
      erDiagramRelationshipColumns({ sourceColumns: ["user_id"], targetColumns: ["id"] }),
    ).toBe("user_id → id");
  });

  it("filters tables by tokens, hides relationships outside the match, and applies a table limit", () => {
    const byUser = buildErDiagram({
      tables: [users, orders, orderItems, categories],
      constraintsByTable: tablesByKey,
      query: "USERS",
      limit: 50,
    });
    expect(byUser.tables.map((entry) => entry.name)).toEqual(["users"]);
    expect(byUser.relationships).toHaveLength(0);
    expect(byUser.stats.hiddenBySearch).toBe(3);
    expect(byUser.stats.hiddenRelationships).toBe(3);

    const schemaAndToken = buildErDiagram({
      tables: [users, orders, orderItems, categories],
      constraintsByTable: tablesByKey,
      query: "public ord",
      limit: 50,
    });
    expect(schemaAndToken.tables.map((entry) => entry.name)).toEqual(["order_items", "orders"]);
    expect(schemaAndToken.relationships).toHaveLength(1);
    expect(schemaAndToken.relationships[0].name).toBe("order_items_order_fkey");

    const limited = buildErDiagram({
      tables: [users, orders, orderItems, categories],
      constraintsByTable: tablesByKey,
      limit: 2,
    });
    expect(limited.tables.map((entry) => entry.name)).toEqual(["categories", "order_items"]);
    expect(limited.stats.hiddenByLimit).toBe(2);
    expect(limited.stats.visibleRelationships).toBe(1);
    expect(limited.relationships[0].name).toBe("categories_parent_fkey");
  });

  it("caps table and relationship counts for large schemas", () => {
    const largeTables = Array.from({ length: 130 }, (_, index) =>
      table(`table_${index.toString().padStart(3, "0")}`, { estimatedRows: available(index) }),
    );
    const constraints: Record<string, DatabaseConstraintSummary[]> = {};
    for (let index = 1; index < 100; index += 1) {
      const name = `table_${index.toString().padStart(3, "0")}`;
      constraints[erDiagramTableKey("app", "public", name)] = Array.from({ length: 5 }, (_, edge) =>
        foreignKey(`${name}_fk_${edge}`, ["parent_id"], "table_000", ["id"], {
          referencedSchema: available("public"),
        }),
      );
    }

    const graph = buildErDiagram({
      tables: largeTables,
      constraintsByTable: constraints,
      limit: 100,
      totalTableCount: 130,
      catalogTruncated: true,
    });

    expect(graph.tables).toHaveLength(100);
    expect(graph.stats).toMatchObject({
      totalTables: 130,
      loadedTables: 130,
      matchedTables: 130,
      visibleTables: 100,
      hiddenByLimit: 30,
      catalogTruncated: true,
    });
    expect(graph.stats.totalRelationships).toBe(495);
    expect(graph.relationships).toHaveLength(ER_DIAGRAM_MAX_RELATIONSHIPS);
    expect(graph.stats.truncatedRelationships).toBe(true);
    expect(graph.stats.visibleRelationships).toBe(ER_DIAGRAM_MAX_RELATIONSHIPS);
    expect(erDiagramTableCountLabel(graph.stats)).toBe("Showing 100 of 130 tables");
  });

  it("reports isolated tables and clear zero-relationship state", () => {
    const graph = buildErDiagram({
      tables: [users, orders, categories],
      constraintsByTable: {},
      limit: 50,
    });
    expect(graph.relationships).toEqual([]);
    expect(graph.stats).toMatchObject({
      totalRelationships: 0,
      visibleRelationships: 0,
      isolatedTables: 3,
      hiddenRelationships: 0,
    });
    expect(graph.stats.unresolvedForeignKeys).toBe(0);
  });

  it("does not invent targets for unresolved foreign keys", () => {
    const graph = buildErDiagram({
      tables: [orderItems],
      constraintsByTable: {
        [erDiagramTableKey("app", "public", "order_items")]: [
          foreignKey("missing_product_fkey", ["product_id"], "products", ["id"]),
        ],
      },
      limit: 50,
    });
    expect(graph.relationships).toEqual([]);
    expect(graph.stats.unresolvedForeignKeys).toBe(1);
  });

  it("deduplicates repeated catalog rows and sorts deterministically", () => {
    const normalized = normalizeErDiagramTables([users, users, orders, categories]);
    expect(normalized.duplicateTables).toBe(1);
    expect(normalized.tables.map((entry) => entry.name)).toEqual(["categories", "orders", "users"]);
    const selection = selectErDiagramTables([users, users, orders, categories], "", 10);
    expect(selection.all).toHaveLength(3);
    expect(selection.visible).toHaveLength(3);
  });

  it("handles empty catalogs without producing partial diagram state", () => {
    const graph = buildErDiagram({ tables: [], constraintsByTable: {}, limit: 50 });
    expect(graph.tables).toEqual([]);
    expect(graph.relationships).toEqual([]);
    expect(graph.stats).toMatchObject({
      totalTables: 0,
      visibleTables: 0,
      isolatedTables: 0,
      visibleRelationships: 0,
    });
  });

  it("builds explorer selections for PostgreSQL and MySQL table nodes", () => {
    const postgres: DatabaseProfile = {
      id: "profile-1",
      serverId: "server-1",
      name: "Primary",
      engine: "postgresql",
      host: "127.0.0.1",
      port: 5432,
      socket: null,
      databaseName: "app",
      username: "app",
      hasCredential: true,
      readOnly: false,
      sslMode: null,
      createdAt: 1,
      updatedAt: 1,
      lastUsedAt: null,
    };
    const selection = erDiagramSelectionForTable(postgres, {
      database: "app",
      schema: "public",
      name: "users",
    });
    expect(selection).toEqual({
      id: objectRowId("profile-1", "app", "public", "tables", "users"),
      profileId: "profile-1",
      kind: "table",
      label: "users",
      engine: "postgresql",
      database: "app",
      schema: "public",
      objectName: "users",
    });

    const mysqlSelection = erDiagramSelectionForTable(
      { ...postgres, engine: "mysql" },
      { database: "metrics", schema: "metrics", name: "events" },
    );
    expect(mysqlSelection?.schema).toBeNull();
    expect(mysqlSelection?.id).toBe(objectRowId("profile-1", "metrics", null, "tables", "events"));
    expect(
      erDiagramSelectionForTable(postgres, { database: null, schema: null, name: "users" }),
    ).toBeNull();
  });

  it("encodes table keys and qualified names safely", () => {
    expect(erDiagramTableKey("app", "public", "orders")).toBe("app::public::orders");
    expect(erDiagramTableKey("app::1", "sch/ema", "a.b")).toBe("app%3A%3A1::sch%2Fema::a.b");
    expect(erDiagramQualifiedName(null, "users")).toBe("users");
    expect(erDiagramQualifiedName("public", "users")).toBe("public.users");
  });
});
