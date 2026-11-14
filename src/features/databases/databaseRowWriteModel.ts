import type {
  DatabaseRowColumn,
  DatabaseRowIdentityValue,
  DatabaseRowReadResult,
  DatabaseRowRecord,
  DatabaseRowValueMode,
  DatabaseRowWriteValue,
} from "@/types/contracts";

/** One field in the insert or edit dialog. */
export type RowFieldDraft = {
  column: DatabaseRowColumn;
  mode: DatabaseRowValueMode;
  value: string;
  /** Original display value for update conflict guards. */
  original: string | null;
  /** True when the server truncated the original cell. */
  truncated: boolean;
  /** True when the user changed this field. */
  changed: boolean;
};

export type RowUpdatePayload = {
  values: DatabaseRowWriteValue[];
  expected: DatabaseRowIdentityValue[];
};

export function rowEditingBlocked(
  result: DatabaseRowReadResult | undefined,
  sessionReadOnly: boolean,
): string | null {
  if (!result) {
    return "Load table rows before editing.";
  }
  if (sessionReadOnly || result.readOnlyReasons.includes("session_read_only")) {
    return "This connection is read-only.";
  }
  if (!result.editable || !result.identity.stable) {
    return "This table has no primary key or complete unique index, so a row cannot be addressed safely.";
  }
  return null;
}

/** Columns that can be written. Generated columns are never editable. */
export function writableColumns(columns: DatabaseRowColumn[]): DatabaseRowColumn[] {
  return columns.filter((column) => !column.generated);
}

export function identityColumnNames(result: DatabaseRowReadResult): Set<string> {
  return new Set(result.identity.columns);
}

export function identityValues(record: DatabaseRowRecord): DatabaseRowIdentityValue[] {
  return record.identity.map((entry) => ({ column: entry.column, value: entry.value }));
}

export function identityKey(record: DatabaseRowRecord): string {
  return JSON.stringify(record.identity.map((entry) => [entry.column, entry.value]));
}

/** Builds insert fields. Every writable column starts on the DEFAULT mode. */
export function insertFields(columns: DatabaseRowColumn[]): RowFieldDraft[] {
  return writableColumns(columns).map((column) => ({
    column,
    mode: "default",
    value: "",
    original: null,
    truncated: false,
    changed: false,
  }));
}

/**
 * Builds edit fields from the visible cells. Hidden columns are not included,
 * and a server-truncated value is marked so it cannot be edited from an
 * incomplete preview.
 */
export function editFields(
  columns: DatabaseRowColumn[],
  result: DatabaseRowReadResult,
  record: DatabaseRowRecord,
): RowFieldDraft[] {
  const identity = identityColumnNames(result);
  const visible =
    result.visibleColumns.length > 0 ? result.visibleColumns : columns.map((c) => c.name);
  const positions = new Map(visible.map((name, index) => [name, index]));
  const fields: RowFieldDraft[] = [];

  for (const column of columns) {
    if (column.generated || identity.has(column.name)) {
      continue;
    }
    const position = positions.get(column.name);
    if (position === undefined) {
      continue;
    }
    const cell = record.cells[position];
    const original = cell?.value ?? null;
    fields.push({
      column,
      mode: original === null ? "null" : "value",
      value: original ?? "",
      original,
      truncated: cell?.truncated ?? false,
      changed: false,
    });
  }
  return fields;
}

export function updateField(
  fields: RowFieldDraft[],
  index: number,
  patch: Partial<Pick<RowFieldDraft, "mode" | "value">>,
): RowFieldDraft[] {
  return fields.map((field, fieldIndex) => {
    if (fieldIndex !== index) {
      return field;
    }
    const next = { ...field, ...patch };
    const modeChanged = next.mode !== field.mode;
    const valueChanged = next.mode === "value" && next.value !== field.value;
    return {
      ...next,
      changed: field.changed || modeChanged || valueChanged,
    };
  });
}

/** Builds the explicit insert payload, including DEFAULT and NULL modes. */
export function buildInsertValues(fields: RowFieldDraft[]): DatabaseRowWriteValue[] {
  return fields.map((field) => {
    if (field.mode === "default") {
      return { column: field.column.name, mode: "default" };
    }
    if (field.mode === "null") {
      return { column: field.column.name, mode: "null" };
    }
    return { column: field.column.name, mode: "value", value: field.value };
  });
}

/**
 * Builds changed values and their original-value guards. A truncated cell can
 * never be changed because the preview does not contain the full value.
 */
export function buildUpdatePayload(fields: RowFieldDraft[]): RowUpdatePayload | string {
  const changed = fields.filter((field) => field.changed);
  if (changed.length === 0) {
    return "Change at least one field before saving.";
  }
  const truncated = changed.find((field) => field.truncated);
  if (truncated) {
    return `${truncated.column.name} was truncated by the server. Open the full value before editing it.`;
  }

  const values: DatabaseRowWriteValue[] = [];
  const expected: DatabaseRowIdentityValue[] = [];
  for (const field of changed) {
    if (field.mode === "null") {
      values.push({ column: field.column.name, mode: "null" });
    } else {
      values.push({ column: field.column.name, mode: "value", value: field.value });
    }
    expected.push({ column: field.column.name, value: field.original });
  }
  return { values, expected };
}

/** Accepts a value field only when that column is writable and not generated. */
export function columnAcceptsValue(column: DatabaseRowColumn): boolean {
  return !column.generated;
}
