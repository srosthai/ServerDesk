import { describe, expect, it } from "vitest";
import {
  buildDatabaseHistoryFilter,
  endOfDaySeconds,
  filterDatabaseSnippets,
  formatHistoryDuration,
  historyFiltersActive,
  snippetCompatibilityReason,
  snippetEngineLabel,
  snippetMatchesEngine,
  snippetPreview,
  startOfDaySeconds,
} from "@/features/databases/databaseHistoryModel";
import type { DatabaseSavedQuery } from "@/types/contracts";

const snippet: DatabaseSavedQuery = {
  id: "snippet-1",
  name: "Active users",
  engine: "postgresql",
  sqlText: "SELECT * FROM users WHERE active = true",
  createdAt: 1,
  updatedAt: 1,
};

describe("database history model", () => {
  it("maps connection, result, and date filters to the backend shape", () => {
    const filter = buildDatabaseHistoryFilter(
      {
        connectionId: "connection-1",
        result: "failure",
        dateFrom: "2026-01-02",
        dateTo: "2026-01-03",
      },
      { serverId: "server-1", limit: 250 },
    );

    expect(filter).toEqual({
      serverId: "server-1",
      connectionId: "connection-1",
      result: "failure",
      startedAfter: startOfDaySeconds("2026-01-02"),
      startedBefore: endOfDaySeconds("2026-01-03"),
      limit: 250,
    });
    expect(filter.startedAfter).toEqual(expect.any(Number));
    expect(filter.startedBefore).toEqual(expect.any(Number));
  });

  it("maps all filters to null and reports whether filters are active", () => {
    const filter = buildDatabaseHistoryFilter(
      { connectionId: "all", result: "all", dateFrom: "", dateTo: "" },
      { serverId: "server-1", limit: 1_000 },
    );
    expect(filter).toEqual({
      serverId: "server-1",
      connectionId: null,
      result: null,
      startedAfter: null,
      startedBefore: null,
      limit: 1_000,
    });
    expect(
      historyFiltersActive({ connectionId: "all", result: "all", dateFrom: "", dateTo: "" }),
    ).toBe(false);
    expect(
      historyFiltersActive({
        connectionId: "connection-1",
        result: "all",
        dateFrom: "",
        dateTo: "",
      }),
    ).toBe(true);
  });

  it("formats durations at millisecond and second scales", () => {
    expect(formatHistoryDuration(42)).toBe("42 ms");
    expect(formatHistoryDuration(1_250)).toBe("1.3 s");
    expect(formatHistoryDuration(-10)).toBe("0 ms");
  });

  it("rejects snippets saved for another engine and labels engines", () => {
    expect(snippetMatchesEngine(snippet, "postgresql")).toBe(true);
    expect(snippetMatchesEngine(snippet, "mysql")).toBe(false);
    expect(snippetCompatibilityReason(snippet, "postgresql")).toBeNull();
    expect(snippetCompatibilityReason(snippet, "mysql")).toContain("Saved for PostgreSQL");
    expect(snippetEngineLabel("mysql")).toBe("MySQL");
    expect(snippetEngineLabel("postgresql")).toBe("PostgreSQL");
  });

  it("searches snippets by name and SQL body", () => {
    expect(filterDatabaseSnippets([snippet], "active")).toHaveLength(1);
    expect(filterDatabaseSnippets([snippet], "FROM users")).toHaveLength(1);
    expect(filterDatabaseSnippets([snippet], "orders")).toHaveLength(0);
    expect(filterDatabaseSnippets([snippet], "")).toHaveLength(1);
  });

  it("summarizes long SQL on one bounded line", () => {
    const preview = snippetPreview("SELECT\n  *\nFROM users", 80);
    expect(preview).toBe("SELECT * FROM users");
    expect(snippetPreview("x".repeat(200), 20)).toHaveLength(20);
    expect(snippetPreview("x".repeat(200), 20).endsWith("…")).toBe(true);
  });
});
