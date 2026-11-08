export type LogLine = {
  number: number;
  text: string;
};

export type FilteredLog = {
  lines: LogLine[];
  totalLines: number;
  matchedLines: number;
  query: string;
};

/**
 * Filters the already retrieved bounded tail on the client. Filtering never
 * triggers another remote read, so large files are not downloaded again.
 */
export function filterLogLines(content: string, query: string): FilteredLog {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (content.length === 0) {
    return { lines: [], totalLines: 0, matchedLines: 0, query: normalizedQuery };
  }

  const text = content.endsWith("\n") ? content.slice(0, -1) : content;
  const allLines = text.split("\n");
  const lines = allLines
    .map((line, index) => ({ number: index + 1, text: line }))
    .filter(
      (line) =>
        normalizedQuery.length === 0 || line.text.toLocaleLowerCase().includes(normalizedQuery),
    );

  return {
    lines,
    totalLines: allLines.length,
    matchedLines: lines.length,
    query: normalizedQuery,
  };
}

export function filteredLogText(filtered: FilteredLog): string {
  return filtered.lines.map((line) => line.text).join("\n");
}

export function formatByteSize(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) {
    return "unknown";
  }
  if (bytes < 1024) {
    return `${bytes.toString()} B`;
  }

  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = units[0];
  for (let index = 1; index < units.length && value >= 1024; index += 1) {
    value /= 1024;
    unit = units[index];
  }

  return `${value.toFixed(value >= 10 ? 0 : 1)} ${unit}`;
}
