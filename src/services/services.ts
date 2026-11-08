import { invoke } from "@tauri-apps/api/core";
import type { ServiceDiscovery } from "@/types/contracts";

export const serviceCommands = {
  discover: "get_systemd_services",
} as const;

export function getSystemdServices(id: string): Promise<ServiceDiscovery> {
  return invoke<ServiceDiscovery>(serviceCommands.discover, { id });
}
