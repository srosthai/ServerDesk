import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseDiscoveryPage,
  DatabasePage,
  DatabaseSessionActionRequest,
  DatabaseSessionActionResult,
  DatabaseSessionSummary,
} from "@/types/contracts";

export const databaseSessionCommands = {
  list: "list_database_sessions",
  run: "run_database_session_action",
} as const;

function pageArgs(page: DatabaseDiscoveryPage = {}) {
  return {
    limit: page.limit ?? null,
    offset: page.offset ?? null,
  };
}

/** Lists active sessions visible to the connected database account. */
export function listDatabaseSessions(
  profileId: string,
  page: DatabaseDiscoveryPage = {},
  runningOnly = false,
): Promise<DatabasePage<DatabaseSessionSummary>> {
  return invoke<DatabasePage<DatabaseSessionSummary>>(databaseSessionCommands.list, {
    profileId,
    runningOnly,
    ...pageArgs(page),
  });
}

/**
 * Cancels a running query or terminates a session. The backend requires the
 * exact typed confirmation phrase, rejects read-only profiles, and verifies
 * the session is gone before reporting success.
 */
export function runDatabaseSessionAction(
  profileId: string,
  request: DatabaseSessionActionRequest,
): Promise<DatabaseSessionActionResult> {
  return invoke<DatabaseSessionActionResult>(databaseSessionCommands.run, {
    profileId,
    request,
  });
}
