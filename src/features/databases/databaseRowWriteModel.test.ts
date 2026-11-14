import { describe, expect, it } from "vitest";
import {
  buildInsertValues,
  buildUpdatePayload,
  editFields,
  identityKey,
  identityValues,
  insertFields,
  rowEditingBlocked,
  updateField,
} from "@/features/databases/databaseRowWriteModel";
import type {
  DatabaseRowColumn,
  DatabaseRowReadResult,
  DatabaseRowRecord,
} from "@/types/contracts";

const columns: DatabaseRowColumn[] = [
  {
    name: "id",
    dataType: "bigint",
    kind: "integer",
    nullable: false,
    ordinalPosition: 1,
    primaryKey: true,
    identity: "by_default",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: false,
  },
  {
    name: "name",
    dataType: "text",
    kind: "text",
    nullable: false,
    ordinalPosition: 2,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: true,
  },
  {
    name: "nickname",
    dataType: "text",
    kind: "text",
    nullable: true,
    ordinalPosition: 3,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: true,
  },
  {
    name: "computed",
    dataType: "text",
    kind: "text",
    nullable: true,
    ordinalPosition: 4,
    primaryKey: false,
    identity: "none",
    generated: true,
    sortable: true,
    filterable: true,
    searchable: true,
  },
];

const record: DatabaseRowRecord = {
  cells: [
    { value: "1", truncated: false },
    { value: "ann", truncated: false },
    { value: null, truncated: false },
  ],
  identity: [{ column: "id", value: "1" }],
};

const readResult: DatabaseRowReadResult = {
  engine: "postgresql",
  table: { database: "app", schema: "public", name: "users" },
  columns,
  visibleColumns: ["id", "name", "nickname"],
  identity: { kind: "primary_key", columns: ["id"], stable: true },
  editable: true,
  readOnlyReasons: [],
  rows: [record],
  page: { limit: 100, offset: 0, returned: 1, hasMore: false },
  counts: { estimatedTableRows: { status: "unavailable" } },
};

describe("databaseRowWriteModel", () => {
  it("keeps NULL, empty string, and DEFAULT distinct in insert requests", () => {
    const fields = insertFields(columns);
    expect(fields.map((field) => field.column.name)).toEqual(["id", "name", "nickname"]);
    expect(fields.every((field) => field.mode === "default")).toBe(true);

    const values = buildInsertValues(fields);
    expect(values).toEqual([
      { column: "id", mode: "default" },
      { column: "name", mode: "default" },
      { column: "nickname", mode: "default" },
    ]);

    const edited = updateField(updateField(fields, 1, { mode: "value", value: "" }), 2, {
      mode: "null",
    });
    expect(buildInsertValues(edited)).toEqual([
      { column: "id", mode: "default" },
      { column: "name", mode: "value", value: "" },
      { column: "nickname", mode: "null" },
    ]);
  });

  it("builds update values with original-value conflict guards", () => {
    const fields = editFields(columns, readResult, record);
    expect(fields.map((field) => field.column.name)).toEqual(["name", "nickname"]);
    expect(fields[0]).toMatchObject({ mode: "value", value: "ann", original: "ann" });
    expect(fields[1]).toMatchObject({ mode: "null", value: "", original: null });

    const changed = updateField(fields, 0, { value: "ann updated" });
    expect(buildUpdatePayload(changed)).toEqual({
      values: [{ column: "name", mode: "value", value: "ann updated" }],
      expected: [{ column: "name", value: "ann" }],
    });

    const nulled = updateField(fields, 0, { mode: "null" });
    expect(buildUpdatePayload(nulled)).toEqual({
      values: [{ column: "name", mode: "null" }],
      expected: [{ column: "name", value: "ann" }],
    });
  });

  it("never edits a generated or identity column", () => {
    const fields = editFields(columns, readResult, record);
    expect(fields.some((field) => field.column.name === "id")).toBe(false);
    expect(fields.some((field) => field.column.name === "computed")).toBe(false);
  });

  it("refuses to update a truncated value from an incomplete preview", () => {
    const truncated: DatabaseRowRecord = {
      cells: [
        { value: "1", truncated: false },
        { value: "ann", truncated: true },
        { value: null, truncated: false },
      ],
      identity: [{ column: "id", value: "1" }],
    };
    const fields = updateField(editFields(columns, readResult, truncated), 0, {
      value: "ann updated",
    });
    expect(buildUpdatePayload(fields)).toContain("truncated by the server");
  });

  it("requires a change before an update is sent", () => {
    const fields = editFields(columns, readResult, record);
    expect(buildUpdatePayload(fields)).toBe("Change at least one field before saving.");
  });

  it("marks tables without a stable identity as read-only for every write", () => {
    expect(rowEditingBlocked(undefined, false)).toContain("Load table rows");
    expect(rowEditingBlocked({ ...readResult, editable: false }, false)).toContain(
      "no primary key",
    );
    expect(rowEditingBlocked(readResult, true)).toContain("read-only");
    expect(rowEditingBlocked(readResult, false)).toBeNull();
  });

  it("uses stable identity values and keys for selection", () => {
    expect(identityValues(record)).toEqual([{ column: "id", value: "1" }]);
    expect(identityKey(record)).toBe(JSON.stringify([["id", "1"]]));
  });
});
