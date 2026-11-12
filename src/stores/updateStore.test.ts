import { beforeEach, describe, expect, it, vi } from "vitest";
import { isTauriRuntime } from "@/lib/tauri";
import { clearLastUpdateCheck, createUpdateProgress, type UpdateInfo } from "@/lib/updates";
import {
  checkForUpdate as checkForUpdateService,
  downloadAndInstallUpdate,
  getUpdateRuntimeInfo,
  openReleasePage as openReleasePageService,
  relaunchApplication,
} from "@/services/updates";
import { useUpdateStore } from "@/stores/updateStore";

vi.mock("@/lib/tauri", () => ({
  isTauriRuntime: vi.fn(() => true),
}));

vi.mock("@/services/updates", () => ({
  getUpdateRuntimeInfo: vi.fn(),
  checkForUpdate: vi.fn(),
  downloadAndInstallUpdate: vi.fn(),
  relaunchApplication: vi.fn(),
  openReleasePage: vi.fn(),
}));

const runtimeInfo = {
  currentVersion: "0.1.0",
  bundleType: "appimage",
  selfUpdateSupported: true,
  manualPackageUpdate: false,
};

const updateInfo: UpdateInfo = {
  version: "0.1.1",
  currentVersion: "0.1.0",
  notes: "Fixes and improvements.",
  date: "2026-11-12T12:00:00Z",
};

function resetStore() {
  useUpdateStore.setState({
    status: "idle",
    currentVersion: "0.1.0",
    bundleType: null,
    selfUpdateSupported: null,
    available: null,
    progress: null,
    error: null,
    lastCheckedAt: null,
  });
}

describe("update store", () => {
  beforeEach(() => {
    vi.stubEnv("DEV", false);
    vi.mocked(isTauriRuntime).mockReturnValue(true);
    vi.mocked(getUpdateRuntimeInfo).mockResolvedValue(runtimeInfo);
    vi.mocked(checkForUpdateService).mockResolvedValue(null);
    vi.mocked(downloadAndInstallUpdate).mockResolvedValue();
    vi.mocked(relaunchApplication).mockResolvedValue();
    vi.mocked(openReleasePageService).mockResolvedValue();
    clearLastUpdateCheck();
    resetStore();
  });

  it("does not check in a web preview", async () => {
    vi.mocked(isTauriRuntime).mockReturnValue(false);

    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });

    expect(useUpdateStore.getState().status).toBe("disabled");
    expect(getUpdateRuntimeInfo).not.toHaveBeenCalled();
  });

  it("does not check in a development build", async () => {
    vi.stubEnv("DEV", true);

    await useUpdateStore.getState().checkForUpdate({ trigger: "startup" });

    expect(useUpdateStore.getState().status).toBe("disabled");
    expect(getUpdateRuntimeInfo).not.toHaveBeenCalled();
  });

  it("skips startup checks while offline but reports manual offline checks", async () => {
    vi.spyOn(window.navigator, "onLine", "get").mockReturnValue(false);

    await useUpdateStore.getState().checkForUpdate({ trigger: "startup" });
    expect(useUpdateStore.getState().status).toBe("idle");
    expect(getUpdateRuntimeInfo).not.toHaveBeenCalled();

    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });
    expect(useUpdateStore.getState().status).toBe("error");
    expect(useUpdateStore.getState().error?.kind).toBe("offline");
    expect(getUpdateRuntimeInfo).not.toHaveBeenCalled();
  });

  it("skips a recent startup check and persists a successful check", async () => {
    const now = 1_800_000_000_000;
    vi.spyOn(Date, "now").mockReturnValue(now);
    vi.mocked(checkForUpdateService).mockResolvedValue(updateInfo);

    await useUpdateStore.getState().checkForUpdate({ trigger: "startup" });

    expect(useUpdateStore.getState().status).toBe("available");
    expect(useUpdateStore.getState().available?.version).toBe("0.1.1");
    expect(useUpdateStore.getState().currentVersion).toBe("0.1.0");
    expect(useUpdateStore.getState().selfUpdateSupported).toBe(true);
    expect(useUpdateStore.getState().lastCheckedAt).toBe(now);
    expect(JSON.parse(localStorage.getItem("serverdesk-last-update-check") ?? "null")).toBe(now);

    vi.mocked(getUpdateRuntimeInfo).mockClear();
    vi.mocked(checkForUpdateService).mockClear();
    await useUpdateStore.getState().checkForUpdate({ trigger: "startup" });
    expect(getUpdateRuntimeInfo).not.toHaveBeenCalled();

    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });
    expect(getUpdateRuntimeInfo).toHaveBeenCalledTimes(1);
  });

  it("reports an up-to-date installation", async () => {
    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });

    expect(useUpdateStore.getState().status).toBe("up-to-date");
    expect(useUpdateStore.getState().available).toBeNull();
    expect(useUpdateStore.getState().lastCheckedAt).not.toBeNull();
  });

  it("deduplicates concurrent checks", async () => {
    let resolveCheck: ((update: UpdateInfo | null) => void) | undefined;
    vi.mocked(checkForUpdateService).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveCheck = resolve;
        }),
    );

    const first = useUpdateStore.getState().checkForUpdate({ trigger: "manual" });
    const second = useUpdateStore.getState().checkForUpdate({ trigger: "manual" });
    expect(useUpdateStore.getState().status).toBe("checking");

    await vi.waitFor(() => expect(checkForUpdateService).toHaveBeenCalledTimes(1));
    resolveCheck?.(null);
    await Promise.all([first, second]);

    expect(checkForUpdateService).toHaveBeenCalledTimes(1);
  });

  it("classifies network and signature check failures", async () => {
    vi.mocked(checkForUpdateService).mockRejectedValueOnce(new Error("failed to fetch"));
    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });
    expect(useUpdateStore.getState().status).toBe("error");
    expect(useUpdateStore.getState().error?.kind).toBe("network");

    vi.mocked(checkForUpdateService).mockRejectedValueOnce(
      new Error("minisign signature verification failed"),
    );
    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });
    expect(useUpdateStore.getState().error?.kind).toBe("signature");
  });

  it("tracks download progress and relaunches only after installation completes", async () => {
    vi.mocked(checkForUpdateService).mockResolvedValue(updateInfo);
    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });

    let resolveInstall: (() => void) | undefined;
    vi.mocked(downloadAndInstallUpdate).mockImplementation(async (onProgress, onStage) => {
      onProgress(createUpdateProgress(50, 200));
      onStage?.("installing");
      await new Promise<void>((resolve) => {
        resolveInstall = resolve;
      });
    });

    const install = useUpdateStore.getState().installUpdate();

    await vi.waitFor(() => {
      expect(useUpdateStore.getState().status).toBe("installing");
    });
    expect(useUpdateStore.getState().progress).toMatchObject({
      downloadedBytes: 50,
      totalBytes: 200,
      percent: 25,
    });
    expect(relaunchApplication).not.toHaveBeenCalled();

    resolveInstall?.();
    await install;

    expect(relaunchApplication).toHaveBeenCalledTimes(1);
    expect(useUpdateStore.getState().status).toBe("restarting");
    expect(useUpdateStore.getState().progress?.percent).toBe(100);
  });

  it("does not relaunch when the signature is rejected during installation", async () => {
    vi.mocked(checkForUpdateService).mockResolvedValue(updateInfo);
    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });
    vi.mocked(downloadAndInstallUpdate).mockRejectedValue(
      new Error("signature verification failed"),
    );

    await useUpdateStore.getState().installUpdate();

    expect(useUpdateStore.getState().status).toBe("error");
    expect(useUpdateStore.getState().error?.kind).toBe("signature");
    expect(useUpdateStore.getState().available).toBeNull();
    expect(relaunchApplication).not.toHaveBeenCalled();
  });

  it("surfaces a relaunch failure after a completed install", async () => {
    vi.mocked(checkForUpdateService).mockResolvedValue(updateInfo);
    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });
    vi.mocked(relaunchApplication).mockRejectedValue(new Error("Could not restart"));

    await useUpdateStore.getState().installUpdate();

    expect(useUpdateStore.getState().status).toBe("installed");
    expect(useUpdateStore.getState().error?.kind).toBe("relaunch");
  });

  it("does not install package-manager builds in place", async () => {
    vi.mocked(getUpdateRuntimeInfo).mockResolvedValue({
      ...runtimeInfo,
      bundleType: "deb",
      selfUpdateSupported: false,
      manualPackageUpdate: true,
    });
    vi.mocked(checkForUpdateService).mockResolvedValue(updateInfo);
    await useUpdateStore.getState().checkForUpdate({ trigger: "manual" });

    await useUpdateStore.getState().installUpdate();

    expect(useUpdateStore.getState().status).toBe("error");
    expect(useUpdateStore.getState().error?.kind).toBe("unsupported");
    expect(downloadAndInstallUpdate).not.toHaveBeenCalled();
    expect(relaunchApplication).not.toHaveBeenCalled();
  });

  it("opens the release page for manual updates", async () => {
    await useUpdateStore.getState().openReleasePage();
    expect(openReleasePageService).toHaveBeenCalledTimes(1);
  });

  it("reports a permission failure when the release page cannot be opened", async () => {
    vi.mocked(openReleasePageService).mockRejectedValue(new Error("permission denied"));

    await useUpdateStore.getState().openReleasePage();

    expect(useUpdateStore.getState().status).toBe("error");
    expect(useUpdateStore.getState().error?.kind).toBe("permission");
  });
});
