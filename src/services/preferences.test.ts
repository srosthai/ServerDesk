import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import { getPreferences, savePreferences } from "@/services/preferences";
import type { UserPreferences } from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const preferences: UserPreferences = {
  theme: "dark",
  monitoringIntervalSeconds: 30,
  lastActiveServerId: null,
};

describe("preference service", () => {
  it("uses the registered command names", async () => {
    vi.mocked(invoke).mockResolvedValue(preferences);

    await expect(getPreferences()).resolves.toEqual(preferences);
    expect(invoke).toHaveBeenCalledWith("get_preferences");

    await expect(savePreferences(preferences)).resolves.toEqual(preferences);
    expect(invoke).toHaveBeenCalledWith("save_preferences", { preferences });
  });
});
