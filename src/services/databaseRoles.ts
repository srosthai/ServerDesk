import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseDiscoveryPage,
  DatabasePage,
  DatabaseRoleActionRequest,
  DatabaseRoleActionResult,
  DatabaseRoleDdlPreview,
  DatabaseRolePrivilegeSnapshot,
  DatabaseRoleRef,
  DatabaseRoleSummary,
} from "@/types/contracts";

export const databaseRoleCommands = {
  list: "list_database_roles",
  privileges: "get_database_role_privileges",
  preview: "preview_database_role_action",
  run: "run_database_role_action",
} as const;

function pageArgs(page: DatabaseDiscoveryPage = {}) {
  return {
    limit: page.limit ?? null,
    offset: page.offset ?? null,
  };
}

/** Lists users and roles without any password or authentication columns. */
export function listDatabaseRoles(
  profileId: string,
  page: DatabaseDiscoveryPage = {},
): Promise<DatabasePage<DatabaseRoleSummary>> {
  return invoke<DatabasePage<DatabaseRoleSummary>>(databaseRoleCommands.list, {
    profileId,
    ...pageArgs(page),
  });
}

/** Reads direct grants, memberships, and effective privileges for one role. */
export function getDatabaseRolePrivileges(
  profileId: string,
  role: DatabaseRoleRef,
): Promise<DatabaseRolePrivilegeSnapshot> {
  return invoke<DatabaseRolePrivilegeSnapshot>(databaseRoleCommands.privileges, {
    profileId,
    role,
  });
}

/** Generates exact DDL and a before/after privilege diff without executing it. */
export function previewDatabaseRoleAction(
  profileId: string,
  request: DatabaseRoleActionRequest,
): Promise<DatabaseRoleDdlPreview> {
  return invoke<DatabaseRoleDdlPreview>(databaseRoleCommands.preview, {
    profileId,
    request,
  });
}

/**
 * Executes one confirmed role or privilege action. The backend re-runs the
 * preview, rejects unconfirmed or read-only mutations, and re-reads effective
 * privileges before reporting success.
 */
export function runDatabaseRoleAction(
  profileId: string,
  request: DatabaseRoleActionRequest,
  confirmed = false,
): Promise<DatabaseRoleActionResult> {
  return invoke<DatabaseRoleActionResult>(databaseRoleCommands.run, {
    profileId,
    request,
    confirmed,
  });
}
