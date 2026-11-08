import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_PREFERENCES,
  isValidMonitoringInterval,
  readStoredPreferences,
  writeStoredPreferences,
} from "@/lib/preferences";

describe("preferences", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("returns defaults when nothing is stored", () => {
    expect(readStoredPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it("reads valid stored preferences", () => {
    localStorage.setItem(
      "serverdesk-preferences",
      JSON.stringify({
        theme: "dark",
        monitoringIntervalSeconds: 60,
        lastActiveServerId: "server-1",
      }),
    );

    expect(readStoredPreferences()).toEqual({
      theme: "dark",
      monitoringIntervalSeconds: 60,
      lastActiveServerId: "server-1",
    });
  });

  it("falls back for invalid values", () => {
    localStorage.setItem(
      "serverdesk-preferences",
      JSON.stringify({
        theme: "sepia",
        monitoringIntervalSeconds: 1,
        lastActiveServerId: 12,
      }),
    );

    expect(readStoredPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it("writes preferences to storage", () => {
    const preferences = {
      theme: "light" as const,
      monitoringIntervalSeconds: 15,
      lastActiveServerId: null,
    };

    writeStoredPreferences(preferences);

    expect(JSON.parse(localStorage.getItem("serverdesk-preferences") ?? "null")).toEqual(
      preferences,
    );
  });

  it("validates the monitoring interval bounds", () => {
    expect(isValidMonitoringInterval(5)).toBe(true);
    expect(isValidMonitoringInterval(300)).toBe(true);
    expect(isValidMonitoringInterval(4)).toBe(false);
    expect(isValidMonitoringInterval(301)).toBe(false);
    expect(isValidMonitoringInterval(30.5)).toBe(false);
    expect(isValidMonitoringInterval("30")).toBe(false);
  });
});
