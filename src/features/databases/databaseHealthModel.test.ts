import { describe, expect, it } from "vitest";
import {
  HEALTH_SAMPLE_STALE_AFTER_MS,
  buildHealthIndicators,
  formatHealthRowEstimate,
  formatHealthSampleAge,
  formatHealthTableName,
  formatHealthTableSize,
  formatHealthVersion,
  healthScopeMismatch,
  healthTables,
  isHealthSampleStale,
} from "@/features/databases/databaseHealthModel";
import type { DatabaseHealthSnapshot, DatabaseTableSize } from "@/types/contracts";

const NOW = 1_700_000_000_000;

function table(overrides: Partial<DatabaseTableSize> = {}): DatabaseTableSize {
  return {
    database: "app",
    schema: "public",
    name: "users",
    kind: "table",
    sizeBytes: { status: "available", value: 16_384 },
    estimatedRows: { status: "available", value: 42 },
    metadataRefreshedAt: {
      status: "available",
      value: new Date(NOW - 60_000).toISOString(),
    },
    ...overrides,
  };
}

function snapshot(overrides: Partial<DatabaseHealthSnapshot> = {}): DatabaseHealthSnapshot {
  return {
    profileId: "profile-1",
    engine: "postgresql",
    database: "app",
    version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
    sampledAt: NOW - 30_000,
    latencyMs: { status: "available", value: 14 },
    databaseSizeBytes: { status: "available", value: 1_048_576 },
    tableSizes: {
      status: "available",
      value: {
        items: [
          table(),
          table({
            name: "orders",
            sizeBytes: { status: "available", value: 32_768 },
            estimatedRows: { status: "available", value: 8 },
          }),
        ],
        limit: 12,
        truncated: false,
      },
    },
    ...overrides,
  };
}

describe("database health formatting", () => {
  it("formats versions and table identities predictably", () => {
    expect(formatHealthVersion({ version: "16.2.0", raw: "PostgreSQL 16.2" })).toBe("16.2.0");
    expect(formatHealthTableName(table())).toBe("public.users");
    expect(formatHealthTableName(table({ schema: "app" }))).toBe("users");
    expect(formatHealthTableSize(table())).toBe("16 KB");
    expect(formatHealthRowEstimate(table())).toBe("~42 est. rows");
    expect(formatHealthSampleAge(NOW - 90_000, NOW)).toBe("1 min ago");
    expect(formatHealthSampleAge(NOW - 30_000, NOW)).toBe("just now");
  });

  it("shows unavailable values instead of zero", () => {
    const unavailable = table({
      sizeBytes: { status: "unavailable" },
      estimatedRows: { status: "unavailable" },
    });

    expect(formatHealthTableSize(unavailable)).toBe("Unavailable");
    expect(formatHealthRowEstimate(unavailable)).toBe("Rows unavailable");
    expect(healthTables(snapshot({ tableSizes: { status: "unavailable" } }))).toBeNull();
  });
});

describe("database health estimates and indicators", () => {
  it("labels catalog sizes as estimates and latency as measured", () => {
    const indicators = buildHealthIndicators(snapshot(), NOW);
    const byKey = Object.fromEntries(indicators.map((entry) => [entry.key, entry]));

    expect(byKey.latency.value).toBe("14 ms");
    expect(byKey.latency.basis).toBe("measured");
    expect(byKey.storage.value).toBe("1 MB");
    expect(byKey.storage.basis).toBe("estimated");
    expect(byKey["table-footprint"].value).toBe("48 KB");
    expect(byKey["table-footprint"].basis).toBe("estimated");
    expect(byKey["largest-table"].value).toBe("public.orders");
    expect(byKey["largest-table"].basis).toBe("estimated");
    expect(byKey.statistics.basis).toBe("measured");
  });

  it("counts only known table sizes and calls attention to partial metadata", () => {
    const indicators = buildHealthIndicators(
      snapshot({
        tableSizes: {
          status: "available",
          value: {
            items: [table(), table({ name: "orders", sizeBytes: { status: "unavailable" } })],
            limit: 12,
            truncated: false,
          },
        },
      }),
      NOW,
    );
    const footprint = indicators.find((entry) => entry.key === "table-footprint");

    expect(footprint?.value).toBe("16 KB");
    expect(footprint?.tone).toBe("attention");
    expect(footprint?.detail).toContain("1 of 2 sampled tables");
  });

  it("marks every unsupported metric unavailable and never zero", () => {
    const indicators = buildHealthIndicators(
      snapshot({
        latencyMs: { status: "unavailable" },
        databaseSizeBytes: { status: "unavailable" },
        tableSizes: { status: "unavailable" },
      }),
      NOW,
    );

    expect(indicators).toHaveLength(5);
    expect(indicators.every((entry) => entry.tone === "unavailable")).toBe(true);
    expect(indicators.every((entry) => entry.value === "Unavailable")).toBe(true);
    expect(indicators.every((entry) => entry.basis === "unavailable")).toBe(true);
  });

  it("reports stale statistics without inventing a timestamp", () => {
    const staleStatistics = buildHealthIndicators(
      snapshot({
        tableSizes: {
          status: "available",
          value: {
            items: [
              table({
                metadataRefreshedAt: {
                  status: "available",
                  value: new Date(NOW - 10 * 24 * 60 * 60_000).toISOString(),
                },
              }),
            ],
            limit: 12,
            truncated: false,
          },
        },
      }),
      NOW,
    );
    expect(staleStatistics.find((entry) => entry.key === "statistics")?.tone).toBe("attention");

    const unavailableStatistics = buildHealthIndicators(
      snapshot({
        tableSizes: {
          status: "available",
          value: {
            items: [table({ metadataRefreshedAt: { status: "unavailable" } })],
            limit: 12,
            truncated: false,
          },
        },
      }),
      NOW,
    );
    expect(unavailableStatistics.find((entry) => entry.key === "statistics")).toMatchObject({
      value: "Unavailable",
      tone: "unavailable",
      basis: "unavailable",
    });
  });
});

describe("database health sample freshness", () => {
  it("marks samples stale after the manual-refresh window", () => {
    expect(isHealthSampleStale(NOW - 60_000, NOW)).toBe(false);
    expect(isHealthSampleStale(NOW - HEALTH_SAMPLE_STALE_AFTER_MS - 1, NOW)).toBe(true);
    expect(isHealthSampleStale(0, NOW)).toBe(true);
  });

  it("detects a selected database that differs from the sampled active database", () => {
    expect(healthScopeMismatch(snapshot(), "reporting")).toBe(true);
    expect(healthScopeMismatch(snapshot(), "app")).toBe(false);
    expect(healthScopeMismatch(snapshot({ database: null }), "reporting")).toBe(false);
  });
});
