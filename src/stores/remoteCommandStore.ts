import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { runRemoteCommand as runRemoteCommandService } from "@/services/remote";
import type { RemoteCommandName, RemoteCommandResult } from "@/types/contracts";

type ServerCommandResults = Partial<Record<RemoteCommandName, RemoteCommandResult>>;

type RemoteCommandStore = {
  results: Record<string, ServerCommandResults>;
  pending: Record<string, RemoteCommandName | null>;
  error: string | null;
  run: (serverId: string, command: RemoteCommandName) => Promise<boolean>;
  clearServer: (serverId: string) => void;
  clearError: () => void;
};

export const useRemoteCommandStore = create<RemoteCommandStore>((set) => ({
  results: {},
  pending: {},
  error: null,

  run: async (serverId, command) => {
    if (!isTauriRuntime()) {
      set({ error: "Remote commands are available in the desktop application." });
      return false;
    }

    set((state) => ({
      pending: { ...state.pending, [serverId]: command },
      error: null,
    }));

    try {
      const result = await runRemoteCommandService(serverId, command);
      set((state) => ({
        results: {
          ...state.results,
          [serverId]: {
            ...state.results[serverId],
            [command]: result,
          },
        },
      }));
      return true;
    } catch (error) {
      set({ error: getCommandErrorMessage(error) });
      return false;
    } finally {
      set((state) => ({
        pending: { ...state.pending, [serverId]: null },
      }));
    }
  },

  clearServer: (serverId) =>
    set((state) => {
      const results = { ...state.results };
      delete results[serverId];
      return {
        results,
        pending: { ...state.pending, [serverId]: null },
      };
    }),

  clearError: () => set({ error: null }),
}));
