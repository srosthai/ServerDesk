import { lazy, Suspense, useMemo, useState } from "react";
import {
  ArrowClockwiseIcon,
  CaretRightIcon,
  DatabaseIcon,
  InfoIcon,
  MagnifyingGlassIcon,
  PlugsIcon,
  ShareNetworkIcon,
  TableIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorState } from "@/components/ui/states";
import {
  ER_DIAGRAM_DEFAULT_TABLE_LIMIT,
  ER_DIAGRAM_TABLE_LIMIT_OPTIONS,
  buildErDiagram,
  erDiagramCardinalityLabel,
  erDiagramRelationshipColumns,
  erDiagramSelectionForTable,
  erDiagramTableCountLabel,
  type ErDiagramTable,
} from "@/features/databases/databaseErDiagramModel";
import { DATABASE_ENGINE_LABELS } from "@/features/databases/databaseProfileForm";
import type { DatabaseExplorerSelection } from "@/features/databases/databaseExplorerTreeModel";
import type { DatabaseTableScope } from "@/features/databases/databaseStructureModel";
import { useDatabaseErDiagram } from "@/hooks/useDatabaseErDiagram";
import { useResolvedTheme } from "@/hooks/useResolvedTheme";
import { cn } from "@/lib/utils";
import type { DatabaseEngine, DatabaseProfile } from "@/types/contracts";

const DatabaseErDiagramCanvas = lazy(() => import("@/features/databases/DatabaseErDiagramCanvas"));

type DatabaseErDiagramViewProps = {
  profile: DatabaseProfile;
  profileName: string;
  engine: DatabaseEngine;
  connected: boolean;
  database: string | null;
  schema: string | null;
  selectedTable: DatabaseTableScope | null;
  onSelectTable: (selection: DatabaseExplorerSelection) => void;
  onConnect: () => void;
};

function DiagramLoading({ message }: { message: string }) {
  return (
    <div className="space-y-3" aria-busy="true" aria-label="Loading ER diagram">
      <div className="flex flex-wrap items-center gap-2">
        <Skeleton className="h-8 w-[18rem]" />
        <Skeleton className="h-8 w-[9.5rem]" />
        <Skeleton className="h-6 w-[8rem] rounded-full" />
      </div>
      <div className="relative h-[32rem] min-h-[24rem] overflow-hidden rounded-md border border-border bg-muted/20 p-6">
        <div className="grid grid-cols-2 gap-5 opacity-70 md:grid-cols-3">
          {Array.from({ length: 6 }).map((_, index) => (
            <div
              key={index}
              className="h-[6.2rem] rounded-md border border-border bg-card p-3"
              style={{ marginTop: `${(index % 3) * 14}px` }}
            >
              <Skeleton className="h-3 w-[62%]" />
              <Skeleton className="mt-2 h-2.5 w-[38%]" />
              <Skeleton className="mt-5 h-2 w-[72%]" />
            </div>
          ))}
        </div>
        <p className="absolute inset-x-0 bottom-4 text-center text-[11px] text-muted-foreground">
          {message}
        </p>
      </div>
    </div>
  );
}

function formatLoadedTime(updatedAt: number | null): string | null {
  if (!updatedAt) {
    return null;
  }
  return new Date(updatedAt).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export default function DatabaseErDiagramView({
  profile,
  profileName,
  engine,
  connected,
  database,
  schema,
  selectedTable,
  onSelectTable,
  onConnect,
}: DatabaseErDiagramViewProps) {
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(ER_DIAGRAM_DEFAULT_TABLE_LIMIT);
  const colorMode = useResolvedTheme();
  const diagram = useDatabaseErDiagram({
    profileId: profile.id,
    database,
    schema,
    connected,
    query,
    limit,
  });

  const graph = useMemo(() => {
    if (diagram.status !== "ready") {
      return null;
    }
    return buildErDiagram({
      tables: diagram.tables,
      constraintsByTable: diagram.constraints,
      query,
      limit,
      totalTableCount: diagram.totalTables,
      catalogTruncated: diagram.catalogTruncated,
    });
  }, [
    diagram.catalogTruncated,
    diagram.constraints,
    diagram.status,
    diagram.tables,
    diagram.totalTables,
    limit,
    query,
  ]);

  const selectedKey = useMemo(() => {
    if (!graph || !selectedTable) {
      return null;
    }
    return (
      graph.tables.find(
        (table) =>
          table.name === selectedTable.table &&
          (selectedTable.database === null || table.database === selectedTable.database),
      )?.key ?? null
    );
  }, [graph, selectedTable]);

  function handleTableSelection(table: ErDiagramTable): void {
    const selection = erDiagramSelectionForTable(profile, table);
    if (selection) {
      onSelectTable(selection);
    }
  }

  const header = (
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0">
        <h2 className="flex items-center gap-1.5 text-[13px] font-semibold tracking-[-0.01em] text-foreground">
          <ShareNetworkIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
          Entity relationship diagram
        </h2>
        <p className="mt-0.5 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
          <span className="font-mono text-foreground">
            {database ?? "No database"}
            {schema ? ` / ${schema}` : engine === "mysql" ? "" : " / all schemas"}
          </span>
          <Badge variant="outline">{DATABASE_ENGINE_LABELS[engine]}</Badge>
          <span>Read-only foreign keys · no schema changes</span>
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {formatLoadedTime(diagram.updatedAt) ? (
          <span className="text-[10px] text-muted-foreground">
            Loaded {formatLoadedTime(diagram.updatedAt)}
          </span>
        ) : null}
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="h-7 px-2 text-[11px]"
          onClick={diagram.reload}
          disabled={!connected || diagram.status === "loading" || diagram.constraintsLoading}
          aria-label="Reload schema relationship metadata"
        >
          <ArrowClockwiseIcon
            className={cn(
              "size-3.5",
              (diagram.status === "loading" || diagram.constraintsLoading) && "animate-spin",
            )}
            aria-hidden="true"
          />
          Refresh
        </Button>
      </div>
    </div>
  );

  if (!connected) {
    return (
      <div className="space-y-4">
        {header}
        <Alert variant="warning">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div className="flex-1 space-y-2">
            <AlertTitle>Connection required</AlertTitle>
            <AlertDescription>
              Connect {profileName} before reading tables and foreign keys for the schema diagram.
            </AlertDescription>
            <Button variant="secondary" size="sm" onClick={onConnect}>
              <PlugsIcon className="size-3.5" aria-hidden="true" />
              Connect
            </Button>
          </div>
        </Alert>
      </div>
    );
  }

  if (!database) {
    return (
      <div className="space-y-4">
        {header}
        <EmptyState
          icon={<DatabaseIcon className="size-4" aria-hidden="true" />}
          title="Select a database or schema"
          description="Choose a database in the explorer to draw its tables and foreign-key relationships. The diagram is read-only and never changes the schema."
        />
      </div>
    );
  }

  if (diagram.status === "error") {
    return (
      <div className="space-y-4">
        {header}
        <ErrorState
          title={
            diagram.error?.code === "permission"
              ? "Relationship metadata is not available"
              : "Could not load schema metadata"
          }
          message={
            diagram.error
              ? diagram.error.code === "permission"
                ? `${diagram.error.message} The database account may need permission to read the catalog views that describe tables and foreign keys.`
                : diagram.error.message
              : "The database did not return table metadata."
          }
          onRetry={diagram.reload}
        />
      </div>
    );
  }

  if (diagram.status === "loading") {
    return (
      <div className="space-y-4">
        {header}
        <DiagramLoading message="Reading table names and relationship metadata from the catalog." />
      </div>
    );
  }

  if (diagram.status !== "ready" || !graph) {
    return (
      <div className="space-y-4">
        {header}
        <EmptyState
          icon={<DatabaseIcon className="size-4" aria-hidden="true" />}
          title="No table metadata loaded"
          description="Open a live connection and refresh the explorer before drawing an entity relationship diagram."
        />
      </div>
    );
  }

  if (diagram.tables.length === 0) {
    return (
      <div className="space-y-4">
        {header}
        <EmptyState
          icon={<TableIcon className="size-4" aria-hidden="true" />}
          title="No tables in this schema"
          description={`${profileName} reports no base tables for ${database}${schema ? ` / ${schema}` : ""}. The ER diagram is empty because there is nothing to relate yet.`}
        />
      </div>
    );
  }

  const constraintsPending = diagram.constraintsLoading;
  const filteredNoMatches = graph.tables.length === 0 && query.trim().length > 0;
  const failure = diagram.constraintFailures[0] ?? null;

  return (
    <div className="space-y-4">
      {header}

      <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Diagram filters">
        <div className="relative min-w-[16rem] flex-1">
          <MagnifyingGlassIcon
            className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Filter tables by name or schema"
            aria-label="Filter diagram tables"
            className="h-8 pr-8 pl-8 text-[12px]"
          />
          {query ? (
            <button
              type="button"
              onClick={() => setQuery("")}
              aria-label="Clear table filter"
              className="absolute top-1/2 right-1.5 grid size-6 -translate-y-1/2 place-items-center rounded-sm text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
            >
              <XIcon className="size-3.5" aria-hidden="true" />
            </button>
          ) : null}
        </div>
        <div className="flex items-center gap-1.5">
          <label
            htmlFor="er-diagram-table-limit"
            className="text-[11px] whitespace-nowrap text-muted-foreground"
          >
            Table limit
          </label>
          <Select value={String(limit)} onValueChange={(value) => setLimit(Number(value))}>
            <SelectTrigger
              id="er-diagram-table-limit"
              aria-label="Maximum tables to draw"
              className="h-8 w-[7rem] text-[12px]"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ER_DIAGRAM_TABLE_LIMIT_OPTIONS.map((option) => (
                <SelectItem key={option} value={String(option)}>
                  {option} tables
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <span
          className="rounded-full border border-border bg-card px-2.5 py-1 text-[11px] text-muted-foreground"
          aria-live="polite"
        >
          {erDiagramTableCountLabel(graph.stats)} ·{" "}
          {constraintsPending
            ? `loading ${diagram.constraintLoaded} of ${diagram.constraintTotal} relationships`
            : `${graph.stats.visibleRelationships} relationships`}
        </span>
      </div>

      {failure ? (
        <Alert variant="warning" className="py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Relationships may be incomplete</AlertTitle>
            <AlertDescription>
              {diagram.constraintFailures.length} table
              {diagram.constraintFailures.length === 1 ? "" : "s"} could not return constraint
              metadata, starting with {failure.table}: {failure.error.message}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {graph.stats.hiddenByLimit > 0 ? (
        <Alert variant="warning" className="py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Diagram table limit reached</AlertTitle>
            <AlertDescription>
              {graph.stats.hiddenByLimit.toLocaleString()} more matching table
              {graph.stats.hiddenByLimit === 1 ? " is" : "s are"} not drawn. Narrow the filter or
              raise the table limit to inspect them.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {diagram.catalogTruncated || graph.stats.unloadedTables > 0 ? (
        <Alert variant="info" className="py-2.5">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Large schema</AlertTitle>
            <AlertDescription>
              The catalog returned {graph.stats.loadedTables.toLocaleString()} table names
              {graph.stats.unloadedTables > 0
                ? ` and reports ${graph.stats.unloadedTables.toLocaleString()} more outside the loaded catalog page`
                : ""}
              . Relationships are read only for the tables that are drawn.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {graph.stats.unresolvedForeignKeys > 0 ? (
        <Alert variant="info" className="py-2.5">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Some foreign keys point outside this view</AlertTitle>
            <AlertDescription>
              {graph.stats.unresolvedForeignKeys} foreign key
              {graph.stats.unresolvedForeignKeys === 1 ? "" : "s"} reference a table that is not in
              the loaded catalog, so no relationship line is drawn for them.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {graph.stats.truncatedRelationships ? (
        <Alert variant="warning" className="py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Relationship limit reached</AlertTitle>
            <AlertDescription>
              Only the first {graph.stats.visibleRelationships} relationships are drawn. Filter by
              table name to inspect a smaller part of this schema.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {!constraintsPending && graph.stats.visibleRelationships === 0 && graph.tables.length > 0 ? (
        <Alert variant="info" className="py-2.5">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>
              {graph.stats.totalRelationships > 0
                ? "No relationships connect the filtered tables"
                : "No foreign-key relationships found"}
            </AlertTitle>
            <AlertDescription>
              {graph.stats.totalRelationships > 0
                ? `${graph.stats.totalRelationships} relationship${graph.stats.totalRelationships === 1 ? " exists" : "s exist"} outside this filter. Clear the search or raise the table limit to view them.`
                : "Tables without foreign keys are still shown so isolated objects remain visible. They are not a schema error."}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {filteredNoMatches ? (
        <EmptyState
          icon={<MagnifyingGlassIcon className="size-4" aria-hidden="true" />}
          title="No tables match this filter"
          description={`No loaded table name or schema matches “${query.trim()}”. Clear the filter to restore the full diagram.`}
          action={
            <Button variant="secondary" size="sm" onClick={() => setQuery("")}>
              Clear filter
            </Button>
          }
        />
      ) : constraintsPending ? (
        <DiagramLoading
          message={`Loading relationship metadata for ${diagram.constraintLoaded} of ${diagram.constraintTotal} visible tables.`}
        />
      ) : (
        <Suspense fallback={<DiagramLoading message="Loading the diagram renderer." />}>
          <DatabaseErDiagramCanvas
            graph={graph}
            selectedKey={selectedKey}
            database={database}
            schema={schema}
            colorMode={colorMode}
            onSelectTable={handleTableSelection}
          />
        </Suspense>
      )}

      {!filteredNoMatches && !constraintsPending ? (
        <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-muted-foreground">
          <span>Drag to pan</span>
          <span aria-hidden="true">·</span>
          <span>Scroll or pinch to zoom</span>
          <span aria-hidden="true">·</span>
          <span>Select a table to open Structure</span>
          {graph.stats.isolatedTables > 0 ? (
            <>
              <span aria-hidden="true">·</span>
              <span>
                {graph.stats.isolatedTables} isolated table
                {graph.stats.isolatedTables === 1 ? "" : "s"}
              </span>
            </>
          ) : null}
        </p>
      ) : null}

      {!filteredNoMatches && !constraintsPending ? (
        <details className="group rounded-md border border-border bg-card">
          <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-2.5 text-[12px] font-medium text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring/40 [&::-webkit-details-marker]:hidden">
            <span className="flex items-center gap-1.5">
              <CaretRightIcon
                className="size-3.5 text-muted-foreground transition-transform group-open:rotate-90"
                aria-hidden="true"
              />
              <TableIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
              Table and relationship index
            </span>
            <span className="text-[11px] font-normal text-muted-foreground">
              {graph.tables.length} tables · {graph.relationships.length} relationships
            </span>
          </summary>
          <div className="grid gap-4 border-t border-border p-3 md:grid-cols-2">
            <section aria-labelledby="er-table-index-heading">
              <h3
                id="er-table-index-heading"
                className="text-[11px] font-semibold tracking-[0.06em] text-muted-foreground uppercase"
              >
                Tables
              </h3>
              <ul className="mt-1.5 max-h-64 divide-y divide-border overflow-auto rounded-sm border border-border">
                {graph.tables.map((table) => (
                  <li key={table.key}>
                    <button
                      type="button"
                      onClick={() => handleTableSelection(table)}
                      className={cn(
                        "flex w-full items-center justify-between gap-3 px-2.5 py-1.5 text-left text-[11px] outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/40",
                        table.key === selectedKey && "bg-primary/5",
                      )}
                      aria-label={`Open structure for ${table.qualifiedName}`}
                    >
                      <span className="truncate font-mono text-foreground">
                        {table.qualifiedName}
                      </span>
                      <span className="shrink-0 text-muted-foreground">
                        {table.incoming} in · {table.outgoing} out
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </section>
            <section aria-labelledby="er-relationship-index-heading">
              <h3
                id="er-relationship-index-heading"
                className="text-[11px] font-semibold tracking-[0.06em] text-muted-foreground uppercase"
              >
                Relationships
              </h3>
              {graph.relationships.length === 0 ? (
                <p className="mt-2 rounded-sm border border-dashed border-border px-2.5 py-3 text-[11px] text-muted-foreground">
                  No foreign-key relationships are visible for this table selection.
                </p>
              ) : (
                <ul className="mt-1.5 max-h-64 divide-y divide-border overflow-auto rounded-sm border border-border">
                  {graph.relationships.map((relationship) => (
                    <li key={relationship.id} className="px-2.5 py-2 text-[11px]">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="font-mono text-foreground">
                          {relationship.sourceTable}
                        </span>
                        <span aria-hidden="true" className="text-muted-foreground">
                          →
                        </span>
                        <span className="font-mono text-foreground">
                          {relationship.targetTable}
                        </span>
                        <Badge variant="outline" className="px-1.5 py-0 text-[9px]">
                          {erDiagramCardinalityLabel(relationship.cardinality)}
                        </Badge>
                      </div>
                      <p className="mt-0.5 text-muted-foreground">
                        {erDiagramRelationshipColumns(relationship)} · {relationship.name}
                        {relationship.onDelete ? ` · on delete ${relationship.onDelete}` : ""}
                      </p>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </div>
        </details>
      ) : null}
    </div>
  );
}
