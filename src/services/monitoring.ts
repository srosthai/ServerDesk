import { invoke } from "@tauri-apps/api/core";
import type { MonitoringSnapshot } from "@/types/contracts";

export const monitoringCommands = {
  snapshot: "get_monitoring_snapshot",
} as const;

export function getMonitoringSnapshot(id: string): Promise<MonitoringSnapshot> {
  return invoke<MonitoringSnapshot>(monitoringCommands.snapshot, { id });
}
