import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { usePreferencesStore } from "@/stores/preferencesStore";
import {
  connectServer as connectServerCommand,
  disconnectServer as disconnectServerCommand,
  getConnectionState,
} from "@/services/connections";
import {
  rejectHostKey as rejectHostKeyCommand,
  trustHostKey as trustHostKeyCommand,
} from "@/services/knownHosts";
import type { ConnectionSnapshot } from "@/types/contracts";

type ConnectionStore = {
  connections: Record<string, ConnectionSnapshot>;
  busy: Record<string, boolean>;
  error: string | null;
  applySnapshot: (snapshot: ConnectionSnapshot) => void;
  refreshStates: (serverIds: string[]) => Promise<void>;
  connect: (serverId: string) => Promise<boolean>;
  disconnect: (serverId: string) => Promise<boolean>;
  trustHostKey: (serverId: string, keyType: string, fingerprint: string) => Promise<boolean>;
  rejectHostKey: (serverId: string, keyType: string, fingerprint: string) => Promise<boolean>;
  clearError: () => void;
};

function mergeSnapshot(
  connections: Record<string, ConnectionSnapshot>,
  snapshot: ConnectionSnapshot,
) {
  const current = connections[snapshot.serverId];
  if (current && current.changedAt > snapshot.changedAt) {
    return connections;
  }

  return { ...connections, [snapshot.serverId]: snapshot };
}

export const useConnectionStore = create<ConnectionStore>((set, get) => ({
  connections: {},
  busy: {},
  error: null,

  applySnapshot: (snapshot) => {
    set((state) => ({
      connections: mergeSnapshot(state.connections, snapshot),
    }));
  },

  refreshStates: async (serverIds) => {
    if (!isTauriRuntime() || serverIds.length === 0) {
      return;
    }

    const snapshots = await Promise.all(
      serverIds.map(async (serverId) => {
        try {
          return await getConnectionState(serverId);
        } catch {
          return null;
        }
      }),
    );

    set((state) => {
      let connections = state.connections;
      for (const snapshot of snapshots) {
        if (snapshot) {
          connections = mergeSnapshot(connections, snapshot);
        }
      }
      return { connections };
    });
  },

  connect: async (serverId) => {
    if (!isTauriRuntime()) {
      set({ error: "SSH connections are available in the desktop application." });
      return false;
    }

    set((state) => ({
      busy: { ...state.busy, [serverId]: true },
      error: null,
    }));
    usePreferencesStore.getState().setLastActiveServerId(serverId);

    try {
      const snapshot = await connectServerCommand(serverId);
      get().applySnapshot(snapshot);
      return true;
    } catch (error) {
      const message = getCommandErrorMessage(error);
      const commandError = isCommandError(error) ? error : null;

      if (commandError?.code === "host_key") {
        try {
          get().applySnapshot(await getConnectionState(serverId));
        } catch {
          // The state-change event carries the same details when available.
        }
      }

      set((state) => {
        const current = state.connections[serverId];
        const transitional =
          !current || current.state === "connecting" || current.state === "reconnecting";

        return {
          connections:
            transitional && commandError
              ? {
                  ...state.connections,
                  [serverId]: {
                    serverId,
                    state: "error",
                    changedAt: current?.changedAt ?? 0,
                    error: commandError,
                  },
                }
              : state.connections,
          error: message,
        };
      });
      return false;
    } finally {
      set((state) => ({
        busy: { ...state.busy, [serverId]: false },
      }));
    }
  },

  disconnect: async (serverId) => {
    if (!isTauriRuntime()) {
      set({ error: "SSH connections are available in the desktop application." });
      return false;
    }

    set((state) => ({
      busy: { ...state.busy, [serverId]: true },
      error: null,
    }));

    try {
      const snapshot = await disconnectServerCommand(serverId);
      get().applySnapshot(snapshot);
      return true;
    } catch (error) {
      set({ error: getCommandErrorMessage(error) });
      return false;
    } finally {
      set((state) => ({
        busy: { ...state.busy, [serverId]: false },
      }));
    }
  },

  trustHostKey: async (serverId, keyType, fingerprint) => {
    if (!isTauriRuntime()) {
      set({ error: "SSH connections are available in the desktop application." });
      return false;
    }

    set((state) => ({
      busy: { ...state.busy, [serverId]: true },
      error: null,
    }));

    try {
      await trustHostKeyCommand(serverId, keyType, fingerprint);
    } catch (error) {
      set({ error: getCommandErrorMessage(error) });
      set((state) => ({
        busy: { ...state.busy, [serverId]: false },
      }));
      return false;
    }

    set((state) => ({
      busy: { ...state.busy, [serverId]: false },
    }));
    return get().connect(serverId);
  },

  rejectHostKey: async (serverId, keyType, fingerprint) => {
    if (!isTauriRuntime()) {
      set({ error: "SSH connections are available in the desktop application." });
      return false;
    }

    set((state) => ({
      busy: { ...state.busy, [serverId]: true },
      error: null,
    }));

    try {
      const snapshot = await rejectHostKeyCommand(serverId, keyType, fingerprint);
      get().applySnapshot(snapshot);
      return true;
    } catch (error) {
      set({ error: getCommandErrorMessage(error) });
      return false;
    } finally {
      set((state) => ({
        busy: { ...state.busy, [serverId]: false },
      }));
    }
  },

  clearError: () => set({ error: null }),
}));
