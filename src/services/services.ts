import { invoke } from "@tauri-apps/api/core";
import type {
  ServiceActionName,
  ServiceActionResult,
  ServiceDiscovery,
  ServiceStatus,
} from "@/types/contracts";

export const serviceCommands = {
  discover: "get_systemd_services",
  status: "get_service_status",
  action: "run_service_action",
} as const;

export function getSystemdServices(id: string): Promise<ServiceDiscovery> {
  return invoke<ServiceDiscovery>(serviceCommands.discover, { id });
}

export function getServiceStatus(id: string, unit: string): Promise<ServiceStatus> {
  return invoke<ServiceStatus>(serviceCommands.status, { id, unit });
}

export function runServiceAction(
  id: string,
  unit: string,
  action: ServiceActionName,
): Promise<ServiceActionResult> {
  return invoke<ServiceActionResult>(serviceCommands.action, { id, unit, action });
}
