import { useCallback, useEffect, useState } from "react";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  getTableDependencies,
  listColumns,
  listConstraints,
  listIndexes,
} from "@/services/databaseDiscovery";
import type {
  CommandError,
  DatabaseColumnSummary,
  DatabaseConstraintSummary,
  DatabaseIndexSummary,
  DatabaseTableDependencies,
} from "@/types/contracts";

export type StructureResource<T> =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; data: T; updatedAt: number }
  | { status: "error"; error: CommandError; updatedAt: number };

export type DatabaseTableStructureState = {
  columns: StructureResource<DatabaseColumnSummary[]>;
  indexes: StructureResource<DatabaseIndexSummary[]>;
  constraints: StructureResource<DatabaseConstraintSummary[]>;
  dependencies: StructureResource<DatabaseTableDependencies>;
  reload: () => void;
};

type TableStructureInput = {
  profileId: string | null;
  database: string | null;
  schema: string | null;
  table: string | null;
  connected: boolean;
};

type Resources = Omit<DatabaseTableStructureState, "reload">;

const idleResources: Resources = {
  columns: { status: "idle" },
  indexes: { status: "idle" },
  constraints: { status: "idle" },
  dependencies: { status: "idle" },
};

const loadingResources: Resources = {
  columns: { status: "loading" },
  indexes: { status: "loading" },
  constraints: { status: "loading" },
  dependencies: { status: "loading" },
};

function commandErrorFor(error: unknown): CommandError {
  if (isCommandError(error)) {
    return error;
  }
  return { code: "internal", message: getCommandErrorMessage(error) };
}

function errorResources(error: CommandError): Resources {
  const updatedAt = Date.now();
  return {
    columns: { status: "error", error, updatedAt },
    indexes: { status: "error", error, updatedAt },
    constraints: { status: "error", error, updatedAt },
    dependencies: { status: "error", error, updatedAt },
  };
}

/**
 * Loads table metadata independently per resource so a permission failure on
 * one catalog does not hide the columns or indexes that are readable. The
 * hook is keyed by the selected connection and table; switching workspace
 * tabs does not refetch because the owning page stays mounted.
 */
export function useDatabaseTableStructure({
  profileId,
  database,
  schema,
  table,
  connected,
}: TableStructureInput): DatabaseTableStructureState {
  const [reloadToken, setReloadToken] = useState(0);
  const [stored, setStored] = useState<{ key: string } & Resources>({
    key: "",
    ...idleResources,
  });
  const key =
    profileId && table && connected
      ? [profileId, database ?? "", schema ?? "", table, reloadToken].join("::")
      : "";

  useEffect(() => {
    if (!key || !profileId || !table || !connected || !isTauriRuntime()) {
      return;
    }

    let active = true;
    const updatedAt = Date.now();
    const ref = { database, schema, table };

    function merge<T>(resource: keyof Resources, value: StructureResource<T>): void {
      setStored((current) => ({
        ...(current.key === key ? current : { key, ...loadingResources }),
        [resource]: value,
      }));
    }

    void listColumns(profileId, ref)
      .then((page) => {
        if (active) {
          merge("columns", { status: "ready", data: page.items, updatedAt });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          merge("columns", { status: "error", error: commandErrorFor(error), updatedAt });
        }
      });

    void listIndexes(profileId, ref)
      .then((page) => {
        if (active) {
          merge("indexes", { status: "ready", data: page.items, updatedAt });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          merge("indexes", { status: "error", error: commandErrorFor(error), updatedAt });
        }
      });

    void listConstraints(profileId, ref)
      .then((page) => {
        if (active) {
          merge("constraints", { status: "ready", data: page.items, updatedAt });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          merge("constraints", { status: "error", error: commandErrorFor(error), updatedAt });
        }
      });

    void getTableDependencies(profileId, ref)
      .then((dependencies) => {
        if (active) {
          merge("dependencies", { status: "ready", data: dependencies, updatedAt });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          merge("dependencies", {
            status: "error",
            error: commandErrorFor(error),
            updatedAt,
          });
        }
      });

    return () => {
      active = false;
    };
  }, [connected, database, key, profileId, schema, table]);

  const reload = useCallback(() => {
    setReloadToken((current) => current + 1);
  }, []);

  if (!key) {
    return { ...idleResources, reload };
  }

  if (!isTauriRuntime()) {
    return {
      ...errorResources({
        code: "connection",
        message: "Table structure metadata is available in the desktop application.",
      }),
      reload,
    };
  }

  if (stored.key !== key) {
    return { ...loadingResources, reload };
  }

  return {
    columns: stored.columns,
    indexes: stored.indexes,
    constraints: stored.constraints,
    dependencies: stored.dependencies,
    reload,
  };
}
