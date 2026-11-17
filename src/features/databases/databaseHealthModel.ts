import { formatBytes } from "@/features/databases/databaseStructureModel";
import { formatLatency } from "@/features/databases/databaseProfileForm";
import type {
  DatabaseHealthSnapshot,
  DatabaseMetadataValue,
  DatabaseTableSize,
  DatabaseVersionInfo,
} from "@/types/contracts";

export const DEFAULT_HEALTH_TABLE_LIMIT = 12;

/** Manual samples are considered stale after five minutes. */
export const HEALTH_SAMPLE_STALE_AFTER_MS = 5 * 60_000;

/** Table statistics older than a week are called out in the health summary. */
export const HEALTH_STATISTICS_STALE_AFTER_MS = 7 * 24 * 60 * 60_000;

export type DatabaseHealthBasis = "measured" | "estimated" | "unavailable";
export type DatabaseHealthTone = "healthy" | "attention" | "unavailable";

export type DatabaseHealthIndicator = {
  key: "latency" | "storage" | "table-footprint" | "statistics" | "largest-table";
  label: string;
  value: string;
  tone: DatabaseHealthTone;
  basis: DatabaseHealthBasis;
  detail: string;
};

export const HEALTH_BASIS_LABELS: Record<DatabaseHealthBasis, string> = {
  measured: "Measured",
  estimated: "Estimated",
  unavailable: "Unavailable",
};

const HEALTH_TONE_LABELS: Record<DatabaseHealthTone, string> = {
  healthy: "Healthy",
  attention: "Attention",
  unavailable: "Unavailable",
};

export function healthToneLabel(tone: DatabaseHealthTone): string {
  return HEALTH_TONE_LABELS[tone];
}

export function formatHealthVersion(version: DatabaseVersionInfo): string {
  return version.version || version.raw;
}

export function formatHealthSampleTime(sampledAt: number): string {
  return new Date(sampledAt).toLocaleString();
}

export function formatHealthSampleAge(sampledAt: number, now = Date.now()): string {
  const elapsed = Math.max(0, now - sampledAt);
  if (elapsed < 60_000) {
    return "just now";
  }
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 60) {
    return `${minutes} min ago`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours} h ago`;
  }
  const days = Math.floor(hours / 24);
  return `${days} d ago`;
}

export function isHealthSampleStale(
  sampledAt: number,
  now = Date.now(),
  maxAgeMs = HEALTH_SAMPLE_STALE_AFTER_MS,
): boolean {
  if (!Number.isFinite(sampledAt) || sampledAt <= 0) {
    return true;
  }
  return now - sampledAt > maxAgeMs;
}

export function healthTables(snapshot: DatabaseHealthSnapshot): DatabaseTableSize[] | null {
  return snapshot.tableSizes.status === "available" ? snapshot.tableSizes.value.items : null;
}

export function formatHealthTableName(table: DatabaseTableSize): string {
  const schema = table.schema && table.schema !== table.database ? table.schema : null;
  return schema ? `${schema}.${table.name}` : table.name;
}

export function formatHealthTableSize(table: DatabaseTableSize): string {
  return formatMetadataNumber(table.sizeBytes, (bytes) => formatBytes(bytes));
}

export function formatHealthRowEstimate(table: DatabaseTableSize): string {
  return formatMetadataNumber(
    table.estimatedRows,
    (rows) => `~${rows.toLocaleString()} est. rows`,
    "Rows unavailable",
  );
}

export function formatHealthStatisticsTime(
  value: DatabaseMetadataValue<string>,
  now = Date.now(),
): string {
  if (value.status === "unavailable") {
    return "Unavailable";
  }
  const parsed = Date.parse(value.value);
  if (Number.isNaN(parsed)) {
    return "Unavailable";
  }
  return formatHealthSampleAge(parsed, now);
}

function formatMetadataNumber<T>(
  value: DatabaseMetadataValue<T>,
  format: (value: T) => string,
  unavailable = "Unavailable",
): string {
  return value.status === "available" ? format(value.value) : unavailable;
}

function knownTableSizeTotal(tables: DatabaseTableSize[]): {
  bytes: number;
  known: number;
} {
  let bytes = 0;
  let known = 0;
  for (const table of tables) {
    if (table.sizeBytes.status === "available") {
      bytes += table.sizeBytes.value;
      known += 1;
    }
  }
  return { bytes, known };
}

function latestStatisticsRefresh(
  tables: DatabaseTableSize[],
): { value: string; at: number } | null {
  let latest: { value: string; at: number } | null = null;
  for (const table of tables) {
    if (table.metadataRefreshedAt.status !== "available") {
      continue;
    }
    const at = Date.parse(table.metadataRefreshedAt.value);
    if (Number.isNaN(at)) {
      continue;
    }
    if (!latest || at > latest.at) {
      latest = { value: table.metadataRefreshedAt.value, at };
    }
  }
  return latest;
}

export function buildHealthIndicators(
  snapshot: DatabaseHealthSnapshot,
  now = Date.now(),
): DatabaseHealthIndicator[] {
  const indicators: DatabaseHealthIndicator[] = [];
  const tables = healthTables(snapshot);

  if (snapshot.latencyMs.status === "available") {
    const latency = snapshot.latencyMs.value;
    indicators.push({
      key: "latency",
      label: "Connection latency",
      value: formatLatency(latency),
      tone: latency <= 150 ? "healthy" : "attention",
      basis: "measured",
      detail:
        latency <= 150
          ? "Round-trip ping measured for this sample."
          : "Round-trip ping is elevated for this sample. Network distance, SSH, and server load all contribute.",
    });
  } else {
    indicators.push({
      key: "latency",
      label: "Connection latency",
      value: "Unavailable",
      tone: "unavailable",
      basis: "unavailable",
      detail: "This account or engine did not allow the latency probe.",
    });
  }

  if (snapshot.databaseSizeBytes.status === "available") {
    indicators.push({
      key: "storage",
      label: "Database size",
      value: formatBytes(snapshot.databaseSizeBytes.value),
      tone: "healthy",
      basis: "estimated",
      detail: "Estimated from engine catalog metadata. Includes table and index allocation.",
    });
  } else {
    indicators.push({
      key: "storage",
      label: "Database size",
      value: "Unavailable",
      tone: "unavailable",
      basis: "unavailable",
      detail: "This engine or account did not report a database size. It is not shown as zero.",
    });
  }

  if (tables === null) {
    indicators.push({
      key: "table-footprint",
      label: "Table footprint",
      value: "Unavailable",
      tone: "unavailable",
      basis: "unavailable",
      detail: "Catalog table sizes could not be read with this account.",
    });
    indicators.push({
      key: "largest-table",
      label: "Largest table",
      value: "Unavailable",
      tone: "unavailable",
      basis: "unavailable",
      detail: "No table sizes are available for this sample.",
    });
    indicators.push({
      key: "statistics",
      label: "Statistics freshness",
      value: "Unavailable",
      tone: "unavailable",
      basis: "unavailable",
      detail: "Statistics timestamps require table catalog metadata.",
    });
  } else {
    const total = knownTableSizeTotal(tables);
    indicators.push({
      key: "table-footprint",
      label: "Table footprint",
      value: total.known === 0 ? "Unavailable" : formatBytes(total.bytes),
      tone:
        total.known === 0 ? "unavailable" : total.known < tables.length ? "attention" : "healthy",
      basis: total.known === 0 ? "unavailable" : "estimated",
      detail:
        total.known === 0
          ? "No sampled table exposed an allocated size."
          : `${total.known} of ${tables.length} sampled tables reported an allocated size. Table sizes are catalog estimates, not live measurements.`,
    });

    const largest = [...tables]
      .filter((table) => table.sizeBytes.status === "available")
      .sort((left, right) => {
        const leftSize = left.sizeBytes.status === "available" ? left.sizeBytes.value : 0;
        const rightSize = right.sizeBytes.status === "available" ? right.sizeBytes.value : 0;
        return rightSize - leftSize;
      })[0];

    if (largest) {
      indicators.push({
        key: "largest-table",
        label: "Largest table",
        value: formatHealthTableName(largest),
        tone: "healthy",
        basis: "estimated",
        detail: `${formatHealthTableSize(largest)} · ${formatHealthRowEstimate(largest)}.`,
      });
    } else {
      indicators.push({
        key: "largest-table",
        label: "Largest table",
        value: "Unavailable",
        tone: "unavailable",
        basis: "unavailable",
        detail: "No sampled table exposed an allocated size.",
      });
    }

    const latest = latestStatisticsRefresh(tables);
    if (latest) {
      const age = now - latest.at;
      indicators.push({
        key: "statistics",
        label: "Statistics freshness",
        value: formatHealthSampleAge(latest.at, now),
        tone: age > HEALTH_STATISTICS_STALE_AFTER_MS ? "attention" : "healthy",
        basis: "measured",
        detail:
          age > HEALTH_STATISTICS_STALE_AFTER_MS
            ? "The newest engine statistics among sampled tables are more than a week old. Estimates may drift from live data."
            : "Newest engine statistics refresh among sampled tables. Row counts remain estimates.",
      });
    } else {
      indicators.push({
        key: "statistics",
        label: "Statistics freshness",
        value: "Unavailable",
        tone: "unavailable",
        basis: "unavailable",
        detail: "The engine or account did not expose statistics refresh times. No age is guessed.",
      });
    }
  }

  return indicators;
}

export function healthScopeMismatch(
  snapshot: DatabaseHealthSnapshot,
  selectedDatabase: string | null,
): boolean {
  return Boolean(selectedDatabase && snapshot.database && selectedDatabase !== snapshot.database);
}
