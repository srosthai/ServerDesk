import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { usePreferencesStore } from "@/stores/preferencesStore";

describe("SettingsPage", () => {
  beforeEach(() => {
    usePreferencesStore.setState({
      preferences: DEFAULT_PREFERENCES,
      status: "unavailable",
      error: null,
    });
  });

  it("shows the web preview state for browser storage", () => {
    render(<SettingsPage />);

    expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByText("Web preview")).toBeInTheDocument();
    expect(screen.getByText(/Preferences are stored locally in SQLite/)).toBeInTheDocument();
  });

  it("applies a theme immediately and stores it", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);

    await user.click(screen.getByRole("button", { name: /Dark/ }));

    expect(usePreferencesStore.getState().preferences.theme).toBe("dark");
    expect(document.documentElement).toHaveClass("dark");
    expect(JSON.parse(localStorage.getItem("serverdesk-preferences") ?? "null")).toMatchObject({
      theme: "dark",
    });
  });

  it("resets preferences to defaults", async () => {
    const user = userEvent.setup();
    usePreferencesStore.setState({
      preferences: {
        theme: "dark",
        monitoringIntervalSeconds: 120,
        lastActiveServerId: "server-1",
      },
    });
    render(<SettingsPage />);

    await user.click(screen.getByRole("button", { name: "Reset defaults" }));

    expect(usePreferencesStore.getState().preferences).toEqual(DEFAULT_PREFERENCES);
    expect(JSON.parse(localStorage.getItem("serverdesk-preferences") ?? "null")).toEqual(
      DEFAULT_PREFERENCES,
    );
  });

  it("rejects an interval below the safe minimum", () => {
    act(() => {
      usePreferencesStore.getState().setMonitoringInterval(1);
    });

    expect(usePreferencesStore.getState().error).toContain("between 5 and 300 seconds");
  });
});
