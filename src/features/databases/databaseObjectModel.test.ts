import { describe, expect, it } from "vitest";
import {
  dropActionForDefinition,
  objectCapabilitiesFor,
  objectDefinitionLines,
  objectRefForSelection,
  refreshActionForDefinition,
  replaceViewAction,
} from "@/features/databases/databaseObjectModel";
import type {
  CapabilityFlags,
  DatabaseObjectDefinition,
  DatabaseObjectRef,
} from "@/types/contracts";

const flags: CapabilityFlags = {
  schemas: true,
  sequences: true,
  materializedViews: true,
  routines: true,
  routineEdit: true,
  triggers: true,
  triggerEdit: true,
  viewManagement: true,
  roles: true,
  sessions: true,
  transactions: true,
  savepoints: true,
  cancelQuery: true,
  explain: true,
  checkConstraints: true,
  generatedColumns: true,
  identityColumns: true,
  autoIncrementColumns: false,
  columnTypeChange: true,
  columnRename: true,
  tableRename: true,
  tableDuplicate: true,
  columnReorder: false,
  databaseRename: true,
  indexRename: true,
  partialIndexes: true,
  queryTimeout: true,
};

function definition(overrides: Partial<DatabaseObjectDefinition> = {}): DatabaseObjectDefinition {
  return {
    kind: "view",
    database: "app",
    schema: "public",
    name: "active_users",
    owner: { status: "available", value: "app" },
    language: { status: "unavailable" },
    routineKind: { status: "unavailable" },
    arguments: { status: "unavailable" },
    returnType: { status: "unavailable" },
    viewQuery: { status: "available", value: "SELECT id FROM users" },
    routineBody: { status: "unavailable" },
    definition: {
      status: "available",
      value: "CREATE VIEW public.active_users AS SELECT id FROM users",
    },
    trigger: null,
    sequence: null,
    truncated: false,
    ...overrides,
  };
}

describe("databaseObjectModel", () => {
  it("builds a typed ref from a selected explorer row", () => {
    expect(
      objectRefForSelection({
        id: "row",
        profileId: "profile",
        kind: "view",
        label: "daily_totals",
        engine: "postgresql",
        database: "app",
        schema: "public",
        objectName: "daily_totals",
        objectKind: "materialized_view",
      }),
    ).toEqual({
      kind: "materialized_view",
      database: "app",
      schema: "public",
      name: "daily_totals",
      table: null,
      arguments: null,
    });

    expect(objectRefForSelection(null)).toBeNull();
  });

  it("hides sequence and materialized-view actions when the engine cannot support them", () => {
    const mysql = objectCapabilitiesFor({
      engine: "mysql",
      version: "8.0.36",
      connected: true,
      readOnly: false,
      flags: {
        ...flags,
        sequences: false,
        materializedViews: false,
      },
    });
    expect(mysql.createSequence.enabled).toBe(false);
    expect(mysql.createSequence.reason).toContain("MySQL");
    expect(mysql.refreshMaterializedView.enabled).toBe(false);

    const pg = objectCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: false,
      flags,
      actions: [
        "create_view",
        "replace_view",
        "drop_view",
        "create_routine",
        "edit_routine",
        "drop_routine",
        "create_trigger",
        "edit_trigger",
        "drop_trigger",
        "create_sequence",
        "alter_sequence",
        "drop_sequence",
        "refresh_materialized_view",
        "drop_materialized_view",
      ],
    });
    expect(pg.createSequence.enabled).toBe(true);
    expect(pg.refreshMaterializedView.enabled).toBe(true);
  });

  it("disables every object change for read-only profiles with an explanation", () => {
    const capabilities = objectCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: true,
      flags,
    });
    expect(capabilities.createView.enabled).toBe(false);
    expect(capabilities.createView.reason).toContain("read-only");
    expect(capabilities.dropSequence.reason).toContain("read-only");
  });

  it("builds drop, replace, and refresh actions from inspected definitions", () => {
    expect(dropActionForDefinition(definition())).toMatchObject({
      kind: "drop_view",
      name: "active_users",
      materialized: false,
    });
    expect(
      dropActionForDefinition(definition({ kind: "materialized_view", name: "daily_totals" })),
    ).toMatchObject({ kind: "drop_materialized_view", name: "daily_totals" });
    expect(replaceViewAction(definition())).toMatchObject({
      kind: "create_view",
      orReplace: true,
      query: "SELECT id FROM users",
    });
    expect(refreshActionForDefinition(definition({ kind: "materialized_view" }))).toMatchObject({
      kind: "refresh_materialized_view",
    });
    expect(refreshActionForDefinition(definition())).toBeNull();
  });

  it("renders trigger and sequence details without inventing unavailable values", () => {
    const rows = objectDefinitionLines(
      definition({
        kind: "sequence",
        name: "users_id_seq",
        viewQuery: { status: "unavailable" },
        sequence: {
          dataType: { status: "available", value: "bigint" },
          startValue: { status: "available", value: "1" },
          increment: { status: "available", value: "1" },
          minimum: { status: "unavailable" },
          maximum: { status: "unavailable" },
          currentValue: { status: "available", value: "42" },
          cache: { status: "unavailable" },
          cycle: { status: "available", value: false },
        },
      }),
    );
    expect(rows.find((row) => row.label === "Current value")?.value).toBe("42");
    expect(rows.find((row) => row.label === "Minimum")?.value).toBe("Unavailable");
    expect(rows.find((row) => row.label === "Cycle")?.value).toBe("No");
  });

  it("accepts object refs with materialized-view kinds", () => {
    const ref: DatabaseObjectRef = {
      kind: "materialized_view",
      database: "app",
      schema: "public",
      name: "daily_totals",
    };
    expect(ref.kind).toBe("materialized_view");
  });
});
