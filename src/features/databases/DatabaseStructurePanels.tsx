import {
  ArrowClockwiseIcon,
  BroomIcon,
  CheckCircleIcon,
  CopyIcon,
  DatabaseIcon,
  KeyIcon,
  LinkIcon,
  PencilSimpleIcon,
  PlugsIcon,
  PlusIcon,
  SlidersHorizontalIcon,
  TableIcon,
  TextTIcon,
  TrashIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { ErrorState, LoadingState } from "@/components/ui/states";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  buildTableRelationships,
  columnDefaultLabel,
  columnGeneratedLabel,
  columnIdentityLabel,
  columnTypeLabel,
  constraintActionDecision,
  constraintKindLabel,
  dependencyGroups,
  dependencyObjectLabel,
  formatMetadataValue,
  indexActionDecisions,
  qualifiedTableName,
  tableSummaryRows,
  unavailableDecisions,
  type DatabaseStructureIntent,
  type DatabaseTableScope,
  type StructureCapabilities,
  type StructureDecision,
  type TableRelationship,
} from "@/features/databases/databaseStructureModel";
import type { DatabaseTableStructureState } from "@/hooks/useDatabaseTableStructure";
import type {
  CommandError,
  DatabaseColumnSummary,
  DatabaseConstraintSummary,
  DatabaseEngine,
  DatabaseIndexSummary,
  DatabaseTableSummary,
} from "@/types/contracts";
import { cn } from "@/lib/utils";

export type DatabaseStructurePanelProps = {
  scope: DatabaseTableScope;
  profileName: string;
  engine: DatabaseEngine;
  connected: boolean;
  summary: DatabaseTableSummary | null;
  structure: DatabaseTableStructureState;
  capabilities: StructureCapabilities;
  onIntent: (intent: DatabaseStructureIntent) => void;
  onConnect: () => void;
  onRefresh: () => void;
};

function permissionScope(error: CommandError): string | null {
  if (!error.scope) {
    return null;
  }
  return `${error.scope.object} ${error.scope.name}`;
}

function ResourceFailure({
  error,
  title,
  onRetry,
}: {
  error: CommandError;
  title: string;
  onRetry: () => void;
}) {
  if (error.code === "permission") {
    return (
      <div role="alert" className="space-y-2">
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div className="flex-1 space-y-1">
            <AlertTitle>Permission denied</AlertTitle>
            <AlertDescription>
              {error.message}
              {permissionScope(error) ? ` Scoped to ${permissionScope(error)}.` : ""}
            </AlertDescription>
            {error.details ? (
              <p className="text-[11px] leading-5 text-muted-foreground">{error.details}</p>
            ) : null}
          </div>
        </Alert>
        <Button variant="secondary" size="sm" onClick={onRetry}>
          <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
          Try again
        </Button>
      </div>
    );
  }

  return <ErrorState title={title} message={error.message} onRetry={onRetry} />;
}

function CapabilityNotice({
  entries,
}: {
  entries: Array<{ label: string; decision: StructureDecision }>;
}) {
  const unavailable = unavailableDecisions(entries);
  if (unavailable.length === 0) {
    return null;
  }

  return (
    <div
      role="note"
      aria-label="Unavailable structure actions"
      className="rounded-md border border-warning/30 bg-warning/5 px-3 py-2.5"
    >
      <p className="flex items-center gap-1.5 text-[11px] font-medium text-warning">
        <WarningCircleIcon className="size-3.5 shrink-0" aria-hidden="true" />
        Some structure actions are unavailable
      </p>
      <ul className="mt-1 space-y-0.5 text-[10px] leading-4 text-muted-foreground">
        {unavailable.map((entry) => (
          <li key={`${entry.label}:${entry.reason}`}>
            <span className="font-medium text-foreground">{entry.label}:</span> {entry.reason}
          </li>
        ))}
      </ul>
    </div>
  );
}

function ActionButton({
  decision,
  label,
  icon,
  onClick,
  destructive = false,
}: {
  decision: StructureDecision;
  label: string;
  icon?: ReactNode;
  onClick: () => void;
  destructive?: boolean;
}) {
  return (
    <Button
      type="button"
      variant={destructive ? "destructive" : "secondary"}
      size="sm"
      disabled={!decision.enabled}
      title={decision.enabled ? label : (decision.reason ?? label)}
      aria-label={label}
      onClick={onClick}
      className={cn(
        !destructive && "bg-card",
        destructive && "shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]",
      )}
    >
      {icon}
      {label}
    </Button>
  );
}

function RowActionButton({
  decision,
  label,
  icon,
  onClick,
  destructive = false,
}: {
  decision: StructureDecision;
  label: string;
  icon: ReactNode;
  onClick: () => void;
  destructive?: boolean;
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      className={cn(
        "size-7",
        destructive
          ? "text-destructive hover:bg-destructive/10 hover:text-destructive"
          : "text-muted-foreground",
      )}
      disabled={!decision.enabled}
      title={decision.enabled ? label : (decision.reason ?? label)}
      aria-label={label}
      onClick={onClick}
    >
      {icon}
    </Button>
  );
}

function TablePanelHeader({
  scope,
  engine,
  summary,
  children,
}: {
  scope: DatabaseTableScope;
  engine: DatabaseEngine;
  summary: DatabaseTableSummary | null;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="grid size-8 shrink-0 place-items-center rounded-md border border-border bg-card text-muted-foreground">
            <TableIcon className="size-4" aria-hidden="true" />
          </span>
          <div className="min-w-0">
            <h2 className="truncate text-[15px] font-semibold tracking-[-0.01em] text-foreground">
              {scope.table}
            </h2>
            <p className="mt-0.5 flex flex-wrap items-center gap-1.5 font-mono text-[10px] text-muted-foreground">
              <span>{scope.database ?? "Active database"}</span>
              {scope.schema ? (
                <>
                  <span aria-hidden="true">/</span>
                  <span>{scope.schema}</span>
                </>
              ) : null}
              <span aria-hidden="true">·</span>
              <span>{engine === "mysql" ? "MySQL" : "PostgreSQL"}</span>
            </p>
          </div>
        </div>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {summary ? <Badge variant="outline">{summary.kind.replaceAll("_", " ")}</Badge> : null}
        {children}
      </div>
    </div>
  );
}

function TableLifecycleActions({
  capabilities,
  onIntent,
}: {
  capabilities: StructureCapabilities;
  onIntent: (intent: DatabaseStructureIntent) => void;
}) {
  return (
    <>
      <ActionButton
        decision={capabilities.renameTable}
        label="Rename table"
        icon={<PencilSimpleIcon className="size-3.5" aria-hidden="true" />}
        onClick={() => onIntent({ type: "rename_table" })}
      />
      <ActionButton
        decision={capabilities.duplicateTable}
        label="Duplicate table"
        icon={<CopyIcon className="size-3.5" aria-hidden="true" />}
        onClick={() => onIntent({ type: "duplicate_table" })}
      />
      <ActionButton
        decision={capabilities.truncateTable}
        label="Truncate table"
        icon={<BroomIcon className="size-3.5" aria-hidden="true" />}
        onClick={() => onIntent({ type: "truncate_table" })}
      />
      <ActionButton
        decision={capabilities.dropTable}
        label="Drop table"
        icon={<TrashIcon className="size-3.5" aria-hidden="true" />}
        onClick={() => onIntent({ type: "drop_table" })}
        destructive
      />
    </>
  );
}

function RelationshipsTable({ relationships }: { relationships: TableRelationship[] }) {
  if (relationships.length === 0) {
    return (
      <p className="rounded-md border border-dashed border-border bg-muted/20 px-3 py-4 text-[11px] text-muted-foreground">
        No foreign-key relationships are reported for this table.
      </p>
    );
  }

  return (
    <Table className="[&_td]:px-2 [&_th]:px-2">
      <TableHeader>
        <TableRow>
          <TableHead>Direction</TableHead>
          <TableHead>Constraint</TableHead>
          <TableHead>Source</TableHead>
          <TableHead>Target</TableHead>
          <TableHead>On update</TableHead>
          <TableHead>On delete</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {relationships.map((relationship) => {
          const source = `${relationship.sourceTable}${
            relationship.sourceColumns.length > 0
              ? ` (${relationship.sourceColumns.join(", ")})`
              : ""
          }`;
          const target = `${relationship.targetTable}${
            relationship.targetColumns.length > 0
              ? ` (${relationship.targetColumns.join(", ")})`
              : ""
          }`;
          return (
            <TableRow key={relationship.id}>
              <TableCell>
                <Badge variant={relationship.direction === "incoming" ? "info" : "outline"}>
                  {relationship.direction === "incoming" ? "Incoming" : "Outgoing"}
                </Badge>
              </TableCell>
              <TableCell className="max-w-44 truncate font-mono text-[11px]">
                {relationship.name}
              </TableCell>
              <TableCell className="max-w-40 truncate font-mono text-[11px]" title={source}>
                {source}
              </TableCell>
              <TableCell className="max-w-40 truncate font-mono text-[11px]" title={target}>
                {target}
              </TableCell>
              <TableCell className="text-[11px] whitespace-nowrap text-muted-foreground">
                {relationship.onUpdate ?? "—"}
              </TableCell>
              <TableCell className="text-[11px] whitespace-nowrap text-muted-foreground">
                {relationship.onDelete ?? "—"}
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

function DependenciesSection({
  structure,
  scope,
}: {
  structure: DatabaseTableStructureState;
  scope: DatabaseTableScope;
}) {
  const dependencies = structure.dependencies;

  return (
    <section aria-label="Table dependencies" className="rounded-md border border-border bg-card">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/15 px-3 py-2">
        <h3 className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
          <LinkIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
          Dependencies
        </h3>
        <span className="text-[10px] text-muted-foreground">
          Bounded inventory for {qualifiedTableName(scope.schema, scope.table)}
        </span>
      </div>
      <div className="p-3">
        {dependencies.status === "idle" || dependencies.status === "loading" ? (
          <LoadingState rows={2} />
        ) : dependencies.status === "error" ? (
          <ResourceFailure
            error={dependencies.error}
            title="Could not load table dependencies"
            onRetry={structure.reload}
          />
        ) : (
          <div className="grid gap-3 md:grid-cols-2">
            {dependencyGroups(dependencies.data).map((group) => (
              <div key={group.key} className="rounded-md border border-border bg-muted/10 p-2.5">
                <div className="flex items-center justify-between gap-2">
                  <p className="text-[11px] font-medium text-foreground">{group.label}</p>
                  <Badge variant={group.state === "available" ? "outline" : "warning"}>
                    {group.state === "available" ? group.items.length : "Unavailable"}
                  </Badge>
                </div>
                {group.state === "unavailable" ? (
                  <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
                    This engine does not expose this dependency group.
                  </p>
                ) : group.items.length === 0 ? (
                  <p className="mt-1 text-[10px] leading-4 text-muted-foreground">None reported.</p>
                ) : (
                  <ul className="mt-1 space-y-1">
                    {group.items.map((item) => (
                      <li
                        key={`${group.key}:${item.schema ?? ""}:${item.table ?? ""}:${item.name}`}
                        className="flex min-w-0 items-start gap-1.5 text-[10px] leading-4 text-muted-foreground"
                      >
                        <CheckCircleIcon
                          className="mt-0.5 size-3 shrink-0 text-muted-foreground"
                          aria-hidden="true"
                        />
                        <span className="min-w-0">
                          <span className="block truncate font-mono text-foreground">
                            {dependencyObjectLabel(item)}
                          </span>
                          {item.columns.length > 0 ? (
                            <span className="block truncate">
                              Columns: {item.columns.join(", ")}
                            </span>
                          ) : null}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
                {group.truncated ? (
                  <p className="mt-1 text-[10px] text-warning">
                    Only the first page of this dependency group is shown.
                  </p>
                ) : null}
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

export function DatabaseTableOverviewPanel({
  scope,
  profileName,
  engine,
  connected,
  summary,
  structure,
  capabilities,
  onIntent,
  onConnect,
  onRefresh,
}: DatabaseStructurePanelProps) {
  if (!connected) {
    return (
      <EmptyState
        icon={<PlugsIcon className="size-4" aria-hidden="true" />}
        title="Connection required"
        description={`Connect ${profileName} over SSH before inspecting or changing ${scope.table}. The selected table stays on this workspace tab.`}
        action={
          <Button size="sm" onClick={onConnect}>
            <PlugsIcon className="size-3.5" aria-hidden="true" />
            Connect {profileName}
          </Button>
        }
      />
    );
  }

  const constraints = structure.constraints.status === "ready" ? structure.constraints.data : [];
  const relationships = buildTableRelationships(
    scope,
    constraints,
    structure.dependencies.status === "ready" ? structure.dependencies.data : null,
  );
  const columnCount = structure.columns.status === "ready" ? structure.columns.data.length : null;
  const indexCount = structure.indexes.status === "ready" ? structure.indexes.data.length : null;
  const constraintCount =
    structure.constraints.status === "ready" ? structure.constraints.data.length : null;

  return (
    <div className="space-y-4">
      <TablePanelHeader scope={scope} engine={engine} summary={summary}>
        <TableLifecycleActions capabilities={capabilities} onIntent={onIntent} />
      </TablePanelHeader>

      <div className="flex flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
        <Badge variant="outline">{columnCount ?? "…"} columns</Badge>
        <Badge variant="outline">{indexCount ?? "…"} indexes</Badge>
        <Badge variant="outline">{constraintCount ?? "…"} constraints</Badge>
        <Button variant="ghost" size="sm" onClick={onRefresh} aria-label="Refresh table metadata">
          <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
          Refresh
        </Button>
      </div>

      <CapabilityNotice
        entries={[
          { label: "Rename table", decision: capabilities.renameTable },
          { label: "Duplicate table", decision: capabilities.duplicateTable },
          { label: "Truncate table", decision: capabilities.truncateTable },
          { label: "Drop table", decision: capabilities.dropTable },
        ]}
      />

      <section
        aria-label="Table summary"
        className="overflow-hidden rounded-md border border-border bg-card"
      >
        <div className="flex items-center gap-1.5 border-b border-border bg-muted/15 px-3 py-2">
          <DatabaseIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-[12px] font-semibold text-foreground">Table summary</h3>
        </div>
        <dl className="grid gap-x-6 gap-y-2.5 px-3 py-3 sm:grid-cols-2 xl:grid-cols-3">
          {tableSummaryRows(summary, scope).map((row) => (
            <div key={row.label} className="min-w-0">
              <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                {row.label}
              </dt>
              <dd
                className={cn(
                  "truncate text-[12px] text-foreground",
                  row.mono && "font-mono text-[11px]",
                )}
                title={row.value}
              >
                {row.value}
              </dd>
            </div>
          ))}
        </dl>
        {!summary ? (
          <p className="border-t border-border px-3 py-2 text-[10px] text-muted-foreground">
            The explorer cache did not include this table summary. Refresh the Tables node to load
            size, owner, encoding, and statistics details.
          </p>
        ) : null}
      </section>

      <section
        aria-label="Table relationships"
        className="overflow-hidden rounded-md border border-border bg-card"
      >
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/15 px-3 py-2">
          <h3 className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
            <LinkIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            Relationships
          </h3>
          <span className="text-[10px] text-muted-foreground">
            Foreign keys from and to this table
          </span>
        </div>
        <div className="p-3">
          {structure.constraints.status === "idle" ||
          structure.constraints.status === "loading" ||
          structure.dependencies.status === "loading" ? (
            <LoadingState rows={2} />
          ) : structure.constraints.status === "error" ? (
            <ResourceFailure
              error={structure.constraints.error}
              title="Could not load table relationships"
              onRetry={structure.reload}
            />
          ) : (
            <>
              <RelationshipsTable relationships={relationships} />
              {structure.dependencies.status === "error" ? (
                <p className="mt-2 flex items-start gap-1.5 text-[10px] leading-4 text-warning">
                  <WarningCircleIcon className="mt-0.5 size-3 shrink-0" aria-hidden="true" />
                  Incoming relationships may be incomplete: {structure.dependencies.error.message}
                </p>
              ) : null}
            </>
          )}
        </div>
      </section>

      <DependenciesSection structure={structure} scope={scope} />
    </div>
  );
}

function ColumnIdentityCell({ column }: { column: DatabaseColumnSummary }) {
  const identity = columnIdentityLabel(column);
  const generated = columnGeneratedLabel(column);
  const values = [identity, generated].filter((value) => value !== "—");
  if (values.length === 0) {
    return <span className="text-muted-foreground">—</span>;
  }
  return (
    <span className="block max-w-44 truncate" title={values.join(" · ")}>
      {values.join(" · ")}
    </span>
  );
}

function ColumnsTable({
  columns,
  capabilities,
  onIntent,
}: {
  columns: DatabaseColumnSummary[];
  capabilities: StructureCapabilities;
  onIntent: (intent: DatabaseStructureIntent) => void;
}) {
  return (
    <Table className="[&_td]:px-2 [&_th]:px-2">
      <TableHeader>
        <TableRow>
          <TableHead>Column</TableHead>
          <TableHead>Type</TableHead>
          <TableHead>Default</TableHead>
          <TableHead>Identity / generated</TableHead>
          <TableHead className="w-24 text-right">Actions</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {columns.map((column) => (
          <TableRow key={column.name}>
            <TableCell className="max-w-44">
              <span className="flex min-w-0 items-center gap-1.5">
                <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                  {column.ordinalPosition}
                </span>
                <span className="truncate font-mono text-[11px] font-medium">{column.name}</span>
                {column.primaryKey ? <Badge variant="info">PK</Badge> : null}
              </span>
              {column.comment.status === "available" && column.comment.value ? (
                <span
                  className="block truncate text-[10px] text-muted-foreground"
                  title={column.comment.value}
                >
                  {column.comment.value}
                </span>
              ) : null}
            </TableCell>
            <TableCell className="max-w-36">
              <span
                className="block truncate font-mono text-[11px]"
                title={columnTypeLabel(column)}
              >
                {columnTypeLabel(column)}
              </span>
              <Badge variant={column.nullable ? "outline" : "secondary"}>
                {column.nullable ? "NULL" : "NOT NULL"}
              </Badge>
            </TableCell>
            <TableCell
              className="max-w-32 truncate font-mono text-[11px]"
              title={columnDefaultLabel(column)}
            >
              {columnDefaultLabel(column)}
            </TableCell>
            <TableCell className="max-w-36 text-[10px] text-muted-foreground">
              <ColumnIdentityCell column={column} />
            </TableCell>
            <TableCell>
              <div className="flex justify-end gap-0.5">
                <RowActionButton
                  decision={capabilities.editColumn}
                  label={`Edit column ${column.name}`}
                  icon={<SlidersHorizontalIcon className="size-3.5" aria-hidden="true" />}
                  onClick={() => onIntent({ type: "edit_column", column })}
                />
                <RowActionButton
                  decision={capabilities.renameColumn}
                  label={`Rename column ${column.name}`}
                  icon={<TextTIcon className="size-3.5" aria-hidden="true" />}
                  onClick={() => onIntent({ type: "rename_column", column })}
                />
                <RowActionButton
                  decision={capabilities.dropColumn}
                  label={`Drop column ${column.name}`}
                  icon={<TrashIcon className="size-3.5" aria-hidden="true" />}
                  onClick={() => onIntent({ type: "drop_column", column })}
                  destructive
                />
              </div>
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

function constraintReference(constraint: DatabaseConstraintSummary): string {
  if (constraint.kind === "check") {
    return constraint.checkExpression.status === "available"
      ? (constraint.checkExpression.value ?? "Check expression unavailable")
      : "Unavailable";
  }
  if (constraint.kind !== "foreign_key") {
    return constraint.columns.length > 0 ? constraint.columns.join(", ") : "—";
  }
  const table =
    constraint.referencedTable.status === "available" ? constraint.referencedTable.value : null;
  if (!table) {
    return "Unavailable";
  }
  const schema =
    constraint.referencedSchema.status === "available" ? constraint.referencedSchema.value : null;
  const columns =
    constraint.referencedColumns.length > 0 ? ` (${constraint.referencedColumns.join(", ")})` : "";
  return `${qualifiedTableName(schema, table)}${columns}`;
}

function ConstraintsTable({
  constraints,
  capabilities,
  onIntent,
}: {
  constraints: DatabaseConstraintSummary[];
  capabilities: StructureCapabilities;
  onIntent: (intent: DatabaseStructureIntent) => void;
}) {
  return (
    <Table className="[&_td]:px-2 [&_th]:px-2">
      <TableHeader>
        <TableRow>
          <TableHead>Name</TableHead>
          <TableHead>Kind</TableHead>
          <TableHead>Columns</TableHead>
          <TableHead>References / expression</TableHead>
          <TableHead className="text-right">Actions</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {constraints.map((constraint) => {
          const decision = constraintActionDecision(constraint, capabilities);
          return (
            <TableRow key={`${constraint.kind}:${constraint.name}`}>
              <TableCell className="font-mono text-[11px] font-medium">{constraint.name}</TableCell>
              <TableCell>
                <Badge variant={constraint.kind === "primary_key" ? "info" : "outline"}>
                  {constraintKindLabel(constraint.kind)}
                </Badge>
              </TableCell>
              <TableCell className="max-w-44 truncate font-mono text-[11px]">
                {constraint.columns.length > 0 ? constraint.columns.join(", ") : "—"}
              </TableCell>
              <TableCell className="max-w-64 truncate font-mono text-[10px] text-muted-foreground">
                {constraintReference(constraint)}
              </TableCell>
              <TableCell>
                <div className="flex justify-end">
                  <RowActionButton
                    decision={decision}
                    label={`Drop constraint ${constraint.name}`}
                    icon={<TrashIcon className="size-3.5" aria-hidden="true" />}
                    onClick={() => onIntent({ type: "drop_constraint", constraint })}
                    destructive
                  />
                </div>
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

export function DatabaseTableStructurePanel({
  scope,
  profileName,
  engine,
  connected,
  summary,
  structure,
  capabilities,
  onIntent,
  onConnect,
  onRefresh,
}: DatabaseStructurePanelProps) {
  if (!connected) {
    return (
      <EmptyState
        icon={<PlugsIcon className="size-4" aria-hidden="true" />}
        title="Connection required"
        description={`Connect ${profileName} before loading columns and constraints for ${scope.table}.`}
        action={
          <Button size="sm" onClick={onConnect}>
            <PlugsIcon className="size-3.5" aria-hidden="true" />
            Connect {profileName}
          </Button>
        }
      />
    );
  }

  return (
    <div className="space-y-4">
      <TablePanelHeader scope={scope} engine={engine} summary={summary}>
        <Button
          variant="secondary"
          size="sm"
          disabled={!capabilities.addColumn.enabled}
          title={
            capabilities.addColumn.enabled
              ? "Add column"
              : (capabilities.addColumn.reason ?? undefined)
          }
          onClick={() => onIntent({ type: "add_column" })}
        >
          <PlusIcon className="size-3.5" aria-hidden="true" />
          Add column
        </Button>
        <Button
          variant="secondary"
          size="sm"
          disabled={!capabilities.addConstraint.enabled}
          title={
            capabilities.addConstraint.enabled
              ? "Add constraint"
              : (capabilities.addConstraint.reason ?? undefined)
          }
          onClick={() => onIntent({ type: "add_constraint" })}
        >
          <PlusIcon className="size-3.5" aria-hidden="true" />
          Add constraint
        </Button>
        <Button variant="ghost" size="sm" onClick={onRefresh} aria-label="Refresh table structure">
          <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
          Refresh
        </Button>
      </TablePanelHeader>

      <CapabilityNotice
        entries={[
          { label: "Add column", decision: capabilities.addColumn },
          { label: "Edit column", decision: capabilities.editColumn },
          { label: "Rename column", decision: capabilities.renameColumn },
          { label: "Drop column", decision: capabilities.dropColumn },
          { label: "Add constraint", decision: capabilities.addConstraint },
          { label: "Check constraints", decision: capabilities.addCheckConstraint },
        ]}
      />

      <section
        aria-label="Table columns"
        className="overflow-hidden rounded-md border border-border bg-card"
      >
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/15 px-3 py-2">
          <h3 className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
            <TableIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            Columns
          </h3>
          <span className="text-[10px] text-muted-foreground">
            {structure.columns.status === "ready"
              ? `${structure.columns.data.length} columns`
              : "Loading column metadata"}
          </span>
        </div>
        {structure.columns.status === "idle" || structure.columns.status === "loading" ? (
          <div className="p-3">
            <LoadingState rows={5} />
          </div>
        ) : structure.columns.status === "error" ? (
          <div className="p-3">
            <ResourceFailure
              error={structure.columns.error}
              title="Could not load columns"
              onRetry={structure.reload}
            />
          </div>
        ) : structure.columns.data.length === 0 ? (
          <div className="p-3">
            <EmptyState
              icon={<TableIcon className="size-4" aria-hidden="true" />}
              title="No columns returned"
              description="The database account can read this table but reported no column metadata."
              action={
                <Button size="sm" onClick={() => onIntent({ type: "add_column" })}>
                  <PlusIcon className="size-3.5" aria-hidden="true" />
                  Add column
                </Button>
              }
            />
          </div>
        ) : (
          <ColumnsTable
            columns={structure.columns.data}
            capabilities={capabilities}
            onIntent={onIntent}
          />
        )}
      </section>

      <section
        aria-label="Table constraints"
        className="overflow-hidden rounded-md border border-border bg-card"
      >
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/15 px-3 py-2">
          <h3 className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
            <KeyIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            Constraints
          </h3>
          <span className="text-[10px] text-muted-foreground">
            {structure.constraints.status === "ready"
              ? `${structure.constraints.data.length} constraints`
              : "Loading constraint metadata"}
          </span>
        </div>
        {structure.constraints.status === "idle" || structure.constraints.status === "loading" ? (
          <div className="p-3">
            <LoadingState rows={3} />
          </div>
        ) : structure.constraints.status === "error" ? (
          <div className="p-3">
            <ResourceFailure
              error={structure.constraints.error}
              title="Could not load constraints"
              onRetry={structure.reload}
            />
          </div>
        ) : structure.constraints.data.length === 0 ? (
          <p className="px-3 py-4 text-[11px] text-muted-foreground">
            No primary-key, unique, foreign-key, or check constraints are reported for this table.
          </p>
        ) : (
          <ConstraintsTable
            constraints={structure.constraints.data}
            capabilities={capabilities}
            onIntent={onIntent}
          />
        )}
      </section>
    </div>
  );
}

function IndexesTable({
  indexes,
  capabilities,
  onIntent,
}: {
  indexes: DatabaseIndexSummary[];
  capabilities: StructureCapabilities;
  onIntent: (intent: DatabaseStructureIntent) => void;
}) {
  return (
    <Table className="[&_td]:px-2 [&_th]:px-2">
      <TableHeader>
        <TableRow>
          <TableHead>Name</TableHead>
          <TableHead>Columns</TableHead>
          <TableHead>Unique</TableHead>
          <TableHead>Primary</TableHead>
          <TableHead>Method</TableHead>
          <TableHead>Predicate</TableHead>
          <TableHead className="w-20 text-right">Actions</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {indexes.map((index) => {
          const decisions = indexActionDecisions(index, capabilities);
          return (
            <TableRow key={index.name}>
              <TableCell className="max-w-40 truncate font-mono text-[11px] font-medium">
                {index.name}
              </TableCell>
              <TableCell className="max-w-40 truncate font-mono text-[11px]">
                {index.columns.join(", ")}
              </TableCell>
              <TableCell>
                <Badge variant={index.unique ? "info" : "outline"}>
                  {index.unique ? "Unique" : "No"}
                </Badge>
              </TableCell>
              <TableCell>
                {index.primary ? (
                  <Badge variant="success">Primary</Badge>
                ) : (
                  <span className="text-muted-foreground">—</span>
                )}
              </TableCell>
              <TableCell className="font-mono text-[11px] text-muted-foreground">
                {formatMetadataValue(index.method, (value) => value ?? "—")}
              </TableCell>
              <TableCell className="max-w-40 truncate font-mono text-[10px] text-muted-foreground">
                {formatMetadataValue(index.predicate, (value) => value ?? "—")}
              </TableCell>
              <TableCell>
                <div className="flex justify-end gap-0.5">
                  <RowActionButton
                    decision={decisions.rename}
                    label={`Rename index ${index.name}`}
                    icon={<TextTIcon className="size-3.5" aria-hidden="true" />}
                    onClick={() => onIntent({ type: "rename_index", index })}
                  />
                  <RowActionButton
                    decision={decisions.drop}
                    label={`Drop index ${index.name}`}
                    icon={<TrashIcon className="size-3.5" aria-hidden="true" />}
                    onClick={() => onIntent({ type: "drop_index", index })}
                    destructive
                  />
                </div>
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

export function DatabaseTableIndexesPanel({
  scope,
  profileName,
  engine,
  connected,
  summary,
  structure,
  capabilities,
  onIntent,
  onConnect,
  onRefresh,
}: DatabaseStructurePanelProps) {
  if (!connected) {
    return (
      <EmptyState
        icon={<PlugsIcon className="size-4" aria-hidden="true" />}
        title="Connection required"
        description={`Connect ${profileName} before loading indexes for ${scope.table}.`}
        action={
          <Button size="sm" onClick={onConnect}>
            <PlugsIcon className="size-3.5" aria-hidden="true" />
            Connect {profileName}
          </Button>
        }
      />
    );
  }

  const indexes = structure.indexes.status === "ready" ? structure.indexes.data : [];

  return (
    <div className="space-y-4">
      <TablePanelHeader scope={scope} engine={engine} summary={summary}>
        <Button
          variant="secondary"
          size="sm"
          disabled={!capabilities.createIndex.enabled}
          title={
            capabilities.createIndex.enabled
              ? "Create index"
              : (capabilities.createIndex.reason ?? undefined)
          }
          onClick={() => onIntent({ type: "create_index" })}
        >
          <PlusIcon className="size-3.5" aria-hidden="true" />
          Create index
        </Button>
        <Button variant="ghost" size="sm" onClick={onRefresh} aria-label="Refresh table indexes">
          <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
          Refresh
        </Button>
      </TablePanelHeader>

      <CapabilityNotice
        entries={[
          { label: "Create index", decision: capabilities.createIndex },
          { label: "Rename index", decision: capabilities.renameIndex },
          { label: "Drop index", decision: capabilities.dropIndex },
          { label: "Partial indexes", decision: capabilities.partialIndexes },
        ]}
      />

      <section
        aria-label="Table indexes"
        className="overflow-hidden rounded-md border border-border bg-card"
      >
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/15 px-3 py-2">
          <h3 className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
            <KeyIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            Indexes
          </h3>
          <span className="text-[10px] text-muted-foreground">
            {structure.indexes.status === "ready"
              ? `${indexes.length} indexes`
              : "Loading index metadata"}
          </span>
        </div>
        {structure.indexes.status === "idle" || structure.indexes.status === "loading" ? (
          <div className="p-3">
            <LoadingState rows={4} />
          </div>
        ) : structure.indexes.status === "error" ? (
          <div className="p-3">
            <ResourceFailure
              error={structure.indexes.error}
              title="Could not load indexes"
              onRetry={structure.reload}
            />
          </div>
        ) : indexes.length === 0 ? (
          <div className="p-3">
            <EmptyState
              icon={<KeyIcon className="size-4" aria-hidden="true" />}
              title="No indexes reported"
              description="This table has no primary, unique, or secondary index metadata that the database account can read."
              action={
                <Button size="sm" onClick={() => onIntent({ type: "create_index" })}>
                  <PlusIcon className="size-3.5" aria-hidden="true" />
                  Create index
                </Button>
              }
            />
          </div>
        ) : (
          <IndexesTable indexes={indexes} capabilities={capabilities} onIntent={onIntent} />
        )}
      </section>
    </div>
  );
}

export function DatabaseTableSelectionEmpty({
  title,
  description,
  canCreate,
  createReason,
  connected,
  profileName,
  onConnect,
  onCreateTable,
}: {
  title: string;
  description: string;
  canCreate: boolean;
  createReason: string | null;
  connected: boolean;
  profileName: string;
  onConnect: () => void;
  onCreateTable: () => void;
}) {
  if (!connected) {
    return (
      <EmptyState
        icon={<PlugsIcon className="size-4" aria-hidden="true" />}
        title="Connection required"
        description={`Connect ${profileName} before browsing table structure.`}
        action={
          <Button size="sm" onClick={onConnect}>
            <PlugsIcon className="size-3.5" aria-hidden="true" />
            Connect {profileName}
          </Button>
        }
      />
    );
  }

  return (
    <div className="space-y-3">
      <EmptyState
        icon={<TableIcon className="size-4" aria-hidden="true" />}
        title={title}
        description={description}
        action={
          <div className="flex flex-col items-start gap-1.5">
            <Button
              size="sm"
              disabled={!canCreate}
              title={createReason ?? undefined}
              onClick={onCreateTable}
            >
              <PlusIcon className="size-3.5" aria-hidden="true" />
              Create table
            </Button>
            {!canCreate && createReason ? (
              <p className="max-w-[52ch] text-[10px] leading-4 text-muted-foreground">
                {createReason}
              </p>
            ) : null}
          </div>
        }
      />
    </div>
  );
}
