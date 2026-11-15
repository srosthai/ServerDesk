import type {
  DatabaseEngine,
  DatabaseQueryHistoryFilter,
  DatabaseQueryHistoryPolicy,
  DatabaseQueryHistoryResult,
  DatabaseSavedQuery,
} from "@/types/contracts";

/** Mirrors the backend policy so the UI stays useful before the command resolves. */
export const DEFAULT_DATABASE_HISTORY_POLICY: DatabaseQueryHistoryPolicy = {
  maxEntries: 1_000,
  maxSqlChars: 8_192,
  errorSummaryMaxChars: 512,
  snippetNameMaxChars: 120,
  snippetSqlMaxChars: 65_536,
  retention:
    "ServerDesk keeps the newest 1,000 queries and automatically removes older entries. Stored SQL is limited to 8,192 characters and is sanitized before it is saved.",
};

export type DatabaseHistoryResultFilter = DatabaseQueryHistoryResult | "all";

export type DatabaseHistoryFilterState = {
  connectionId: string;
  result: DatabaseHistoryResultFilter;
  dateFrom: string;
  dateTo: string;
};

export const EMPTY_DATABASE_HISTORY_FILTER: DatabaseHistoryFilterState = {
  connectionId: "all",
  result: "all",
  dateFrom: "",
  dateTo: "",
};

export const DATABASE_QUERY_RESULT_LABELS: Record<DatabaseQueryHistoryResult, string> = {
  success: "Success",
  failure: "Failure",
  cancelled: "Cancelled",
};

export const DATABASE_QUERY_RESULT_BADGES: Record<
  DatabaseQueryHistoryResult,
  "success" | "danger" | "warning"
> = {
  success: "success",
  failure: "danger",
  cancelled: "warning",
};

/** Converts a date input value to the start of that local day in Unix seconds. */
export function startOfDaySeconds(value: string): number | null {
  if (!value) {
    return null;
  }
  const date = new Date(`${value}T00:00:00`);
  return Number.isNaN(date.getTime()) ? null : Math.floor(date.getTime() / 1_000);
}

/** Converts a date input value to the end of that local day in Unix seconds. */
export function endOfDaySeconds(value: string): number | null {
  if (!value) {
    return null;
  }
  const date = new Date(`${value}T23:59:59.999`);
  return Number.isNaN(date.getTime()) ? null : Math.floor(date.getTime() / 1_000);
}

/**
 * Builds the backend filter. An empty connection id or `"all"` means every
 * saved connection on the active server.
 */
export function buildDatabaseHistoryFilter(
  state: DatabaseHistoryFilterState,
  options: { serverId: string; limit: number },
): DatabaseQueryHistoryFilter {
  return {
    serverId: options.serverId,
    connectionId: state.connectionId === "all" ? null : state.connectionId,
    result: state.result === "all" ? null : state.result,
    startedAfter: startOfDaySeconds(state.dateFrom),
    startedBefore: endOfDaySeconds(state.dateTo),
    limit: options.limit,
  };
}

export function historyFiltersActive(state: DatabaseHistoryFilterState): boolean {
  return (
    state.connectionId !== "all" ||
    state.result !== "all" ||
    state.dateFrom.length > 0 ||
    state.dateTo.length > 0
  );
}

export function formatHistoryTimestamp(seconds: number): string {
  const date = new Date(seconds * 1_000);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

export function historyTimestampValue(seconds: number): string | undefined {
  const date = new Date(seconds * 1_000);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

export function formatHistoryDuration(durationMs: number): string {
  if (durationMs < 1_000) {
    return `${Math.max(0, Math.round(durationMs)).toString()} ms`;
  }
  return `${(durationMs / 1_000).toFixed(1)} s`;
}

/** True when a snippet can be inserted into a tab for the active engine. */
export function snippetMatchesEngine(snippet: DatabaseSavedQuery, engine: DatabaseEngine): boolean {
  return snippet.engine === engine;
}

export function snippetEngineLabel(engine: DatabaseEngine): string {
  return engine === "mysql" ? "MySQL" : "PostgreSQL";
}

export function snippetCompatibilityReason(
  snippet: DatabaseSavedQuery,
  engine: DatabaseEngine,
): string | null {
  if (snippetMatchesEngine(snippet, engine)) {
    return null;
  }
  return `Saved for ${snippetEngineLabel(snippet.engine)}. Open a ${snippetEngineLabel(
    snippet.engine,
  )} connection to reuse this snippet.`;
}

/** Case-insensitive search across a snippet's name and SQL body. */
export function filterDatabaseSnippets(
  snippets: readonly DatabaseSavedQuery[],
  query: string,
): DatabaseSavedQuery[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) {
    return [...snippets];
  }
  return snippets.filter(
    (snippet) =>
      snippet.name.toLowerCase().includes(normalized) ||
      snippet.sqlText.toLowerCase().includes(normalized),
  );
}

/** A one-line preview that never renders the entire stored SQL body. */
export function snippetPreview(sqlText: string, maxChars = 140): string {
  const singleLine = sqlText.replace(/\s+/g, " ").trim();
  if (singleLine.length <= maxChars) {
    return singleLine;
  }
  return `${singleLine.slice(0, Math.max(0, maxChars - 1))}…`;
}
