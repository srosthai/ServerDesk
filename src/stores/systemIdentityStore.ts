import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { getSystemIdentity as getSystemIdentityService } from "@/services/identity";
import type { SystemIdentity } from "@/types/contracts";

type SystemIdentityStore = {
  identities: Record<string, SystemIdentity>;
  pending: Record<string, boolean>;
  stale: Record<string, boolean>;
  staleReason: Record<string, string | null>;
  lastAttemptAt: Record<string, number | null>;
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
    stale: {},
    staleReason: {},
    lastAttemptAt: {},
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
          stale: { ...state.stale, [serverId]: false },
          staleReason: { ...state.staleReason, [serverId]: null },
          lastAttemptAt: { ...state.lastAttemptAt, [serverId]: Date.now() },
        }));
        return true;
      } catch (error) {
        if (requestIds[serverId] === requestId) {
          const message = getCommandErrorMessage(error);
          set((state) => ({
            error: message,
            stale: state.identities[serverId] ? { ...state.stale, [serverId]: true } : state.stale,
            staleReason: state.identities[serverId]
              ? { ...state.staleReason, [serverId]: message }
              : state.staleReason,
            lastAttemptAt: { ...state.lastAttemptAt, [serverId]: Date.now() },
          }));
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
        const stale = { ...state.stale };
        const staleReason = { ...state.staleReason };
        const lastAttemptAt = { ...state.lastAttemptAt };
        delete identities[serverId];
        delete stale[serverId];
        delete staleReason[serverId];
        delete lastAttemptAt[serverId];
        return {
          identities,
          pending: { ...state.pending, [serverId]: false },
          stale,
          staleReason,
          lastAttemptAt,
          error: null,
        };
      });
    },

    clearError: () => set({ error: null }),
  };
});
