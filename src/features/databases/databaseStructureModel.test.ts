import { describe, expect, it } from "vitest";
import {
  buildTableRelationships,
  columnDefaultLabel,
  columnIdentityLabel,
  columnTypeLabel,
  formatBytes,
  indexActionDecisions,
  structureCapabilitiesFor,
  tableSummaryRows,
} from "@/features/databases/databaseStructureModel";
import type {
  CapabilityFlags,
  DatabaseColumnSummary,
  DatabaseConstraintSummary,
  DatabaseIndexSummary,
  DatabaseTableDependencies,
  DatabaseTableSummary,
} from "@/types/contracts";

const flags: CapabilityFlags = {
  schemas: true,
  sequences: true,
  materializedViews: true,
  routines: true,
  triggers: true,
  roles: true,
  sessions: true,
  transactions: true,
  savepoints: true,
  cancelQuery: true,
  explain: true,
  checkConstraints: false,
  generatedColumns: true,
  identityColumns: true,
  autoIncrementColumns: false,
  columnTypeChange: true,
  columnRename: false,
  tableRename: true,
  tableDuplicate: true,
  columnReorder: false,
  databaseRename: false,
  indexRename: false,
  partialIndexes: false,
  queryTimeout: true,
};

const available = <T>(value: T) => ({ status: "available" as const, value });
const unavailable = { status: "unavailable" as const };

describe("database structure model", () => {
  it("disables unsupported actions with a versioned explanation", () => {
    const capabilities = structureCapabilitiesFor({
      engine: "mysql",
      version: "5.7.44",
      connected: true,
      readOnly: false,
      actions: [
        "create_table",
        "add_column",
        "edit_column",
        "rename_column",
        "drop_column",
        "create_index",
        "drop_index",
      ],
    });

    expect(capabilities.addColumn.enabled).toBe(true);
    expect(capabilities.renameColumn.enabled).toBe(true);
    expect(capabilities.renameIndex.enabled).toBe(false);
    expect(capabilities.renameIndex.reason).toBe(
      "Index renaming is not supported by MySQL 5.7.44.",
    );
    expect(capabilities.addCheckConstraint.enabled).toBe(false);
    expect(capabilities.partialIndexes.enabled).toBe(false);
  });

  it("disables every write when the profile is read-only", () => {
    const capabilities = structureCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: true,
      actions: ["create_table", "add_column"],
    });

    expect(capabilities.createTable.enabled).toBe(false);
    expect(capabilities.createTable.reason).toContain("read-only");
    expect(capabilities.addColumn.reason).toContain("read-only");
  });

  it("disables disconnected actions before the server is available", () => {
    const capabilities = structureCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: false,
      readOnly: false,
      flags,
    });

    expect(capabilities.dropTable.enabled).toBe(false);
    expect(capabilities.dropTable.reason).toContain("Connect this database profile");
  });

  it("keeps primary-key index management attached to its constraint", () => {
    const primary: DatabaseIndexSummary = {
      name: "users_pkey",
      columns: ["id"],
      unique: true,
      primary: true,
      method: available("btree"),
      predicate: available(null),
    };
    const capabilities = structureCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: false,
      flags,
    });

    const decisions = indexActionDecisions(primary, capabilities);
    expect(decisions.rename.enabled).toBe(false);
    expect(decisions.drop.enabled).toBe(false);
    expect(decisions.drop.reason).toContain("primary-key constraint");
  });

  it("builds outgoing and incoming foreign-key relationships", () => {
    const constraints: DatabaseConstraintSummary[] = [
      {
        name: "users_team_id_fkey",
        kind: "foreign_key",
        columns: ["team_id"],
        referencedSchema: available("public"),
        referencedTable: available("teams"),
        referencedColumns: ["id"],
        onUpdate: available("CASCADE"),
        onDelete: available("SET NULL"),
        checkExpression: unavailable,
        deferrable: available(false),
        initiallyDeferred: available(false),
      },
    ];
    const dependencies: DatabaseTableDependencies = {
      incomingForeignKeys: available({
        items: [
          {
            kind: "incoming_foreign_key",
            name: "orders_user_id_fkey",
            database: "app",
            schema: "public",
            table: "orders",
            columns: ["user_id"],
            referencedColumns: ["id"],
            onUpdate: available("CASCADE"),
            onDelete: available("CASCADE"),
            details: available("orders.user_id references users.id"),
          },
        ],
        limit: 100,
        truncated: false,
      }),
      dependentViews: unavailable,
      ownedSequences: unavailable,
      triggers: unavailable,
    };

    const relationships = buildTableRelationships(
      { database: "app", schema: "public", table: "users" },
      constraints,
      dependencies,
    );

    expect(relationships).toHaveLength(2);
    expect(relationships[0]).toMatchObject({
      direction: "outgoing",
      sourceTable: "public.users",
      targetTable: "public.teams",
    });
    expect(relationships[1]).toMatchObject({
      direction: "incoming",
      sourceTable: "public.orders",
      targetTable: "public.users",
    });
  });

  it("formats column types, defaults, identity, and size values", () => {
    const column: DatabaseColumnSummary = {
      name: "price",
      dataType: "numeric",
      length: available(null),
      precision: available(10),
      scale: available(2),
      nullable: true,
      defaultValue: available("0.00"),
      ordinalPosition: 3,
      primaryKey: false,
      identity: available("none"),
      generated: available(false),
      generationExpression: available(null),
      comment: available(null),
      collation: available(null),
    };

    expect(columnTypeLabel(column)).toBe("numeric(10,2)");
    expect(columnDefaultLabel(column)).toBe("0.00");
    expect(columnIdentityLabel(column)).toBe("—");
    expect(formatBytes(16_384)).toBe("16 KB");
  });

  it("reports unavailable table metadata instead of guessing", () => {
    const table: DatabaseTableSummary = {
      database: "app",
      schema: "public",
      name: "users",
      kind: "table",
      estimatedRows: unavailable,
      metadataRefreshedAt: unavailable,
      owner: unavailable,
      encoding: unavailable,
      collation: unavailable,
      sizeBytes: unavailable,
      engine: unavailable,
    };

    const rows = tableSummaryRows(table, {
      database: "app",
      schema: "public",
      table: "users",
    });
    expect(rows.find((row) => row.label === "Estimated rows")?.value).toBe("Unavailable");
    expect(rows.find((row) => row.label === "Size")?.value).toBe("Unavailable");
  });
});
