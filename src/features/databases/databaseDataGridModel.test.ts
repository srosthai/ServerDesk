import { describe, expect, it } from "vitest";
import {
  MAX_COLUMN_WIDTH,
  MIN_COLUMN_WIDTH,
  buildCellViewer,
  buildRowFilter,
  cellPresentation,
  columnWidth,
  dataGridTableKey,
  defaultColumnWidth,
  filterDraftValue,
  filterOperatorsFor,
  filterSummary,
  nextPageOffset,
  pageWindowLabel,
  previousPageOffset,
  sortDirectionFor,
  toggleSort,
  visibleColumnNames,
} from "@/features/databases/databaseDataGridModel";
import type { DatabaseRowCell, DatabaseRowColumn, DatabaseRowColumnKind } from "@/types/contracts";

function column(
  name: string,
  kind: DatabaseRowColumnKind,
  overrides: Partial<DatabaseRowColumn> = {},
): DatabaseRowColumn {
  return {
    name,
    dataType:
      kind === "integer"
        ? "bigint"
        : kind === "boolean"
          ? "boolean"
          : kind === "temporal"
            ? "timestamp with time zone"
            : kind === "json"
              ? "jsonb"
              : kind === "binary"
                ? "bytea"
                : "text",
    kind,
    nullable: true,
    ordinalPosition: 1,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: !["json", "binary", "other"].includes(kind),
    filterable: !["json", "binary", "other"].includes(kind),
    searchable: kind === "text",
    ...overrides,
  };
}

function cell(value: string | null, truncated = false): DatabaseRowCell {
  return { value, truncated };
}

describe("database data grid model", () => {
  it("renders SQL NULL and an empty string as distinct values", () => {
    const notes = column("notes", "text");
    expect(cellPresentation(cell(null), notes)).toMatchObject({
      kind: "null",
      text: "NULL",
      canView: false,
    });
    expect(cellPresentation(cell(""), notes)).toMatchObject({
      kind: "empty",
      text: '"" empty',
      canView: false,
    });
    expect(cellPresentation(cell(null), notes).title).toContain("not an empty string");
    expect(cellPresentation(cell(""), notes).title).toContain("not NULL");
  });

  it("renders booleans, numbers, dates, JSON, and binary values by column kind", () => {
    expect(cellPresentation(cell("true"), column("active", "boolean"))).toMatchObject({
      kind: "boolean",
      text: "true",
      numeric: false,
    });
    expect(cellPresentation(cell("0"), column("active", "boolean"))).toMatchObject({
      kind: "boolean",
      text: "false",
    });
    expect(cellPresentation(cell("42"), column("id", "integer"))).toMatchObject({
      kind: "number",
      text: "42",
      numeric: true,
    });
    expect(cellPresentation(cell("12.50"), column("balance", "decimal"))).toMatchObject({
      kind: "number",
      numeric: true,
    });
    expect(
      cellPresentation(cell("2026-02-01 12:00:00+00"), column("created_at", "temporal")),
    ).toMatchObject({
      kind: "temporal",
      text: "2026-02-01 12:00:00+00",
    });
    expect(
      cellPresentation(cell('{"items": [1, 2], "ok": true}'), column("metadata", "json")),
    ).toMatchObject({
      kind: "json",
      canView: true,
    });
    expect(cellPresentation(cell("AAAAAA=="), column("avatar", "binary"))).toMatchObject({
      kind: "binary",
      text: "Binary · 4 B",
      canView: true,
    });
  });

  it("keeps long value previews bounded and marks server truncation", () => {
    const long = "x".repeat(500);
    const longCell = cellPresentation(cell(long), column("notes", "text"));
    expect(longCell.text.length).toBeLessThan(140);
    expect(longCell.canView).toBe(true);

    const truncated = cellPresentation(cell("x".repeat(120), true), column("blob", "json"));
    expect(truncated.truncated).toBe(true);
    expect(truncated.canView).toBe(true);
  });

  it("pretty-prints valid JSON only in the viewer", () => {
    const viewer = buildCellViewer(cell('{"b":2,"a":1}'), column("metadata", "json"));
    expect(viewer).toMatchObject({ format: "json", jsonValid: true });
    expect(viewer.content).toBe('{\n  "b": 2,\n  "a": 1\n}');
  });

  it("shows raw text with an explicit note when JSON is invalid or truncated", () => {
    const invalid = buildCellViewer(cell("{not json"), column("metadata", "json"));
    expect(invalid.format).toBe("json");
    expect(invalid.jsonValid).toBe(false);
    expect(invalid.content).toBe("{not json");
    expect(invalid.note).toContain("not valid JSON");

    const truncated = buildCellViewer(cell('{"a":', true), column("metadata", "json"));
    expect(truncated.note).toContain("4,096");
    expect(truncated.note).toContain("remainder was not read");
  });

  it("decodes base64 binary into a hex dump and preserves text values verbatim", () => {
    const binary = buildCellViewer(cell("AAECQUJD"), column("payload", "binary"));
    expect(binary.format).toBe("binary");
    expect(binary.content).toContain("00000000");
    expect(binary.content).toContain("00 01 02 41 42 43");
    expect(binary.content).toContain("|...ABC");
    expect(binary.note).toContain("6 B");

    const text = buildCellViewer(cell("line one\nline two"), column("notes", "text"));
    expect(text).toMatchObject({ format: "text", content: "line one\nline two" });

    const longText = buildCellViewer(cell("x".repeat(200), true), column("notes", "text"));
    expect(longText.note).toContain("4,096");
  });

  it("describes NULL and empty-string viewer states explicitly", () => {
    expect(buildCellViewer(cell(null), column("notes", "text"))).toMatchObject({
      format: "null",
      content: "NULL",
    });
    expect(buildCellViewer(cell(""), column("notes", "text"))).toMatchObject({
      format: "empty",
      content: '""',
    });
  });

  it("limits filter operators to what the backend can safely bind", () => {
    expect(filterOperatorsFor("text")).toContain("contains");
    expect(filterOperatorsFor("json")).toEqual(["is_null", "is_not_null"]);
    expect(filterOperatorsFor("binary")).toEqual(["is_null", "is_not_null"]);
    expect(filterOperatorsFor("boolean")).not.toContain("contains");
    expect(filterOperatorsFor("integer")).toContain("greater_than_or_equal");
  });

  it("builds typed filters and rejects values that do not match the column", () => {
    expect(buildRowFilter(column("name", "text"), "contains", "ann")).toEqual({
      filter: {
        column: "name",
        operator: "contains",
        value: { kind: "text", value: "ann" },
      },
      error: null,
    });
    expect(buildRowFilter(column("id", "integer"), "greater_than", "42")).toEqual({
      filter: {
        column: "id",
        operator: "greater_than",
        value: { kind: "integer", value: 42 },
      },
      error: null,
    });
    expect(buildRowFilter(column("active", "boolean"), "equals", "false")).toEqual({
      filter: {
        column: "active",
        operator: "equals",
        value: { kind: "boolean", value: false },
      },
      error: null,
    });
    expect(buildRowFilter(column("balance", "decimal"), "less_than", "10.50")).toEqual({
      filter: {
        column: "balance",
        operator: "less_than",
        value: { kind: "text", value: "10.50" },
      },
      error: null,
    });
    expect(buildRowFilter(column("name", "text"), "is_null", "")).toEqual({
      filter: { column: "name", operator: "is_null" },
      error: null,
    });
    expect(buildRowFilter(column("name", "text"), "in", "ann, bob")).toEqual({
      filter: {
        column: "name",
        operator: "in",
        values: [
          { kind: "text", value: "ann" },
          { kind: "text", value: "bob" },
        ],
      },
      error: null,
    });
    expect(buildRowFilter(column("id", "integer"), "equals", "not-a-number").error).toBe(
      "Enter a whole number.",
    );
    expect(buildRowFilter(column("metadata", "json"), "equals", "x").error).toContain(
      "does not support this operator",
    );
  });

  it("summarizes applied filters and rebuilds their draft values", () => {
    const filter = buildRowFilter(column("name", "text"), "contains", "ann").filter;
    expect(filter).not.toBeNull();
    if (!filter) {
      return;
    }
    expect(filterSummary(filter)).toBe('name contains "ann"');
    expect(filterDraftValue(filter)).toBe("ann");

    const nullFilter = buildRowFilter(column("email", "text"), "is_null", "").filter;
    expect(nullFilter && filterSummary(nullFilter)).toBe("email is NULL");
  });

  it("cycles sorting through ascending, descending, and unsorted", () => {
    const ascending = toggleSort([], "name", false);
    expect(ascending).toEqual([{ column: "name", direction: "asc" }]);
    expect(sortDirectionFor(ascending, "name")).toBe("asc");

    const descending = toggleSort(ascending, "name", false);
    expect(descending).toEqual([{ column: "name", direction: "desc" }]);

    expect(toggleSort(descending, "name", false)).toEqual([]);
  });

  it("supports additive multi-column sorting with a bounded list", () => {
    const first = toggleSort([], "name", true);
    const second = toggleSort(first, "email", true);
    expect(second).toEqual([
      { column: "name", direction: "asc" },
      { column: "email", direction: "asc" },
    ]);
    expect(toggleSort(second, "name", true)).toEqual([
      { column: "name", direction: "desc" },
      { column: "email", direction: "asc" },
    ]);

    const full = Array.from({ length: 8 }, (_, index) => ({
      column: `column-${index}`,
      direction: "asc" as const,
    }));
    expect(toggleSort(full, "extra", true)).toEqual(full);
  });

  it("clamps widths and converts hidden columns into the backend visible list", () => {
    const name = column("name", "text");
    expect(columnWidth(name, {})).toBe(defaultColumnWidth("text"));
    expect(columnWidth(name, { name: 10 })).toBe(MIN_COLUMN_WIDTH);
    expect(columnWidth(name, { name: MAX_COLUMN_WIDTH + 100 })).toBe(MAX_COLUMN_WIDTH);

    const columns = [column("id", "integer"), column("name", "text")];
    expect(visibleColumnNames(columns, [])).toBeUndefined();
    expect(visibleColumnNames(columns, ["name"])).toEqual(["id"]);
    expect(visibleColumnNames(columns, ["id", "name"])).toEqual(["id"]);
  });

  it("computes safe page offsets and labels estimates separately from exact counts", () => {
    expect(previousPageOffset(100, 100)).toBe(0);
    expect(previousPageOffset(25, 100)).toBe(0);
    expect(nextPageOffset(0, 100)).toBe(100);
    expect(nextPageOffset(1_000_000, 100)).toBe(1_000_000);

    expect(
      pageWindowLabel({
        offset: 0,
        returned: 100,
        hasMore: true,
        exactRows: null,
      }),
    ).toBe("Rows 1–100 · more rows available");

    expect(
      pageWindowLabel({
        offset: 100,
        returned: 25,
        hasMore: false,
        exactRows: { value: 125, filtered: false },
      }),
    ).toBe("Rows 101–125 of 125");
  });

  it("scopes grid state keys by database, schema, and table", () => {
    expect(dataGridTableKey({ database: "app", schema: "public", table: "users" })).not.toBe(
      dataGridTableKey({ database: "app", schema: "audit", table: "users" }),
    );
    expect(dataGridTableKey({ database: "app", schema: null, table: "users" })).not.toBe(
      dataGridTableKey({ database: "app", schema: "public", table: "users" }),
    );
  });
});
