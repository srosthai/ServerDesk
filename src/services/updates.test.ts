import { getBundleType, getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  checkForUpdate,
  downloadAndInstallUpdate,
  getUpdateRuntimeInfo,
  isManualPackageUpdate,
  isSelfUpdateSupported,
  openReleasePage,
  relaunchApplication,
} from "@/services/updates";

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(),
  getBundleType: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn(),
}));

type FakeUpdate = {
  currentVersion: string;
  version: string;
  date?: string;
  body?: string;
  downloadAndInstall: ReturnType<typeof vi.fn>;
  close: ReturnType<typeof vi.fn>;
};

function fakeUpdate(overrides: Partial<FakeUpdate> = {}): FakeUpdate {
  return {
    currentVersion: "0.1.0",
    version: "0.1.1",
    date: "2026-11-12T12:00:00Z",
    body: "  Release notes  ",
    downloadAndInstall: vi.fn(),
    close: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

function mockCheckedUpdate(update: FakeUpdate) {
  vi.mocked(check).mockResolvedValue(update as never);
  return update;
}

describe("update service", () => {
  beforeEach(async () => {
    vi.mocked(check).mockResolvedValue(null);
    await checkForUpdate();
    vi.mocked(check).mockReset();
    vi.mocked(getVersion).mockResolvedValue("0.1.0");
    vi.mocked(getBundleType).mockResolvedValue("appimage" as never);
    vi.mocked(openUrl).mockResolvedValue();
    vi.mocked(relaunch).mockResolvedValue();
  });

  it("reports bundle support without replacing package-manager installs", () => {
    expect(isSelfUpdateSupported("appimage")).toBe(true);
    expect(isSelfUpdateSupported("nsis")).toBe(true);
    expect(isSelfUpdateSupported("app")).toBe(true);
    expect(isSelfUpdateSupported("deb")).toBe(false);
    expect(isSelfUpdateSupported("rpm")).toBe(false);
    expect(isManualPackageUpdate("deb")).toBe(true);
    expect(isManualPackageUpdate("rpm")).toBe(true);
    expect(isManualPackageUpdate("appimage")).toBe(false);
  });

  it("returns the running version and bundle type", async () => {
    vi.mocked(getBundleType).mockResolvedValue("deb" as never);

    await expect(getUpdateRuntimeInfo()).resolves.toEqual({
      currentVersion: "0.1.0",
      bundleType: "deb",
      selfUpdateSupported: false,
      manualPackageUpdate: true,
    });
  });

  it("returns update metadata and ignores versions that are not newer", async () => {
    mockCheckedUpdate(fakeUpdate());

    await expect(checkForUpdate()).resolves.toEqual({
      version: "0.1.1",
      currentVersion: "0.1.0",
      notes: "Release notes",
      date: "2026-11-12T12:00:00Z",
    });
    expect(check).toHaveBeenCalledWith({ timeout: 30_000 });

    const stale = fakeUpdate({ version: "0.1.0" });
    mockCheckedUpdate(stale);
    await expect(checkForUpdate()).resolves.toBeNull();
    expect(stale.close).toHaveBeenCalled();
  });

  it("forwards download progress and the installing stage", async () => {
    const update = mockCheckedUpdate(fakeUpdate());
    update.downloadAndInstall.mockImplementation(async (onEvent: (event: unknown) => void) => {
      onEvent({ event: "Started", data: { contentLength: 100 } });
      onEvent({ event: "Progress", data: { chunkLength: 25 } });
      onEvent({ event: "Progress", data: { chunkLength: 25 } });
      onEvent({ event: "Finished" });
    });
    await checkForUpdate();

    const progress = vi.fn();
    const stage = vi.fn();
    await downloadAndInstallUpdate(progress, stage);

    expect(stage).toHaveBeenNthCalledWith(1, "downloading");
    expect(stage).toHaveBeenLastCalledWith("installing");
    expect(progress).toHaveBeenCalledWith({
      downloadedBytes: 100,
      totalBytes: 100,
      percent: 100,
    });
    expect(update.downloadAndInstall).toHaveBeenCalledWith(expect.any(Function), {
      timeout: 15 * 60_000,
    });
  });

  it("propagates signature failures and releases the pending update", async () => {
    const update = mockCheckedUpdate(fakeUpdate());
    update.downloadAndInstall.mockRejectedValue(new Error("signature verification failed"));
    await checkForUpdate();

    await expect(downloadAndInstallUpdate(vi.fn())).rejects.toThrow(
      "signature verification failed",
    );
    expect(update.close).toHaveBeenCalled();
    await expect(downloadAndInstallUpdate(vi.fn())).rejects.toThrow(/Check for updates again/);
  });

  it("relaunches and opens the release page", async () => {
    await relaunchApplication();
    expect(relaunch).toHaveBeenCalledTimes(1);

    await openReleasePage();
    expect(openUrl).toHaveBeenCalledWith("https://github.com/srosthai/ServerDesk/releases/latest");
  });
});
