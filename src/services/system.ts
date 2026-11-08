import { invoke } from "@tauri-apps/api/core";
import type { AppInfo } from "@/types/contracts";

export const systemCommands = {
  appInfo: "get_app_info",
} as const;

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>(systemCommands.appInfo);
}
