import { describe, expect, it } from "vitest";
import {
  defaultPrivilegeTarget,
  privilegeOptionsFor,
  privilegeTargetKindOptions,
  roleCapabilitiesFor,
  roleLabel,
} from "@/features/databases/databaseRolesModel";
import type { CapabilityFlags } from "@/types/contracts";

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

describe("databaseRolesModel", () => {
  it("derives browse and manage capability decisions", () => {
    const connected = roleCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: false,
      flags,
    });
    expect(connected.browse.enabled).toBe(true);
    expect(connected.manage.enabled).toBe(true);

    const readOnly = roleCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: true,
      flags,
    });
    expect(readOnly.browse.enabled).toBe(true);
    expect(readOnly.manage.enabled).toBe(false);
    expect(readOnly.manage.reason).toContain("read-only");

    const unsupported = roleCapabilitiesFor({
      engine: "mysql",
      version: "5.7.44",
      connected: true,
      readOnly: false,
      flags: { ...flags, roles: false },
    });
    expect(unsupported.browse.enabled).toBe(false);
    expect(unsupported.browse.reason).toContain("not supported");
  });

  it("uses the engine-specific target and privilege vocabulary", () => {
    expect(privilegeTargetKindOptions("mysql")).toEqual(["global", "database", "table"]);
    expect(privilegeTargetKindOptions("postgresql")).toEqual([
      "database",
      "schema",
      "table",
      "sequence",
    ]);
    expect(privilegeOptionsFor("mysql", "table")).toContain("insert");
    expect(privilegeOptionsFor("mysql", "table")).not.toContain("connect");
    expect(privilegeOptionsFor("postgresql", "schema")).toEqual(["usage", "create"]);
    expect(privilegeOptionsFor("postgresql", "sequence")).toEqual(["usage", "select", "update"]);
  });

  it("builds a scoped default target and labels MySQL hosts", () => {
    expect(
      defaultPrivilegeTarget("postgresql", "table", {
        database: "app",
        schema: "public",
        table: "orders",
      }),
    ).toEqual({ kind: "table", database: null, schema: "public", object: "orders" });

    expect(
      defaultPrivilegeTarget("mysql", "table", {
        database: "app",
        schema: null,
        table: "orders",
      }),
    ).toEqual({ kind: "table", database: "app", schema: null, object: "orders" });

    expect(roleLabel({ name: "reporting", host: "%" })).toBe("reporting@%");
    expect(roleLabel({ name: "reporting", host: null })).toBe("reporting");
  });
});
