import { isThemePreference, readStoredTheme } from "@/lib/theme";
import type { UserPreferences } from "@/types/contracts";

export const MIN_MONITORING_INTERVAL_SECONDS = 5;
export const MAX_MONITORING_INTERVAL_SECONDS = 300;
export const DEFAULT_MONITORING_INTERVAL_SECONDS = 30;

const PREFERENCES_KEY = "serverdesk-preferences";

export const DEFAULT_PREFERENCES: UserPreferences = {
  theme: "system",
  monitoringIntervalSeconds: DEFAULT_MONITORING_INTERVAL_SECONDS,
  lastActiveServerId: null,
};

export function isValidMonitoringInterval(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isInteger(value) &&
    value >= MIN_MONITORING_INTERVAL_SECONDS &&
    value <= MAX_MONITORING_INTERVAL_SECONDS
  );
}

export function readStoredPreferences(): UserPreferences {
  const fallbackTheme = readStoredTheme();

  try {
    const parsed = JSON.parse(
      localStorage.getItem(PREFERENCES_KEY) ?? "null",
    ) as Partial<UserPreferences> | null;

    if (!parsed || typeof parsed !== "object") {
      return { ...DEFAULT_PREFERENCES, theme: fallbackTheme };
    }

    return {
      theme: isThemePreference(parsed.theme) ? parsed.theme : fallbackTheme,
      monitoringIntervalSeconds: isValidMonitoringInterval(parsed.monitoringIntervalSeconds)
        ? parsed.monitoringIntervalSeconds
        : DEFAULT_MONITORING_INTERVAL_SECONDS,
      lastActiveServerId:
        typeof parsed.lastActiveServerId === "string" ? parsed.lastActiveServerId : null,
    };
  } catch {
    return { ...DEFAULT_PREFERENCES, theme: fallbackTheme };
  }
}

export function writeStoredPreferences(preferences: UserPreferences) {
  localStorage.setItem(PREFERENCES_KEY, JSON.stringify(preferences));
}
