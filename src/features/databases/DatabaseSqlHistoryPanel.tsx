import { useEffect, useMemo, useState } from "react";
import {
  ClockCounterClockwiseIcon,
  FloppyDiskIcon,
  MagnifyingGlassIcon,
  PlusIcon,
  ShieldCheckIcon,
  TrashIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { EngineBadge } from "@/features/databases/DatabaseConnectionList";
import {
  buildDatabaseHistoryFilter,
  DATABASE_QUERY_RESULT_BADGES,
  DATABASE_QUERY_RESULT_LABELS,
  EMPTY_DATABASE_HISTORY_FILTER,
  filterDatabaseSnippets,
  formatHistoryDuration,
  formatHistoryTimestamp,
  historyFiltersActive,
  historyTimestampValue,
  snippetCompatibilityReason,
  snippetEngineLabel,
  snippetPreview,
  type DatabaseHistoryFilterState,
} from "@/features/databases/databaseHistoryModel";
import { useDatabaseHistoryStore } from "@/stores/databaseHistoryStore";
import type {
  DatabaseEngine,
  DatabaseProfile,
  DatabaseQueryHistoryEntry,
  DatabaseSavedQuery,
} from "@/types/contracts";

const DISPLAYED_HISTORY_LIMIT = 50;

type DatabaseSqlHistoryPanelProps = {
  profileId: string;
  serverId: string;
  engine: DatabaseEngine;
  connectionName: string;
  currentSql: string;
  connections: readonly DatabaseProfile[];
  /** Changes after every execution so the panel reloads the newest row. */
  refreshToken?: number;
  onInsertSnippet: (snippet: DatabaseSavedQuery) => void;
};

function connectionLabel(
  entry: DatabaseQueryHistoryEntry,
  connections: readonly DatabaseProfile[],
): string {
  return (
    connections.find((connection) => connection.id === entry.connectionId)?.name ??
    "Deleted connection"
  );
}

function HistoryRow({
  entry,
  connections,
}: {
  entry: DatabaseQueryHistoryEntry;
  connections: readonly DatabaseProfile[];
}) {
  const preview = snippetPreview(entry.sqlText, 180);
  return (
    <li className="border-b border-border/70 px-3 py-2 last:border-0">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <Badge variant={DATABASE_QUERY_RESULT_BADGES[entry.result]}>
          {DATABASE_QUERY_RESULT_LABELS[entry.result]}
        </Badge>
        <time
          dateTime={historyTimestampValue(entry.startedAt)}
          className="text-[10px] text-muted-foreground"
        >
          {formatHistoryTimestamp(entry.startedAt)}
        </time>
        <span className="font-mono text-[10px] text-muted-foreground">
          {formatHistoryDuration(entry.durationMs)}
        </span>
        <span className="ml-auto max-w-[16ch] truncate text-[10px] text-muted-foreground">
          {connectionLabel(entry, connections)}
        </span>
      </div>
      <p
        className="mt-1 break-words font-mono text-[11px] leading-5 text-foreground"
        title={entry.sqlText}
      >
        {preview}
      </p>
      <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
        {entry.databaseName ? <span>{entry.databaseName}</span> : null}
        {entry.rowCount !== null ? <span>{entry.rowCount} rows</span> : null}
        {entry.errorSummary ? (
          <span
            className="line-clamp-1 max-w-[52ch] text-destructive/90"
            title={entry.errorSummary}
          >
            {entry.errorSummary}
          </span>
        ) : null}
      </div>
    </li>
  );
}

function SnippetRow({
  snippet,
  engine,
  onInsert,
  onDelete,
}: {
  snippet: DatabaseSavedQuery;
  engine: DatabaseEngine;
  onInsert: (snippet: DatabaseSavedQuery) => void;
  onDelete: (snippet: DatabaseSavedQuery) => void;
}) {
  const incompatibleReason = snippetCompatibilityReason(snippet, engine);
  return (
    <li className="border-b border-border/70 px-3 py-2 last:border-0">
      <div className="flex flex-wrap items-center gap-2">
        <p className="min-w-0 flex-1 truncate text-[12px] font-medium text-foreground">
          {snippet.name}
        </p>
        <EngineBadge engine={snippet.engine} />
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="h-7 px-2 text-[11px]"
          aria-label={`Insert snippet ${snippet.name}`}
          disabled={incompatibleReason !== null}
          title={incompatibleReason ?? `Insert ${snippet.name} into the current query tab`}
          onClick={() => onInsert(snippet)}
        >
          Insert
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-[11px] text-destructive hover:text-destructive"
          aria-label={`Delete snippet ${snippet.name}`}
          title={`Delete ${snippet.name}`}
          onClick={() => onDelete(snippet)}
        >
          <TrashIcon className="size-3.5" aria-hidden="true" />
        </Button>
      </div>
      <p
        className="mt-1 line-clamp-2 break-words font-mono text-[10px] leading-4 text-muted-foreground"
        title={snippet.sqlText}
      >
        {snippetPreview(snippet.sqlText, 180)}
      </p>
      {incompatibleReason ? (
        <p className="mt-1 text-[10px] text-warning">{incompatibleReason}</p>
      ) : null}
    </li>
  );
}

export function DatabaseSqlHistoryPanel({
  profileId,
  serverId,
  engine,
  connectionName,
  currentSql,
  connections,
  refreshToken,
  onInsertSnippet,
}: DatabaseSqlHistoryPanelProps) {
  const history = useDatabaseHistoryStore((state) => state.history);
  const snippets = useDatabaseHistoryStore((state) => state.snippets);
  const policy = useDatabaseHistoryStore((state) => state.policy);
  const historyStatus = useDatabaseHistoryStore((state) => state.historyStatus);
  const snippetStatus = useDatabaseHistoryStore((state) => state.snippetStatus);
  const historyError = useDatabaseHistoryStore((state) => state.historyError);
  const snippetError = useDatabaseHistoryStore((state) => state.snippetError);
  const loadHistory = useDatabaseHistoryStore((state) => state.loadHistory);
  const loadSnippets = useDatabaseHistoryStore((state) => state.loadSnippets);
  const loadPolicy = useDatabaseHistoryStore((state) => state.loadPolicy);
  const clearHistory = useDatabaseHistoryStore((state) => state.clearHistory);
  const createSnippet = useDatabaseHistoryStore((state) => state.createSnippet);
  const deleteSnippet = useDatabaseHistoryStore((state) => state.deleteSnippet);

  const [filters, setFilters] = useState<DatabaseHistoryFilterState>({
    ...EMPTY_DATABASE_HISTORY_FILTER,
    connectionId: profileId,
  });
  const [snippetQuery, setSnippetQuery] = useState("");
  const [draftName, setDraftName] = useState("");
  const [draftSql, setDraftSql] = useState("");
  const [draftEngine, setDraftEngine] = useState<DatabaseEngine>(engine);
  const [savingSnippet, setSavingSnippet] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<DatabaseSavedQuery | null>(null);
  const [deletingSnippet, setDeletingSnippet] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    void loadPolicy();
    void loadSnippets();
  }, [loadPolicy, loadSnippets]);

  useEffect(() => {
    void loadHistory(
      buildDatabaseHistoryFilter(filters, {
        serverId,
        limit: policy.maxEntries,
      }),
    );
  }, [filters, loadHistory, policy.maxEntries, refreshToken, serverId]);

  const visibleHistory = useMemo(() => history.slice(0, DISPLAYED_HISTORY_LIMIT), [history]);
  const visibleSnippets = useMemo(
    () => filterDatabaseSnippets(snippets, snippetQuery),
    [snippetQuery, snippets],
  );
  const filtersActive = historyFiltersActive(filters);

  function updateFilter(patch: Partial<DatabaseHistoryFilterState>) {
    setFilters((current) => ({ ...current, ...patch }));
  }

  function clearFilters() {
    setFilters({ ...EMPTY_DATABASE_HISTORY_FILTER });
  }

  async function saveSnippet() {
    if (!draftName.trim() || !draftSql.trim()) {
      return;
    }
    setSavingSnippet(true);
    const created = await createSnippet({
      name: draftName,
      engine: draftEngine,
      sql: draftSql,
    });
    setSavingSnippet(false);
    if (created) {
      setDraftName("");
      setDraftSql("");
      setNotice(`Saved snippet "${created.name}".`);
    }
  }

  async function confirmDeleteSnippet() {
    if (!deleteTarget) {
      return;
    }
    setDeletingSnippet(true);
    const deleted = await deleteSnippet(deleteTarget.id, true);
    setDeletingSnippet(false);
    if (deleted) {
      setNotice(`Deleted snippet "${deleteTarget.name}".`);
      setDeleteTarget(null);
    }
  }

  async function confirmClearHistory() {
    setClearing(true);
    const cleared = await clearHistory(true);
    setClearing(false);
    if (cleared) {
      setNotice("Query history cleared.");
      setClearOpen(false);
    }
  }

  return (
    <section
      aria-label="Query history and saved snippets"
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-center gap-2 border-b border-border bg-muted/15 px-3 py-2">
        <ClockCounterClockwiseIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
        <h2 className="text-[12px] font-semibold text-foreground">Query history &amp; snippets</h2>
        <span className="font-mono text-[10px] text-muted-foreground">
          {history.length} matching {history.length === 1 ? "query" : "queries"}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="ml-auto h-7 px-2 text-[11px] text-destructive hover:text-destructive"
          onClick={() => setClearOpen(true)}
          disabled={historyStatus === "loading"}
        >
          <TrashIcon className="size-3.5" aria-hidden="true" />
          Clear history
        </Button>
      </div>

      <div className="border-b border-border bg-info/5 px-3 py-2">
        <p className="flex items-start gap-1.5 text-[10px] leading-4 text-muted-foreground">
          <ShieldCheckIcon className="mt-0.5 size-3 shrink-0 text-info" aria-hidden="true" />
          <span>
            SQL and error summaries are sanitized before storage. Result grids, parameters,
            passwords, keys, tokens, and dotenv content are never stored. {policy.retention}
          </span>
        </p>
      </div>

      <div className="grid gap-0 xl:grid-cols-2">
        <div className="min-w-0 border-b border-border xl:border-r xl:border-b-0">
          <div className="space-y-2 border-b border-border px-3 py-2">
            <div className="grid gap-2 sm:grid-cols-2">
              <label className="min-w-0 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                Connection
                <Select
                  value={filters.connectionId}
                  onValueChange={(value) => updateFilter({ connectionId: value })}
                >
                  <SelectTrigger aria-label="History connection filter" className="mt-1 h-8">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All connections</SelectItem>
                    {connections.map((connection) => (
                      <SelectItem key={connection.id} value={connection.id}>
                        {connection.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <label className="min-w-0 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                Result
                <Select
                  value={filters.result}
                  onValueChange={(value) =>
                    updateFilter({
                      result: value as DatabaseHistoryFilterState["result"],
                    })
                  }
                >
                  <SelectTrigger aria-label="History result filter" className="mt-1 h-8">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All results</SelectItem>
                    <SelectItem value="success">Success</SelectItem>
                    <SelectItem value="failure">Failure</SelectItem>
                    <SelectItem value="cancelled">Cancelled</SelectItem>
                  </SelectContent>
                </Select>
              </label>
              <label className="min-w-0 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                From date
                <Input
                  aria-label="History from date"
                  type="date"
                  value={filters.dateFrom}
                  onChange={(event) => updateFilter({ dateFrom: event.target.value })}
                  className="mt-1 h-8 text-[12px]"
                />
              </label>
              <label className="min-w-0 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                To date
                <Input
                  aria-label="History to date"
                  type="date"
                  value={filters.dateTo}
                  onChange={(event) => updateFilter({ dateTo: event.target.value })}
                  className="mt-1 h-8 text-[12px]"
                />
              </label>
            </div>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-[10px] text-muted-foreground" aria-live="polite">
                {historyStatus === "loading"
                  ? "Loading query history..."
                  : filtersActive
                    ? `${history.length} matching ${history.length === 1 ? "query" : "queries"}`
                    : `${history.length} recorded ${history.length === 1 ? "query" : "queries"}`}
              </p>
              {filtersActive ? (
                <Button type="button" variant="ghost" size="sm" onClick={clearFilters}>
                  Clear filters
                </Button>
              ) : null}
            </div>
            {historyError ? (
              <p role="alert" className="flex items-center gap-1.5 text-[10px] text-destructive">
                <WarningCircleIcon className="size-3" aria-hidden="true" />
                {historyError}
              </p>
            ) : null}
          </div>

          {visibleHistory.length > 0 ? (
            <ul aria-label="Query history entries" className="max-h-[24rem] overflow-auto">
              {visibleHistory.map((entry) => (
                <HistoryRow key={entry.id} entry={entry} connections={connections} />
              ))}
            </ul>
          ) : (
            <EmptyState
              className="m-3 border-0 bg-transparent px-3 py-6"
              icon={<ClockCounterClockwiseIcon className="size-4" aria-hidden="true" />}
              title={filtersActive ? "No queries match these filters" : "No query history yet"}
              description={
                filtersActive
                  ? "Try another connection, result, or date range. Recorded history is unchanged."
                  : `Queries executed on ${connectionName} appear here with their sanitized SQL, result, duration, and timestamp.`
              }
            />
          )}
          {history.length > DISPLAYED_HISTORY_LIMIT ? (
            <p className="border-t border-border px-3 py-1.5 text-[10px] text-muted-foreground">
              Showing the newest {DISPLAYED_HISTORY_LIMIT} of {history.length} matching queries.
              Narrow the filters to find older entries.
            </p>
          ) : null}
        </div>

        <div className="min-w-0">
          <div className="space-y-2 border-b border-border px-3 py-2">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-[11px] font-semibold text-foreground">Saved snippets</h3>
              <span className="font-mono text-[10px] text-muted-foreground">
                {snippetStatus === "loading" ? "Loading..." : `${snippets.length} saved`}
              </span>
              <div className="relative ml-auto min-w-[10rem] flex-1 sm:max-w-56">
                <MagnifyingGlassIcon
                  className="pointer-events-none absolute top-1/2 left-2 size-3 -translate-y-1/2 text-muted-foreground"
                  aria-hidden="true"
                />
                <Input
                  aria-label="Search snippets"
                  value={snippetQuery}
                  onChange={(event) => setSnippetQuery(event.target.value)}
                  placeholder="Search snippets"
                  className="h-8 pl-7 text-[12px]"
                />
              </div>
            </div>
            <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_9rem_auto]">
              <Input
                aria-label="Snippet name"
                value={draftName}
                onChange={(event) => setDraftName(event.target.value)}
                placeholder="Snippet name"
                maxLength={policy.snippetNameMaxChars}
                className="h-8 text-[12px]"
              />
              <Select
                value={draftEngine}
                onValueChange={(value) => setDraftEngine(value as DatabaseEngine)}
              >
                <SelectTrigger aria-label="Snippet engine" className="h-8">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="postgresql">PostgreSQL</SelectItem>
                  <SelectItem value="mysql">MySQL</SelectItem>
                </SelectContent>
              </Select>
              <Button
                type="button"
                size="sm"
                className="h-8"
                onClick={() => setDraftSql(currentSql)}
                disabled={!currentSql.trim()}
                title="Copy the current query tab SQL into this snippet"
              >
                Use current SQL
              </Button>
            </div>
            <Textarea
              aria-label="Snippet SQL"
              value={draftSql}
              onChange={(event) => setDraftSql(event.target.value)}
              placeholder="SELECT * FROM users WHERE active = true"
              maxLength={policy.snippetSqlMaxChars}
              className="min-h-16 font-mono text-[11px]"
            />
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-[10px] text-muted-foreground">
                Snippets are local and reusable on any connection with a matching engine.
              </p>
              <Button
                type="button"
                size="sm"
                className="h-8"
                onClick={() => void saveSnippet()}
                disabled={savingSnippet || !draftName.trim() || !draftSql.trim()}
              >
                <PlusIcon className="size-3.5" aria-hidden="true" />
                {savingSnippet ? "Saving..." : "Save snippet"}
              </Button>
            </div>
            {snippetError ? (
              <p role="alert" className="flex items-center gap-1.5 text-[10px] text-destructive">
                <WarningCircleIcon className="size-3" aria-hidden="true" />
                {snippetError}
              </p>
            ) : null}
            {notice ? (
              <p role="status" className="text-[10px] text-muted-foreground">
                {notice}
              </p>
            ) : null}
          </div>

          {visibleSnippets.length > 0 ? (
            <ul aria-label="Saved query snippets" className="max-h-[24rem] overflow-auto">
              {visibleSnippets.map((snippet) => (
                <SnippetRow
                  key={snippet.id}
                  snippet={snippet}
                  engine={engine}
                  onInsert={(selected) => {
                    onInsertSnippet(selected);
                    setNotice(`Inserted "${selected.name}" into the current query tab.`);
                  }}
                  onDelete={setDeleteTarget}
                />
              ))}
            </ul>
          ) : (
            <EmptyState
              className="m-3 border-0 bg-transparent px-3 py-6"
              icon={<FloppyDiskIcon className="size-4" aria-hidden="true" />}
              title={snippetQuery ? "No snippets match this search" : "No saved snippets yet"}
              description={
                snippetQuery
                  ? "Clear the search to see every saved snippet."
                  : "Save a query once, then insert it into any matching query tab. Stored SQL is sanitized first."
              }
            />
          )}
        </div>
      </div>

      <ConfirmDialog
        open={clearOpen}
        onOpenChange={(open) => {
          if (!open && !clearing) {
            setClearOpen(false);
          }
        }}
        title="Clear query history?"
        description="Every stored query-history entry on this machine is deleted. Saved snippets are not affected."
        confirmLabel="Clear history"
        onConfirm={() => void confirmClearHistory()}
        pending={clearing}
        details={[
          { label: "Matching entries", value: String(history.length) },
          { label: "Connection", value: connectionName },
          { label: "Retention", value: `Newest ${policy.maxEntries.toLocaleString()} kept` },
        ]}
        warningTitle="This cannot be undone."
        warningDescription="Only the sanitized query history is removed. Results and snippets remain unchanged."
      />

      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deletingSnippet) {
            setDeleteTarget(null);
          }
        }}
        title={deleteTarget ? `Delete snippet "${deleteTarget.name}"?` : "Delete snippet?"}
        description="The saved SQL body is removed from this machine. Query tabs that already contain the SQL are unchanged."
        confirmLabel="Delete snippet"
        onConfirm={() => void confirmDeleteSnippet()}
        pending={deletingSnippet}
        details={
          deleteTarget
            ? [
                { label: "Snippet", value: deleteTarget.name },
                { label: "Engine", value: snippetEngineLabel(deleteTarget.engine) },
              ]
            : []
        }
        warningTitle="This removes the saved snippet."
        warningDescription="You can save the SQL again from a query tab if needed."
      />
    </section>
  );
}
