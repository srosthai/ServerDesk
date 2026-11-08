import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { getSystemIdentity as getSystemIdentityService } from "@/services/identity";
import type { SystemIdentity } from "@/types/contracts";

type SystemIdentityStore = {
  identities: Record<string, SystemIdentity>;
  pending: Record<string, boolean>;
  error: string | null;
  refresh: (serverId: string) => Promise<boolean>;
  clearServer: (serverId: string) => void;
  clearError: () => void;
};

export const useSystemIdentityStore = create<SystemIdentityStore>((set) => {
  let requestCounter = 0;
  const requestIds: Record<string, number> = {};

  return {
    identities: {},
    pending: {},
    error: null,

    refresh: async (serverId) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++requestCounter;
      requestIds[serverId] = requestId;
      set((state) => ({
        pending: { ...state.pending, [serverId]: true },
        error: null,
      }));

      try {
        const identity = await getSystemIdentityService(serverId);
        if (requestIds[serverId] !== requestId) {
          return false;
        }

        set((state) => ({
          identities: { ...state.identities, [serverId]: identity },
        }));
        return true;
      } catch (error) {
        if (requestIds[serverId] === requestId) {
          set({ error: getCommandErrorMessage(error) });
        }
        return false;
      } finally {
        if (requestIds[serverId] === requestId) {
          set((state) => ({
            pending: { ...state.pending, [serverId]: false },
          }));
        }
      }
    },

    clearServer: (serverId) => {
      requestIds[serverId] = ++requestCounter;
      set((state) => {
        const identities = { ...state.identities };
        delete identities[serverId];
        return {
          identities,
          pending: { ...state.pending, [serverId]: false },
          error: null,
        };
      });
    },

    clearError: () => set({ error: null }),
  };
});
