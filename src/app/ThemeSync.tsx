import { useEffect } from "react";
import { applyTheme } from "@/lib/theme";
import { usePreferencesStore } from "@/stores/preferencesStore";

function ThemeSync() {
  const theme = usePreferencesStore((state) => state.preferences.theme);
  const hydrate = usePreferencesStore((state) => state.hydrate);

  useEffect(() => {
    void hydrate();
  }, [hydrate]);

  useEffect(() => {
    applyTheme(theme);

    if (theme !== "system") {
      return;
    }

    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => applyTheme("system");

    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [theme]);

  return null;
}

export { ThemeSync };
