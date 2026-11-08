import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { getSystemdServices as getSystemdServicesService } from "@/services/services";
import type { ServiceDiscovery } from "@/types/contracts";

export type ServiceDiscoveryEntry = {
  discovery: ServiceDiscovery | null;
  pending: boolean;
  error: string | null;
  lastAttemptAt: number | null;
};

type ServiceStore = {
  entries: Record<string, ServiceDiscoveryEntry>;
  refresh: (serverId: string) => Promise<boolean>;
  clearServer: (serverId: string) => void;
  clearError: (serverId: string) => void;
};

function emptyEntry(): ServiceDiscoveryEntry {
  return {
    discovery: null,
    pending: false,
    error: null,
    lastAttemptAt: null,
  };
}

export const useServiceStore = create<ServiceStore>((set) => {
  let requestCounter = 0;
  const requestIds: Record<string, number> = {};

  return {
    entries: {},

    refresh: async (serverId) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++requestCounter;
      requestIds[serverId] = requestId;
      set((state) => {
        const current = state.entries[serverId] ?? emptyEntry();
        return {
          entries: {
            ...state.entries,
            [serverId]: { ...current, pending: true, error: null },
          },
        };
      });

      try {
        const discovery = await getSystemdServicesService(serverId);
        if (requestIds[serverId] !== requestId) {
          return false;
        }

        set((state) => {
          const current = state.entries[serverId] ?? emptyEntry();
          return {
            entries: {
              ...state.entries,
              [serverId]: {
                ...current,
                discovery,
                pending: false,
                error: null,
                lastAttemptAt: Date.now(),
              },
            },
          };
        });
        return true;
      } catch (error) {
        const message = getCommandErrorMessage(error);
        if (requestIds[serverId] === requestId) {
          set((state) => {
            const current = state.entries[serverId] ?? emptyEntry();
            return {
              entries: {
                ...state.entries,
                [serverId]: {
                  ...current,
                  pending: false,
                  error: message,
                  lastAttemptAt: Date.now(),
                },
              },
            };
          });
        }
        return false;
      }
    },

    clearServer: (serverId) => {
      requestIds[serverId] = ++requestCounter;
      set((state) => {
        const entries = { ...state.entries };
        delete entries[serverId];
        return { entries };
      });
    },

    clearError: (serverId) =>
      set((state) => {
        const current = state.entries[serverId];
        if (!current) {
          return state;
        }

        return {
          entries: {
            ...state.entries,
            [serverId]: { ...current, error: null },
          },
        };
      }),
  };
});
