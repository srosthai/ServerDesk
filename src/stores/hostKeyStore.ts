import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { deleteKnownHost, listKnownHosts } from "@/services/knownHosts";
import type { TrustedHostKey } from "@/types/contracts";

type HostKeyStatus = "idle" | "loading" | "ready" | "error";

type HostKeyStore = {
  knownHosts: Record<string, TrustedHostKey[]>;
  status: Record<string, HostKeyStatus>;
  error: string | null;
  fetchKnownHosts: (serverId: string) => Promise<void>;
  removeKnownHost: (serverId: string, keyType: string) => Promise<boolean>;
  clearError: () => void;
};

export const useHostKeyStore = create<HostKeyStore>((set) => ({
  knownHosts: {},
  status: {},
  error: null,

  fetchKnownHosts: async (serverId) => {
    if (!isTauriRuntime()) {
      set((state) => ({
        knownHosts: { ...state.knownHosts, [serverId]: [] },
        status: { ...state.status, [serverId]: "ready" },
        error: null,
      }));
      return;
    }

    set((state) => ({
      status: { ...state.status, [serverId]: "loading" },
      error: null,
    }));

    try {
      const knownHosts = await listKnownHosts(serverId);
      set((state) => ({
        knownHosts: { ...state.knownHosts, [serverId]: knownHosts },
        status: { ...state.status, [serverId]: "ready" },
        error: null,
      }));
    } catch (error) {
      set((state) => ({
        status: { ...state.status, [serverId]: "error" },
        error: getCommandErrorMessage(error),
      }));
    }
  },

  removeKnownHost: async (serverId, keyType) => {
    if (!isTauriRuntime()) {
      set((state) => ({
        knownHosts: {
          ...state.knownHosts,
          [serverId]: (state.knownHosts[serverId] ?? []).filter((host) => host.keyType !== keyType),
        },
        error: null,
      }));
      return true;
    }

    try {
      await deleteKnownHost(serverId, keyType);
      set((state) => ({
        knownHosts: {
          ...state.knownHosts,
          [serverId]: (state.knownHosts[serverId] ?? []).filter((host) => host.keyType !== keyType),
        },
        error: null,
      }));
      return true;
    } catch (error) {
      set({ error: getCommandErrorMessage(error) });
      return false;
    }
  },

  clearError: () => set({ error: null }),
}));
