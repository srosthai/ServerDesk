import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  createServer as createServerCommand,
  deleteServer as deleteServerCommand,
  listServers,
  updateServer as updateServerCommand,
} from "@/services/servers";
import type { ServerProfile, ServerProfileInput } from "@/types/contracts";

type ServerStatus = "idle" | "loading" | "ready" | "error";

type ServerStore = {
  servers: ServerProfile[];
  status: ServerStatus;
  error: string | null;
  fetchServers: () => Promise<void>;
  createServer: (input: ServerProfileInput) => Promise<boolean>;
  updateServer: (id: string, input: ServerProfileInput) => Promise<boolean>;
  deleteServer: (id: string) => Promise<boolean>;
  clearError: () => void;
};

const PREVIEW_KEY = "serverdesk-servers-preview";

function nowUnix() {
  return Math.floor(Date.now() / 1000);
}

function newId() {
  return globalThis.crypto?.randomUUID?.() ?? `preview-${Date.now()}`;
}

function sortServers(servers: ServerProfile[]) {
  return [...servers].sort((left, right) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: "base" }),
  );
}

function readPreviewServers(): ServerProfile[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(PREVIEW_KEY) ?? "[]") as unknown;
    return Array.isArray(parsed) ? sortServers(parsed as ServerProfile[]) : [];
  } catch {
    return [];
  }
}

function writePreviewServers(servers: ServerProfile[]) {
  localStorage.setItem(PREVIEW_KEY, JSON.stringify(servers));
}

export const useServerStore = create<ServerStore>((set, get) => ({
  servers: [],
  status: "idle",
  error: null,

  fetchServers: async () => {
    if (!isTauriRuntime()) {
      set({ servers: readPreviewServers(), status: "ready", error: null });
      return;
    }

    set({ status: "loading", error: null });

    try {
      const servers = await listServers();
      set({ servers: sortServers(servers), status: "ready", error: null });
    } catch (error) {
      set({ status: "error", error: getCommandErrorMessage(error) });
    }
  },

  createServer: async (input) => {
    try {
      if (!isTauriRuntime()) {
        const now = nowUnix();
        const server: ServerProfile = {
          ...input,
          id: newId(),
          createdAt: now,
          updatedAt: now,
        };
        const servers = sortServers([...get().servers, server]);
        writePreviewServers(servers);
        set({ servers, status: "ready", error: null });
        return true;
      }

      const server = await createServerCommand(input);
      set({
        servers: sortServers([...get().servers, server]),
        status: "ready",
        error: null,
      });
      return true;
    } catch (error) {
      set({ error: getCommandErrorMessage(error) });
      return false;
    }
  },

  updateServer: async (id, input) => {
    try {
      if (!isTauriRuntime()) {
        const existing = get().servers.find((server) => server.id === id);
        if (!existing) {
          set({ error: "Server not found." });
          return false;
        }

        const server: ServerProfile = {
          ...existing,
          ...input,
          id,
          updatedAt: nowUnix(),
        };
        const servers = sortServers(
          get().servers.map((current) => (current.id === id ? server : current)),
        );
        writePreviewServers(servers);
        set({ servers, status: "ready", error: null });
        return true;
      }

      const server = await updateServerCommand(id, input);
      set({
        servers: sortServers(
          get().servers.map((current) => (current.id === id ? server : current)),
        ),
        status: "ready",
        error: null,
      });
      return true;
    } catch (error) {
      set({ error: getCommandErrorMessage(error) });
      return false;
    }
  },

  deleteServer: async (id) => {
    try {
      if (!isTauriRuntime()) {
        const servers = get().servers.filter((server) => server.id !== id);
        writePreviewServers(servers);
        set({ servers, status: "ready", error: null });
        return true;
      }

      await deleteServerCommand(id);
      set({
        servers: get().servers.filter((server) => server.id !== id),
        status: "ready",
        error: null,
      });
      return true;
    } catch (error) {
      set({ error: getCommandErrorMessage(error) });
      return false;
    }
  },

  clearError: () => set({ error: null }),
}));
