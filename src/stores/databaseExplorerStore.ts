import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { loadExplorerNode, refreshExplorerNode } from "@/services/databaseDiscovery";
import type {
  CommandError,
  DatabaseDiscoveryPage,
  DatabaseExplorerNode,
  DatabaseExplorerNodePayload,
  DatabasePage,
} from "@/types/contracts";

export type DatabaseExplorerNodeStatus = "idle" | "loading" | "ready" | "error";

export type DatabaseExplorerNodeEntry = {
  status: DatabaseExplorerNodeStatus;
  payload?: DatabaseExplorerNodePayload;
  error?: CommandError;
  errorMessage?: string;
  updatedAt?: number;
  loadingMore?: boolean;
  loadMoreError?: string;
};

type DatabaseExplorerStore = {
  nodes: Record<string, DatabaseExplorerNodeEntry>;
  error: string | null;
  commandError: CommandError | null;
  loadNode: (
    profileId: string,
    node: DatabaseExplorerNode,
    page?: DatabaseDiscoveryPage,
  ) => Promise<DatabaseExplorerNodePayload | null>;
  loadMoreNode: (
    profileId: string,
    node: DatabaseExplorerNode,
  ) => Promise<DatabaseExplorerNodePayload | null>;
  refreshNode: (
    profileId: string,
    node: DatabaseExplorerNode,
    page?: DatabaseDiscoveryPage,
  ) => Promise<DatabaseExplorerNodePayload | null>;
  resetProfile: (profileId: string) => void;
  clearError: () => void;
};

/**
 * A stable key per connection and explorer node. Pagination is not part of the
 * key because the store holds one current page for each node.
 */
export function databaseExplorerNodeKey(profileId: string, node: DatabaseExplorerNode): string {
  switch (node.kind) {
    case "databases":
    case "schemas":
      return `${profileId}::${node.kind}`;
    default:
      return `${profileId}::${node.kind}::${node.database ?? ""}::${node.schema ?? ""}`;
  }
}

function commandErrorFor(error: unknown): CommandError | null {
  return isCommandError(error) ? error : null;
}

function payloadMatchesNode(
  node: DatabaseExplorerNode,
  payload: DatabaseExplorerNodePayload,
): boolean {
  return payload.nodeKind === node.kind;
}

function mergePage<T>(current: DatabasePage<T>, next: DatabasePage<T>): DatabasePage<T> {
  return {
    ...current,
    items: [...current.items, ...next.items],
    hasMore: next.hasMore,
    total: next.total ?? current.total,
  };
}

function appendPage(
  current: DatabaseExplorerNodePayload,
  next: DatabaseExplorerNodePayload,
): DatabaseExplorerNodePayload | null {
  if (current.nodeKind !== next.nodeKind || current.nodeKind === "objectCounts") {
    return null;
  }

  switch (current.nodeKind) {
    case "databases":
      return next.nodeKind === "databases"
        ? { nodeKind: "databases", payload: mergePage(current.payload, next.payload) }
        : null;
    case "schemas":
      return next.nodeKind === "schemas"
        ? { nodeKind: "schemas", payload: mergePage(current.payload, next.payload) }
        : null;
    case "tables":
      return next.nodeKind === "tables"
        ? { nodeKind: "tables", payload: mergePage(current.payload, next.payload) }
        : null;
    case "views":
      return next.nodeKind === "views"
        ? { nodeKind: "views", payload: mergePage(current.payload, next.payload) }
        : null;
    case "sequences":
      return next.nodeKind === "sequences"
        ? { nodeKind: "sequences", payload: mergePage(current.payload, next.payload) }
        : null;
    default:
      return null;
  }
}

function firstPage(payload: DatabaseExplorerNodePayload): DatabasePage<unknown> | null {
  return payload.nodeKind === "objectCounts" ? null : payload.payload;
}

export const useDatabaseExplorerStore = create<DatabaseExplorerStore>((set, get) => ({
  nodes: {},
  error: null,
  commandError: null,

  loadNode: async (profileId, node, page) => {
    const key = databaseExplorerNodeKey(profileId, node);
    const existing = get().nodes[key];
    if (existing?.status === "ready" && existing.payload) {
      return existing.payload;
    }

    if (!isTauriRuntime()) {
      const commandError: CommandError = {
        code: "connection",
        message: "Database discovery is available in the desktop application.",
      };
      set((state) => ({
        nodes: {
          ...state.nodes,
          [key]: {
            ...state.nodes[key],
            status: "error",
            error: commandError,
            errorMessage: commandError.message,
          },
        },
        error: commandError.message,
        commandError,
      }));
      return null;
    }

    set((state) => ({
      nodes: {
        ...state.nodes,
        [key]: {
          ...state.nodes[key],
          status: "loading",
          error: undefined,
          errorMessage: undefined,
        },
      },
      error: null,
      commandError: null,
    }));

    try {
      const payload = await loadExplorerNode(profileId, node, page);
      if (!payloadMatchesNode(node, payload)) {
        throw {
          code: "internal",
          message: "The database returned metadata for a different explorer node.",
        } satisfies CommandError;
      }
      set((state) => ({
        nodes: {
          ...state.nodes,
          [key]: {
            status: "ready",
            payload,
            updatedAt: Date.now(),
          },
        },
        error: null,
        commandError: null,
      }));
      return payload;
    } catch (error) {
      const commandError = commandErrorFor(error);
      const message = commandError?.message ?? getCommandErrorMessage(error);
      set((state) => ({
        nodes: {
          ...state.nodes,
          [key]: {
            ...state.nodes[key],
            status: "error",
            error: commandError ?? undefined,
            errorMessage: message,
          },
        },
        error: message,
        commandError,
      }));
      return null;
    }
  },

  loadMoreNode: async (profileId, node) => {
    const key = databaseExplorerNodeKey(profileId, node);
    const existing = get().nodes[key];
    if (!existing?.payload || existing.loadingMore) {
      return existing?.payload ?? null;
    }
    const page = firstPage(existing.payload);
    if (!page?.hasMore) {
      return existing.payload;
    }
    if (!isTauriRuntime()) {
      return existing.payload;
    }

    set((state) => ({
      nodes: {
        ...state.nodes,
        [key]: {
          ...state.nodes[key],
          loadingMore: true,
          loadMoreError: undefined,
        },
      },
    }));

    try {
      const next = await loadExplorerNode(profileId, node, {
        limit: page.limit,
        offset: page.offset + page.items.length,
      });
      if (!payloadMatchesNode(node, next)) {
        throw {
          code: "internal",
          message: "The database returned metadata for a different explorer node.",
        } satisfies CommandError;
      }
      const merged = appendPage(existing.payload, next);
      if (!merged) {
        throw {
          code: "internal",
          message: "The database returned a different page type for this explorer node.",
        } satisfies CommandError;
      }
      set((state) => ({
        nodes: {
          ...state.nodes,
          [key]: {
            ...state.nodes[key],
            status: "ready",
            payload: merged,
            loadingMore: false,
            loadMoreError: undefined,
            updatedAt: Date.now(),
          },
        },
      }));
      return merged;
    } catch (error) {
      const commandError = commandErrorFor(error);
      const message = commandError?.message ?? getCommandErrorMessage(error);
      set((state) => ({
        nodes: {
          ...state.nodes,
          [key]: {
            ...state.nodes[key],
            loadingMore: false,
            loadMoreError: message,
          },
        },
      }));
      return null;
    }
  },

  refreshNode: async (profileId, node, page) => {
    const key = databaseExplorerNodeKey(profileId, node);
    if (!isTauriRuntime()) {
      return get().loadNode(profileId, node, page);
    }

    set((state) => ({
      nodes: {
        ...state.nodes,
        [key]: {
          ...state.nodes[key],
          status: "loading",
          error: undefined,
          errorMessage: undefined,
        },
      },
      error: null,
      commandError: null,
    }));

    try {
      const payload = await refreshExplorerNode(profileId, node, page);
      if (!payloadMatchesNode(node, payload)) {
        throw {
          code: "internal",
          message: "The database returned metadata for a different explorer node.",
        } satisfies CommandError;
      }
      set((state) => ({
        nodes: {
          ...state.nodes,
          [key]: {
            status: "ready",
            payload,
            updatedAt: Date.now(),
          },
        },
        error: null,
        commandError: null,
      }));
      return payload;
    } catch (error) {
      const commandError = commandErrorFor(error);
      const message = commandError?.message ?? getCommandErrorMessage(error);
      set((state) => ({
        nodes: {
          ...state.nodes,
          [key]: {
            ...state.nodes[key],
            status: "error",
            error: commandError ?? undefined,
            errorMessage: message,
          },
        },
        error: message,
        commandError,
      }));
      return null;
    }
  },

  resetProfile: (profileId) => {
    set((state) => {
      const prefix = `${profileId}::`;
      const nodes = Object.fromEntries(
        Object.entries(state.nodes).filter(([key]) => !key.startsWith(prefix)),
      );
      return { nodes };
    });
  },

  clearError: () => set({ error: null, commandError: null }),
}));
