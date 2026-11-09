import type { QueueWorkerKind, SupervisorProgramState, SupervisorStatus } from "@/types/contracts";

/**
 * Mirrors the Rust `QUEUE_RESTART_DISTINCTION` text. A Laravel queue restart is
 * deliberately described as a worker restart, never as a Supervisor restart.
 */
export const QUEUE_RESTART_DISTINCTION =
  "A Laravel queue restart runs php artisan queue:restart inside the project directory. It " +
  "signals the application's queue workers to finish their current job and restart gracefully. " +
  "It does not restart Supervisor, supervisord, or the supervisor.service systemd unit, and " +
  "ServerDesk never rewrites Supervisor configuration.";

/** The MVP never reads or writes Supervisor configuration files. */
export const SUPERVISOR_CONFIG_NOTE =
  "ServerDesk reads Supervisor status only. This MVP does not read or rewrite Supervisor " +
  "configuration files.";

export const supervisorStatusLabels: Record<SupervisorStatus, string> = {
  available: "Installed",
  missing: "Not installed",
  unavailable: "Unavailable",
};

export const supervisorProgramStateLabels: Record<SupervisorProgramState, string> = {
  running: "Running",
  stopped: "Stopped",
  starting: "Starting",
  backoff: "Backoff",
  fatal: "Fatal",
  exited: "Exited",
  unknown: "Unknown",
};

export const queueWorkerKindLabels: Record<QueueWorkerKind, string> = {
  queue_work: "queue:work",
  queue_listen: "queue:listen",
  horizon: "Horizon",
};

/** Formats a process elapsed time without inventing a start timestamp. */
export function formatWorkerUptime(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) {
    return "Unknown";
  }

  const seconds = Math.floor(totalSeconds);
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  const remainder = seconds % 60;

  if (days > 0) {
    return `${days.toString()}d ${hours.toString()}h`;
  }
  if (hours > 0) {
    return `${hours.toString()}h ${minutes.toString()}m`;
  }
  if (minutes > 0) {
    return `${minutes.toString()}m ${remainder.toString()}s`;
  }
  return `${remainder.toString()}s`;
}
