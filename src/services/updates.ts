import { getBundleType, getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
  createUpdateProgress,
  isNewerVersion,
  UPDATE_RELEASES_URL,
  type UpdateInfo,
  type UpdateProgress,
} from "@/lib/updates";

export type UpdateRuntimeInfo = {
  currentVersion: string;
  bundleType: string;
  selfUpdateSupported: boolean;
  manualPackageUpdate: boolean;
};

export type UpdateInstallStage = "downloading" | "installing";

let pendingUpdate: Update | null = null;

export function isSelfUpdateSupported(bundleType: string): boolean {
  return bundleType === "appimage" || bundleType === "nsis" || bundleType === "app";
}

export function isManualPackageUpdate(bundleType: string): boolean {
  return bundleType === "deb" || bundleType === "rpm";
}

export async function getUpdateRuntimeInfo(): Promise<UpdateRuntimeInfo> {
  const [currentVersion, bundleType] = await Promise.all([getVersion(), getBundleType()]);

  return {
    currentVersion,
    bundleType,
    selfUpdateSupported: isSelfUpdateSupported(bundleType),
    manualPackageUpdate: isManualPackageUpdate(bundleType),
  };
}

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  await closePendingUpdate();

  const update = await check({ timeout: 30_000 });
  if (!update) {
    return null;
  }

  if (!isNewerVersion(update.version, update.currentVersion)) {
    await update.close().catch(() => undefined);
    return null;
  }

  pendingUpdate = update;
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body?.trim() ?? "",
    date: update.date ?? null,
  };
}

export async function downloadAndInstallUpdate(
  onProgress: (progress: UpdateProgress) => void,
  onStage?: (stage: UpdateInstallStage) => void,
): Promise<void> {
  const update = pendingUpdate;
  if (!update) {
    throw new Error("No checked update is ready to install. Check for updates again.");
  }

  let downloadedBytes = 0;
  let totalBytes: number | null = null;

  onStage?.("downloading");
  onProgress(createUpdateProgress(downloadedBytes, totalBytes));

  try {
    await update.downloadAndInstall(
      (event) => {
        switch (event.event) {
          case "Started":
            totalBytes = event.data.contentLength ?? null;
            onProgress(createUpdateProgress(downloadedBytes, totalBytes));
            break;
          case "Progress":
            downloadedBytes += event.data.chunkLength;
            onProgress(createUpdateProgress(downloadedBytes, totalBytes));
            break;
          case "Finished":
            onProgress(
              createUpdateProgress(totalBytes ?? downloadedBytes, totalBytes ?? downloadedBytes),
            );
            onStage?.("installing");
            break;
        }
      },
      { timeout: 15 * 60_000 },
    );
  } catch (error) {
    await closePendingUpdate();
    throw error;
  }

  pendingUpdate = null;
}

export async function relaunchApplication(): Promise<void> {
  await relaunch();
}

export async function openReleasePage(): Promise<void> {
  await openUrl(UPDATE_RELEASES_URL);
}

async function closePendingUpdate() {
  const update = pendingUpdate;
  pendingUpdate = null;

  if (update) {
    await update.close().catch(() => undefined);
  }
}
