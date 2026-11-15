import { useEffect, useMemo, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import {
  BracketsCurlyIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  ClockIcon,
  CodeIcon,
  DatabaseIcon,
  FileCsvIcon,
  InfoIcon,
  LockSimpleIcon,
  PlayIcon,
  PlusIcon,
  RowsIcon,
  ShieldCheckIcon,
  StopIcon,
  TreeStructureIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { CopyButton } from "@/components/ui/copy-button";
import { EngineBadge } from "@/features/databases/DatabaseConnectionList";
import { DatabaseSqlEditor } from "@/features/databases/DatabaseSqlEditor";
import {
  downloadTextFile,
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
} from "@/features/databases/databaseSqlWorkspaceModel";
import {
  databaseConnectionStateLabel,
  formatDatabaseEndpoint,
} from "@/features/databases/databaseProfileForm";
import { useDatabaseQueryStore, type DatabaseQueryEntry } from "@/stores/databaseQueryStore";
import {
  useDatabaseWorkspaceStore,
  type DatabaseSqlLastRun,
  type DatabaseSqlQueryTab,
  type DatabaseWorkspaceTab,
} from "@/stores/databaseWorkspaceStore";
import type {
  CommandError,
  DatabaseConnectionSnapshot,
  DatabaseQueryResultSet,
  DatabaseSqlAnalysis,
} from "@/types/contracts";

const ROW_LIMIT_OPTIONS = [100, 500, 1_000, 5_000] as const;
const TIMEOUT_OPTIONS = [5_000, 10_000, 30_000] as const;

type PendingRun = {
  queryTabId: string;
  sql: string;
  source: DatabaseSqlLastRun["source"];
  statementNumber: number;
  statementCount: number;
  analysis: DatabaseSqlAnalysis;
};

type DatabaseSqlWorkspaceProps = {
  tab: DatabaseWorkspaceTab;
  snapshot: DatabaseConnectionSnapshot | undefined;
  onConnect: () => void;
  onConnectionMayHaveClosed?: () => void;
};

function schemaLabel(
  tab: DatabaseWorkspaceTab,
  database: string | null,
): { value: string; title: string } {
  const schema = tab.selection?.schema;
  if (schema) {
    return { value: schema, title: `Schema ${schema}` };
  }
  if (tab.engine === "mysql") {
    return {
      value: database ?? "not selected",
      title: "MySQL uses the selected database as its schema.",
    };
  }
  return {
    value: "default",
    title: "PostgreSQL uses the session search_path until a schema is selected.",
  };
}

function QueryStatusGlyph({ status }: { status: string | undefined }) {
  if (status === "running" || status === "analyzing") {
    return (
      <CircleNotchIcon className="size-3 shrink-0 animate-spin text-primary" aria-hidden="true" />
    );
  }
  if (status === "error" || status === "blocked") {
    return <WarningCircleIcon className="size-3 shrink-0 text-destructive" aria-hidden="true" />;
  }
  if (status === "cancelled") {
    return <StopIcon className="size-3 shrink-0 text-muted-foreground" aria-hidden="true" />;
  }
  if (status === "succeeded") {
    return <CheckCircleIcon className="size-3 shrink-0 text-success" aria-hidden="true" />;
  }
  return <CodeIcon className="size-3 shrink-0 text-muted-foreground/70" aria-hidden="true" />;
}

function CursorReadout({ tab }: { tab: DatabaseSqlQueryTab }) {
  const sql = tab.sql;
  const before = sql.slice(0, Math.min(tab.selectionStart, sql.length));
  const line = before.split("\n").length;
  const column = before.length - before.lastIndexOf("\n");
  const selected = Math.abs(tab.selectionEnd - tab.selectionStart);
  return (
    <span className="font-mono text-[10px] text-muted-foreground">
      Ln {line}, Col {column}
      {selected > 0 ? ` · ${selected} selected` : ""}
    </span>
  );
}

function ReadOnlyControl({
  enabled,
  locked,
  onToggle,
}: {
  enabled: boolean;
  locked: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex min-w-0 items-center gap-2.5 rounded-md border border-border bg-muted/20 px-2.5 py-1.5">
      <span
        className={
          enabled
            ? "grid size-6 shrink-0 place-items-center rounded-sm border border-warning/30 bg-warning/10 text-warning"
            : "grid size-6 shrink-0 place-items-center rounded-sm border border-border bg-card text-muted-foreground"
        }
      >
        {enabled ? (
          <LockSimpleIcon className="size-3.5" aria-hidden="true" />
        ) : (
          <ShieldCheckIcon className="size-3.5" aria-hidden="true" />
        )}
      </span>
      <div className="min-w-0">
        <p className="text-[11px] font-medium text-foreground">Read-only mode</p>
        <p className="max-w-64 truncate text-[10px] text-muted-foreground">
          {locked
            ? "Profile-enforced · adapter rejects writes"
            : enabled
              ? "Guard on · writes blocked before execution"
              : "Off · write statements can run"}
        </p>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-label="Read-only mode"
        title={
          locked
            ? "This connection profile is read-only. Edit the profile to enable writes."
            : enabled
              ? "Disable the SQL workspace read-only guard"
              : "Block write statements in this SQL workspace"
        }
        disabled={locked}
        onClick={onToggle}
        className={
          enabled
            ? "relative ml-1 h-5 w-9 shrink-0 rounded-full border border-warning/40 bg-warning/25 outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-60"
            : "relative ml-1 h-5 w-9 shrink-0 rounded-full border border-input bg-muted outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-60"
        }
      >
        <span
          className={
            enabled
              ? "absolute top-0.5 right-0.5 size-3.5 rounded-full bg-warning shadow-sm"
              : "absolute top-0.5 left-0.5 size-3.5 rounded-full bg-muted-foreground/70 shadow-sm"
          }
        />
      </button>
    </div>
  );
}

function sourceLabel(source: DatabaseSqlLastRun["source"]): string {
  if (source === "selection") {
    return "selection";
  }
  if (source === "all") {
    return "all statements";
  }
  return "current statement";
}

function ResultGrid({
  tab,
  resultSet,
}: {
  tab: DatabaseWorkspaceTab;
  resultSet: DatabaseQueryResultSet;
}) {
  if (resultSet.columns.length === 0) {
    return (
      <div className="space-y-2 px-4 py-5">
        <p className="text-[12px] font-medium text-foreground">
          {resultSet.affectedRows !== null
            ? `${resultSet.affectedRows} row${resultSet.affectedRows === 1 ? "" : "s"} affected`
            : "Statement completed without a result grid."}
        </p>
        {resultSet.lastInsertId !== null ? (
          <p className="font-mono text-[11px] text-muted-foreground">
            Last insert id: {resultSet.lastInsertId}
          </p>
        ) : null}
        {resultSet.affectedRows === null && resultSet.lastInsertId === null ? (
          <p className="text-[11px] text-muted-foreground">
            The engine did not report a row count for this statement.
          </p>
        ) : null}
      </div>
    );
  }

  return (
    <div className="max-h-[22rem] overflow-auto">
      <table
        aria-label={`Result set ${resultSet.index + 1} grid for ${tab.name}`}
        className="w-full table-fixed border-collapse text-left text-[11px]"
        style={{ minWidth: `${Math.max(40, resultSet.columns.length * 7)}rem` }}
      >
        <colgroup>
          <col style={{ width: "2.5rem" }} />
          {resultSet.columns.map((column, index) => (
            <col key={`${column.name}-${index}`} />
          ))}
        </colgroup>
        <thead className="sticky top-0 z-10 bg-card">
          <tr className="border-b border-border">
            <th className="w-10 border-r border-border bg-muted/25 px-2 py-1.5 text-right text-[10px] font-medium text-muted-foreground">
              #
            </th>
            {resultSet.columns.map((column, index) => (
              <th
                key={`${column.name}-${index}`}
                className="border-r border-border px-2.5 py-1.5 align-bottom last:border-r-0"
              >
                <span className="block truncate font-medium text-foreground">{column.name}</span>
                <span className="mt-0.5 block truncate font-mono text-[9px] font-normal text-muted-foreground/80">
                  {column.dataType}
                </span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {resultSet.rows.length === 0 ? (
            <tr>
              <td
                colSpan={resultSet.columns.length + 1}
                className="px-3 py-6 text-center text-[11px] text-muted-foreground"
              >
                The statement returned columns but no rows.
              </td>
            </tr>
          ) : (
            resultSet.rows.map((row, rowIndex) => (
              <tr
                key={rowIndex}
                className="border-b border-border/70 last:border-0 hover:bg-muted/30"
              >
                <td className="border-r border-border bg-muted/15 px-2 py-1.5 text-right font-mono text-[10px] text-muted-foreground/70 tabular-nums">
                  {rowIndex + 1}
                </td>
                {row.map((cell, cellIndex) => (
                  <td
                    key={cellIndex}
                    className="truncate border-r border-border/60 px-2.5 py-1.5 font-mono text-foreground last:border-r-0"
                    title={
                      cell.truncated
                        ? `${cell.value ?? "NULL"} (truncated by the result bound)`
                        : (cell.value ?? "NULL")
                    }
                  >
                    {cell.value === null ? (
                      <span className="text-[10px] text-muted-foreground/75 italic">NULL</span>
                    ) : cell.value === "" ? (
                      <span className="text-muted-foreground/70">""</span>
                    ) : (
                      <>
                        {cell.value}
                        {cell.truncated ? <span className="text-warning"> …</span> : null}
                      </>
                    )}
                  </td>
                ))}
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}

function ResultPanel({
  tab,
  queryTab,
  queryEntry,
}: {
  tab: DatabaseWorkspaceTab;
  queryTab: DatabaseSqlQueryTab;
  queryEntry: DatabaseQueryEntry | undefined;
}) {
  const result = queryEntry?.result;
  const [selection, setSelection] = useState<{ result: typeof result; index: number }>({
    result,
    index: preferredResultSetIndex(result),
  });
  const [exportState, setExportState] = useState<{
    result: typeof result;
    message: string;
  } | null>(null);
  if (!result) {
    return null;
  }
  const resultSetIndex =
    selection.result === result ? selection.index : preferredResultSetIndex(result);
  const resultSet = result.results[resultSetIndex] ?? result.results[0];
  const exportNotice = exportState?.result === result ? exportState.message : null;

  function exportResult(format: "csv" | "json") {
    const selectedSet = result?.results[resultSetIndex] ?? result?.results[0];
    if (!result || !selectedSet) {
      return;
    }
    const filename = resultSetFileName(tab.name, queryTab.title, resultSetIndex, format);
    const content = format === "csv" ? resultSetToCsv(selectedSet) : resultSetToJson(selectedSet);
    const exported = downloadTextFile(
      filename,
      format === "csv" ? "text/csv;charset=utf-8" : "application/json;charset=utf-8",
      content,
    );
    setExportState({
      result,
      message: exported ? `Exported ${filename}.` : "Export is unavailable in this environment.",
    });
  }
  const lastRun =
    queryTab.lastRun && queryTab.lastRun.sql === queryEntry?.executedSql ? queryTab.lastRun : null;
  return (
    <section
      role="region"
      aria-label={`SQL results for ${tab.name}`}
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-border bg-muted/15 px-3 py-2">
        <p className="flex items-center gap-1.5 text-[11px] font-semibold text-foreground">
          <CheckCircleIcon className="size-3.5 text-success" aria-hidden="true" />
          Query succeeded
        </p>
        <span className="font-mono text-[10px] text-muted-foreground">
          {formatQueryDuration(result.durationMs)}
        </span>
        <span className="text-[10px] text-muted-foreground">
          {result.results.length} result set{result.results.length === 1 ? "" : "s"}
        </span>
        {result.truncated ? (
          <Badge variant="warning">Bounded at {result.maxRows.toLocaleString()} rows</Badge>
        ) : null}
        {result.readOnly ? <Badge variant="warning">Read-only session</Badge> : null}
        <span className="ml-auto font-mono text-[10px] text-muted-foreground">
          {queryTab.title}
        </span>
      </div>

      {result.warnings.length > 0 ? (
        <div className="border-b border-warning/20 bg-warning/5 px-3 py-2">
          <p className="flex items-center gap-1.5 text-[10px] font-medium text-warning">
            <WarningCircleIcon className="size-3.5" aria-hidden="true" />
            Destructive statements were executed after confirmation.
          </p>
          <ul className="mt-1 space-y-0.5 pl-5 text-[10px] leading-4 text-muted-foreground">
            {result.warnings.map((warning, index) => (
              <li key={`${warning.code}-${index}`} className="list-disc">
                {warning.message}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {result.results.length > 1 ? (
        <div
          role="tablist"
          aria-label="Result sets"
          className="flex flex-wrap items-center gap-1 border-b border-border bg-muted/10 px-3 py-1.5"
        >
          {result.results.map((set, index) => (
            <button
              key={set.index}
              type="button"
              role="tab"
              aria-selected={index === resultSetIndex}
              onClick={() => setSelection({ result, index })}
              className={
                index === resultSetIndex
                  ? "rounded-sm border border-border bg-card px-2 py-1 text-[10px] font-medium text-foreground"
                  : "rounded-sm border border-transparent px-2 py-1 text-[10px] text-muted-foreground hover:bg-muted hover:text-foreground"
              }
            >
              {resultSetLabel(set, index)}
            </button>
          ))}
        </div>
      ) : null}

      {resultSet ? (
        <>
          <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-1.5">
            <span className="font-mono text-[10px] text-muted-foreground">
              {lastRun
                ? `${lastRun.statementCount} statement${
                    lastRun.statementCount === 1 ? "" : "s"
                  } · ${sourceLabel(lastRun.source)}`
                : "executed SQL"}
            </span>
            {resultSet.columnsTruncated ? <Badge variant="warning">Columns truncated</Badge> : null}
            {resultSet.truncated ? <Badge variant="warning">Rows truncated</Badge> : null}
            {resultSet.columns.length > 0 ? (
              <div className="ml-auto flex flex-wrap items-center gap-1">
                <CopyButton value={resultSetToTsv(resultSet)} label="Copy result" />
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1.5 px-2 text-[11px]"
                  onClick={() => exportResult("csv")}
                  title={`Export ${resultSetLabel(resultSet, resultSetIndex)} as CSV`}
                >
                  <FileCsvIcon className="size-3.5" aria-hidden="true" />
                  CSV
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1.5 px-2 text-[11px]"
                  onClick={() => exportResult("json")}
                  title={`Export ${resultSetLabel(resultSet, resultSetIndex)} as JSON`}
                >
                  <BracketsCurlyIcon className="size-3.5" aria-hidden="true" />
                  JSON
                </Button>
              </div>
            ) : null}
          </div>
          <ResultGrid tab={tab} resultSet={resultSet} />
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-t border-border px-3 py-1.5 font-mono text-[10px] text-muted-foreground">
            <span>{resultSet.returnedRows} returned</span>
            {resultSet.affectedRows !== null ? (
              <span>{resultSet.affectedRows} affected</span>
            ) : null}
            {resultSet.lastInsertId !== null ? (
              <span>insert id {resultSet.lastInsertId}</span>
            ) : null}
            {resultSet.truncated ? <span className="text-warning">more rows discarded</span> : null}
          </div>
          {exportNotice ? (
            <p
              role="status"
              className="border-t border-border px-3 py-1.5 text-[10px] text-muted-foreground"
            >
              {exportNotice}
            </p>
          ) : null}
        </>
      ) : (
        <p className="px-3 py-4 text-[11px] text-muted-foreground">
          The engine returned no result sets for this statement.
        </p>
      )}
      <p className="border-t border-border px-3 py-1 font-mono text-[9px] leading-relaxed text-muted-foreground/60">
        {result.results.length} set{result.results.length === 1 ? "" : "s"} · bounded result
        {result.truncated ? " · at least one bound was applied" : ""}
      </p>
    </section>
  );
}

function ErrorPanel({
  tab,
  queryTab,
  error,
  onRetry,
  onGoToError,
}: {
  tab: DatabaseWorkspaceTab;
  queryTab: DatabaseSqlQueryTab;
  error: CommandError;
  onRetry: () => void;
  onGoToError: (position: number) => void;
}) {
  const executedSql = queryTab.lastRun?.sql ?? "";
  const location = sqlErrorLocation(executedSql, error.position);
  const canLocate = location !== null && queryTab.lastRun !== null;
  const title =
    error.code === "timeout"
      ? "Query timed out"
      : error.code === "permission"
        ? "Read-only enforcement"
        : error.code === "query"
          ? "SQL error"
          : error.code === "connection"
            ? "Connection unavailable"
            : "Query failed";
  const closedConnection = error.code === "timeout" || error.code === "connection";

  return (
    <Alert variant="destructive" role="alert" aria-label={`${title} for ${tab.name}`}>
      <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
      <div className="min-w-0 flex-1 space-y-2">
        <AlertTitle>{title}</AlertTitle>
        <AlertDescription>{error.message}</AlertDescription>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
          <span className="font-mono">code {error.code}</span>
          {error.databaseCode ? (
            <span className="font-mono">
              {tab.engine === "mysql" ? "MySQL error" : "SQLSTATE"} {error.databaseCode}
            </span>
          ) : null}
          {location ? (
            <span className="font-mono">
              line {location.line}, column {location.column}
            </span>
          ) : null}
          {error.durationMs !== undefined ? (
            <span className="font-mono">{formatQueryDuration(error.durationMs)}</span>
          ) : null}
        </div>
        {location ? (
          <div className="space-y-1">
            <p className="text-[10px] font-medium text-foreground">Position</p>
            <pre className="overflow-x-auto rounded-sm border border-border bg-muted/30 px-2.5 py-1.5 font-mono text-[10px] leading-5 whitespace-pre text-foreground">
              {location.lineText}
              {"\n"}
              {" ".repeat(location.caretColumn)}
              <span className="text-destructive">^</span>
            </pre>
          </div>
        ) : null}
        {error.details ? (
          <p className="text-[10px] leading-5 text-muted-foreground">{error.details}</p>
        ) : null}
        {closedConnection ? (
          <p className="text-[10px] leading-5 text-warning">
            The database session is closed after a timeout or lost connection. Reconnect the profile
            before running another query.
          </p>
        ) : null}
        <div className="flex flex-wrap items-center gap-2 pt-0.5">
          {canLocate && error.position ? (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-7 text-[11px]"
              onClick={() => onGoToError(error.position as number)}
            >
              Go to error
            </Button>
          ) : null}
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="h-7 text-[11px]"
            onClick={onRetry}
          >
            Run again
          </Button>
        </div>
      </div>
    </Alert>
  );
}

function BlockedPanel({
  tab,
  message,
  analysis,
  profileReadOnly,
}: {
  tab: DatabaseWorkspaceTab;
  message: string | undefined;
  analysis: DatabaseSqlAnalysis | undefined;
  profileReadOnly: boolean;
}) {
  return (
    <Alert variant="warning" role="alert" aria-label={`Read-only mode blocked SQL for ${tab.name}`}>
      <LockSimpleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
      <div className="min-w-0 flex-1 space-y-1.5">
        <AlertTitle>Blocked by read-only mode</AlertTitle>
        <AlertDescription>
          {message ?? "This SQL contains write statements. No statement was sent to the database."}
        </AlertDescription>
        <p className="text-[10px] leading-5 text-muted-foreground">
          {profileReadOnly
            ? "The connection profile is read-only, so write statements cannot be enabled here. Edit the profile to allow writes."
            : "The workspace read-only guard is on. Turn it off only when you intend to change data."}
        </p>
        {analysis && analysis.warnings.length > 0 ? (
          <ul className="space-y-0.5 pl-5 text-[10px] leading-4 text-muted-foreground">
            {analysis.warnings.map((warning, index) => (
              <li key={`${warning.code}-${index}`} className="list-disc">
                {warning.message}
              </li>
            ))}
          </ul>
        ) : null}
        <p className="font-mono text-[10px] text-muted-foreground">
          {analysis?.statementCount ?? 0} statement
          {analysis?.statementCount === 1 ? "" : "s"} checked · 0 executed
        </p>
      </div>
    </Alert>
  );
}

function RunningPanel({
  timeoutMs,
  startedAt,
  onCancel,
  cancelling,
}: {
  timeoutMs: number;
  startedAt: number | undefined;
  onCancel: () => void;
  cancelling: boolean;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!startedAt) {
      return;
    }
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [startedAt]);

  return (
    <div
      role="status"
      aria-label="Query running"
      className="flex flex-wrap items-center gap-3 rounded-md border border-border bg-card px-3 py-3"
    >
      <CircleNotchIcon className="size-4 animate-spin text-primary" aria-hidden="true" />
      <div className="min-w-0">
        <p className="text-[11px] font-medium text-foreground">Running query</p>
        <p className="flex flex-wrap items-center gap-x-2 text-[10px] text-muted-foreground">
          <ClockIcon className="size-3" aria-hidden="true" />
          {startedAt ? formatQueryDuration(Math.max(0, now - startedAt)) : "starting"}
          <span>· timeout {formatQueryDuration(timeoutMs)}</span>
        </p>
      </div>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        className="ml-auto h-7 text-[11px]"
        onClick={onCancel}
        disabled={cancelling}
      >
        <StopIcon className="size-3.5" aria-hidden="true" />
        {cancelling ? "Cancelling..." : "Cancel execution"}
      </Button>
    </div>
  );
}

export function DatabaseSqlWorkspace({
  tab,
  snapshot,
  onConnect,
  onConnectionMayHaveClosed,
}: DatabaseSqlWorkspaceProps) {
  const workspaceTab = useDatabaseWorkspaceStore((state) => state.tabs[tab.profileId] ?? tab);
  const queryTabs = workspaceTab.sqlTabs;
  const activeQueryTab =
    queryTabs.find((queryTab) => queryTab.id === workspaceTab.activeSqlTabId) ?? queryTabs[0];
  const queryEntries = useDatabaseQueryStore((state) => state.entries);
  const queryEntry = activeQueryTab ? queryEntries[activeQueryTab.id] : undefined;
  const [confirmation, setConfirmation] = useState<PendingRun | null>(null);
  const [cancelling, setCancelling] = useState(false);

  const connectionState = snapshot?.state ?? "disconnected";
  const connected = connectionState === "connected";
  const profileReadOnly = workspaceTab.readOnly || (snapshot?.readOnly ?? false);
  const readOnly = workspaceTab.sqlReadOnly || profileReadOnly;
  const database =
    workspaceTab.selection?.database ?? snapshot?.databaseName ?? workspaceTab.databaseName;
  const schema = schemaLabel(workspaceTab, database);
  const status = queryEntry?.status ?? "idle";
  const busy = status === "running" || status === "analyzing";
  const running = status === "running";
  const result = queryEntry?.result;
  const statementCount = useMemo(
    () =>
      activeQueryTab ? findSqlStatementRanges(activeQueryTab.sql, workspaceTab.engine).length : 0,
    [activeQueryTab, workspaceTab.engine],
  );

  async function executeResolved(
    queryTab: DatabaseSqlQueryTab,
    resolved: {
      sql: string;
      source: DatabaseSqlLastRun["source"];
      statementNumber: number;
      statementCount: number;
    },
    confirmed: boolean,
  ) {
    const workspace = useDatabaseWorkspaceStore.getState().tabs[tab.profileId];
    if (!workspace) {
      return;
    }
    const currentQuery = workspace.sqlTabs.find((entry) => entry.id === queryTab.id);
    if (!currentQuery) {
      return;
    }
    const lastRun: DatabaseSqlLastRun = {
      sql: resolved.sql,
      source: resolved.source,
      statementNumber: resolved.statementNumber,
      statementCount: resolved.statementCount,
      requestedAt: Date.now(),
    };
    useDatabaseWorkspaceStore.getState().setSqlQueryLastRun(tab.profileId, queryTab.id, lastRun);
    const executed = await useDatabaseQueryStore
      .getState()
      .execute(queryTab.id, tab.profileId, resolved.sql, {
        maxRows: currentQuery.maxRows,
        timeoutMs: currentQuery.timeoutMs,
        confirmed,
      });
    const entry = useDatabaseQueryStore.getState().entries[queryTab.id];
    if (
      !executed &&
      (entry?.status === "cancelled" ||
        entry?.error?.code === "timeout" ||
        entry?.error?.code === "connection")
    ) {
      onConnectionMayHaveClosed?.();
    }
  }

  async function runSource(source: "auto" | "all") {
    const workspace = useDatabaseWorkspaceStore.getState().tabs[tab.profileId];
    if (!workspace) {
      return;
    }
    const queryTab =
      workspace.sqlTabs.find((entry) => entry.id === workspace.activeSqlTabId) ??
      workspace.sqlTabs[0];
    if (!queryTab) {
      return;
    }
    if (!connected) {
      useDatabaseQueryStore.getState().fail(queryTab.id, {
        code: "connection",
        message: "Connect this database profile before running SQL.",
      });
      return;
    }
    if (
      useDatabaseQueryStore.getState().entries[queryTab.id]?.status === "running" ||
      useDatabaseQueryStore.getState().entries[queryTab.id]?.status === "analyzing"
    ) {
      return;
    }

    const resolved = resolveSqlForExecution({
      sql: queryTab.sql,
      engine: workspace.engine,
      selectionStart: queryTab.selectionStart,
      selectionEnd: queryTab.selectionEnd,
      source,
    });
    if (!resolved.sql.trim()) {
      useDatabaseQueryStore.getState().fail(queryTab.id, {
        code: "validation",
        message: "Enter a SQL statement or select SQL before running it.",
      });
      return;
    }

    useDatabaseWorkspaceStore.getState().setSqlQueryLastRun(tab.profileId, queryTab.id, {
      sql: resolved.sql,
      source: resolved.source,
      statementNumber: resolved.statementNumber,
      statementCount: resolved.statementCount,
      requestedAt: Date.now(),
    });
    const analysis = await useDatabaseQueryStore
      .getState()
      .analyze(queryTab.id, workspace.engine, resolved.sql);
    if (!analysis) {
      return;
    }

    const liveWorkspace = useDatabaseWorkspaceStore.getState().tabs[tab.profileId];
    const liveReadOnly =
      liveWorkspace?.sqlReadOnly || liveWorkspace?.readOnly || (snapshot?.readOnly ?? false);
    if (liveReadOnly && analysis.hasWrites) {
      useDatabaseQueryStore
        .getState()
        .block(
          queryTab.id,
          analysis,
          "This SQL contains write statements and read-only mode is on. No statement was sent to the database.",
        );
      return;
    }

    if (analysis.requiresConfirmation) {
      setConfirmation({
        queryTabId: queryTab.id,
        sql: resolved.sql,
        source: resolved.source,
        statementNumber: resolved.statementNumber,
        statementCount: resolved.statementCount,
        analysis,
      });
      return;
    }

    await executeResolved(queryTab, resolved, false);
  }

  async function executeConfirmed(pending: PendingRun) {
    const workspace = useDatabaseWorkspaceStore.getState().tabs[tab.profileId];
    const queryTab = workspace?.sqlTabs.find((entry) => entry.id === pending.queryTabId);
    if (!queryTab) {
      return;
    }
    await executeResolved(
      queryTab,
      {
        sql: pending.sql,
        source: pending.source,
        statementNumber: pending.statementNumber,
        statementCount: pending.statementCount,
      },
      true,
    );
  }

  async function handleCancel() {
    if (!activeQueryTab || !running) {
      return;
    }
    setCancelling(true);
    const cancelled = await useDatabaseQueryStore.getState().cancel(activeQueryTab.id);
    setCancelling(false);
    if (cancelled) {
      onConnectionMayHaveClosed?.();
    }
  }

  async function handleCloseQueryTab(queryTab: DatabaseSqlQueryTab) {
    const entry = useDatabaseQueryStore.getState().entries[queryTab.id];
    if (entry?.status === "running") {
      await useDatabaseQueryStore.getState().cancel(queryTab.id);
    }
    useDatabaseQueryStore.getState().clear(queryTab.id);
    useDatabaseWorkspaceStore.getState().closeSqlQueryTab(tab.profileId, queryTab.id);
    setConfirmation((current) => (current?.queryTabId === queryTab.id ? null : current));
  }

  function handleClear() {
    if (!activeQueryTab || busy) {
      return;
    }
    useDatabaseWorkspaceStore.getState().setSqlQueryText(tab.profileId, activeQueryTab.id, "");
    useDatabaseWorkspaceStore
      .getState()
      .setSqlQuerySelection(tab.profileId, activeQueryTab.id, 0, 0);
    useDatabaseWorkspaceStore.getState().setSqlQueryLastRun(tab.profileId, activeQueryTab.id, null);
    useDatabaseQueryStore.getState().clear(activeQueryTab.id);
    setConfirmation(null);
  }

  function handleToggleReadOnly() {
    if (profileReadOnly) {
      return;
    }
    useDatabaseWorkspaceStore.getState().setSqlReadOnly(tab.profileId, !workspaceTab.sqlReadOnly);
  }

  function handleQueryTabKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>, index: number) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }
    event.preventDefault();
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? queryTabs.length - 1
          : event.key === "ArrowLeft"
            ? (index - 1 + queryTabs.length) % queryTabs.length
            : (index + 1) % queryTabs.length;
    const next = queryTabs[nextIndex];
    if (!next) {
      return;
    }
    useDatabaseWorkspaceStore.getState().activateSqlQueryTab(tab.profileId, next.id);
    document.getElementById(sqlQueryTabElementId(tab.profileId, next.id))?.focus();
  }

  function goToError(position: number) {
    const textarea = document.getElementById(
      `database-sql-editor-${tab.profileId}`,
    ) as HTMLTextAreaElement | null;
    if (!textarea || !activeQueryTab) {
      return;
    }
    const index = Math.min(Math.max(position - 1, 0), textarea.value.length);
    textarea.focus();
    textarea.setSelectionRange(index, index);
    useDatabaseWorkspaceStore
      .getState()
      .setSqlQuerySelection(tab.profileId, activeQueryTab.id, index, index);
  }

  if (!activeQueryTab) {
    return (
      <div className="rounded-md border border-border bg-card p-4">
        <p className="text-[12px] text-muted-foreground">
          No SQL query tabs are open. Close and reopen this connection workspace to start one.
        </p>
      </div>
    );
  }

  const error = queryEntry?.error;
  const canRun = connected && !busy;
  return (
    <div className="space-y-3">
      <section
        aria-label={`SQL context for ${tab.name}`}
        className="overflow-hidden rounded-md border border-border bg-card"
      >
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3 py-2">
          <span className="inline-flex min-w-0 items-center gap-1.5">
            <CodeIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="text-[10px] tracking-wide text-muted-foreground uppercase">
              Connection
            </span>
            <span className="max-w-44 truncate text-[12px] font-semibold text-foreground">
              {tab.name}
            </span>
          </span>
          <EngineBadge engine={tab.engine} />
          <Badge variant={connected ? "success" : "outline"}>
            {databaseConnectionStateLabel(connectionState)}
          </Badge>
          <span className="h-4 w-px bg-border" aria-hidden="true" />
          <span className="inline-flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground">
            <DatabaseIcon className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="tracking-wide uppercase">Database</span>
            <span className="max-w-44 truncate rounded-sm border border-border bg-muted/25 px-1.5 py-0.5 font-mono text-[10px] text-foreground">
              {database ?? "not selected"}
            </span>
          </span>
          <span className="inline-flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground">
            <TreeStructureIcon className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="tracking-wide uppercase">Schema</span>
            <span
              className="max-w-44 truncate rounded-sm border border-border bg-muted/25 px-1.5 py-0.5 font-mono text-[10px] text-foreground"
              title={schema.title}
            >
              {schema.value}
            </span>
          </span>
          <span
            className="hidden truncate font-mono text-[10px] text-muted-foreground md:inline"
            title={`${formatDatabaseEndpoint(tab.host, tab.port, tab.socket)} · ${workspaceTab.sqlTabs.length} query tab${
              workspaceTab.sqlTabs.length === 1 ? "" : "s"
            }`}
          >
            {formatDatabaseEndpoint(tab.host, tab.port, tab.socket)}
          </span>
          <div className="ml-auto">
            <ReadOnlyControl
              enabled={readOnly}
              locked={profileReadOnly}
              onToggle={handleToggleReadOnly}
            />
          </div>
        </div>
        <p className="border-t border-border bg-muted/15 px-3 py-1 text-[10px] leading-5 text-muted-foreground">
          {profileReadOnly
            ? "This profile is read-only. The adapter rejects writes before execution; the workspace switch is locked on."
            : readOnly
              ? "Read-only guard on. SELECT, SHOW, and EXPLAIN run normally; write statements are blocked before the driver sees them."
              : "Read-only guard off. Write statements execute with the privileges of this profile."}
        </p>
      </section>

      <section className="overflow-hidden rounded-md border border-border bg-card">
        <div className="flex flex-wrap items-center gap-2 border-b border-border bg-muted/15 px-2 py-1.5">
          <div
            role="tablist"
            aria-label={`SQL query tabs for ${tab.name}`}
            className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto"
          >
            {queryTabs.map((queryTab, index) => {
              const entry = queryEntries[queryTab.id];
              const selected = queryTab.id === activeQueryTab.id;
              return (
                <div
                  key={queryTab.id}
                  className={
                    selected
                      ? "flex shrink-0 items-center rounded-sm border border-border bg-card shadow-sm"
                      : "flex shrink-0 items-center rounded-sm border border-transparent hover:bg-muted/50"
                  }
                >
                  <button
                    type="button"
                    role="tab"
                    id={sqlQueryTabElementId(tab.profileId, queryTab.id)}
                    aria-selected={selected}
                    aria-controls={sqlQueryPanelElementId(tab.profileId, queryTab.id)}
                    tabIndex={selected ? 0 : -1}
                    onClick={() =>
                      useDatabaseWorkspaceStore
                        .getState()
                        .activateSqlQueryTab(tab.profileId, queryTab.id)
                    }
                    onKeyDown={(event) => handleQueryTabKeyDown(event, index)}
                    title={`${queryTab.title} · ${tab.name}`}
                    className="flex h-7 max-w-44 items-center gap-1.5 rounded-sm px-2.5 text-[11px] font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40 aria-selected:text-foreground"
                  >
                    <QueryStatusGlyph status={entry?.status} />
                    <span className="truncate">{queryTab.title}</span>
                  </button>
                  <button
                    type="button"
                    aria-label={`Close ${queryTab.title}`}
                    title={`Close ${queryTab.title}`}
                    onClick={() => void handleCloseQueryTab(queryTab)}
                    className="mr-1 grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
                  >
                    <XIcon className="size-3" aria-hidden="true" />
                  </button>
                </div>
              );
            })}
          </div>
          <button
            type="button"
            aria-label="New SQL query tab"
            title="Open another query tab on this connection"
            onClick={() => {
              useDatabaseWorkspaceStore.getState().addSqlQueryTab(tab.profileId);
            }}
            className="grid size-7 shrink-0 place-items-center rounded-sm border border-border bg-card text-muted-foreground outline-none hover:border-input hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
          >
            <PlusIcon className="size-3.5" aria-hidden="true" />
          </button>
          <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
            {workspaceTab.sqlTabs.length} tab{workspaceTab.sqlTabs.length === 1 ? "" : "s"}
          </span>
        </div>

        <div
          role="tabpanel"
          id={sqlQueryPanelElementId(tab.profileId, activeQueryTab.id)}
          aria-labelledby={sqlQueryTabElementId(tab.profileId, activeQueryTab.id)}
        >
          <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
            <Button
              type="button"
              size="sm"
              onClick={() => void runSource("auto")}
              disabled={!canRun}
              title={
                connected
                  ? "Run the selection, or the statement at the cursor (Ctrl+Enter)"
                  : "Connect this database profile before running SQL"
              }
            >
              {status === "analyzing" ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <PlayIcon className="size-3.5" aria-hidden="true" />
              )}
              Run
            </Button>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void runSource("all")}
              disabled={!canRun}
              title="Run every statement in this query tab (Ctrl+Shift+Enter)"
            >
              <PlayIcon className="size-3.5" aria-hidden="true" />
              Run all
            </Button>
            {busy ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => void handleCancel()}
                disabled={!running || cancelling}
                title={
                  running
                    ? "Cancel this execution and close the database session"
                    : "Waiting for SQL analysis"
                }
              >
                <StopIcon className="size-3.5" aria-hidden="true" />
                {cancelling ? "Cancelling..." : running ? "Cancel query" : "Analyzing..."}
              </Button>
            ) : null}
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={handleClear}
              disabled={busy || (activeQueryTab.sql.length === 0 && !queryEntry)}
              title="Clear this query tab's SQL and result"
            >
              Clear
            </Button>
            {!connected ? (
              <Button type="button" variant="secondary" size="sm" onClick={onConnect}>
                Connect
              </Button>
            ) : null}
            <span className="ml-auto hidden text-[10px] text-muted-foreground lg:inline">
              Ctrl+Enter current · Ctrl+Shift+Enter all
            </span>
          </div>

          <DatabaseSqlEditor
            id={`database-sql-editor-${tab.profileId}`}
            ariaLabel={`SQL editor for ${tab.name}`}
            value={activeQueryTab.sql}
            engine={workspaceTab.engine}
            placeholder="-- SELECT current_database();"
            selectionStart={activeQueryTab.selectionStart}
            onChange={(value) =>
              useDatabaseWorkspaceStore
                .getState()
                .setSqlQueryText(tab.profileId, activeQueryTab.id, value)
            }
            onSelectionChange={(start, end) =>
              useDatabaseWorkspaceStore
                .getState()
                .setSqlQuerySelection(tab.profileId, activeQueryTab.id, start, end)
            }
            onRun={() => void runSource("auto")}
            onRunAll={() => void runSource("all")}
          />

          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-border bg-muted/10 px-3 py-1.5">
            <label className="inline-flex items-center gap-1 text-[10px] text-muted-foreground">
              Rows
              <select
                aria-label="Result row limit"
                value={activeQueryTab.maxRows}
                disabled={busy}
                onChange={(event) =>
                  useDatabaseWorkspaceStore
                    .getState()
                    .setSqlQueryOptions(tab.profileId, activeQueryTab.id, {
                      maxRows: Number(event.currentTarget.value),
                    })
                }
                className="h-6 rounded-sm border border-border bg-card px-1.5 font-mono text-[10px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50"
              >
                {ROW_LIMIT_OPTIONS.map((value) => (
                  <option key={value} value={value}>
                    {value.toLocaleString()}
                  </option>
                ))}
              </select>
            </label>
            <label className="inline-flex items-center gap-1 text-[10px] text-muted-foreground">
              Timeout
              <select
                aria-label="Query timeout"
                value={activeQueryTab.timeoutMs}
                disabled={busy}
                onChange={(event) =>
                  useDatabaseWorkspaceStore
                    .getState()
                    .setSqlQueryOptions(tab.profileId, activeQueryTab.id, {
                      timeoutMs: Number(event.currentTarget.value),
                    })
                }
                className="h-6 rounded-sm border border-border bg-card px-1.5 font-mono text-[10px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50"
              >
                {TIMEOUT_OPTIONS.map((value) => (
                  <option key={value} value={value}>
                    {formatQueryDuration(value)}
                  </option>
                ))}
              </select>
            </label>
            <CursorReadout tab={activeQueryTab} />
            <span className="font-mono text-[10px] text-muted-foreground">
              {statementCount} statement{statementCount === 1 ? "" : "s"}
            </span>
            <span className="ml-auto truncate font-mono text-[10px] text-muted-foreground">
              {activeQueryTab.title} · {tab.engine === "mysql" ? "MySQL" : "PostgreSQL"} ·{" "}
              {database ?? "no database"} / {schema.value}
            </span>
          </div>
        </div>
      </section>

      {running ? (
        <RunningPanel
          timeoutMs={activeQueryTab.timeoutMs}
          startedAt={queryEntry?.startedAt}
          onCancel={() => void handleCancel()}
          cancelling={cancelling}
        />
      ) : null}

      {status === "analyzing" ? (
        <div
          role="status"
          className="flex items-center gap-2 rounded-md border border-border bg-card px-3 py-3 text-[11px] text-muted-foreground"
        >
          <CircleNotchIcon className="size-4 animate-spin text-primary" aria-hidden="true" />
          Analyzing the SQL for writes and destructive statements...
        </div>
      ) : null}

      {status === "blocked" ? (
        <BlockedPanel
          tab={tab}
          message={queryEntry?.errorMessage}
          analysis={queryEntry?.analysis}
          profileReadOnly={profileReadOnly}
        />
      ) : null}

      {error && status !== "blocked" ? (
        <ErrorPanel
          tab={tab}
          queryTab={activeQueryTab}
          error={error}
          onRetry={() => void runSource("auto")}
          onGoToError={goToError}
        />
      ) : null}

      {status === "cancelled" ? (
        <Alert variant="warning" role="status" aria-label={`Query cancelled for ${tab.name}`}>
          <StopIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Query cancelled</AlertTitle>
            <AlertDescription>
              {queryEntry?.errorMessage ??
                "The execution was cancelled. The database session was closed to keep the stream safe."}
            </AlertDescription>
            <p className="mt-1 text-[10px] text-muted-foreground">
              Reconnect this profile before running another query if the connection does not
              recover.
            </p>
          </div>
        </Alert>
      ) : null}

      {status === "succeeded" && result ? (
        <ResultPanel tab={tab} queryTab={activeQueryTab} queryEntry={queryEntry} />
      ) : null}

      {!result &&
      !error &&
      !busy &&
      status !== "blocked" &&
      status !== "cancelled" &&
      status !== "succeeded" ? (
        <div className="flex items-start gap-3 rounded-md border border-dashed border-border bg-card/35 px-4 py-5">
          <RowsIcon className="mt-0.5 size-4 text-muted-foreground" aria-hidden="true" />
          <div>
            <p className="text-[12px] font-medium text-foreground">No results yet</p>
            <p className="mt-0.5 max-w-[60ch] text-[11px] leading-relaxed text-muted-foreground">
              Run the current statement or selected SQL with the Run button, Ctrl+Enter, or
              Ctrl+Shift+Enter for every statement. Results, affected rows, duration, and bound
              notices stay with this query tab.
            </p>
          </div>
        </div>
      ) : null}

      <p className="flex items-center gap-1.5 px-1 text-[10px] leading-5 text-muted-foreground">
        <InfoIcon className="size-3 shrink-0" aria-hidden="true" />
        Results are bounded on the server. Copy uses TSV; exports download CSV or JSON for the
        selected result set.
      </p>

      <ConfirmDialog
        open={confirmation !== null}
        onOpenChange={(open) => {
          if (!open) {
            setConfirmation(null);
          }
        }}
        title="Run destructive SQL?"
        description={`This SQL can permanently change ${database ?? "the selected database"}. It will run on ${tab.name} only after you confirm.`}
        confirmLabel="Run destructive SQL"
        onConfirm={() => {
          const pending = confirmation;
          setConfirmation(null);
          if (pending) {
            void executeConfirmed(pending);
          }
        }}
        details={[
          { label: "Connection", value: tab.name },
          { label: "Engine", value: tab.engine === "mysql" ? "MySQL" : "PostgreSQL" },
          { label: "Database", value: database ?? "not selected" },
          { label: "Schema", value: schema.value },
          { label: "Statements", value: String(confirmation?.statementCount ?? 0) },
        ]}
        preview={confirmation ? { label: "SQL to execute", text: confirmation.sql } : null}
        destructive
        warningTitle="This SQL is destructive."
        warningDescription={
          confirmation?.analysis.warnings.map((warning) => warning.message).join(" ") ||
          "Review the SQL before running it."
        }
      />
    </div>
  );
}

function sqlQueryTabElementId(profileId: string, queryTabId: string): string {
  return `database-sql-query-tab-${profileId}-${queryTabId}`;
}

function sqlQueryPanelElementId(profileId: string, queryTabId: string): string {
  return `database-sql-query-panel-${profileId}-${queryTabId}`;
}
