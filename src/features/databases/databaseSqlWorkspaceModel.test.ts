import { describe, expect, it } from "vitest";
import {
  findSqlStatementRanges,
  formatQueryDuration,
  preferredResultSetIndex,
  resolveSqlForExecution,
  resultSetFileName,
  resultSetLabel,
  resultSetToCsv,
  resultSetToJson,
  resultSetToTsv,
  sqlErrorLocation,
  tokenizeSql,
} from "@/features/databases/databaseSqlWorkspaceModel";
import type { DatabaseQueryExecutionResult, DatabaseQueryResultSet } from "@/types/contracts";

const resultSet: DatabaseQueryResultSet = {
  index: 0,
  columns: [
    { name: "id", dataType: "int4" },
    { name: "name", dataType: "text" },
    { name: "note", dataType: "text" },
  ],
  rows: [
    [
      { value: "1", truncated: false },
      { value: 'Ann, "Ace"', truncated: false },
      { value: "line one\nline two", truncated: false },
    ],
    [
      { value: "2", truncated: false },
      { value: null, truncated: false },
      { value: "", truncated: false },
    ],
  ],
  returnedRows: 2,
  affectedRows: null,
  lastInsertId: null,
  truncated: false,
  columnsTruncated: false,
};

const executionResult: DatabaseQueryExecutionResult = {
  engine: "postgresql",
  readOnly: false,
  results: [
    {
      ...resultSet,
      index: 0,
      columns: [],
      rows: [],
      returnedRows: 0,
      affectedRows: 5,
    },
    resultSet,
  ],
  warnings: [],
  destructive: false,
  requiresConfirmation: false,
  truncated: false,
  maxRows: 500,
  durationMs: 4,
};

describe("SQL workspace model", () => {
  it("highlights engine-specific keywords without dropping characters", () => {
    const postgres = tokenizeSql(
      "SELECT id FROM users WHERE name ILIKE 'a%' -- note",
      "postgresql",
    );
    const mysql = tokenizeSql("SELECT id FROM users WHERE name LIKE 'a%' # note", "mysql");

    expect(postgres.map((token) => token.text).join("")).toBe(
      "SELECT id FROM users WHERE name ILIKE 'a%' -- note",
    );
    expect(postgres.some((token) => token.kind === "keyword" && token.text === "ILIKE")).toBe(true);
    expect(postgres.some((token) => token.kind === "string" && token.text === "'a%'")).toBe(true);
    expect(postgres.some((token) => token.kind === "comment" && token.text === "-- note")).toBe(
      true,
    );
    expect(mysql.some((token) => token.kind === "comment" && token.text === "# note")).toBe(true);
    expect(
      tokenizeSql("CREATE TABLE t (id int) ENGINE=InnoDB", "mysql").some(
        (token) => token.kind === "keyword" && token.text === "ENGINE",
      ),
    ).toBe(true);
  });

  it("tokenizes PostgreSQL dollar-quoted strings and MySQL backtick identifiers", () => {
    const postgres = tokenizeSql("SELECT $tag$hello; world$tag$;", "postgresql");
    expect(postgres.some((token) => token.kind === "string")).toBe(true);
    expect(postgres.find((token) => token.kind === "string")?.text).toBe("$tag$hello; world$tag$");
    expect(findSqlStatementRanges("SELECT $tag$a;b$tag$; SELECT 2;", "postgresql")).toHaveLength(2);

    const mysql = tokenizeSql("SELECT `weird name` FROM t", "mysql");
    expect(
      mysql.some((token) => token.kind === "identifier" && token.text === "`weird name`"),
    ).toBe(true);
  });

  it("finds statements outside strings, comments, and dollar quotes", () => {
    const sql = "SELECT ';' AS semi; -- a ; comment\nSELECT 2; /* ; */ SELECT 3";
    const ranges = findSqlStatementRanges(sql, "postgresql");
    expect(ranges).toHaveLength(3);
    expect(ranges.map((range) => sql.slice(range.start, range.end))).toEqual([
      "SELECT ';' AS semi",
      "-- a ; comment\nSELECT 2",
      "/* ; */ SELECT 3",
    ]);
  });

  it("resolves selected SQL, the statement at the cursor, and all SQL", () => {
    const sql = "SELECT 1;\nSELECT 2;\nSELECT 3;";
    const selected = resolveSqlForExecution({
      sql,
      engine: "postgresql",
      selectionStart: 10,
      selectionEnd: 18,
    });
    expect(selected).toMatchObject({ sql: "SELECT 2", source: "selection" });

    const current = resolveSqlForExecution({
      sql,
      engine: "postgresql",
      selectionStart: 3,
      selectionEnd: 3,
    });
    expect(current).toMatchObject({
      sql: "SELECT 1",
      source: "statement",
      statementNumber: 1,
      statementCount: 3,
    });

    const atSecond = resolveSqlForExecution({
      sql,
      engine: "postgresql",
      selectionStart: 15,
      selectionEnd: 15,
    });
    expect(atSecond).toMatchObject({ sql: "SELECT 2", statementNumber: 2 });

    const all = resolveSqlForExecution({
      sql,
      engine: "postgresql",
      selectionStart: 3,
      selectionEnd: 3,
      source: "all",
    });
    expect(all).toMatchObject({ sql: sql.trim(), source: "all", statementCount: 3 });
  });

  it("positions a one-based engine error on a line and column", () => {
    expect(sqlErrorLocation("SELECT 1;\nSELEC 2;", 17)).toEqual({
      line: 2,
      column: 7,
      lineText: "SELEC 2;",
      caretColumn: 6,
    });
    expect(sqlErrorLocation("SELECT 1", undefined)).toBeNull();
    expect(sqlErrorLocation("SELECT 1", 0)).toBeNull();
  });

  it("exports CSV, JSON, and TSV with headers and safe quoting", () => {
    expect(resultSetToCsv(resultSet)).toBe(
      'id,name,note\r\n1,"Ann, ""Ace""","line one\nline two"\r\n2,,\r\n',
    );
    expect(JSON.parse(resultSetToJson(resultSet))).toEqual([
      { id: "1", name: 'Ann, "Ace"', note: "line one\nline two" },
      { id: "2", name: null, note: "" },
    ]);
    expect(resultSetToTsv(resultSet)).toBe(
      'id\tname\tnote\n1\tAnn, "Ace"\tline one\\nline two\n2\tNULL\t',
    );
  });

  it("suffixes duplicate column names in JSON exports", () => {
    const duplicate: DatabaseQueryResultSet = {
      ...resultSet,
      columns: [
        { name: "id", dataType: "int4" },
        { name: "id", dataType: "int4" },
      ],
      rows: [
        [
          { value: "1", truncated: false },
          { value: "2", truncated: false },
        ],
      ],
      returnedRows: 1,
    };
    expect(JSON.parse(resultSetToJson(duplicate))).toEqual([{ id: "1", id_2: "2" }]);
  });

  it("formats durations, labels, file names, and prefers a row result", () => {
    expect(formatQueryDuration(0)).toBe("<1 ms");
    expect(formatQueryDuration(12)).toBe("12 ms");
    expect(formatQueryDuration(1_250)).toBe("1.25 s");
    expect(resultSetLabel(resultSet, 0)).toBe("Result 1 · 2 rows");
    expect(resultSetLabel(executionResult.results[0], 0)).toBe("Result 1 · 5 affected");
    expect(resultSetFileName("Primary DB", "Query 2", 0, "csv")).toBe(
      "primary-db-query-2-result-1.csv",
    );
    expect(preferredResultSetIndex(executionResult)).toBe(1);
    expect(preferredResultSetIndex({ ...executionResult, results: [] })).toBe(0);
  });
});
