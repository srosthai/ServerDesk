import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from "react";
import {
  ArrowClockwiseIcon,
  ArrowsDownUpIcon,
  BinaryIcon,
  BracketsCurlyIcon,
  CaretDownIcon,
  CaretLeftIcon,
  CaretRightIcon,
  CaretUpIcon,
  CircleNotchIcon,
  ClockIcon,
  ColumnsIcon,
  EyeIcon,
  FunnelIcon,
  MagnifyingGlassIcon,
  PencilSimpleIcon,
  PlugsIcon,
  PlusIcon,
  ProhibitIcon,
  RowsIcon,
  TrashIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { CopyButton } from "@/components/ui/copy-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { ErrorState } from "@/components/ui/states";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  DEFAULT_DATA_GRID_STATE,
  DATA_GRID_PAGE_SIZES,
  MAX_COLUMN_WIDTH,
  MAX_FILTERS,
  MIN_COLUMN_WIDTH,
  buildCellViewer,
  buildRowFilter,
  cellPresentation,
  columnWidth,
  dataGridTableKey,
  defaultColumnWidth,
  filterDraftValue,
  filterOperatorLabel,
  filterOperatorsFor,
  filterSummary,
  formatEstimatedCount,
  formatSampleTime,
  nextPageOffset,
  pageWindowLabel,
  previousPageOffset,
  sortDirectionFor,
  toggleSort,
  visibleColumnNames,
  type CellPresentation,
} from "@/features/databases/databaseDataGridModel";
import {
  EditRowDialog,
  InsertRowDialog,
  RowWriteInfo,
  RowWriteResultBanner,
} from "@/features/databases/DatabaseRowWriteDialogs";
import {
  identityKey,
  rowEditingBlocked as rowEditingBlockedReason,
} from "@/features/databases/databaseRowWriteModel";
import { databaseRowKey, useDatabaseRowStore } from "@/stores/databaseRowStore";
import { useDatabaseRowWriteStore } from "@/stores/databaseRowWriteStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";
import type {
  DatabaseRowCell,
  DatabaseRowColumn,
  DatabaseRowFilter,
  DatabaseRowFilterOperator,
  DatabaseRowIdentityValue,
  DatabaseRowReadResult,
  DatabaseRowRecord,
  DatabaseRowTable,
  DatabaseRowWriteValue,
} from "@/types/contracts";
import { cn } from "@/lib/utils";

export type DatabaseDataGridProps = {
  profileId: string;
  profileName: string;
  database: string | null;
  schema: string | null;
  table: string;
  connected: boolean;
  readOnly: boolean;
  onConnect: () => void;
};

type FilterDraft = {
  id: string;
  column: string;
  operator: DatabaseRowFilterOperator;
  value: string;
};

type ViewerTarget = {
  cell: DatabaseRowCell;
  column: DatabaseRowColumn;
  rowNumber: number;
};

type ResizeSession = {
  column: string;
  startX: number;
  startWidth: number;
  width: number;
};

const EMPTY_COLUMNS: DatabaseRowColumn[] = [];
const EMPTY_ROWS: DatabaseRowRecord[] = [];

function draftsFromFilters(
  filters: DatabaseRowFilter[],
  columns: DatabaseRowColumn[],
): FilterDraft[] {
  return filters
    .filter((filter) => columns.some((column) => column.name === filter.column))
    .map((filter, index) => ({
      id: `applied-${index}-${filter.column}`,
      column: filter.column,
      operator: filter.operator,
      value: filterDraftValue(filter),
    }));
}

function defaultDraft(columns: DatabaseRowColumn[], id: string): FilterDraft | null {
  const column = columns[0];
  if (!column) {
    return null;
  }
  return {
    id,
    column: column.name,
    operator: filterOperatorsFor(column.kind)[0] ?? "is_null",
    value: column.kind === "boolean" ? "true" : "",
  };
}

function CellValue({
  presentation,
  columnName,
  rowNumber,
  onView,
}: {
  presentation: CellPresentation;
  columnName: string;
  rowNumber: number;
  onView: () => void;
}) {
  const viewerButton = presentation.canView ? (
    <button
      type="button"
      className="ml-1 inline-flex shrink-0 items-center rounded-sm p-0.5 text-muted-foreground/80 outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
      aria-label={`View ${columnName} value in row ${rowNumber}`}
      title="Open the full value"
      onClick={(event) => {
        event.stopPropagation();
        onView();
      }}
    >
      <EyeIcon className="size-3" aria-hidden="true" />
    </button>
  ) : null;

  if (presentation.kind === "null") {
    return (
      <span className="font-mono text-[10px] tracking-wide text-muted-foreground/80 italic">
        NULL
      </span>
    );
  }

  if (presentation.kind === "empty") {
    return <span className="font-mono text-[10px] text-muted-foreground">&quot;&quot; empty</span>;
  }

  if (presentation.kind === "boolean") {
    return (
      <span
        className={cn(
          "font-mono text-[11px] font-medium",
          presentation.text === "true" ? "text-success" : "text-muted-foreground",
        )}
      >
        {presentation.text}
      </span>
    );
  }

  if (presentation.kind === "json" || presentation.kind === "binary") {
    const Icon = presentation.kind === "json" ? BracketsCurlyIcon : BinaryIcon;
    return (
      <span className="flex min-w-0 items-center gap-1">
        <Icon className="size-3 shrink-0 text-muted-foreground/70" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/90">
          {presentation.text}
        </span>
        {presentation.truncated ? (
          <span className="shrink-0 rounded-sm border border-warning/30 px-1 text-[9px] text-warning">
            cut
          </span>
        ) : null}
        {viewerButton}
      </span>
    );
  }

  return (
    <span className="flex min-w-0 items-center">
      <span
        className={cn(
          "min-w-0 flex-1 truncate font-mono text-[11px]",
          presentation.kind === "temporal" || presentation.kind === "uuid"
            ? "text-foreground/85"
            : "text-foreground",
        )}
        title={presentation.title}
      >
        {presentation.text}
      </span>
      {presentation.truncated ? (
        <span className="ml-1 shrink-0 rounded-sm border border-warning/30 px-1 text-[9px] text-warning">
          cut
        </span>
      ) : null}
      {viewerButton}
    </span>
  );
}

function ColumnSettingsDialog({
  open,
  onOpenChange,
  columns,
  hiddenColumns,
  columnWidths,
  onApply,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  columns: DatabaseRowColumn[];
  hiddenColumns: string[];
  columnWidths: Record<string, number>;
  onApply: (hidden: string[], widths: Record<string, number>) => void;
}) {
  if (!open) {
    return null;
  }
  return (
    <ColumnSettingsDialogContent
      onOpenChange={onOpenChange}
      columns={columns}
      hiddenColumns={hiddenColumns}
      columnWidths={columnWidths}
      onApply={onApply}
    />
  );
}

function ColumnSettingsDialogContent({
  onOpenChange,
  columns,
  hiddenColumns,
  columnWidths,
  onApply,
}: {
  onOpenChange: (open: boolean) => void;
  columns: DatabaseRowColumn[];
  hiddenColumns: string[];
  columnWidths: Record<string, number>;
  onApply: (hidden: string[], widths: Record<string, number>) => void;
}) {
  const [hidden, setHidden] = useState<string[]>(() => [...hiddenColumns]);
  const [widths, setWidths] = useState<Record<string, number>>(() => ({ ...columnWidths }));

  const visibleCount = columns.length - hidden.length;

  function toggleColumn(name: string) {
    setHidden((current) => {
      if (current.includes(name)) {
        return current.filter((entry) => entry !== name);
      }
      if (columns.length - current.length <= 1) {
        return current;
      }
      return [...current, name];
    });
  }

  function updateWidth(name: string, raw: string) {
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      return;
    }
    setWidths((current) => ({
      ...current,
      [name]: Math.min(MAX_COLUMN_WIDTH, Math.max(MIN_COLUMN_WIDTH, Math.round(parsed))),
    }));
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Columns and widths</DialogTitle>
          <DialogDescription>
            Choose which columns this table page requests. Hidden columns are not fetched, and
            widths stay scoped to this workspace tab.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
          <span>
            {visibleCount} of {columns.length} columns visible
          </span>
          <span className="font-mono text-[10px]">
            Width {MIN_COLUMN_WIDTH}–{MAX_COLUMN_WIDTH}px
          </span>
        </div>

        <div className="max-h-[45vh] divide-y divide-border overflow-y-auto rounded-md border border-border bg-card">
          {columns.map((column) => {
            const checked = !hidden.includes(column.name);
            return (
              <div
                key={column.name}
                className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
              >
                <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2">
                  <input
                    type="checkbox"
                    className="size-3.5 shrink-0 accent-[var(--primary)]"
                    checked={checked}
                    disabled={checked && visibleCount <= 1}
                    aria-label={`Show column ${column.name}`}
                    onChange={() => toggleColumn(column.name)}
                  />
                  <span className="min-w-0">
                    <span className="block truncate font-mono text-[11px] text-foreground">
                      {column.name}
                    </span>
                    <span className="block truncate text-[10px] text-muted-foreground">
                      {column.dataType}
                      {column.primaryKey ? " · primary key" : ""}
                      {column.nullable ? " · nullable" : ""}
                    </span>
                  </span>
                </label>
                <label className="flex shrink-0 items-center gap-1.5 text-[10px] text-muted-foreground">
                  Width
                  <Input
                    type="number"
                    inputMode="numeric"
                    min={MIN_COLUMN_WIDTH}
                    max={MAX_COLUMN_WIDTH}
                    step={8}
                    aria-label={`Width for ${column.name}`}
                    value={widths[column.name] ?? defaultColumnWidth(column.kind)}
                    onChange={(event) => updateWidth(column.name, event.target.value)}
                    className="h-7 w-20 px-2 font-mono text-[11px]"
                  />
                </label>
              </div>
            );
          })}
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="secondary"
            onClick={() => {
              setHidden([]);
              setWidths({});
            }}
          >
            Reset defaults
          </Button>
          <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button type="button" onClick={() => onApply(hidden, widths)}>
            Apply columns
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function LoadedValueViewer({
  target,
  tableName,
  onClose,
}: {
  target: ViewerTarget | null;
  tableName: string;
  onClose: () => void;
}) {
  const viewer = useMemo(
    () => (target ? buildCellViewer(target.cell, target.column) : null),
    [target],
  );

  return (
    <Dialog open={target !== null} onOpenChange={(open) => (!open ? onClose() : undefined)}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>{target ? `${target.column.name} value` : "Cell value"}</DialogTitle>
          <DialogDescription>
            {target
              ? `${tableName} · row ${target.rowNumber} · ${target.column.dataType}`
              : "Full cell value"}
          </DialogDescription>
        </DialogHeader>

        {target && viewer ? (
          <>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-1.5">
                <Badge variant="outline">{viewer.format}</Badge>
                {viewer.format === "json" ? (
                  <Badge variant={viewer.jsonValid ? "success" : "warning"}>
                    {viewer.jsonValid ? "Valid JSON" : "Raw text"}
                  </Badge>
                ) : null}
                {target.cell.truncated ? (
                  <Badge variant="warning">Truncated by server</Badge>
                ) : null}
              </div>
              <CopyButton
                value={target.cell.value ?? "NULL"}
                label="Copy value"
                className="text-muted-foreground"
              />
            </div>
            <pre
              aria-label={`Full value for ${target.column.name}`}
              className="max-h-[55vh] overflow-auto rounded-md border border-border bg-muted/30 p-3 font-mono text-[12px] leading-5 break-words whitespace-pre-wrap text-foreground"
            >
              {viewer.content}
            </pre>
            {viewer.note ? (
              <p
                role="note"
                className="rounded-md border border-border bg-muted/20 px-3 py-2 text-[11px] leading-4 text-muted-foreground"
              >
                {viewer.note}
              </p>
            ) : null}
            <DialogFooter>
              <Button type="button" variant="secondary" onClick={onClose}>
                Close
              </Button>
            </DialogFooter>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function GridSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading table data" className="space-y-3 p-3">
      <div className="flex items-center gap-2">
        <Skeleton className="h-6 w-28" />
        <Skeleton className="h-6 w-20" />
        <Skeleton className="h-6 w-24" />
      </div>
      <div className="space-y-2">
        {Array.from({ length: 8 }).map((_, rowIndex) => (
          <div key={rowIndex} className="flex items-center gap-3 border-b border-border/60 pb-2">
            <Skeleton className="h-3 w-6 shrink-0" />
            <Skeleton className="h-3 w-[18%]" />
            <Skeleton className="h-3 w-[26%]" />
            <Skeleton className="h-3 w-[14%]" />
            <Skeleton className="h-3 w-[22%]" />
          </div>
        ))}
      </div>
    </div>
  );
}

function countBadges(result: DatabaseRowReadResult | undefined) {
  const estimate =
    result?.counts.estimatedTableRows.status === "available"
      ? result.counts.estimatedTableRows.value
      : null;
  const exact = result?.counts.exactRows;
  return { estimate, exact };
}

function recordKey(record: DatabaseRowRecord, fallback: string): string {
  if (record.identity.length === 0) {
    return fallback;
  }
  return JSON.stringify(record.identity.map((entry) => [entry.column, entry.value]));
}

export function DatabaseDataGrid({
  profileId,
  profileName,
  database,
  schema,
  table,
  connected,
  readOnly,
  onConnect,
}: DatabaseDataGridProps) {
  const tableRef = useMemo<DatabaseRowTable>(
    () => ({ database, schema, table }),
    [database, schema, table],
  );
  const tableKey = useMemo(() => dataGridTableKey(tableRef), [tableRef]);
  const rowKey = databaseRowKey(profileId, tableRef);

  const storedGrid = useDatabaseWorkspaceStore(
    (state) => state.tabs[profileId]?.dataGrid?.[tableKey],
  );
  const setDataGridState = useDatabaseWorkspaceStore((state) => state.setDataGridState);
  const grid = storedGrid ?? DEFAULT_DATA_GRID_STATE;
  const entry = useDatabaseRowStore((state) => state.entries[rowKey]);
  const loadRows = useDatabaseRowStore((state) => state.loadRows);
  const cancelRows = useDatabaseRowStore((state) => state.cancelRows);
  const writeEntry = useDatabaseRowWriteStore((state) => state.entries[rowKey]);
  const insertRow = useDatabaseRowWriteStore((state) => state.insertRow);
  const updateRow = useDatabaseRowWriteStore((state) => state.updateRow);
  const deleteRows = useDatabaseRowWriteStore((state) => state.deleteRows);
  const clearWriteEntry = useDatabaseRowWriteStore((state) => state.clearEntry);

  const [refreshToken, setRefreshToken] = useState(0);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [filterDrafts, setFilterDrafts] = useState<FilterDraft[]>([]);
  const [filterError, setFilterError] = useState<string | null>(null);
  const [searchDraft, setSearchDraft] = useState(grid.search);
  const [columnsOpen, setColumnsOpen] = useState(false);
  const [viewer, setViewer] = useState<ViewerTarget | null>(null);
  const [insertOpen, setInsertOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [deleteTargets, setDeleteTargets] = useState<DatabaseRowIdentityValue[][] | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [selectedRows, setSelectedRows] = useState<Set<string>>(() => new Set());
  const [activeCell, setActiveCell] = useState({ row: 0, column: 0 });
  const [resizePreview, setResizePreview] = useState<{ column: string; width: number } | null>(
    null,
  );

  const gridRef = useRef<HTMLTableElement>(null);
  const resizeSession = useRef<ResizeSession | null>(null);
  const draftCounter = useRef(0);
  const rawId = useId();
  const gridId = `data-grid-${rawId.replaceAll(":", "")}`;

  useEffect(() => {
    if (!connected) {
      return;
    }
    const current = useDatabaseRowStore.getState().entries[rowKey];
    const visibleColumns = visibleColumnNames(current?.result?.columns ?? [], grid.hiddenColumns);
    void loadRows(profileId, tableRef, {
      visibleColumns,
      sort: grid.sort,
      filters: grid.filters,
      search: grid.search.length > 0 ? { query: grid.search, columns: [] } : null,
      limit: grid.limit,
      offset: grid.offset,
      includeExactCount: grid.includeExactCount,
    });
  }, [
    connected,
    grid.filters,
    grid.hiddenColumns,
    grid.includeExactCount,
    grid.limit,
    grid.offset,
    grid.search,
    grid.sort,
    loadRows,
    profileId,
    refreshToken,
    rowKey,
    tableRef,
  ]);

  useEffect(() => {
    function handlePointerMove(event: PointerEvent) {
      const session = resizeSession.current;
      if (!session) {
        return;
      }
      const width = Math.min(
        MAX_COLUMN_WIDTH,
        Math.max(MIN_COLUMN_WIDTH, session.startWidth + (event.clientX - session.startX)),
      );
      session.width = width;
      setResizePreview({ column: session.column, width });
    }

    function handlePointerUp() {
      const session = resizeSession.current;
      if (!session) {
        return;
      }
      resizeSession.current = null;
      setResizePreview(null);
      setDataGridState(profileId, tableKey, {
        columnWidths: { ...grid.columnWidths, [session.column]: session.width },
      });
    }

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", handlePointerUp);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerUp);
    };
  }, [grid.columnWidths, profileId, setDataGridState, tableKey]);

  const result = entry?.result;
  const status = entry?.status ?? (connected ? "loading" : "idle");
  const loading = connected && (status === "loading" || status === "idle");
  const allColumns = result?.columns ?? EMPTY_COLUMNS;
  const columnsByName = useMemo(
    () => new Map(allColumns.map((column) => [column.name, column])),
    [allColumns],
  );
  const visibleColumns = useMemo(() => {
    const names =
      result && result.visibleColumns.length > 0
        ? result.visibleColumns
        : allColumns.map((column) => column.name);
    return names
      .map((name) => columnsByName.get(name))
      .filter((column): column is DatabaseRowColumn => column !== undefined);
  }, [allColumns, columnsByName, result]);
  const rows = result?.rows ?? EMPTY_ROWS;
  const { estimate, exact } = countBadges(result);

  const active = {
    row: Math.min(activeCell.row, Math.max(0, rows.length - 1)),
    column: Math.min(activeCell.column, Math.max(0, visibleColumns.length - 1)),
  };

  function updateGrid(patch: Parameters<typeof setDataGridState>[2]) {
    setSelectedRows(new Set());
    setDataGridState(profileId, tableKey, patch);
  }

  function openViewer(
    record: DatabaseRowRecord,
    columnIndex: number,
    rowIndex: number,
    column: DatabaseRowColumn,
  ) {
    const cell = record.cells[columnIndex];
    if (!cell || cell.value === null) {
      return;
    }
    setViewer({
      cell,
      column,
      rowNumber: grid.offset + rowIndex + 1,
    });
  }

  function handleSort(column: DatabaseRowColumn, additive: boolean) {
    updateGrid({ sort: toggleSort(grid.sort, column.name, additive), offset: 0 });
  }

  function handleFiltersToggle() {
    if (!filtersOpen) {
      setFilterDrafts(draftsFromFilters(grid.filters, allColumns));
      setFilterError(null);
    }
    setFiltersOpen((open) => !open);
  }

  function handleAddFilter() {
    draftCounter.current += 1;
    const draft = defaultDraft(allColumns, `draft-${draftCounter.current}`);
    if (draft) {
      setFilterDrafts((current) => [...current, draft]);
    }
  }

  function handleFilterColumnChange(index: number, name: string) {
    const column = columnsByName.get(name);
    if (!column) {
      return;
    }
    setFilterDrafts((current) =>
      current.map((draft, draftIndex) =>
        draftIndex === index
          ? {
              ...draft,
              column: name,
              operator: filterOperatorsFor(column.kind)[0] ?? "is_null",
              value: column.kind === "boolean" ? "true" : "",
            }
          : draft,
      ),
    );
  }

  function handleApplyFilters() {
    const filters: DatabaseRowFilter[] = [];
    for (let index = 0; index < filterDrafts.length; index += 1) {
      const draft = filterDrafts[index];
      const column = columnsByName.get(draft.column);
      if (!column) {
        setFilterError("Choose a column for every filter before applying.");
        return;
      }
      const built = buildRowFilter(column, draft.operator, draft.value);
      if (built.error !== null || built.filter === null) {
        setFilterError(`${column.name}: ${built.error ?? "The filter value is not valid."}`);
        return;
      }
      filters.push(built.filter);
    }
    setFilterError(null);
    setFiltersOpen(false);
    updateGrid({ filters, offset: 0 });
  }

  function clearFilters() {
    setFilterError(null);
    setFilterDrafts([]);
    updateGrid({ filters: [], search: "", offset: 0 });
  }

  function removeFilter(index: number) {
    updateGrid({
      filters: grid.filters.filter((_, filterIndex) => filterIndex !== index),
      offset: 0,
    });
  }

  function applySearch() {
    updateGrid({ search: searchDraft.trim(), offset: 0 });
  }

  function handleRefresh() {
    setSelectedRows(new Set());
    setRefreshToken((token) => token + 1);
  }

  function beginResize(event: ReactPointerEvent<HTMLSpanElement>, column: DatabaseRowColumn) {
    event.preventDefault();
    event.stopPropagation();
    const startWidth = columnWidth(column, grid.columnWidths);
    resizeSession.current = {
      column: column.name,
      startX: event.clientX,
      startWidth,
      width: startWidth,
    };
    setResizePreview({ column: column.name, width: startWidth });
  }

  function resizeWithKeyboard(
    event: ReactKeyboardEvent<HTMLSpanElement>,
    column: DatabaseRowColumn,
  ) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight" && event.key !== "Home") {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const current = columnWidth(column, grid.columnWidths);
    const next =
      event.key === "Home"
        ? defaultColumnWidth(column.kind)
        : Math.min(
            MAX_COLUMN_WIDTH,
            Math.max(MIN_COLUMN_WIDTH, current + (event.key === "ArrowRight" ? 16 : -16)),
          );
    updateGrid({ columnWidths: { ...grid.columnWidths, [column.name]: next } });
  }

  function focusCell(row: number, column: number) {
    setActiveCell({ row, column });
    gridRef.current?.focus();
  }

  function handleGridKeyDown(event: React.KeyboardEvent<HTMLTableElement>) {
    if (event.target !== event.currentTarget || rows.length === 0 || visibleColumns.length === 0) {
      return;
    }
    const lastRow = rows.length - 1;
    const lastColumn = visibleColumns.length - 1;
    let next = active;

    switch (event.key) {
      case "ArrowDown":
        next = { ...active, row: Math.min(lastRow, active.row + 1) };
        break;
      case "ArrowUp":
        next = { ...active, row: Math.max(0, active.row - 1) };
        break;
      case "ArrowRight":
        next = { ...active, column: Math.min(lastColumn, active.column + 1) };
        break;
      case "ArrowLeft":
        next = { ...active, column: Math.max(0, active.column - 1) };
        break;
      case "Home":
        next = { ...active, column: 0 };
        break;
      case "End":
        next = { ...active, column: lastColumn };
        break;
      case "PageDown":
        next = { ...active, row: Math.min(lastRow, active.row + 10) };
        break;
      case "PageUp":
        next = { ...active, row: Math.max(0, active.row - 10) };
        break;
      case "Enter":
      case " ": {
        const column = visibleColumns[active.column];
        const record = rows[active.row];
        if (column && record) {
          event.preventDefault();
          openViewer(record, active.column, active.row, column);
        }
        return;
      }
      default:
        return;
    }

    event.preventDefault();
    setActiveCell(next);
  }

  function cellId(row: number, column: number) {
    return `${gridId}-r${row}-c${column}`;
  }

  function toggleRowSelection(record: DatabaseRowRecord, selected: boolean) {
    const key = identityKey(record);
    setSelectedRows((current) => {
      const next = new Set(current);
      if (selected) {
        next.add(key);
      } else {
        next.delete(key);
      }
      return next;
    });
  }

  function togglePageSelection(selected: boolean) {
    if (!selected) {
      setSelectedRows(new Set());
      return;
    }
    setSelectedRows(new Set(rows.map(identityKey)));
  }

  async function handleInsert(values: Parameters<typeof insertRow>[2]) {
    if (!result) {
      return;
    }
    const written = await insertRow(profileId, tableRef, values);
    if (written) {
      setInsertOpen(false);
      handleRefresh();
    }
  }

  async function handleUpdate(
    identity: DatabaseRowIdentityValue[],
    payload: { values: DatabaseRowWriteValue[]; expected: DatabaseRowIdentityValue[] },
  ) {
    if (!result) {
      return;
    }
    const written = await updateRow(
      profileId,
      tableRef,
      identity,
      payload.values,
      payload.expected,
    );
    if (written) {
      setEditOpen(false);
      handleRefresh();
    }
  }

  async function handleDelete() {
    const targets = deleteTargets;
    if (!targets || targets.length === 0) {
      return;
    }
    const written = await deleteRows(profileId, tableRef, targets, true);
    if (written) {
      setDeleteOpen(false);
      setSelectedRows(new Set());
      handleRefresh();
    } else {
      setDeleteOpen(false);
    }
  }

  function beginDeleteActive() {
    if (!activeRecord) {
      return;
    }
    setDeleteTargets([activeRecord.identity]);
    setDeleteOpen(true);
  }

  function beginDeleteSelected() {
    const targets = rows
      .filter((record) => selectedRows.has(identityKey(record)))
      .map((record) => record.identity);
    if (targets.length > 0) {
      setDeleteTargets(targets);
      setDeleteOpen(true);
    }
  }

  const canPrevious = grid.offset > 0;
  const canNext = result?.page.hasMore ?? false;
  const currentPage = Math.floor(grid.offset / grid.limit) + 1;
  const totalPages = exact ? Math.max(1, Math.ceil(exact.value / grid.limit)) : null;
  const sampleTime = formatSampleTime(entry?.updatedAt);
  const filtersActive = grid.filters.length > 0 || grid.search.length > 0;
  const searchable = result?.columns.some((column) => column.searchable) ?? false;
  const rowEditingReason = rowEditingBlockedReason(result, readOnly);
  const rowEditingBlocked = rowEditingReason !== null;
  const writePending = writeEntry?.status === "running";
  const writeResult = writeEntry?.status === "succeeded" ? (writeEntry.result ?? null) : null;
  const writeError =
    writeEntry?.status === "failed"
      ? (writeEntry.errorMessage ?? writeEntry.error?.message ?? "The row write failed.")
      : null;
  const writeErrorDetails =
    writeEntry?.status === "failed" ? (writeEntry.error?.details ?? null) : null;
  const activeRecord = rows[active.row];
  const selectedCount = selectedRows.size;
  const allPageSelected =
    rows.length > 0 && rows.every((record) => selectedRows.has(identityKey(record)));

  if (!connected) {
    return (
      <div className="space-y-4">
        <div className="min-w-0">
          <h2 className="truncate text-[15px] font-semibold tracking-[-0.01em] text-foreground">
            {table}
          </h2>
          <p className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground">
            {database ?? "Active database"}
            {schema ? ` / ${schema}` : ""} · {profileName}
          </p>
        </div>
        <EmptyState
          icon={<PlugsIcon className="size-4" aria-hidden="true" />}
          title="Connection required"
          description={`Connect ${profileName} over SSH before browsing ${table}. Cached rows are hidden while the connection is not live.`}
          action={
            <Button size="sm" onClick={onConnect}>
              <PlugsIcon className="size-3.5" aria-hidden="true" />
              Connect {profileName}
            </Button>
          }
        />
      </div>
    );
  }

  const initialError = status === "error" && !result;
  const cancelled = status === "cancelled" && !result;

  return (
    <div className="min-w-0 space-y-3" aria-label={`Data browser for ${table}`}>
      <div className="flex flex-wrap items-start justify-between gap-x-3 gap-y-2">
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <span className="grid size-8 shrink-0 place-items-center rounded-md border border-border bg-card text-muted-foreground">
              <RowsIcon className="size-4" aria-hidden="true" />
            </span>
            <div className="min-w-0">
              <h2 className="truncate text-[15px] font-semibold tracking-[-0.01em] text-foreground">
                {table}
              </h2>
              <p className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground">
                {database ?? "Active database"}
                {schema ? ` / ${schema}` : ""} · {result?.engine ?? "database"} ·{" "}
                {result
                  ? `${visibleColumns.length} of ${allColumns.length} columns visible`
                  : "Columns loading…"}
              </p>
            </div>
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            {estimate !== null ? (
              <Badge
                variant="outline"
                title="Engine statistics estimate. It is not an exact row count."
              >
                ~{formatEstimatedCount(estimate)} est. rows
              </Badge>
            ) : (
              <Badge variant="outline" title="The engine has no row-count estimate for this table.">
                Row estimate unavailable
              </Badge>
            )}
            {exact ? (
              <Badge
                variant="info"
                title={`COUNT(*) ${exact.filtered ? "with the current filters and search" : "for the whole table"}.`}
              >
                {formatEstimatedCount(exact.value)} exact{exact.filtered ? " filtered" : ""}
              </Badge>
            ) : null}
            {sampleTime ? (
              <span
                className="inline-flex items-center gap-1 text-[10px] text-muted-foreground"
                title={entry?.updatedAt ? new Date(entry.updatedAt).toLocaleString() : undefined}
              >
                <ClockIcon className="size-3" aria-hidden="true" />
                Sampled {sampleTime}
              </span>
            ) : null}
            {result && result.readOnlyReasons.includes("no_stable_identity") ? (
              <Badge variant="warning" title="Row editing requires a stable row identity.">
                No stable identity
              </Badge>
            ) : null}
            {readOnly ? <Badge variant="warning">Read-only session</Badge> : null}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          <Button
            variant="secondary"
            size="sm"
            onClick={handleRefresh}
            disabled={loading}
            aria-label="Refresh table rows"
          >
            {loading ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            Refresh
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void cancelRows(profileId, tableRef)}
            disabled={!loading}
            aria-label="Cancel row read"
          >
            <XIcon className="size-3.5" aria-hidden="true" />
            Cancel
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setColumnsOpen(true)}
            disabled={!result}
            aria-label="Choose columns and widths"
          >
            <ColumnsIcon className="size-3.5" aria-hidden="true" />
            Columns
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setInsertOpen(true)}
            disabled={rowEditingBlocked || writePending}
            title={
              rowEditingReason ?? "Insert a row with typed values and explicit NULL/default modes"
            }
            aria-label="Insert row"
          >
            <PlusIcon className="size-3.5" aria-hidden="true" />
            Insert row
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setEditOpen(true)}
            disabled={rowEditingBlocked || writePending || !activeRecord}
            title={rowEditingReason ?? "Edit the active row"}
            aria-label="Edit row"
          >
            <PencilSimpleIcon className="size-3.5" aria-hidden="true" />
            Edit row
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={beginDeleteActive}
            disabled={rowEditingBlocked || writePending || !activeRecord}
            title={rowEditingReason ?? "Delete the active row"}
            aria-label="Delete row"
          >
            <TrashIcon className="size-3.5" aria-hidden="true" />
            Delete row
          </Button>
          <Button
            variant={selectedCount > 0 ? "destructive" : "secondary"}
            size="sm"
            onClick={beginDeleteSelected}
            disabled={rowEditingBlocked || writePending || selectedCount === 0}
            title={
              rowEditingReason ??
              `Delete ${selectedCount} selected row${selectedCount === 1 ? "" : "s"} after confirmation`
            }
            aria-label="Delete selected rows"
          >
            <TrashIcon className="size-3.5" aria-hidden="true" />
            Delete selected ({selectedCount})
          </Button>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <form
          className="flex min-w-0 flex-1 items-center gap-1.5 sm:max-w-md"
          onSubmit={(event) => {
            event.preventDefault();
            applySearch();
          }}
        >
          <div className="relative min-w-0 flex-1">
            <MagnifyingGlassIcon
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <Input
              aria-label="Search text columns"
              value={searchDraft}
              maxLength={256}
              onChange={(event) => setSearchDraft(event.target.value)}
              placeholder="Search text columns…"
              disabled={!result || !searchable}
              className="h-8 bg-card pr-2 pl-8 text-[12px]"
            />
          </div>
          <Button
            type="submit"
            variant="secondary"
            size="sm"
            disabled={!result || !searchable || searchDraft.trim() === grid.search}
            aria-label="Apply search"
          >
            Search
          </Button>
          {grid.search ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => {
                setSearchDraft("");
                updateGrid({ search: "", offset: 0 });
              }}
              aria-label="Clear search"
            >
              <XIcon className="size-3.5" aria-hidden="true" />
            </Button>
          ) : null}
        </form>

        <Button
          type="button"
          variant={filtersOpen || grid.filters.length > 0 ? "secondary" : "ghost"}
          size="sm"
          onClick={handleFiltersToggle}
          disabled={!result}
          aria-expanded={filtersOpen}
          aria-controls="database-data-filters"
          aria-label="Table filters"
        >
          <FunnelIcon className="size-3.5" aria-hidden="true" />
          Filters{grid.filters.length > 0 ? ` (${grid.filters.length})` : ""}
        </Button>

        <label
          className="inline-flex items-center gap-1.5 text-[10px] text-muted-foreground"
          title="Run an additional COUNT(*) with the current filters and search."
        >
          <input
            type="checkbox"
            className="size-3.5 accent-[var(--primary)]"
            checked={grid.includeExactCount}
            aria-label="Include exact row count"
            onChange={(event) => updateGrid({ includeExactCount: event.target.checked })}
          />
          Exact count
        </label>
      </div>

      {grid.filters.length > 0 || grid.search ? (
        <div aria-label="Applied filters" className="flex flex-wrap items-center gap-1.5">
          <FunnelIcon className="size-3 text-muted-foreground" aria-hidden="true" />
          {grid.filters.map((filter, index) => {
            const summary = filterSummary(filter);
            return (
              <span
                key={`${summary}-${index}`}
                className="inline-flex max-w-full items-center gap-1 rounded-full border border-border bg-muted/30 py-0.5 pr-0.5 pl-2"
              >
                <span className="truncate font-mono text-[10px] text-foreground">{summary}</span>
                <button
                  type="button"
                  className="grid size-4 shrink-0 place-items-center rounded-full text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
                  aria-label={`Remove filter ${summary}`}
                  onClick={() => removeFilter(index)}
                >
                  <XIcon className="size-2.5" aria-hidden="true" />
                </button>
              </span>
            );
          })}
          {grid.search ? (
            <span className="inline-flex max-w-full items-center gap-1 rounded-full border border-border bg-muted/30 py-0.5 pr-0.5 pl-2">
              <span className="truncate font-mono text-[10px] text-foreground">
                search &quot;{grid.search}&quot;
              </span>
              <button
                type="button"
                className="grid size-4 shrink-0 place-items-center rounded-full text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
                aria-label="Remove search filter"
                onClick={() => {
                  setSearchDraft("");
                  updateGrid({ search: "", offset: 0 });
                }}
              >
                <XIcon className="size-2.5" aria-hidden="true" />
              </button>
            </span>
          ) : null}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 px-2 text-[10px]"
            onClick={clearFilters}
          >
            Clear all
          </Button>
        </div>
      ) : null}

      {filtersOpen ? (
        <div
          id="database-data-filters"
          role="region"
          aria-label="Filter builder"
          className="space-y-2 rounded-md border border-border bg-muted/15 p-3"
        >
          {filterDrafts.length === 0 ? (
            <p className="text-[11px] text-muted-foreground">
              Add a filter to reduce the page on the server. Values are bound parameters, never SQL
              text.
            </p>
          ) : (
            <div className="space-y-2">
              {filterDrafts.map((draft, index) => {
                const column = columnsByName.get(draft.column);
                const operators = column ? filterOperatorsFor(column.kind) : [];
                const operator = draft.operator;
                const needsValue = operator !== "is_null" && operator !== "is_not_null";
                return (
                  <div
                    key={draft.id}
                    className="flex flex-wrap items-end gap-2 rounded-md border border-border/70 bg-card/60 p-2"
                  >
                    <div className="min-w-32 flex-1">
                      <span className="mb-1 block text-[9px] font-medium tracking-wide text-muted-foreground uppercase">
                        Column
                      </span>
                      <Select
                        value={draft.column}
                        onValueChange={(value) => handleFilterColumnChange(index, value)}
                      >
                        <SelectTrigger
                          aria-label={`Filter column ${index + 1}`}
                          className="h-8 bg-card text-[12px]"
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {allColumns.map((entry) => (
                            <SelectItem key={entry.name} value={entry.name}>
                              {entry.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="min-w-28 flex-1">
                      <span className="mb-1 block text-[9px] font-medium tracking-wide text-muted-foreground uppercase">
                        Operator
                      </span>
                      <Select
                        value={operator}
                        onValueChange={(value) =>
                          setFilterDrafts((current) =>
                            current.map((entry, draftIndex) =>
                              draftIndex === index
                                ? { ...entry, operator: value as DatabaseRowFilterOperator }
                                : entry,
                            ),
                          )
                        }
                      >
                        <SelectTrigger
                          aria-label={`Filter operator ${index + 1}`}
                          className="h-8 bg-card text-[12px]"
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {operators.map((entry) => (
                            <SelectItem key={entry} value={entry}>
                              {filterOperatorLabel(entry)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="min-w-36 flex-[1.4]">
                      <span className="mb-1 block text-[9px] font-medium tracking-wide text-muted-foreground uppercase">
                        Value
                      </span>
                      {!needsValue ? (
                        <div className="flex h-8 items-center text-[11px] text-muted-foreground">
                          No value needed
                        </div>
                      ) : column?.kind === "boolean" ? (
                        <Select
                          value={draft.value || "true"}
                          onValueChange={(value) =>
                            setFilterDrafts((current) =>
                              current.map((entry, draftIndex) =>
                                draftIndex === index ? { ...entry, value } : entry,
                              ),
                            )
                          }
                        >
                          <SelectTrigger
                            aria-label={`Filter value ${index + 1}`}
                            className="h-8 bg-card text-[12px]"
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="true">true</SelectItem>
                            <SelectItem value="false">false</SelectItem>
                          </SelectContent>
                        </Select>
                      ) : (
                        <Input
                          aria-label={`Filter value ${index + 1}`}
                          value={draft.value}
                          maxLength={512}
                          onChange={(event) =>
                            setFilterDrafts((current) =>
                              current.map((entry, draftIndex) =>
                                draftIndex === index
                                  ? { ...entry, value: event.target.value }
                                  : entry,
                              ),
                            )
                          }
                          placeholder={operator === "in" ? "value1, value2" : "Filter value"}
                          className="h-8 bg-card text-[12px]"
                        />
                      )}
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Remove filter ${index + 1}`}
                      onClick={() =>
                        setFilterDrafts((current) =>
                          current.filter((_, draftIndex) => draftIndex !== index),
                        )
                      }
                    >
                      <XIcon className="size-3.5" aria-hidden="true" />
                    </Button>
                  </div>
                );
              })}
            </div>
          )}

          {filterError ? (
            <p role="alert" className="text-[11px] text-destructive">
              {filterError}
            </p>
          ) : null}

          <div className="flex flex-wrap items-center gap-1.5">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={handleAddFilter}
              disabled={filterDrafts.length >= MAX_FILTERS}
            >
              <FunnelIcon className="size-3.5" aria-hidden="true" />
              Add filter
            </Button>
            <Button type="button" size="sm" onClick={handleApplyFilters}>
              Apply filters
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={clearFilters}
              disabled={filterDrafts.length === 0 && grid.filters.length === 0}
            >
              Clear filters
            </Button>
            <span className="text-[10px] text-muted-foreground">
              Applying a filter resets to the first page.
            </span>
          </div>
        </div>
      ) : null}

      {rowEditingBlocked && result ? (
        <div
          role="note"
          aria-label="Row editing unavailable"
          className="flex items-start gap-2 rounded-md border border-warning/30 bg-warning/5 px-3 py-2"
        >
          <ProhibitIcon className="mt-0.5 size-3.5 shrink-0 text-warning" aria-hidden="true" />
          <p className="text-[10px] leading-4 text-muted-foreground">
            <span className="font-medium text-warning">Row editing is disabled.</span>{" "}
            {rowEditingReason}
          </p>
        </div>
      ) : null}

      {result && !rowEditingBlocked ? <RowWriteInfo /> : null}

      <RowWriteResultBanner
        result={writeResult}
        errorMessage={writeError}
        errorDetails={writeErrorDetails}
        onDismiss={() => clearWriteEntry(profileId, tableRef)}
        onRetry={handleRefresh}
      />

      {status === "error" && result ? (
        <Alert variant="destructive" className="py-2.5">
          <AlertTitle className="text-[12px]">Could not refresh table data</AlertTitle>
          <AlertDescription className="flex flex-wrap items-center gap-2">
            <span>{entry?.errorMessage ?? entry?.error?.message ?? "The row read failed."}</span>
            <Button variant="secondary" size="sm" onClick={handleRefresh}>
              Try again
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {status === "cancelled" && result ? (
        <Alert variant="warning" className="py-2.5">
          <AlertTitle className="text-[12px]">Row read cancelled</AlertTitle>
          <AlertDescription className="flex flex-wrap items-center gap-2">
            <span>The last page is still shown. Refresh to read it again.</span>
            <Button variant="secondary" size="sm" onClick={handleRefresh}>
              Refresh
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      <div className="overflow-hidden rounded-md border border-border bg-card">
        {loading && !result ? (
          <GridSkeleton />
        ) : initialError ? (
          <div className="p-3">
            <ErrorState
              title="Could not read table rows"
              message={entry?.errorMessage ?? entry?.error?.message ?? "The row read failed."}
              onRetry={handleRefresh}
            />
          </div>
        ) : cancelled ? (
          <div className="p-3">
            <EmptyState
              icon={<XIcon className="size-4" aria-hidden="true" />}
              title="Row read cancelled"
              description="No rows were replaced. Start the read again when you are ready."
              action={
                <Button size="sm" onClick={handleRefresh}>
                  <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
                  Try again
                </Button>
              }
            />
          </div>
        ) : rows.length === 0 ? (
          <div className="p-3">
            <EmptyState
              icon={<RowsIcon className="size-4" aria-hidden="true" />}
              title={filtersActive ? "No rows match" : "This table has no rows"}
              description={
                filtersActive
                  ? "The server returned no records for the current filters and search. Clear them to see the unfiltered page."
                  : grid.offset > 0
                    ? "This page is past the end of the table. Return to the first page to continue."
                    : "The table exists but contains no records yet."
              }
              action={
                filtersActive ? (
                  <Button size="sm" variant="secondary" onClick={clearFilters}>
                    Clear filters
                  </Button>
                ) : grid.offset > 0 ? (
                  <Button size="sm" variant="secondary" onClick={() => updateGrid({ offset: 0 })}>
                    First page
                  </Button>
                ) : undefined
              }
            />
          </div>
        ) : (
          <div
            className="max-h-[62vh] overflow-auto"
            aria-busy={loading}
            data-loading={loading ? "true" : undefined}
          >
            <table
              ref={gridRef}
              role="grid"
              tabIndex={0}
              aria-label={`Table data for ${table}`}
              aria-colcount={visibleColumns.length + 1}
              aria-activedescendant={
                rows.length > 0 && visibleColumns.length > 0
                  ? cellId(active.row, active.column)
                  : undefined
              }
              onKeyDown={handleGridKeyDown}
              className="w-full table-fixed border-collapse text-left text-[12px] outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/30"
              style={{
                minWidth: `${visibleColumns.reduce(
                  (total, column) =>
                    total +
                    (resizePreview?.column === column.name
                      ? resizePreview.width
                      : columnWidth(column, grid.columnWidths)),
                  76,
                )}px`,
              }}
            >
              <colgroup>
                <col style={{ width: 76 }} />
                {visibleColumns.map((column) => (
                  <col
                    key={column.name}
                    style={{
                      width:
                        resizePreview?.column === column.name
                          ? resizePreview.width
                          : columnWidth(column, grid.columnWidths),
                    }}
                  />
                ))}
              </colgroup>
              <thead role="rowgroup" className="sticky top-0 z-10 bg-card">
                <tr role="row" className="border-b border-border">
                  <th
                    role="columnheader"
                    aria-label="Row number"
                    className="border-r border-border/70 px-2 py-1.5 text-right text-[9px] font-medium tracking-wide text-muted-foreground"
                  >
                    <label className="flex items-center justify-end gap-1">
                      <input
                        type="checkbox"
                        className="size-3 accent-[var(--primary)]"
                        checked={allPageSelected}
                        disabled={rows.length === 0}
                        aria-label="Select all rows on this page"
                        onChange={(event) => togglePageSelection(event.target.checked)}
                      />
                      #
                    </label>
                  </th>
                  {visibleColumns.map((column, columnIndex) => {
                    const direction = sortDirectionFor(grid.sort, column.name);
                    const sortIndex = grid.sort.findIndex((entry) => entry.column === column.name);
                    return (
                      <th
                        key={column.name}
                        role="columnheader"
                        aria-colindex={columnIndex + 2}
                        aria-sort={
                          direction === "asc"
                            ? "ascending"
                            : direction === "desc"
                              ? "descending"
                              : "none"
                        }
                        aria-selected={active.column === columnIndex}
                        className="relative border-r border-border/70 px-1.5 py-1 text-left align-middle"
                        title={`${column.name} · ${column.dataType}${
                          column.nullable ? " · nullable" : ""
                        }${column.primaryKey ? " · primary key" : ""}`}
                      >
                        <div className="flex min-w-0 items-center gap-1">
                          <button
                            type="button"
                            className="group flex min-w-0 flex-1 items-center gap-1 rounded-sm text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                            aria-label={`Sort by ${column.name}`}
                            aria-pressed={direction !== null}
                            title={`${column.sortable ? "Sort" : "This column is not sortable"} by ${column.name}. Shift-click to add a secondary sort.`}
                            disabled={!column.sortable}
                            onFocus={() =>
                              setActiveCell((current) => ({
                                ...current,
                                column: columnIndex,
                              }))
                            }
                            onClick={(event) => handleSort(column, event.shiftKey)}
                          >
                            <span className="min-w-0 flex-1">
                              <span className="flex min-w-0 items-center gap-1">
                                <span className="truncate font-mono text-[11px] font-medium text-foreground">
                                  {column.name}
                                </span>
                                {column.primaryKey ? (
                                  <span
                                    className="text-[9px] text-warning"
                                    title="Primary key"
                                    aria-label="Primary key"
                                  >
                                    PK
                                  </span>
                                ) : null}
                                {sortIndex >= 0 && grid.sort.length > 1 ? (
                                  <span className="rounded-sm bg-muted px-1 text-[8px] text-muted-foreground">
                                    {sortIndex + 1}
                                  </span>
                                ) : null}
                              </span>
                              <span className="block truncate text-[9px] font-normal text-muted-foreground/70">
                                {column.dataType}
                              </span>
                            </span>
                            {direction === "asc" ? (
                              <CaretUpIcon
                                className="size-3 shrink-0 text-primary"
                                aria-hidden="true"
                              />
                            ) : direction === "desc" ? (
                              <CaretDownIcon
                                className="size-3 shrink-0 text-primary"
                                aria-hidden="true"
                              />
                            ) : (
                              <ArrowsDownUpIcon
                                className={cn(
                                  "size-3 shrink-0 text-muted-foreground/40",
                                  column.sortable && "group-hover:text-muted-foreground",
                                )}
                                aria-hidden="true"
                              />
                            )}
                          </button>
                          <span
                            role="separator"
                            aria-orientation="vertical"
                            aria-label={`Resize ${column.name} column`}
                            aria-valuemin={MIN_COLUMN_WIDTH}
                            aria-valuemax={MAX_COLUMN_WIDTH}
                            aria-valuenow={columnWidth(column, grid.columnWidths)}
                            tabIndex={0}
                            className="absolute top-0 right-0 h-full w-1.5 cursor-col-resize touch-none bg-transparent outline-none hover:bg-primary/40 focus-visible:bg-primary/50"
                            title={`Resize ${column.name}. Use arrow keys for keyboard resizing.`}
                            onPointerDown={(event) => beginResize(event, column)}
                            onKeyDown={(event) => resizeWithKeyboard(event, column)}
                          />
                        </div>
                      </th>
                    );
                  })}
                </tr>
              </thead>
              <tbody role="rowgroup">
                {rows.map((record, rowIndex) => {
                  const key = recordKey(record, `${grid.offset + rowIndex}`);
                  return (
                    <tr
                      key={key}
                      role="row"
                      aria-rowindex={rowIndex + 2}
                      className={cn(
                        "border-b border-border/70 last:border-0",
                        active.row === rowIndex && "bg-muted/20",
                      )}
                    >
                      <th
                        role="rowheader"
                        aria-colindex={1}
                        className="border-r border-border/70 px-2 py-1.5 text-right font-mono text-[9px] font-normal text-muted-foreground/70"
                      >
                        <label className="flex items-center justify-end gap-1">
                          <input
                            type="checkbox"
                            className="size-3 accent-[var(--primary)]"
                            checked={selectedRows.has(key)}
                            aria-label={`Select row ${grid.offset + rowIndex + 1}`}
                            onChange={(event) => toggleRowSelection(record, event.target.checked)}
                          />
                          {(grid.offset + rowIndex + 1).toLocaleString("en-US")}
                        </label>
                      </th>
                      {visibleColumns.map((column, columnIndex) => {
                        const cell = record.cells[columnIndex];
                        const presentation = cellPresentation(cell, column);
                        const isActive = active.row === rowIndex && active.column === columnIndex;
                        return (
                          <td
                            key={column.name}
                            id={cellId(rowIndex, columnIndex)}
                            role="gridcell"
                            aria-colindex={columnIndex + 2}
                            aria-selected={active.column === columnIndex}
                            tabIndex={-1}
                            title={presentation.title}
                            className={cn(
                              "border-r border-border/50 px-2 py-1.5 align-middle",
                              presentation.numeric && "text-right tabular-nums",
                              active.column === columnIndex && "bg-primary/[0.035]",
                              isActive && "relative z-[1] ring-2 ring-inset ring-primary/60",
                            )}
                            onClick={() => focusCell(rowIndex, columnIndex)}
                            onDoubleClick={() => openViewer(record, columnIndex, rowIndex, column)}
                          >
                            <CellValue
                              presentation={presentation}
                              columnName={column.name}
                              rowNumber={grid.offset + rowIndex + 1}
                              onView={() => openViewer(record, columnIndex, rowIndex, column)}
                            />
                          </td>
                        );
                      })}
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}

        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border bg-muted/15 px-2.5 py-2">
          <p role="status" aria-live="polite" className="min-w-0 text-[10px] text-muted-foreground">
            {loading
              ? "Loading page…"
              : result
                ? pageWindowLabel({
                    offset: result.page.offset,
                    returned: result.page.returned,
                    hasMore: result.page.hasMore,
                    exactRows: exact,
                  })
                : "No page loaded"}
            {result ? (
              <>
                {" "}
                · {result.page.returned} returned · {result.columns.length} columns in metadata
              </>
            ) : null}
          </p>

          <div className="flex flex-wrap items-center gap-1.5">
            <label className="flex items-center gap-1 text-[10px] text-muted-foreground">
              Rows
              <Select
                value={String(grid.limit)}
                onValueChange={(value) => updateGrid({ limit: Number(value), offset: 0 })}
              >
                <SelectTrigger
                  aria-label="Rows per page"
                  className="h-7 w-16 bg-card px-2 text-[11px]"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {DATA_GRID_PAGE_SIZES.map((size) => (
                    <SelectItem key={size} value={String(size)}>
                      {size}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <Button
              type="button"
              variant="secondary"
              size="icon-sm"
              className="size-7"
              onClick={() => updateGrid({ offset: previousPageOffset(grid.offset, grid.limit) })}
              disabled={!canPrevious || loading}
              aria-label="Previous page"
            >
              <CaretLeftIcon className="size-3.5" aria-hidden="true" />
            </Button>
            <span className="min-w-16 text-center font-mono text-[10px] text-muted-foreground">
              Page {currentPage}
              {totalPages ? ` of ${totalPages}` : ""}
            </span>
            <Button
              type="button"
              variant="secondary"
              size="icon-sm"
              className="size-7"
              onClick={() => updateGrid({ offset: nextPageOffset(grid.offset, grid.limit) })}
              disabled={!canNext || loading}
              aria-label="Next page"
            >
              <CaretRightIcon className="size-3.5" aria-hidden="true" />
            </Button>
          </div>
        </div>
      </div>

      {insertOpen ? (
        <InsertRowDialog
          open
          onOpenChange={(open) => {
            setInsertOpen(open);
            if (!open) {
              clearWriteEntry(profileId, tableRef);
            }
          }}
          tableName={table}
          columns={allColumns}
          pending={writePending}
          error={writeError}
          onInsert={(values) => void handleInsert(values)}
        />
      ) : null}

      {editOpen && result ? (
        <EditRowDialog
          key={activeRecord ? identityKey(activeRecord) : "empty"}
          open
          onOpenChange={(open) => {
            setEditOpen(open);
            if (!open) {
              clearWriteEntry(profileId, tableRef);
            }
          }}
          tableName={table}
          result={result}
          record={activeRecord ?? null}
          pending={writePending}
          error={writeError}
          onUpdate={(identity, payload) => {
            if (typeof payload !== "string") {
              void handleUpdate(identity, payload);
            }
          }}
        />
      ) : null}

      <ConfirmDialog
        open={deleteOpen}
        onOpenChange={(open) => {
          if (!open && !writePending) {
            setDeleteOpen(false);
            clearWriteEntry(profileId, tableRef);
          }
        }}
        title={
          deleteTargets?.length === 1
            ? `Delete 1 row from ${table}?`
            : `Delete ${deleteTargets?.length ?? 0} rows from ${table}?`
        }
        description="Only the selected row identities are deleted. The delete runs in one transaction and reports the affected rows after verification."
        confirmLabel={deleteTargets?.length === 1 ? "Delete row" : "Delete rows"}
        details={[
          { label: "Connection", value: profileName },
          {
            label: "Table",
            value: `${database ?? "active database"}${schema ? ` / ${schema}` : ""} / ${table}`,
          },
          { label: "Rows", value: String(deleteTargets?.length ?? 0) },
          {
            label: "Scope",
            value:
              deleteTargets?.length === 1
                ? "Single row by stable identity"
                : "Bulk delete scoped to the selected stable identities",
          },
        ]}
        warningTitle="This permanently deletes the selected rows."
        warningDescription="No table-wide or predicate-based delete is accepted. The transaction rolls back if any selected row is missing or changed."
        onConfirm={() => void handleDelete()}
        pending={writePending}
      />

      <ColumnSettingsDialog
        open={columnsOpen}
        onOpenChange={setColumnsOpen}
        columns={allColumns}
        hiddenColumns={grid.hiddenColumns}
        columnWidths={grid.columnWidths}
        onApply={(hidden, widths) => {
          setColumnsOpen(false);
          updateGrid({ hiddenColumns: hidden, columnWidths: widths, offset: 0 });
        }}
      />

      <LoadedValueViewer target={viewer} tableName={table} onClose={() => setViewer(null)} />
    </div>
  );
}
