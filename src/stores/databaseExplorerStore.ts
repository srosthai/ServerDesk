import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { loadExplorerNode, refreshExplorerNode } from "@/services/databaseDiscovery";
import type {
  CommandError,
  DatabaseDiscoveryPage,
  DatabaseExplorerNode,
  DatabaseExplorerNodePayload,
} from "@/types/contracts";

export type DatabaseExplorerNodeStatus = "idle" | "loading" | "ready" | "error";

export type DatabaseExplorerNodeEntry = {
  status: DatabaseExplorerNodeStatus;
  payload?: DatabaseExplorerNodePayload;
  error?: CommandError;
  errorMessage?: string;
  updatedAt?: number;
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
