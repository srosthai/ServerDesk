import type { ArtisanActionName, ArtisanActionResult } from "@/types/contracts";

export type ArtisanActionConfig = {
  action: ArtisanActionName;
  label: string;
  argument: string;
  destructive: boolean;
  description: string;
  confirmation: string | null;
};

/**
 * The fixed Artisan allowlist mirrored from the Rust backend.
 *
 * The backend rejects any action value outside this union. Commands are built
 * only from the fixed argument and the stored project path; the UI never
 * accepts custom command text.
 */
export const ARTISAN_ACTIONS: ArtisanActionConfig[] = [
  {
    action: "about",
    label: "About",
    argument: "about",
    destructive: false,
    description:
      "Reads the framework, environment, cache, and driver overview. The command is read-only.",
    confirmation: null,
  },
  {
    action: "route-list",
    label: "Route list",
    argument: "route:list",
    destructive: false,
    description: "Lists the routes registered by the application. The command is read-only.",
    confirmation: null,
  },
  {
    action: "migrate-status",
    label: "Migration status",
    argument: "migrate:status",
    destructive: false,
    description:
      "Shows which migrations have run and which are pending. The command does not change the database.",
    confirmation: null,
  },
  {
    action: "queue-restart",
    label: "Restart queue workers",
    argument: "queue:restart",
    destructive: true,
    description:
      "Signals queue workers to finish their current job and restart gracefully. This is a Laravel-level restart and does not restart Supervisor or a systemd service.",
    confirmation:
      "Queue workers will finish their current job and restart gracefully. This does not restart Supervisor or the systemd service that supervises them.",
  },
  {
    action: "config-cache",
    label: "Config cache",
    argument: "config:cache",
    destructive: true,
    description:
      "Builds bootstrap/cache/config.php so Laravel loads configuration from the cache. Clear the cache after changing configuration.",
    confirmation:
      "Laravel will stop reading .env for configuration until the cached configuration is cleared. The target path and exact command are shown above.",
  },
  {
    action: "config-clear",
    label: "Config clear",
    argument: "config:clear",
    destructive: true,
    description:
      "Removes the cached configuration file so Laravel reads configuration and .env again.",
    confirmation:
      "The cached configuration file will be removed. Laravel will read configuration and .env again on the next request.",
  },
  {
    action: "cache-clear",
    label: "Clear application cache",
    argument: "cache:clear",
    destructive: true,
    description: "Flushes the application cache store. Cached application data can be lost.",
    confirmation:
      "The application cache store will be flushed. Cached application data can be lost and cannot be recovered from ServerDesk.",
  },
  {
    action: "optimize-clear",
    label: "Clear compiled caches",
    argument: "optimize:clear",
    destructive: true,
    description:
      "Clears compiled views, cached configuration, routes, events, and the application cache.",
    confirmation:
      "Compiled views, cached configuration, routes, events, and the application cache will be cleared. The next request may be slower while caches rebuild.",
  },
];

export const artisanActionConfig: Record<ArtisanActionName, ArtisanActionConfig> =
  Object.fromEntries(ARTISAN_ACTIONS.map((config) => [config.action, config])) as Record<
    ArtisanActionName,
    ArtisanActionConfig
  >;

/** Quotes one value as a single POSIX shell argument, matching the Rust runner. */
export function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

/** The exact fixed command shown before execution. */
export function artisanCommand(remotePath: string, action: ArtisanActionName): string {
  const config = artisanActionConfig[action];
  return `cd -- ${shellQuote(remotePath)} && php artisan ${config.argument}`;
}

/** Copyable transcript for one completed action. */
export function artisanTranscript(result: ArtisanActionResult): string {
  const exit =
    result.exitCode === null ? "No exit status" : `Exit code ${result.exitCode.toString()}`;
  return [
    result.command,
    `Working directory: ${result.targetPath}`,
    `${exit} · ${result.durationMs.toString()} ms`,
    "",
    "stdout:",
    result.stdout || "(empty)",
    "",
    "stderr:",
    result.stderr || "(empty)",
  ].join("\n");
}
