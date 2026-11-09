import { invoke } from "@tauri-apps/api/core";
import type { QueueSupervisorStatus } from "@/types/contracts";

export const queueCommands = {
  status: "get_queue_supervisor_status",
} as const;

/**
 * Reads Supervisor presence/programs and queue worker processes with fixed,
 * read-only remote commands. It never restarts Supervisor or writes its
 * configuration.
 */
export function getQueueSupervisorStatus(serverId: string): Promise<QueueSupervisorStatus> {
  return invoke<QueueSupervisorStatus>(queueCommands.status, { id: serverId });
}
