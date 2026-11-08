import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import {
  DEFAULT_PREFERENCES,
  isValidMonitoringInterval,
  readStoredPreferences,
  writeStoredPreferences,
} from "@/lib/preferences";
import { isTauriRuntime } from "@/lib/tauri";
import { applyTheme } from "@/lib/theme";
import { getPreferences, savePreferences } from "@/services/preferences";
import type { ThemePreference, UserPreferences } from "@/types/contracts";

export type PreferencesStatus = "loading" | "ready" | "unavailable" | "error";

type PreferencesStore = {
  preferences: UserPreferences;
  status: PreferencesStatus;
  error: string | null;
  hydrate: () => Promise<void>;
  setTheme: (theme: ThemePreference) => void;
  setMonitoringInterval: (seconds: number) => void;
  setLastActiveServerId: (serverId: string | null) => void;
  reset: () => void;
};

type SetPreferences = (partial: Partial<PreferencesStore>) => void;

let saveSequence = 0;

export const usePreferencesStore = create<PreferencesStore>((set, get) => ({
  preferences: readStoredPreferences(),
  status: isTauriRuntime() ? "loading" : "unavailable",
  error: null,

  hydrate: async () => {
    if (!isTauriRuntime()) {
      set({ status: "unavailable" });
      return;
    }

    try {
      const preferences = await getPreferences();
      applyTheme(preferences.theme);
      writeStoredPreferences(preferences);
      set({ preferences, status: "ready", error: null });
    } catch (error) {
      set({ status: "error", error: getCommandErrorMessage(error) });
    }
  },

  setTheme: (theme) => {
    void persist({ ...get().preferences, theme }, set);
  },

  setMonitoringInterval: (seconds) => {
    if (!isValidMonitoringInterval(seconds)) {
      set({
        error: `Refresh interval must be between 5 and 300 seconds.`,
      });
      return;
    }

    void persist({ ...get().preferences, monitoringIntervalSeconds: seconds }, set);
  },

  setLastActiveServerId: (serverId) => {
    void persist(
      {
        ...get().preferences,
        lastActiveServerId: serverId,
      },
      set,
    );
  },

  reset: () => {
    void persist(DEFAULT_PREFERENCES, set);
  },
}));

async function persist(next: UserPreferences, set: SetPreferences) {
  set({ preferences: next, error: null });
  applyTheme(next.theme);
  writeStoredPreferences(next);

  if (!isTauriRuntime()) {
    set({ status: "unavailable" });
    return;
  }

  const sequence = ++saveSequence;

  try {
    const saved = await savePreferences(next);
    if (sequence === saveSequence) {
      set({ preferences: saved, status: "ready", error: null });
    }
  } catch (error) {
    if (sequence === saveSequence) {
      set({ status: "error", error: getCommandErrorMessage(error) });
    }
  }
}
