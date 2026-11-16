import type {
  CapabilityFlags,
  DatabaseAction,
  DatabaseEngine,
  DatabaseMetadataValue,
  DatabasePrivilege,
  DatabasePrivilegeTarget,
  DatabasePrivilegeTargetKind,
  DatabaseRoleKind,
  DatabaseRoleRef,
  DatabaseRoleSummary,
} from "@/types/contracts";
import {
  engineLabel,
  type DatabaseTableScope,
  type StructureDecision,
} from "@/features/databases/databaseStructureModel";

export type RoleCapabilities = {
  browse: StructureDecision;
  manage: StructureDecision;
};

function enabled(): StructureDecision {
  return { enabled: true, reason: null };
}

function disabled(reason: string): StructureDecision {
  return { enabled: false, reason };
}

function unsupportedReason(
  label: string,
  engine: DatabaseEngine,
  version: string | null | undefined,
): string {
  return `${label} is not supported by ${engineLabel(engine)} ${version ?? "on this server"}.`;
}

/** Derives role listing and mutation availability from the capability document. */
export function roleCapabilitiesFor(input: {
  engine: DatabaseEngine;
  version?: string | null;
  connected: boolean;
  readOnly: boolean;
  actions?: DatabaseAction[];
  flags?: CapabilityFlags;
}): RoleCapabilities {
  const { actions, flags } = input;
  const actionSet = actions ? new Set(actions) : null;
  const connectionReason = !input.connected
    ? "Connect this database profile before listing or changing users and roles."
    : null;

  function decide(
    action: DatabaseAction,
    label: string,
    readOnlyBlocks: boolean,
  ): StructureDecision {
    if (connectionReason) {
      return disabled(connectionReason);
    }
    if (readOnlyBlocks && input.readOnly) {
      return disabled(
        "This database profile is read-only. Save a writable profile before changing roles or grants.",
      );
    }
    if (actionSet && !actionSet.has(action)) {
      return disabled(unsupportedReason(label, input.engine, input.version));
    }
    if (!actionSet) {
      if (!flags) {
        return disabled(
          "Capability details are unavailable for this connection. Reconnect to refresh them before using roles and grants.",
        );
      }
      if (!flags.roles) {
        return disabled(unsupportedReason(label, input.engine, input.version));
      }
    }
    return enabled();
  }

  return {
    browse: decide("browse_roles", "Role browsing", false),
    manage: decide("manage_roles", "Role and privilege management", true),
  };
}

export function roleLabel(role: DatabaseRoleRef): string {
  return role.host ? `${role.name}@${role.host}` : role.name;
}

export function roleKindLabel(kind: DatabaseRoleKind): string {
  return kind === "user" ? "User" : "Role";
}

export function metadataBoolLabel(value: DatabaseMetadataValue<boolean>): string {
  return value.status === "available" ? (value.value ? "Yes" : "No") : "Unavailable";
}

export function roleAttributeRows(role: DatabaseRoleSummary): Array<{
  label: string;
  value: string;
}> {
  return [
    { label: "Can log in", value: metadataBoolLabel(role.canLogin) },
    { label: "Superuser", value: metadataBoolLabel(role.superuser) },
    { label: "Create databases", value: metadataBoolLabel(role.createDatabase) },
    { label: "Create roles", value: metadataBoolLabel(role.createRole) },
    { label: "Inherit roles", value: metadataBoolLabel(role.inherit) },
  ];
}

const PRIVILEGE_LABELS: Record<DatabasePrivilege, string> = {
  select: "SELECT",
  insert: "INSERT",
  update: "UPDATE",
  delete: "DELETE",
  truncate: "TRUNCATE",
  references: "REFERENCES",
  trigger: "TRIGGER",
  create: "CREATE",
  drop: "DROP",
  alter: "ALTER",
  index: "INDEX",
  execute: "EXECUTE",
  usage: "USAGE",
  connect: "CONNECT",
  temporary: "TEMPORARY",
  create_temporary_tables: "CREATE TEMPORARY TABLES",
  lock_tables: "LOCK TABLES",
  create_view: "CREATE VIEW",
  show_view: "SHOW VIEW",
  create_routine: "CREATE ROUTINE",
  alter_routine: "ALTER ROUTINE",
  event: "EVENT",
};

export function privilegeLabel(privilege: string): string {
  return PRIVILEGE_LABELS[privilege as DatabasePrivilege] ?? privilege;
}

export function privilegeTargetKindLabel(kind: DatabasePrivilegeTargetKind): string {
  switch (kind) {
    case "global":
      return "Global";
    case "database":
      return "Database";
    case "schema":
      return "Schema";
    case "table":
      return "Table";
    default:
      return "Sequence";
  }
}

export function privilegeTargetLabel(target: DatabasePrivilegeTarget): string {
  const parts = [target.database, target.schema, target.object].filter((part): part is string =>
    Boolean(part),
  );
  if (parts.length === 0) {
    return "All databases (*.*)";
  }
  return parts.join(".");
}

export function privilegeTargetKindOptions(engine: DatabaseEngine): DatabasePrivilegeTargetKind[] {
  if (engine === "mysql") {
    return ["global", "database", "table"];
  }
  return ["database", "schema", "table", "sequence"];
}

const MYSQL_GLOBAL: DatabasePrivilege[] = [
  "select",
  "insert",
  "update",
  "delete",
  "create",
  "drop",
  "alter",
  "index",
  "references",
  "trigger",
  "execute",
  "create_temporary_tables",
  "lock_tables",
  "create_view",
  "show_view",
  "create_routine",
  "alter_routine",
  "event",
];

const MYSQL_TABLE: DatabasePrivilege[] = [
  "select",
  "insert",
  "update",
  "delete",
  "create",
  "drop",
  "alter",
  "index",
  "references",
  "trigger",
];

/** The typed privilege vocabulary for one engine and target kind. */
export function privilegeOptionsFor(
  engine: DatabaseEngine,
  kind: DatabasePrivilegeTargetKind,
): DatabasePrivilege[] {
  if (engine === "mysql") {
    if (kind === "global" || kind === "database") {
      return MYSQL_GLOBAL;
    }
    if (kind === "table") {
      return MYSQL_TABLE;
    }
    return [];
  }
  switch (kind) {
    case "database":
      return ["connect", "create", "temporary"];
    case "schema":
      return ["usage", "create"];
    case "table":
      return ["select", "insert", "update", "delete", "truncate", "references", "trigger"];
    case "sequence":
      return ["usage", "select", "update"];
    default:
      return [];
  }
}

/** Builds the default target for the current explorer scope. */
export function defaultPrivilegeTarget(
  engine: DatabaseEngine,
  kind: DatabasePrivilegeTargetKind,
  scope: DatabaseTableScope,
): DatabasePrivilegeTarget {
  const schema = scope.schema ?? (engine === "postgresql" ? "public" : null);
  switch (kind) {
    case "global":
      return { kind };
    case "database":
      return { kind, database: scope.database };
    case "schema":
      return { kind, schema };
    case "table":
      return {
        kind,
        database: engine === "mysql" ? scope.database : null,
        schema: engine === "postgresql" ? schema : null,
        object: scope.table || null,
      };
    default:
      return {
        kind,
        schema: engine === "postgresql" ? schema : null,
        object: null,
      };
  }
}

export function roleDiffCounts(diff: { added: unknown[]; removed: unknown[] }): string {
  return `${diff.added.length} added · ${diff.removed.length} removed`;
}
