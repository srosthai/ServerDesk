import type {
  DatabaseRowCell,
  DatabaseRowColumn,
  DatabaseRowColumnKind,
  DatabaseRowExactCount,
  DatabaseRowFilter,
  DatabaseRowFilterOperator,
  DatabaseRowFilterValue,
  DatabaseRowSort,
} from "@/types/contracts";

/** Page sizes offered by the data grid. The backend accepts 1 through 200. */
export const DATA_GRID_PAGE_SIZES = [25, 50, 100, 200] as const;
export const DEFAULT_DATA_GRID_LIMIT = 100;
export const MAX_ROW_OFFSET = 1_000_000;
export const MAX_FILTERS = 16;
export const MAX_SORT_COLUMNS = 8;
export const MIN_COLUMN_WIDTH = 72;
export const MAX_COLUMN_WIDTH = 640;
export const CELL_PREVIEW_CHARACTERS = 120;
/** Mirrors the backend's hard per-cell bound. */
export const SERVER_CELL_CHARACTER_LIMIT = 4_096;

export type DatabaseDataGridState = {
  limit: number;
  offset: number;
  sort: DatabaseRowSort[];
  filters: DatabaseRowFilter[];
  search: string;
  hiddenColumns: string[];
  columnWidths: Record<string, number>;
  includeExactCount: boolean;
};

export function createDataGridState(): DatabaseDataGridState {
  return {
    limit: DEFAULT_DATA_GRID_LIMIT,
    offset: 0,
    sort: [],
    filters: [],
    search: "",
    hiddenColumns: [],
    columnWidths: {},
    includeExactCount: false,
  };
}

/** Stable per-table key inside one connection workspace tab. */
export function dataGridTableKey(table: {
  database?: string | null;
  schema?: string | null;
  table: string;
}): string {
  return JSON.stringify([table.database ?? null, table.schema ?? null, table.table]);
}

export const DEFAULT_DATA_GRID_STATE: DatabaseDataGridState = createDataGridState();

export type CellPresentationKind =
  "null" | "empty" | "boolean" | "number" | "temporal" | "json" | "binary" | "uuid" | "text";

export type CellPresentation = {
  kind: CellPresentationKind;
  text: string;
  title: string;
  numeric: boolean;
  canView: boolean;
  truncated: boolean;
};

function collapseWhitespace(value: string): string {
  const collapsed = value.replace(/\s+/g, " ").trim();
  return collapsed.length > 0 ? collapsed : "⟨whitespace⟩";
}

function previewText(value: string, limit = CELL_PREVIEW_CHARACTERS): string {
  if (value.length <= limit) {
    return collapseWhitespace(value);
  }
  return `${value.slice(0, limit).replace(/\s+/g, " ").trimEnd()}…`;
}

export function base64ByteLength(value: string): number {
  const normalized = value.replace(/\s+/g, "");
  if (normalized.length === 0) {
    return 0;
  }
  const padding = normalized.endsWith("==") ? 2 : normalized.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((normalized.length * 3) / 4) - padding);
}

export function formatByteCount(bytes: number): string {
  if (bytes < 1_024) {
    return `${bytes} B`;
  }
  if (bytes < 1_024 * 1_024) {
    return `${(bytes / 1_024).toFixed(1)} KiB`;
  }
  return `${(bytes / (1_024 * 1_024)).toFixed(1)} MiB`;
}

export function formatEstimatedCount(value: number): string {
  return value.toLocaleString("en-US");
}

/**
 * Builds the view-model for one cell. Nothing here parses JSON or decodes
 * binary data; previews stay O(preview length) so a page of large values never
 * blocks the grid. Full payloads are built only when the viewer opens.
 */
export function cellPresentation(
  cell: DatabaseRowCell | undefined,
  column: DatabaseRowColumn,
): CellPresentation {
  const value = cell?.value ?? null;
  const truncated = cell?.truncated ?? false;

  if (value === null) {
    return {
      kind: "null",
      text: "NULL",
      title: "SQL NULL — not an empty string",
      numeric: false,
      canView: false,
      truncated: false,
    };
  }

  if (value === "") {
    return {
      kind: "empty",
      text: '"" empty',
      title: 'Empty string ("") — not NULL',
      numeric: false,
      canView: false,
      truncated: false,
    };
  }

  const base = {
    numeric: ["integer", "float", "decimal"].includes(column.kind),
    truncated,
  };
  const typeTitle = `${column.dataType}${column.nullable ? " · nullable" : ""}`;

  switch (column.kind) {
    case "boolean": {
      const normalized = value.toLowerCase();
      return {
        ...base,
        kind: "boolean",
        text:
          normalized === "true" || normalized === "1"
            ? "true"
            : normalized === "false" || normalized === "0"
              ? "false"
              : value,
        title: `${value} · ${typeTitle}`,
        canView: false,
      };
    }
    case "integer":
    case "float":
    case "decimal":
      return {
        ...base,
        kind: "number",
        text: value,
        title: `${value} · ${typeTitle}`,
        canView: truncated,
      };
    case "temporal":
      return {
        ...base,
        kind: "temporal",
        text: value,
        title: `${value} · ${typeTitle}`,
        canView: truncated,
      };
    case "uuid":
      return {
        ...base,
        kind: "uuid",
        text: value,
        title: `${value} · ${typeTitle}`,
        canView: truncated,
      };
    case "json":
      return {
        ...base,
        kind: "json",
        text: previewText(value),
        title: `${column.dataType} value · open the viewer for the full payload`,
        canView: true,
      };
    case "binary": {
      const bytes = base64ByteLength(value);
      return {
        ...base,
        kind: "binary",
        text: `Binary · ${formatByteCount(bytes)}`,
        title: `${formatByteCount(bytes)} of binary data · open the hex viewer`,
        canView: true,
      };
    }
    default:
      return {
        ...base,
        kind: "text",
        text: previewText(value),
        title:
          value.length > CELL_PREVIEW_CHARACTERS || truncated
            ? "Long value · open the viewer for the full text"
            : `${value.slice(0, 300)} · ${typeTitle}`,
        canView: truncated || value.length > CELL_PREVIEW_CHARACTERS || /[\n\r\t]/.test(value),
      };
  }
}

export type CellViewerFormat = "json" | "binary" | "text" | "null" | "empty";

export type CellViewerContent = {
  format: CellViewerFormat;
  content: string;
  note?: string;
  jsonValid?: boolean;
};

function decodeBase64(value: string): Uint8Array | null {
  const normalized = value.replace(/\s+/g, "");
  if (normalized.length === 0) {
    return new Uint8Array();
  }
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(normalized) || normalized.length % 4 !== 0) {
    return null;
  }
  try {
    const binary = globalThis.atob(normalized);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  } catch {
    return null;
  }
}

export function hexDump(bytes: Uint8Array): string {
  if (bytes.length === 0) {
    return "(empty binary value)";
  }
  const lines: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += 16) {
    const chunk = bytes.slice(offset, offset + 16);
    const hex = Array.from(chunk, (byte) => byte.toString(16).padStart(2, "0")).join(" ");
    const ascii = Array.from(chunk, (byte) =>
      byte >= 32 && byte <= 126 ? String.fromCharCode(byte) : ".",
    ).join("");
    lines.push(
      `${offset.toString(16).padStart(8, "0")}  ${hex.padEnd(47, " ")}  |${ascii.padEnd(16, " ")}|`,
    );
  }
  return lines.join("\n");
}

/**
 * Builds the full value shown by the safe viewer. JSON is validated and
 * pretty-printed; binary base64 is decoded into a hex dump; text is shown
 * verbatim. A backend-truncated value always carries an explicit note.
 */
export function buildCellViewer(
  cell: DatabaseRowCell,
  column: DatabaseRowColumn,
): CellViewerContent {
  const truncationNote = cell.truncated
    ? `The server returned only the first ${SERVER_CELL_CHARACTER_LIMIT.toLocaleString("en-US")} characters of this value. The remainder was not read by this page request.`
    : undefined;

  if (cell.value === null) {
    return { format: "null", content: "NULL", note: "This cell is SQL NULL." };
  }
  if (cell.value === "") {
    return {
      format: "empty",
      content: '""',
      note: "This cell is an empty string, not NULL.",
    };
  }

  if (column.kind === "json") {
    try {
      const parsed: unknown = JSON.parse(cell.value);
      return {
        format: "json",
        content: JSON.stringify(parsed, null, 2),
        jsonValid: true,
        note: truncationNote,
      };
    } catch {
      return {
        format: "json",
        content: cell.value,
        jsonValid: false,
        note: truncationNote ?? "This value is not valid JSON. The raw text is shown unchanged.",
      };
    }
  }

  if (column.kind === "binary") {
    const bytes = decodeBase64(cell.value);
    if (!bytes) {
      return {
        format: "binary",
        content: cell.value,
        note:
          truncationNote ??
          "This binary value was not valid base64, so the raw text is shown instead of a hex dump.",
      };
    }
    return {
      format: "binary",
      content: hexDump(bytes),
      note:
        truncationNote ??
        `${formatByteCount(bytes.length)} decoded from base64. Offsets are in hexadecimal.`,
    };
  }

  return { format: "text", content: cell.value, note: truncationNote };
}

const FILTER_OPERATOR_LABELS: Record<DatabaseRowFilterOperator, string> = {
  equals: "=",
  not_equals: "≠",
  contains: "contains",
  starts_with: "starts with",
  ends_with: "ends with",
  greater_than: ">",
  greater_than_or_equal: "≥",
  less_than: "<",
  less_than_or_equal: "≤",
  in: "in",
  is_null: "is NULL",
  is_not_null: "is not NULL",
};

export function filterOperatorLabel(operator: DatabaseRowFilterOperator): string {
  return FILTER_OPERATOR_LABELS[operator];
}

const NULL_FILTERS: DatabaseRowFilterOperator[] = ["is_null", "is_not_null"];
const EQUALITY_FILTERS: DatabaseRowFilterOperator[] = [
  "equals",
  "not_equals",
  "is_null",
  "is_not_null",
];
const ORDERED_FILTERS: DatabaseRowFilterOperator[] = [
  "equals",
  "not_equals",
  "greater_than",
  "greater_than_or_equal",
  "less_than",
  "less_than_or_equal",
  "in",
  "is_null",
  "is_not_null",
];

/** Operators the backend can safely apply to one column kind. */
export function filterOperatorsFor(kind: DatabaseRowColumnKind): DatabaseRowFilterOperator[] {
  switch (kind) {
    case "text":
      return [
        "equals",
        "not_equals",
        "contains",
        "starts_with",
        "ends_with",
        "in",
        "is_null",
        "is_not_null",
      ];
    case "integer":
    case "float":
    case "decimal":
    case "temporal":
      return ORDERED_FILTERS;
    case "uuid":
      return ["equals", "not_equals", "in", "is_null", "is_not_null"];
    case "boolean":
      return EQUALITY_FILTERS;
    default:
      return NULL_FILTERS;
  }
}

export type FilterBuildResult = {
  filter: DatabaseRowFilter | null;
  error: string | null;
};

type ParsedFilterValue = {
  value: DatabaseRowFilterValue | null;
  error: string | null;
};

function parseFilterScalar(column: DatabaseRowColumn, raw: string): ParsedFilterValue {
  switch (column.kind) {
    case "integer": {
      const trimmed = raw.trim();
      if (!/^[+-]?\d+$/.test(trimmed)) {
        return { value: null, error: "Enter a whole number." };
      }
      const value = Number(trimmed);
      if (!Number.isSafeInteger(value)) {
        return { value: null, error: "This number is outside the safely representable range." };
      }
      return { value: { kind: "integer", value }, error: null };
    }
    case "float": {
      const value = Number(raw.trim());
      if (raw.trim().length === 0 || !Number.isFinite(value)) {
        return { value: null, error: "Enter a finite number." };
      }
      return { value: { kind: "float", value }, error: null };
    }
    case "decimal":
      if (raw.trim().length === 0) {
        return { value: null, error: "Enter a numeric value." };
      }
      // Decimal values stay text so precision is not lost through a JS number.
      return { value: { kind: "text", value: raw.trim() }, error: null };
    case "boolean": {
      const normalized = raw.trim().toLowerCase();
      if (normalized === "true" || normalized === "1") {
        return { value: { kind: "boolean", value: true }, error: null };
      }
      if (normalized === "false" || normalized === "0") {
        return { value: { kind: "boolean", value: false }, error: null };
      }
      return { value: null, error: "Choose true or false." };
    }
    default:
      return { value: { kind: "text", value: raw }, error: null };
  }
}

/** Converts one filter-builder row into the typed backend contract. */
export function buildRowFilter(
  column: DatabaseRowColumn,
  operator: DatabaseRowFilterOperator,
  rawValue: string,
): FilterBuildResult {
  const allowed = filterOperatorsFor(column.kind);
  if (!allowed.includes(operator)) {
    return { filter: null, error: `${column.dataType} does not support this operator.` };
  }

  if (operator === "is_null" || operator === "is_not_null") {
    return { filter: { column: column.name, operator }, error: null };
  }

  if (operator === "in") {
    const pieces = rawValue
      .split(",")
      .map((piece) => piece.trim())
      .filter((piece) => piece.length > 0);
    if (pieces.length === 0) {
      return { filter: null, error: "Enter at least one comma-separated value." };
    }
    if (pieces.length > 50) {
      return { filter: null, error: "An in filter accepts at most 50 values." };
    }
    const values: DatabaseRowFilterValue[] = [];
    for (const piece of pieces) {
      const parsed = parseFilterScalar(column, piece);
      if (parsed.error !== null || parsed.value === null) {
        return { filter: null, error: parsed.error ?? "The filter value is not valid." };
      }
      values.push(parsed.value);
    }
    return { filter: { column: column.name, operator, values }, error: null };
  }

  const parsed = parseFilterScalar(column, rawValue);
  if (parsed.error !== null || parsed.value === null) {
    return { filter: null, error: parsed.error ?? "The filter value is not valid." };
  }
  return { filter: { column: column.name, operator, value: parsed.value }, error: null };
}

function formatFilterValue(value: DatabaseRowFilterValue): string {
  switch (value.kind) {
    case "text":
      return `"${value.value}"`;
    case "boolean":
      return value.value ? "true" : "false";
    default:
      return String(value.value);
  }
}

export function filterSummary(filter: DatabaseRowFilter): string {
  if (filter.operator === "is_null" || filter.operator === "is_not_null") {
    return `${filter.column} ${filterOperatorLabel(filter.operator)}`;
  }
  if (filter.operator === "in") {
    return `${filter.column} in (${(filter.values ?? []).map(formatFilterValue).join(", ")})`;
  }
  return `${filter.column} ${filterOperatorLabel(filter.operator)} ${
    filter.value ? formatFilterValue(filter.value) : ""
  }`.trim();
}

/** Raw text used to rebuild the filter builder from an applied filter. */
export function filterDraftValue(filter: DatabaseRowFilter): string {
  if (filter.operator === "is_null" || filter.operator === "is_not_null") {
    return "";
  }
  if (filter.operator === "in") {
    return (filter.values ?? []).map((value) => String(value.value)).join(", ");
  }
  return filter.value ? String(filter.value.value) : "";
}

/**
 * Cycles one column through ascending, descending, and unsorted. With
 * `additive`, other sort columns are kept and the backend's eight-column bound
 * is enforced.
 */
export function toggleSort(
  current: DatabaseRowSort[],
  column: string,
  additive = false,
): DatabaseRowSort[] {
  const index = current.findIndex((entry) => entry.column === column);
  const existing = index >= 0 ? current[index] : null;

  if (additive) {
    if (!existing) {
      if (current.length >= MAX_SORT_COLUMNS) {
        return current;
      }
      return [...current, { column, direction: "asc" }];
    }
    if (existing.direction === "asc") {
      return current.map((entry, entryIndex) =>
        entryIndex === index ? { ...entry, direction: "desc" as const } : entry,
      );
    }
    return current.filter((_, entryIndex) => entryIndex !== index);
  }

  if (!existing) {
    return [{ column, direction: "asc" }];
  }
  if (existing.direction === "asc") {
    return [{ column, direction: "desc" }];
  }
  return [];
}

export function sortDirectionFor(
  sort: DatabaseRowSort[],
  column: string,
): DatabaseRowSort["direction"] | null {
  return sort.find((entry) => entry.column === column)?.direction ?? null;
}

export function columnWidth(column: DatabaseRowColumn, widths: Record<string, number>): number {
  const configured = widths[column.name];
  if (typeof configured === "number" && Number.isFinite(configured)) {
    return Math.min(MAX_COLUMN_WIDTH, Math.max(MIN_COLUMN_WIDTH, Math.round(configured)));
  }
  return defaultColumnWidth(column.kind);
}

export function defaultColumnWidth(kind: DatabaseRowColumnKind): number {
  switch (kind) {
    case "integer":
      return 96;
    case "float":
    case "decimal":
      return 120;
    case "boolean":
      return 96;
    case "temporal":
      return 176;
    case "json":
      return 200;
    case "binary":
      return 156;
    case "uuid":
      return 172;
    default:
      return 200;
  }
}

/**
 * Converts the hidden-column set into the backend's `visibleColumns` list.
 * `undefined` keeps every column, which is the first-load shape. At least one
 * column always stays visible even if stored preferences are stale.
 */
export function visibleColumnNames(
  columns: DatabaseRowColumn[],
  hiddenColumns: string[],
): string[] | undefined {
  if (columns.length === 0) {
    return undefined;
  }
  const hidden = new Set(hiddenColumns);
  const visible = columns.filter((column) => !hidden.has(column.name)).map((column) => column.name);
  if (visible.length === 0) {
    return [columns[0].name];
  }
  return visible.length === columns.length ? undefined : visible;
}

export function nextPageOffset(offset: number, limit: number): number {
  return Math.min(MAX_ROW_OFFSET, offset + limit);
}

export function previousPageOffset(offset: number, limit: number): number {
  return Math.max(0, offset - limit);
}

export function pageWindowLabel({
  offset,
  returned,
  hasMore,
  exactRows,
}: {
  offset: number;
  returned: number;
  hasMore: boolean;
  exactRows?: DatabaseRowExactCount | null;
}): string {
  const start = returned > 0 ? offset + 1 : offset;
  const end = offset + returned;
  const range =
    returned > 0
      ? `Rows ${start.toLocaleString("en-US")}–${end.toLocaleString("en-US")}`
      : "No rows on this page";
  const total = exactRows
    ? ` of ${exactRows.value.toLocaleString("en-US")}${exactRows.filtered ? " filtered" : ""}`
    : hasMore
      ? " · more rows available"
      : "";
  return `${range}${total}`;
}

export function formatSampleTime(timestamp: number | undefined): string | null {
  if (!timestamp) {
    return null;
  }
  return new Date(timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
