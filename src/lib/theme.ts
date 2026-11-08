import type { ThemePreference } from "@/types/contracts";

const THEME_KEY = "serverdesk-theme";

export function isThemePreference(value: unknown): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

export function readStoredTheme(): ThemePreference {
  const value = localStorage.getItem(THEME_KEY);
  return isThemePreference(value) ? value : "system";
}

export function resolveTheme(theme: ThemePreference): "light" | "dark" {
  if (theme === "system") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }

  return theme;
}

export function applyTheme(theme: ThemePreference) {
  const resolved = resolveTheme(theme);
  document.documentElement.classList.toggle("dark", resolved === "dark");
  localStorage.setItem(THEME_KEY, theme);
}
