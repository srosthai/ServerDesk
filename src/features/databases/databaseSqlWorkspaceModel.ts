import type {
  DatabaseEngine,
  DatabaseQueryExecutionResult,
  DatabaseQueryResultSet,
} from "@/types/contracts";
import type { DatabaseSqlExecutionSource } from "@/stores/databaseWorkspaceStore";

export type SqlTokenKind =
  | "plain"
  | "keyword"
  | "function"
  | "string"
  | "number"
  | "comment"
  | "identifier"
  | "operator"
  | "punctuation";

export type SqlToken = {
  text: string;
  kind: SqlTokenKind;
};

const COMMON_KEYWORDS = new Set(
  `add all alter and any as asc begin between by cascade case cast check column commit constraint create cross current current_date current_time current_timestamp database default delete desc distinct do drop else end escape except exists explain false fetch for foreign from full grant group having if in index inner insert intersect into is join key left like limit natural not null offset on or order outer primary references returning revoke right rollback row rows schema select set table temporary then to transaction trigger true truncate union unique update using values view when where with`.split(
    " ",
  ),
);

const POSTGRESQL_KEYWORDS = new Set(
  `analyze array binary bit boolean char character concurrently conflict cube current_schema current_user deallocate declare deferrable deferred exclude filter generated grouping ilike immutable inherits initially json jsonb language lateral lock materialized nothing nullif over partition placing prepare prior quote recursive refresh reindex replace role sequence serializable session session_user share similar stable start statement strict tablespace template text time timestamp timestamptz treat type unbounded uuid vacuum valid validate verbose work xml`.split(
    " ",
  ),
);

const MYSQL_KEYWORDS = new Set(
  `auto_increment bigint blob boolean byte change charset collate compressed databases day_hour delayed describe discard distinctrow div dual engine enum escaped flush force format fulltext geometry high_priority ignore inplace int integer json longblob longtext lock mediumint mediumtext mod modify no_write_to_binlog optimize option outfile password purge quick range read regexp rename repair repeatable require restrict rlike security show spatial sql_calc_found_rows sql_no_cache starting straight_join terminated tinyint tinytext unlock unsigned use utc_date utc_time utc_timestamp varbinary varchar varying write xor year zerofill`.split(
    " ",
  ),
);

const OPERATOR_CHARACTERS = new Set("+-*/%=<>!|&^~:");
const PUNCTUATION_CHARACTERS = new Set("(),;.[]{}");

function keywordsFor(engine: DatabaseEngine): Set<string> {
  const keywords = new Set(COMMON_KEYWORDS);
  const engineKeywords = engine === "mysql" ? MYSQL_KEYWORDS : POSTGRESQL_KEYWORDS;
  engineKeywords.forEach((keyword) => keywords.add(keyword));
  return keywords;
}

function isWordStart(character: string): boolean {
  return /[A-Za-z_]/.test(character);
}

function isWordCharacter(character: string): boolean {
  return /[A-Za-z0-9_$]/.test(character);
}

function nextNonWhitespace(sql: string, from: number): string | null {
  for (let index = from; index < sql.length; index += 1) {
    if (!/\s/.test(sql[index])) {
      return sql[index];
    }
  }
  return null;
}

/**
 * Tokenizes SQL for display while preserving every character exactly. This is
 * a highlighter, not a parser: it understands comments, quoted strings and
 * identifiers, dollar-quoted PostgreSQL strings, numbers, engine keywords, and
 * function calls so the editor can color MySQL and PostgreSQL dialects.
 */
export function tokenizeSql(sql: string, engine: DatabaseEngine): SqlToken[] {
  const keywords = keywordsFor(engine);
  const tokens: SqlToken[] = [];
  let plain = "";
  let index = 0;

  function push(kind: SqlTokenKind, text: string) {
    if (plain) {
      tokens.push({ text: plain, kind: "plain" });
      plain = "";
    }
    if (text) {
      tokens.push({ text, kind });
    }
  }

  while (index < sql.length) {
    const character = sql[index];
    const next = sql[index + 1];

    if (character === "-" && next === "-") {
      let end = index + 2;
      while (end < sql.length && sql[end] !== "\n") {
        end += 1;
      }
      push("comment", sql.slice(index, end));
      index = end;
      continue;
    }

    if (engine === "mysql" && character === "#") {
      let end = index + 1;
      while (end < sql.length && sql[end] !== "\n") {
        end += 1;
      }
      push("comment", sql.slice(index, end));
      index = end;
      continue;
    }

    if (character === "/" && next === "*") {
      const close = sql.indexOf("*/", index + 2);
      const end = close < 0 ? sql.length : close + 2;
      push("comment", sql.slice(index, end));
      index = end;
      continue;
    }

    if (character === "'") {
      let end = index + 1;
      while (end < sql.length) {
        if (sql[end] === "\\" && engine === "mysql" && end + 1 < sql.length) {
          end += 2;
          continue;
        }
        if (sql[end] === "'" && sql[end + 1] === "'") {
          end += 2;
          continue;
        }
        if (sql[end] === "'") {
          end += 1;
          break;
        }
        end += 1;
      }
      push("string", sql.slice(index, end));
      index = end;
      continue;
    }

    if (character === '"' || (engine === "mysql" && character === "`")) {
      const quote = character;
      let end = index + 1;
      while (end < sql.length) {
        if (sql[end] === quote && sql[end + 1] === quote) {
          end += 2;
          continue;
        }
        if (sql[end] === quote) {
          end += 1;
          break;
        }
        end += 1;
      }
      push("identifier", sql.slice(index, end));
      index = end;
      continue;
    }

    if (engine === "postgresql" && character === "$") {
      const remainder = sql.slice(index);
      const delimiter = remainder.match(/^\$[A-Za-z_][A-Za-z0-9_]*\$|^\$\$/)?.[0];
      if (delimiter) {
        const close = sql.indexOf(delimiter, index + delimiter.length);
        const end = close < 0 ? sql.length : close + delimiter.length;
        push("string", sql.slice(index, end));
        index = end;
        continue;
      }
    }

    if (/[0-9]/.test(character) || (character === "." && /[0-9]/.test(next ?? ""))) {
      let end = index + 1;
      while (end < sql.length && /[0-9A-Fa-f_xX.]/.test(sql[end])) {
        end += 1;
      }
      push("number", sql.slice(index, end));
      index = end;
      continue;
    }

    if (isWordStart(character)) {
      let end = index + 1;
      while (end < sql.length && isWordCharacter(sql[end])) {
        end += 1;
      }
      const word = sql.slice(index, end);
      const lower = word.toLowerCase();
      if (keywords.has(lower)) {
        push("keyword", word);
      } else if (nextNonWhitespace(sql, end) === "(") {
        push("function", word);
      } else {
        push("plain", word);
      }
      index = end;
      continue;
    }

    if (OPERATOR_CHARACTERS.has(character)) {
      push("operator", character);
      index += 1;
      continue;
    }

    if (PUNCTUATION_CHARACTERS.has(character)) {
      push("punctuation", character);
      index += 1;
      continue;
    }

    plain += character;
    index += 1;
  }

  if (plain) {
    tokens.push({ text: plain, kind: "plain" });
  }
  return tokens;
}

type RawStatementRange = {
  segmentStart: number;
  segmentEnd: number;
  start: number;
  end: number;
};

function trimRange(sql: string, start: number, end: number): RawStatementRange | null {
  while (start < end && /\s/.test(sql[start])) {
    start += 1;
  }
  while (end > start && /\s/.test(sql[end - 1])) {
    end -= 1;
  }
  if (start >= end) {
    return null;
  }
  return { segmentStart: start, segmentEnd: end, start, end };
}

/**
 * Finds statement boundaries outside quotes, comments, and PostgreSQL dollar
 * quotes. Returned ranges are trimmed and exclude the semicolon separator.
 */
export function findSqlStatementRanges(
  sql: string,
  engine: DatabaseEngine,
): Array<{ start: number; end: number; segmentStart: number; segmentEnd: number }> {
  const ranges: RawStatementRange[] = [];
  let segmentStart = 0;
  let index = 0;

  while (index < sql.length) {
    const character = sql[index];
    const next = sql[index + 1];

    if (character === "-" && next === "-") {
      index = sql.indexOf("\n", index + 2);
      if (index < 0) {
        index = sql.length;
      }
      continue;
    }

    if (engine === "mysql" && character === "#") {
      index = sql.indexOf("\n", index + 1);
      if (index < 0) {
        index = sql.length;
      }
      continue;
    }

    if (character === "/" && next === "*") {
      const close = sql.indexOf("*/", index + 2);
      index = close < 0 ? sql.length : close + 2;
      continue;
    }

    if (character === "'") {
      index += 1;
      while (index < sql.length) {
        if (sql[index] === "\\" && engine === "mysql" && index + 1 < sql.length) {
          index += 2;
          continue;
        }
        if (sql[index] === "'" && sql[index + 1] === "'") {
          index += 2;
          continue;
        }
        if (sql[index] === "'") {
          index += 1;
          break;
        }
        index += 1;
      }
      continue;
    }

    if (character === '"' || (engine === "mysql" && character === "`")) {
      const quote = character;
      index += 1;
      while (index < sql.length) {
        if (sql[index] === quote && sql[index + 1] === quote) {
          index += 2;
          continue;
        }
        if (sql[index] === quote) {
          index += 1;
          break;
        }
        index += 1;
      }
      continue;
    }

    if (engine === "postgresql" && character === "$") {
      const remainder = sql.slice(index);
      const delimiter = remainder.match(/^\$[A-Za-z_][A-Za-z0-9_]*\$|^\$\$/)?.[0];
      if (delimiter) {
        const close = sql.indexOf(delimiter, index + delimiter.length);
        index = close < 0 ? sql.length : close + delimiter.length;
        continue;
      }
    }

    if (character === ";") {
      const range = trimRange(sql, segmentStart, index);
      if (range) {
        ranges.push({ ...range, segmentStart, segmentEnd: index });
      }
      segmentStart = index + 1;
    }

    index += 1;
  }

  const finalRange = trimRange(sql, segmentStart, sql.length);
  if (finalRange) {
    ranges.push({ ...finalRange, segmentStart, segmentEnd: sql.length });
  }

  return ranges;
}

function selectionIsUsable(start: number, end: number): boolean {
  return Number.isFinite(start) && Number.isFinite(end) && end > start;
}

/**
 * Resolves what the Run button should submit. A selection wins; otherwise the
 * statement at the cursor is used. `all` executes the entire draft.
 */
export function resolveSqlForExecution({
  sql,
  engine,
  selectionStart,
  selectionEnd,
  source = "auto",
}: {
  sql: string;
  engine: DatabaseEngine;
  selectionStart: number;
  selectionEnd: number;
  source?: DatabaseSqlExecutionSource | "auto";
}): {
  sql: string;
  source: DatabaseSqlExecutionSource;
  start: number;
  end: number;
  statementNumber: number;
  statementCount: number;
} {
  if (source === "all") {
    const ranges = findSqlStatementRanges(sql, engine);
    return {
      sql: sql.trim(),
      source: "all",
      start: 0,
      end: sql.length,
      statementNumber: 1,
      statementCount: Math.max(1, ranges.length),
    };
  }

  if (
    source === "selection" ||
    (source === "auto" && selectionIsUsable(selectionStart, selectionEnd))
  ) {
    const start = Math.max(0, Math.min(selectionStart, sql.length));
    const end = Math.max(start, Math.min(selectionEnd, sql.length));
    return {
      sql: sql.slice(start, end).trim(),
      source: "selection",
      start,
      end,
      statementNumber: 1,
      statementCount: 1,
    };
  }

  const ranges = findSqlStatementRanges(sql, engine);
  if (ranges.length === 0) {
    return {
      sql: sql.trim(),
      source: "statement",
      start: 0,
      end: sql.length,
      statementNumber: 1,
      statementCount: 0,
    };
  }

  const cursor = Math.max(0, Math.min(selectionStart, sql.length));
  let index = ranges.findIndex(
    (range) => cursor >= range.segmentStart && cursor <= range.segmentEnd,
  );
  if (index < 0) {
    const nextIndex = ranges.findIndex((range) => range.start > cursor);
    index = nextIndex < 0 ? ranges.length - 1 : nextIndex;
  }
  const range = ranges[index];
  return {
    sql: sql.slice(range.start, range.end),
    source: "statement",
    start: range.start,
    end: range.end,
    statementNumber: index + 1,
    statementCount: ranges.length,
  };
}

export type SqlErrorLocation = {
  line: number;
  column: number;
  lineText: string;
  caretColumn: number;
};

/** Converts the backend's one-based character position into a line/column. */
export function sqlErrorLocation(
  sql: string,
  position: number | undefined,
): SqlErrorLocation | null {
  if (!position || position < 1) {
    return null;
  }
  const index = Math.min(position - 1, Math.max(0, sql.length));
  const before = sql.slice(0, index);
  const lastNewline = before.lastIndexOf("\n");
  const line = before.split("\n").length;
  const column = index - lastNewline;
  const fullLineText = sql.split("\n")[line - 1] ?? "";
  const caret = Math.max(0, column - 1);
  if (fullLineText.length <= 240) {
    return { line, column, lineText: fullLineText, caretColumn: caret };
  }
  const windowSize = 200;
  const start = Math.max(0, Math.min(caret - 60, fullLineText.length - windowSize));
  const prefix = start > 0 ? "…" : "";
  const suffix = start + windowSize < fullLineText.length ? "…" : "";
  return {
    line,
    column,
    lineText: `${prefix}${fullLineText.slice(start, start + windowSize)}${suffix}`,
    caretColumn: caret - start + prefix.length,
  };
}

function uniqueColumnKeys(columns: readonly { name: string }[]): string[] {
  const used = new Map<string, number>();
  return columns.map((column) => {
    const count = (used.get(column.name) ?? 0) + 1;
    used.set(column.name, count);
    return count === 1 ? column.name : `${column.name}_${count}`;
  });
}

/** Copies rows as tab-separated text with a header. Tabs/newlines are escaped. */
export function resultSetToTsv(resultSet: DatabaseQueryResultSet): string {
  const escape = (value: string | null) =>
    value === null
      ? "NULL"
      : value.replace(/\t/g, "\\t").replace(/\r/g, "\\r").replace(/\n/g, "\\n");
  const lines = [resultSet.columns.map((column) => escape(column.name)).join("\t")];
  resultSet.rows.forEach((row) => {
    lines.push(row.map((cell) => escape(cell.value)).join("\t"));
  });
  return lines.join("\n");
}

function csvCell(value: string | null): string {
  if (value === null) {
    return "";
  }
  if (/[",\r\n]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

/** RFC 4180-style CSV with a header and CRLF row endings. */
export function resultSetToCsv(resultSet: DatabaseQueryResultSet): string {
  const lines = [resultSet.columns.map((column) => csvCell(column.name)).join(",")];
  resultSet.rows.forEach((row) => {
    lines.push(row.map((cell) => csvCell(cell.value)).join(","));
  });
  return `${lines.join("\r\n")}\r\n`;
}

/** JSON array of row objects. NULL stays null; duplicate names get a suffix. */
export function resultSetToJson(resultSet: DatabaseQueryResultSet): string {
  const keys = uniqueColumnKeys(resultSet.columns);
  const rows = resultSet.rows.map((row) =>
    Object.fromEntries(keys.map((key, index) => [key, row[index]?.value ?? null])),
  );
  return JSON.stringify(rows, null, 2);
}

export function resultSetLabel(resultSet: DatabaseQueryResultSet, index: number): string {
  if (resultSet.columns.length === 0 && resultSet.affectedRows !== null) {
    return `Result ${index + 1} · ${resultSet.affectedRows} affected`;
  }
  return `Result ${index + 1} · ${resultSet.returnedRows} row${
    resultSet.returnedRows === 1 ? "" : "s"
  }`;
}

export function formatQueryDuration(durationMs: number): string {
  if (durationMs < 1) {
    return "<1 ms";
  }
  if (durationMs < 1_000) {
    return `${Math.round(durationMs)} ms`;
  }
  return `${(durationMs / 1_000).toFixed(2)} s`;
}

export function formatElapsedMs(startedAt: number, now: number): string {
  return formatQueryDuration(Math.max(0, now - startedAt));
}

export function resultSetFileName(
  profileName: string,
  queryTitle: string,
  resultIndex: number,
  extension: "csv" | "json",
): string {
  const base = `${profileName}-${queryTitle}-result-${resultIndex + 1}`
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
  return `${base || "query-result"}.${extension}`;
}

/** Starts a browser download. Returns false when the runtime cannot create URLs. */
export function downloadTextFile(filename: string, mimeType: string, content: string): boolean {
  if (typeof URL.createObjectURL !== "function") {
    return false;
  }
  const url = URL.createObjectURL(new Blob([content], { type: mimeType }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.rel = "noopener";
  anchor.style.display = "none";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
  return true;
}

export function preferredResultSetIndex(result: DatabaseQueryExecutionResult | undefined): number {
  if (!result) {
    return 0;
  }
  return Math.max(
    0,
    result.results.findIndex((resultSet) => resultSet.columns.length > 0),
  );
}
