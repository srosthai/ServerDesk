import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  ArrowClockwiseIcon,
  CaretRightIcon,
  CircleNotchIcon,
  DatabaseIcon,
  EyeIcon,
  KeyIcon,
  ListNumbersIcon,
  LockKeyIcon,
  MagnifyingGlassIcon,
  PlugsIcon,
  TableIcon,
  TreeStructureIcon,
  WarningCircleIcon,
  WifiSlashIcon,
} from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  buildDatabaseExplorerRows,
  databaseConnectionRowId,
  databaseExplorerSelectionForRow,
  loadedDiscoveryNodes,
  pageRequestForEntry,
  type BuildDatabaseExplorerRowsInput,
  type DatabaseExplorerRow,
  type DatabaseExplorerSelection,
} from "@/features/databases/databaseExplorerTreeModel";
import { cn } from "@/lib/utils";
import { databaseExplorerNodeKey, useDatabaseExplorerStore } from "@/stores/databaseExplorerStore";
import type { DatabaseConnectionSnapshot, DatabaseProfile } from "@/types/contracts";

type DatabaseExplorerTreeProps = {
  profiles: DatabaseProfile[];
  profileStatus: "idle" | "loading" | "ready" | "error";
  snapshots: Record<string, DatabaseConnectionSnapshot>;
  selectedProfileId: string | null;
  selection: DatabaseExplorerSelection | null;
  onSelectProfile: (profileId: string) => void;
  onSelectionChange: (selection: DatabaseExplorerSelection | null) => void;
  onConnectProfile: (profile: DatabaseProfile) => void;
  onAddConnection: () => void;
  onRetryProfiles: () => void;
};

const ROW_HEIGHT = 32;
const OVERSCAN = 8;
const FALLBACK_VIEWPORT_HEIGHT = 420;

function RowIcon({ row }: { row: DatabaseExplorerRow }) {
  const className = "size-3.5 shrink-0";
  const ariaHidden = true;

  switch (row.kind) {
    case "connection":
      return <PlugsIcon className={className} aria-hidden={ariaHidden} />;
    case "database":
      return <DatabaseIcon className={className} aria-hidden={ariaHidden} />;
    case "schemas-group":
      return <TreeStructureIcon className={className} aria-hidden={ariaHidden} />;
    case "schema":
      return <TreeStructureIcon className={className} aria-hidden={ariaHidden} />;
    case "object-group":
      return row.label === "Sequences" ? (
        <ListNumbersIcon className={className} aria-hidden={ariaHidden} />
      ) : row.label === "Views" ? (
        <EyeIcon className={className} aria-hidden={ariaHidden} />
      ) : (
        <TableIcon className={className} aria-hidden={ariaHidden} />
      );
    case "view":
      return <EyeIcon className={className} aria-hidden={ariaHidden} />;
    case "sequence":
      return <KeyIcon className={className} aria-hidden={ariaHidden} />;
    default:
      return <TableIcon className={className} aria-hidden={ariaHidden} />;
  }
}

function StatusRow({
  row,
  onAction,
}: {
  row: DatabaseExplorerRow;
  onAction: (row: DatabaseExplorerRow) => void;
}) {
  const paddingLeft = 10 + Math.max(0, row.level - 1) * 14;

  if (row.status === "loading") {
    return (
      <div
        role="status"
        aria-label={row.label || "Loading metadata"}
        className="flex h-full items-center gap-2 pr-2"
        style={{ paddingLeft }}
      >
        <Skeleton
          className={cn("h-2.5", row.id.endsWith("loading-1") ? "w-32" : "w-20 opacity-60")}
        />
      </div>
    );
  }

  if (row.action === "connect") {
    return (
      <div className="flex h-full items-center" style={{ paddingLeft }}>
        <button
          type="button"
          className="inline-flex h-6 items-center gap-1.5 rounded-sm border border-border bg-card px-2 text-[10px] font-medium text-foreground outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/40"
          onClick={(event) => {
            event.stopPropagation();
            onAction(row);
          }}
        >
          <PlugsIcon className="size-3" aria-hidden="true" />
          {row.label}
        </button>
      </div>
    );
  }

  const destructive = row.status === "error" || row.status === "permission";
  const Icon =
    row.status === "permission"
      ? LockKeyIcon
      : row.status === "disconnected"
        ? WifiSlashIcon
        : row.status === "error"
          ? WarningCircleIcon
          : null;
  const retry = row.action === "retry-node" || row.action === "retry-load-more" ? row.action : null;

  return (
    <div
      role={row.status === "error" || row.status === "permission" ? "alert" : "status"}
      title={row.detail ?? row.label}
      className={cn(
        "flex h-full min-w-0 items-center gap-1.5 pr-2 text-[10px] leading-4",
        destructive ? "text-destructive" : "text-muted-foreground",
      )}
      style={{ paddingLeft }}
    >
      {Icon ? <Icon className="size-3 shrink-0" aria-hidden="true" /> : null}
      <span className="truncate">
        {row.label}
        {row.detail ? <span className="ml-1 opacity-70">· {row.detail}</span> : null}
      </span>
      {retry ? (
        <button
          type="button"
          className="ml-auto shrink-0 rounded-sm px-1.5 py-0.5 text-[10px] font-medium underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/40"
          onClick={(event) => {
            event.stopPropagation();
            onAction(row);
          }}
        >
          Try again
        </button>
      ) : null}
    </div>
  );
}

function LoadMoreRow({
  row,
  onLoadMore,
}: {
  row: DatabaseExplorerRow;
  onLoadMore: (row: DatabaseExplorerRow) => void;
}) {
  return (
    <div
      className="flex h-full items-center"
      style={{ paddingLeft: 10 + Math.max(0, row.level - 1) * 14 }}
    >
      <button
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          onLoadMore(row);
        }}
        disabled={row.busy}
        className="inline-flex h-6 items-center gap-1.5 rounded-sm px-1.5 text-[10px] font-medium text-primary outline-none hover:bg-primary/10 focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-60"
      >
        {row.busy ? (
          <CircleNotchIcon className="size-3 animate-spin" aria-hidden="true" />
        ) : (
          <ArrowClockwiseIcon className="size-3" aria-hidden="true" />
        )}
        {row.label}
      </button>
    </div>
  );
}

function DatabaseExplorerTree({
  profiles,
  profileStatus,
  snapshots,
  selectedProfileId,
  selection,
  onSelectProfile,
  onSelectionChange,
  onConnectProfile,
  onAddConnection,
  onRetryProfiles,
}: DatabaseExplorerTreeProps) {
  const nodes = useDatabaseExplorerStore((state) => state.nodes);
  const loadNode = useDatabaseExplorerStore((state) => state.loadNode);
  const loadMoreNode = useDatabaseExplorerStore((state) => state.loadMoreNode);
  const refreshNode = useDatabaseExplorerStore((state) => state.refreshNode);

  const [query, setQuery] = useState("");
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => new Set());
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [treeRefreshing, setTreeRefreshing] = useState(false);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const pendingFocusId = useRef<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const modelInput: BuildDatabaseExplorerRowsInput = useMemo(
    () => ({
      profiles,
      snapshots,
      nodes,
      expandedIds,
      query,
    }),
    [profiles, snapshots, nodes, expandedIds, query],
  );

  const rows = useMemo(() => buildDatabaseExplorerRows(modelInput), [modelInput]);
  const searchActive = query.trim().length > 0;
  const navigableRows = useMemo(() => rows.filter((row) => row.selectable), [rows]);
  const selectedId =
    selection?.profileId === selectedProfileId && selection
      ? selection.id
      : selectedProfileId
        ? databaseConnectionRowId(selectedProfileId)
        : null;
  const fallbackFocusId = selectedId ?? navigableRows[0]?.id ?? null;
  const activeFocusId =
    focusedId && navigableRows.some((row) => row.id === focusedId) ? focusedId : fallbackFocusId;
  const focusedIndex = activeFocusId
    ? navigableRows.findIndex((row) => row.id === activeFocusId)
    : -1;

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    const update = () => {
      setViewportHeight(container.clientHeight);
      setScrollTop(container.scrollTop);
    };
    update();
    container.addEventListener("scroll", update, { passive: true });
    window.addEventListener("resize", update);
    const observer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => update());
    observer?.observe(container);

    return () => {
      container.removeEventListener("scroll", update);
      window.removeEventListener("resize", update);
      observer?.disconnect();
    };
  }, []);

  useLayoutEffect(() => {
    if (pendingFocusId.current !== activeFocusId || !activeFocusId) {
      return;
    }
    const container = containerRef.current;
    const index = rows.findIndex((row) => row.id === activeFocusId);
    if (!container || index < 0) {
      return;
    }

    const top = index * ROW_HEIGHT;
    const height = container.clientHeight || FALLBACK_VIEWPORT_HEIGHT;
    if (top < container.scrollTop) {
      container.scrollTop = top;
    } else if (top + ROW_HEIGHT > container.scrollTop + height) {
      container.scrollTop = Math.max(0, top + ROW_HEIGHT - height);
    }

    const element = container.querySelector<HTMLElement>(
      `[data-explorer-row-id="${activeFocusId}"]`,
    );
    element?.focus({ preventScroll: true });
    pendingFocusId.current = null;
  }, [activeFocusId, rows]);

  const ensureNode = useCallback(
    (row: DatabaseExplorerRow) => {
      if (!row.discoveryNode) {
        return;
      }
      const snapshot = snapshots[row.profileId];
      if (snapshot?.state !== "connected") {
        return;
      }
      const entry = nodes[databaseExplorerNodeKey(row.profileId, row.discoveryNode)];
      if (entry?.status === "loading" || (entry?.status === "ready" && entry.payload)) {
        return;
      }
      void loadNode(row.profileId, row.discoveryNode);
    },
    [loadNode, nodes, snapshots],
  );

  const selectRow = useCallback(
    (row: DatabaseExplorerRow) => {
      const nextSelection = databaseExplorerSelectionForRow(row);
      if (!nextSelection) {
        return;
      }
      onSelectionChange(nextSelection);
      onSelectProfile(row.profileId);
    },
    [onSelectProfile, onSelectionChange],
  );

  const setExpanded = useCallback((row: DatabaseExplorerRow, expanded: boolean) => {
    setExpandedIds((current) => {
      const next = new Set(current);
      if (expanded) {
        next.add(row.id);
      } else {
        next.delete(row.id);
      }
      return next;
    });
  }, []);

  const expandRow = useCallback(
    (row: DatabaseExplorerRow) => {
      setExpanded(row, true);
      ensureNode(row);
    },
    [ensureNode, setExpanded],
  );

  const toggleRow = useCallback(
    (row: DatabaseExplorerRow) => {
      if (!row.expandable) {
        return;
      }
      if (expandedIds.has(row.id)) {
        setExpanded(row, false);
      } else {
        expandRow(row);
      }
    },
    [expandRow, expandedIds, setExpanded],
  );

  const focusRow = useCallback((rowId: string) => {
    pendingFocusId.current = rowId;
    setFocusedId(rowId);
  }, []);

  const refreshRow = useCallback(
    (row: DatabaseExplorerRow) => {
      if (!row.discoveryNode) {
        return;
      }
      const entry = nodes[databaseExplorerNodeKey(row.profileId, row.discoveryNode)];
      if (entry?.status === "loading") {
        return;
      }
      void refreshNode(row.profileId, row.discoveryNode, pageRequestForEntry(entry));
    },
    [nodes, refreshNode],
  );

  const loadMore = useCallback(
    (row: DatabaseExplorerRow) => {
      if (!row.discoveryNode) {
        return;
      }
      const entry = nodes[databaseExplorerNodeKey(row.profileId, row.discoveryNode)];
      if (entry?.status === "loading" || entry?.loadingMore || !entry?.payload) {
        return;
      }
      void loadMoreNode(row.profileId, row.discoveryNode);
    },
    [loadMoreNode, nodes],
  );

  const refreshTree = useCallback(async () => {
    const targets = loadedDiscoveryNodes({ ...modelInput, expandAll: true });
    if (targets.length === 0) {
      return;
    }
    setTreeRefreshing(true);
    try {
      await Promise.allSettled(
        targets.map(({ profileId, node }) => {
          const entry = nodes[databaseExplorerNodeKey(profileId, node)];
          return refreshNode(profileId, node, pageRequestForEntry(entry));
        }),
      );
    } finally {
      setTreeRefreshing(false);
    }
  }, [modelInput, nodes, refreshNode]);

  const handleTreeKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      if ((event.target as HTMLElement).closest("button, input, a")) {
        return;
      }
      const currentIndex =
        focusedIndex >= 0
          ? focusedIndex
          : activeFocusId
            ? navigableRows.findIndex((row) => row.id === activeFocusId)
            : -1;
      const currentRow = currentIndex >= 0 ? navigableRows[currentIndex] : undefined;

      switch (event.key) {
        case "ArrowDown": {
          event.preventDefault();
          const next = navigableRows[Math.min(navigableRows.length - 1, currentIndex + 1)];
          if (next) {
            focusRow(next.id);
          }
          break;
        }
        case "ArrowUp": {
          event.preventDefault();
          const next = navigableRows[Math.max(0, currentIndex - 1)];
          if (next) {
            focusRow(next.id);
          }
          break;
        }
        case "Home": {
          event.preventDefault();
          if (navigableRows[0]) {
            focusRow(navigableRows[0].id);
          }
          break;
        }
        case "End": {
          event.preventDefault();
          const last = navigableRows[navigableRows.length - 1];
          if (last) {
            focusRow(last.id);
          }
          break;
        }
        case "ArrowRight": {
          if (!currentRow) {
            return;
          }
          event.preventDefault();
          if (currentRow.expandable && !currentRow.expanded) {
            expandRow(currentRow);
            return;
          }
          const firstChild = rows.find((row) => row.parentId === currentRow.id && row.selectable);
          if (firstChild) {
            focusRow(firstChild.id);
          }
          break;
        }
        case "ArrowLeft": {
          if (!currentRow) {
            return;
          }
          event.preventDefault();
          if (currentRow.expandable && currentRow.expanded) {
            setExpanded(currentRow, false);
            return;
          }
          const parent = currentRow.parentId
            ? rows.find((row) => row.id === currentRow.parentId && row.selectable)
            : undefined;
          if (parent) {
            focusRow(parent.id);
          }
          break;
        }
        case "Enter":
        case " ": {
          if (!currentRow) {
            return;
          }
          event.preventDefault();
          selectRow(currentRow);
          if (!searchActive && currentRow.expandable) {
            toggleRow(currentRow);
          }
          break;
        }
        default:
          break;
      }
    },
    [
      activeFocusId,
      expandRow,
      focusRow,
      focusedIndex,
      navigableRows,
      rows,
      searchActive,
      selectRow,
      setExpanded,
      toggleRow,
    ],
  );

  const handleRowAction = useCallback(
    (row: DatabaseExplorerRow) => {
      if (row.action === "connect") {
        const profile = profiles.find((entry) => entry.id === row.profileId);
        if (profile) {
          onConnectProfile(profile);
        }
        return;
      }
      if (row.action === "load-more" || row.action === "retry-load-more") {
        loadMore(row);
        return;
      }
      if (row.action === "retry-node" && row.discoveryNode) {
        void loadNode(row.profileId, row.discoveryNode);
      }
    },
    [loadMore, loadNode, onConnectProfile, profiles],
  );

  const effectiveViewport = viewportHeight > 0 ? viewportHeight : FALLBACK_VIEWPORT_HEIGHT;
  const totalHeight = rows.length * ROW_HEIGHT;
  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const endIndex = Math.min(
    rows.length,
    Math.ceil((scrollTop + effectiveViewport) / ROW_HEIGHT) + OVERSCAN,
  );
  const renderedIndexes: number[] = [];
  for (let index = startIndex; index < endIndex; index += 1) {
    renderedIndexes.push(index);
  }
  if (focusedIndex >= 0 && (focusedIndex < startIndex || focusedIndex >= endIndex)) {
    renderedIndexes.push(focusedIndex);
    renderedIndexes.sort((left, right) => left - right);
  }

  if (profileStatus === "loading" || (profileStatus === "idle" && profiles.length === 0)) {
    return (
      <div
        className="border-b border-border px-3 py-3"
        aria-busy="true"
        aria-label="Loading databases"
      >
        <div className="space-y-2">
          {Array.from({ length: 4 }).map((_, index) => (
            <div key={index} className="flex items-center gap-2">
              <Skeleton className="size-3.5 rounded-sm" />
              <Skeleton className={cn("h-2.5", index % 2 === 0 ? "w-36" : "w-24 opacity-70")} />
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (profileStatus === "error") {
    return (
      <div className="px-3 py-3">
        <div className="rounded-md border border-destructive/30 bg-destructive/8 px-2.5 py-2">
          <p className="flex items-center gap-1.5 text-[11px] font-medium text-destructive">
            <WarningCircleIcon className="size-3.5" aria-hidden="true" />
            Could not load connections
          </p>
          <Button variant="ghost" size="sm" className="mt-1 h-7 px-1.5" onClick={onRetryProfiles}>
            Try again
          </Button>
        </div>
      </div>
    );
  }

  if (profiles.length === 0) {
    return (
      <div className="px-3 py-3">
        <EmptyState
          className="border-border/80 bg-transparent px-3 py-4"
          icon={<PlugsIcon className="size-4" aria-hidden="true" />}
          title="No saved connections"
          description="Add a database profile to start exploring databases, schemas, tables, views, and sequences."
          action={
            <Button size="sm" variant="secondary" onClick={onAddConnection}>
              Add connection
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-b border-border px-3 py-2">
        <div className="flex items-center gap-1.5">
          <label htmlFor="database-explorer-search" className="sr-only">
            Search database explorer
          </label>
          <div className="relative min-w-0 flex-1">
            <MagnifyingGlassIcon
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <Input
              id="database-explorer-search"
              type="search"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Search objects"
              className="h-8 pl-8 text-[11px]"
            />
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => void refreshTree()}
            disabled={treeRefreshing}
            aria-label="Refresh entire tree"
            title="Refresh every loaded explorer node"
          >
            {treeRefreshing ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
          </Button>
        </div>
        <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
          {searchActive
            ? "Searching loaded branches. Expand a connection to include more objects."
            : "Objects load when expanded. Cached nodes refresh in place."}
        </p>
      </div>

      {rows.length === 0 ? (
        <div className="px-3 py-4">
          {searchActive ? (
            <p role="status" className="text-[11px] leading-5 text-muted-foreground">
              No loaded connections or objects match “{query.trim()}”. Expand a connection to load
              deeper metadata.
            </p>
          ) : (
            <p className="text-[11px] leading-5 text-muted-foreground">
              No database objects are available yet.
            </p>
          )}
        </div>
      ) : (
        <div
          ref={containerRef}
          role="tree"
          aria-label="Database object explorer"
          aria-busy={treeRefreshing}
          className="relative min-h-56 flex-1 overflow-auto py-1 lg:min-h-0"
          onKeyDown={handleTreeKeyDown}
        >
          <div className="relative" role="none" style={{ height: totalHeight }}>
            {renderedIndexes.map((index) => {
              const row = rows[index];
              if (!row) {
                return null;
              }

              if (row.kind === "status") {
                return (
                  <div
                    key={row.id}
                    role="none"
                    className="absolute inset-x-0"
                    style={{ top: index * ROW_HEIGHT, height: ROW_HEIGHT }}
                  >
                    {/* Status rows keep errors, permission and disconnection inside the tree. */}
                    <StatusRow row={row} onAction={handleRowAction} />
                  </div>
                );
              }

              if (row.kind === "load-more") {
                return (
                  <div
                    key={row.id}
                    role="none"
                    className="absolute inset-x-0"
                    style={{ top: index * ROW_HEIGHT, height: ROW_HEIGHT }}
                  >
                    <LoadMoreRow row={row} onLoadMore={loadMore} />
                  </div>
                );
              }

              const selected = selectedId === row.id;
              const isFocused = activeFocusId === row.id;

              return (
                <div
                  key={row.id}
                  role="treeitem"
                  aria-level={row.level}
                  aria-selected={selected}
                  aria-expanded={row.expandable ? row.expanded : undefined}
                  aria-busy={row.busy || undefined}
                  tabIndex={isFocused ? 0 : -1}
                  data-explorer-row-id={row.id}
                  data-explorer-index={index}
                  title={row.detail ? `${row.label} · ${row.detail}` : row.label}
                  onClick={() => {
                    selectRow(row);
                    toggleRow(row);
                  }}
                  onFocus={() => setFocusedId(row.id)}
                  className={cn(
                    "group absolute inset-x-0 flex cursor-default items-center gap-1.5 pr-1.5 text-[11px] outline-none select-none",
                    selected
                      ? "border-l-2 border-l-primary bg-primary/12 text-foreground"
                      : "border-l-2 border-l-transparent text-muted-foreground hover:bg-muted/55 hover:text-foreground focus-visible:bg-muted/55 focus-visible:text-foreground",
                  )}
                  style={{
                    top: index * ROW_HEIGHT,
                    height: ROW_HEIGHT,
                    paddingLeft: 6 + Math.max(0, row.level - 1) * 14,
                  }}
                >
                  <span className="grid size-3.5 shrink-0 place-items-center">
                    {row.expandable ? (
                      <CaretRightIcon
                        className={cn(
                          "size-3 transition-transform",
                          row.expanded && "rotate-90",
                          row.busy && "opacity-50",
                        )}
                        aria-hidden="true"
                      />
                    ) : null}
                  </span>
                  <RowIcon row={row} />
                  <span className="min-w-0 flex-1 truncate font-medium">{row.label}</span>
                  {row.system ? (
                    <span className="shrink-0 rounded-sm border border-border px-1 text-[9px] leading-4 text-muted-foreground">
                      system
                    </span>
                  ) : null}
                  {row.active ? (
                    <span className="shrink-0 rounded-sm border border-primary/35 bg-primary/10 px-1 text-[9px] leading-4 text-primary">
                      active
                    </span>
                  ) : null}
                  {typeof row.count === "number" ? (
                    <span className="shrink-0 text-[9px] tabular-nums text-muted-foreground">
                      {row.count}
                    </span>
                  ) : null}
                  {row.discoveryNode ? (
                    <button
                      type="button"
                      aria-label={`Refresh ${row.label}`}
                      title={`Refresh ${row.label}`}
                      disabled={row.busy}
                      onClick={(event) => {
                        event.stopPropagation();
                        refreshRow(row);
                      }}
                      className="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground opacity-0 outline-none transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:opacity-100 disabled:opacity-80"
                    >
                      {row.busy ? (
                        <CircleNotchIcon className="size-3 animate-spin" aria-hidden="true" />
                      ) : (
                        <ArrowClockwiseIcon className="size-3" aria-hidden="true" />
                      )}
                    </button>
                  ) : null}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

export { DatabaseExplorerTree };
