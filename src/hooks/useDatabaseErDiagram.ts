import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  ER_DIAGRAM_CATALOG_LIMIT,
  ER_DIAGRAM_DEFAULT_TABLE_LIMIT,
  erDiagramTableKey,
  selectErDiagramTables,
} from "@/features/databases/databaseErDiagramModel";
import { listConstraints, listTables } from "@/services/databaseDiscovery";
import type {
  CommandError,
  DatabaseConstraintSummary,
  DatabaseTableSummary,
} from "@/types/contracts";

export const ER_DIAGRAM_CONSTRAINT_BATCH_SIZE = 4;

type ConstraintEntry =
  | { status: "loading" }
  | { status: "ready"; items: DatabaseConstraintSummary[]; updatedAt: number }
  | { status: "error"; error: CommandError; updatedAt: number };

type CatalogEntry =
  | { status: "idle" }
  | { status: "loading" }
  | {
      status: "ready";
      tables: DatabaseTableSummary[];
      totalTables: number | null;
      catalogTruncated: boolean;
      updatedAt: number;
    }
  | { status: "error"; error: CommandError };

type StoredDiagram = {
  key: string;
  catalog: CatalogEntry;
  constraints: Record<string, ConstraintEntry>;
};

export type DatabaseErDiagramConstraintFailure = {
  key: string;
  table: string;
  error: CommandError;
};

export type DatabaseErDiagramState = {
  status: "idle" | "loading" | "ready" | "error";
  error: CommandError | null;
  tables: DatabaseTableSummary[];
  totalTables: number | null;
  catalogTruncated: boolean;
  constraints: Record<string, DatabaseConstraintSummary[]>;
  constraintFailures: DatabaseErDiagramConstraintFailure[];
  constraintLoaded: number;
  constraintTotal: number;
  constraintsLoading: boolean;
  updatedAt: number | null;
  reload: () => void;
};

export type UseDatabaseErDiagramInput = {
  profileId: string | null;
  database: string | null;
  schema: string | null;
  connected: boolean;
  query?: string;
  limit?: number;
};

const EMPTY_TABLES: DatabaseTableSummary[] = [];
const EMPTY_CONSTRAINTS: Record<string, DatabaseConstraintSummary[]> = {};
const EMPTY_FAILURES: DatabaseErDiagramConstraintFailure[] = [];
const EMPTY_STORED: StoredDiagram = {
  key: "",
  catalog: { status: "idle" },
  constraints: {},
};

function scopeKeyFor(profileId: string, database: string, schema: string | null): string {
  return [profileId, database, schema ?? ""].map(encodeURIComponent).join("::");
}

function commandErrorFor(error: unknown): CommandError {
  if (isCommandError(error)) {
    return error;
  }
  return { code: "internal", message: getCommandErrorMessage(error) };
}

function tableRef(table: DatabaseTableSummary) {
  return { database: table.database, schema: table.schema, table: table.name };
}

function tableKeyFor(table: DatabaseTableSummary): string {
  return erDiagramTableKey(table.database, table.schema, table.name);
}

/**
 * Loads the table catalog once, then only the constraint pages needed for the
 * currently visible tables. Query changes reuse the constraint cache, so a
 * large schema is filtered without re-reading metadata for every keystroke.
 */
export function useDatabaseErDiagram({
  profileId,
  database,
  schema,
  connected,
  query = "",
  limit = ER_DIAGRAM_DEFAULT_TABLE_LIMIT,
}: UseDatabaseErDiagramInput): DatabaseErDiagramState {
  const [reloadToken, setReloadToken] = useState(0);
  const [stored, setStored] = useState<StoredDiagram>(EMPTY_STORED);
  const storedRef = useRef(stored);
  const runtimeAvailable = isTauriRuntime();

  const key = profileId && database && connected ? scopeKeyFor(profileId, database, schema) : "";

  useEffect(() => {
    storedRef.current = stored;
  }, [stored]);

  useEffect(() => {
    if (!key || !profileId || !database || !connected || !runtimeAvailable) {
      return;
    }

    let active = true;

    void listTables(profileId, { database, schema }, { limit: ER_DIAGRAM_CATALOG_LIMIT, offset: 0 })
      .then((page) => {
        if (!active) {
          return;
        }
        const tables = page.items.slice(0, ER_DIAGRAM_CATALOG_LIMIT);
        setStored({
          key,
          catalog: {
            status: "ready",
            tables,
            totalTables: page.total ?? null,
            catalogTruncated: page.hasMore || page.items.length > ER_DIAGRAM_CATALOG_LIMIT,
            updatedAt: Date.now(),
          },
          constraints: {},
        });
      })
      .catch((error: unknown) => {
        if (active) {
          setStored({
            key,
            catalog: { status: "error", error: commandErrorFor(error) },
            constraints: {},
          });
        }
      });

    return () => {
      active = false;
    };
  }, [connected, database, key, profileId, reloadToken, runtimeAvailable, schema]);

  const catalogTables =
    stored.key === key && stored.catalog.status === "ready" ? stored.catalog.tables : EMPTY_TABLES;
  const selection = useMemo(
    () => selectErDiagramTables(catalogTables, query, limit),
    [catalogTables, limit, query],
  );
  const visibleTablesRef = useRef(selection.visible);
  const visibleSignature = selection.visible.map(tableKeyFor).join("|");

  useEffect(() => {
    visibleTablesRef.current = selection.visible;
  }, [selection]);

  useEffect(() => {
    if (!key || !profileId || !connected || !runtimeAvailable || !visibleSignature) {
      return;
    }

    let active = true;
    const tables = visibleTablesRef.current;
    const activeProfileId = profileId;

    async function loadVisibleConstraints(): Promise<void> {
      const snapshot = storedRef.current;
      if (snapshot.key !== key || snapshot.catalog.status !== "ready") {
        return;
      }
      const missing = tables.filter((table) => {
        const entry = snapshot.constraints[tableKeyFor(table)];
        return !entry || entry.status === "loading";
      });
      if (missing.length === 0) {
        return;
      }

      setStored((current) => {
        if (current.key !== key) {
          return current;
        }
        const constraints = { ...current.constraints };
        for (const table of missing) {
          constraints[tableKeyFor(table)] = { status: "loading" };
        }
        return { ...current, constraints };
      });

      for (let index = 0; index < missing.length; index += ER_DIAGRAM_CONSTRAINT_BATCH_SIZE) {
        if (!active) {
          return;
        }
        const batch = missing.slice(index, index + ER_DIAGRAM_CONSTRAINT_BATCH_SIZE);
        const results = await Promise.allSettled(
          batch.map((table) =>
            listConstraints(activeProfileId, tableRef(table), { limit: 500, offset: 0 }),
          ),
        );
        if (!active) {
          return;
        }

        setStored((current) => {
          if (current.key !== key) {
            return current;
          }
          const constraints = { ...current.constraints };
          const updatedAt = Date.now();
          batch.forEach((table, batchIndex) => {
            const tableKey = tableKeyFor(table);
            const result = results[batchIndex];
            if (result.status === "fulfilled") {
              if (result.value.hasMore) {
                constraints[tableKey] = {
                  status: "error",
                  error: {
                    code: "internal",
                    message: `${table.name} has more than 500 constraints, so its relationships may be incomplete.`,
                  },
                  updatedAt,
                };
                return;
              }
              constraints[tableKey] = {
                status: "ready",
                items: result.value.items,
                updatedAt,
              };
              return;
            }
            constraints[tableKey] = {
              status: "error",
              error: commandErrorFor(result.reason),
              updatedAt,
            };
          });
          return { ...current, constraints };
        });
      }
    }

    void loadVisibleConstraints();

    return () => {
      active = false;
    };
  }, [connected, key, profileId, runtimeAvailable, visibleSignature]);

  const reload = useCallback(() => {
    if (!key) {
      return;
    }
    setStored({ key, catalog: { status: "loading" }, constraints: {} });
    setReloadToken((current) => current + 1);
  }, [key]);

  return useMemo<DatabaseErDiagramState>(() => {
    if (!connected || !key) {
      return {
        status: "idle",
        error: null,
        tables: EMPTY_TABLES,
        totalTables: null,
        catalogTruncated: false,
        constraints: EMPTY_CONSTRAINTS,
        constraintFailures: EMPTY_FAILURES,
        constraintLoaded: 0,
        constraintTotal: 0,
        constraintsLoading: false,
        updatedAt: null,
        reload,
      };
    }

    if (!runtimeAvailable) {
      return {
        status: "error",
        error: {
          code: "connection",
          message: "Database relationship metadata is available in the desktop application.",
        },
        tables: EMPTY_TABLES,
        totalTables: null,
        catalogTruncated: false,
        constraints: EMPTY_CONSTRAINTS,
        constraintFailures: EMPTY_FAILURES,
        constraintLoaded: 0,
        constraintTotal: 0,
        constraintsLoading: false,
        updatedAt: null,
        reload,
      };
    }

    if (stored.key !== key || stored.catalog.status === "loading") {
      return {
        status: "loading",
        error: null,
        tables: EMPTY_TABLES,
        totalTables: null,
        catalogTruncated: false,
        constraints: EMPTY_CONSTRAINTS,
        constraintFailures: EMPTY_FAILURES,
        constraintLoaded: 0,
        constraintTotal: selection.visible.length,
        constraintsLoading: selection.visible.length > 0,
        updatedAt: null,
        reload,
      };
    }

    if (stored.catalog.status === "error") {
      return {
        status: "error",
        error: stored.catalog.error,
        tables: EMPTY_TABLES,
        totalTables: null,
        catalogTruncated: false,
        constraints: EMPTY_CONSTRAINTS,
        constraintFailures: EMPTY_FAILURES,
        constraintLoaded: 0,
        constraintTotal: 0,
        constraintsLoading: false,
        updatedAt: null,
        reload,
      };
    }

    if (stored.catalog.status === "idle") {
      return {
        status: "idle",
        error: null,
        tables: EMPTY_TABLES,
        totalTables: null,
        catalogTruncated: false,
        constraints: EMPTY_CONSTRAINTS,
        constraintFailures: EMPTY_FAILURES,
        constraintLoaded: 0,
        constraintTotal: 0,
        constraintsLoading: false,
        updatedAt: null,
        reload,
      };
    }

    const constraints: Record<string, DatabaseConstraintSummary[]> = {};
    const constraintFailures: DatabaseErDiagramConstraintFailure[] = [];
    let constraintLoaded = 0;

    for (const table of selection.visible) {
      const tableKey = tableKeyFor(table);
      const entry = stored.constraints[tableKey];
      if (entry?.status === "ready") {
        constraints[tableKey] = entry.items;
        constraintLoaded += 1;
      } else if (entry?.status === "error") {
        constraintFailures.push({
          key: tableKey,
          table: table.schema ? `${table.schema}.${table.name}` : table.name,
          error: entry.error,
        });
        constraintLoaded += 1;
      }
    }

    return {
      status: "ready",
      error: null,
      tables: stored.catalog.tables,
      totalTables: stored.catalog.totalTables,
      catalogTruncated: stored.catalog.catalogTruncated,
      constraints,
      constraintFailures,
      constraintLoaded,
      constraintTotal: selection.visible.length,
      constraintsLoading: constraintLoaded < selection.visible.length,
      updatedAt: stored.catalog.updatedAt,
      reload,
    };
  }, [connected, key, reload, runtimeAvailable, selection, stored]);
}
