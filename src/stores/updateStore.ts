import { create } from "zustand";
import { isTauriRuntime } from "@/lib/tauri";
import {
  classifyUpdateError,
  createUpdateProgress,
  decideUpdateCheck,
  getUpdateErrorMessage,
  readLastUpdateCheck,
  writeLastUpdateCheck,
  type UpdateCheckTrigger,
  type UpdateError,
  type UpdateInfo,
  type UpdateProgress,
} from "@/lib/updates";
import {
  checkForUpdate as checkForUpdateService,
  downloadAndInstallUpdate,
  getUpdateRuntimeInfo,
  openReleasePage as openReleasePageService,
  relaunchApplication,
} from "@/services/updates";

export type UpdateStatus =
  | "idle"
  | "disabled"
  | "checking"
  | "up-to-date"
  | "available"
  | "downloading"
  | "installing"
  | "restarting"
  | "installed"
  | "error";

type CheckOptions = {
  trigger?: UpdateCheckTrigger;
};

type UpdateStore = {
  status: UpdateStatus;
  currentVersion: string;
  bundleType: string | null;
  selfUpdateSupported: boolean | null;
  available: UpdateInfo | null;
  progress: UpdateProgress | null;
  error: UpdateError | null;
  lastCheckedAt: number | null;
  checkForUpdate: (options?: CheckOptions) => Promise<void>;
  installUpdate: () => Promise<void>;
  openReleasePage: () => Promise<void>;
};

let checkPromise: Promise<void> | null = null;

const updatesEnabled = isTauriRuntime() && !import.meta.env.DEV;

const OFFLINE_ERROR: UpdateError = {
  kind: "offline",
  message: "The device is offline.",
  details: null,
};

export const useUpdateStore = create<UpdateStore>((set, get) => ({
  status: updatesEnabled ? "idle" : "disabled",
  currentVersion: __APP_VERSION__,
  bundleType: null,
  selfUpdateSupported: null,
  available: null,
  progress: null,
  error: null,
  lastCheckedAt: readLastUpdateCheck(),

  checkForUpdate: async (options = {}) => {
    const trigger = options.trigger ?? "manual";

    if (checkPromise) {
      return checkPromise;
    }

    const status = get().status;
    if (status === "downloading" || status === "installing" || status === "restarting") {
      return;
    }

    const decision = decideUpdateCheck({
      runtime: isTauriRuntime(),
      development: import.meta.env.DEV,
      online: typeof navigator === "undefined" || navigator.onLine !== false,
      lastCheckedAt: get().lastCheckedAt,
      now: Date.now(),
      trigger,
    });

    if (decision === "disabled") {
      set({
        status: "disabled",
        available: null,
        progress: null,
        error: null,
      });
      return;
    }

    if (decision === "recent") {
      return;
    }

    if (decision === "offline") {
      if (trigger === "startup") {
        set({ status: get().available ? "available" : "idle" });
        return;
      }

      set({
        status: "error",
        error: OFFLINE_ERROR,
        progress: null,
      });
      return;
    }

    checkPromise = performCheck(set);

    try {
      await checkPromise;
    } finally {
      checkPromise = null;
    }
  },

  installUpdate: async () => {
    const state = get();
    if (state.status !== "available" || !state.available) {
      return;
    }

    if (!state.selfUpdateSupported) {
      set({
        status: "error",
        progress: null,
        available: null,
        error: {
          kind: "unsupported",
          message: "This installation type cannot install updates in place.",
          details: null,
        },
      });
      return;
    }

    set({
      status: "downloading",
      error: null,
      progress: createUpdateProgress(0, null),
    });

    try {
      await downloadAndInstallUpdate(
        (progress) => {
          set({ progress });
        },
        (stage) => {
          set({ status: stage === "installing" ? "installing" : "downloading" });
        },
      );
    } catch (error) {
      const classified = classifyUpdateError(error);
      set({
        status: "error",
        available: null,
        progress: null,
        error: {
          ...classified,
          kind: classified.kind === "unknown" ? "install" : classified.kind,
        },
      });
      return;
    }

    const progress = get().progress;
    set({
      status: "restarting",
      progress: progress
        ? {
            ...progress,
            downloadedBytes: progress.totalBytes ?? progress.downloadedBytes,
            percent: 100,
          }
        : createUpdateProgress(0, 0),
    });

    try {
      await relaunchApplication();
    } catch (error) {
      set({
        status: "installed",
        error: {
          kind: "relaunch",
          message: getUpdateErrorMessage(error),
          details: null,
        },
      });
    }
  },

  openReleasePage: async () => {
    try {
      await openReleasePageService();
    } catch (error) {
      set({
        status: "error",
        progress: null,
        error: classifyUpdateError(error),
      });
    }
  },
}));

async function performCheck(set: (partial: Partial<UpdateStore>) => void) {
  set({
    status: "checking",
    available: null,
    progress: null,
    error: null,
  });

  try {
    const runtime = await getUpdateRuntimeInfo();
    const available = await checkForUpdateService();
    const checkedAt = Date.now();
    writeLastUpdateCheck(checkedAt);

    if (!available) {
      set({
        status: "up-to-date",
        currentVersion: runtime.currentVersion,
        bundleType: runtime.bundleType,
        selfUpdateSupported: runtime.selfUpdateSupported,
        available: null,
        progress: null,
        error: null,
        lastCheckedAt: checkedAt,
      });
      return;
    }

    set({
      status: "available",
      currentVersion: available.currentVersion || runtime.currentVersion,
      bundleType: runtime.bundleType,
      selfUpdateSupported: runtime.selfUpdateSupported,
      available,
      progress: null,
      error: null,
      lastCheckedAt: checkedAt,
    });
  } catch (error) {
    set({
      status: "error",
      progress: null,
      error: classifyUpdateError(error),
    });
  }
}
