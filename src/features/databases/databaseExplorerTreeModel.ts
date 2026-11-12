import { databaseExplorerNodeKey } from "@/stores/databaseExplorerStore";
import type { DatabaseExplorerNodeEntry } from "@/stores/databaseExplorerStore";
import type {
  CommandError,
  DatabaseConnectionSnapshot,
  DatabaseEngine,
  DatabaseExplorerNode,
  DatabaseExplorerNodePayload,
  DatabasePage,
  DatabaseProfile,
  DatabaseSchemaSummary,
  DatabaseSequenceSummary,
  DatabaseSummary,
  DatabaseTableSummary,
  DatabaseViewSummary,
} from "@/types/contracts";

export type DatabaseObjectSummary =
  DatabaseTableSummary | DatabaseViewSummary | DatabaseSequenceSummary;

export type DatabaseObjectGroupKind = "tables" | "views" | "sequences";

export type DatabaseExplorerRowKind =
  | "connection"
  | "database"
  | "schemas-group"
  | "schema"
  | "object-group"
  | "table"
  | "view"
  | "sequence"
  | "status"
  | "load-more";

export type DatabaseExplorerStatusTone =
  "loading" | "empty" | "error" | "permission" | "disconnected" | "info";

export type DatabaseExplorerRowAction = "retry-node" | "retry-load-more" | "load-more" | "connect";

export type DatabaseExplorerSelection = {
  id: string;
  profileId: string;
  kind: DatabaseExplorerRowKind;
  label: string;
  engine: DatabaseEngine;
  database: string | null;
  schema: string | null;
  objectName: string | null;
};

export type DatabaseExplorerRow = {
  id: string;
  kind: DatabaseExplorerRowKind;
  label: string;
  level: number;
  parentId: string | null;
  profileId: string;
  profileName: string;
  engine: DatabaseEngine;
  database: string | null;
  schema: string | null;
  objectName: string | null;
  discoveryNode: DatabaseExplorerNode | null;
  expandable: boolean;
  expanded: boolean;
  selectable: boolean;
  status?: DatabaseExplorerStatusTone;
  detail?: string;
  system?: boolean;
  active?: boolean;
  count?: number;
  hasMore?: boolean;
  busy?: boolean;
  action?: DatabaseExplorerRowAction;
};

export type BuildDatabaseExplorerRowsInput = {
  profiles: DatabaseProfile[];
  snapshots: Record<string, DatabaseConnectionSnapshot>;
  nodes: Record<string, DatabaseExplorerNodeEntry>;
  expandedIds: ReadonlySet<string>;
  query?: string;
  /** Build every loaded branch, used by search and whole-tree refresh. */
  expandAll?: boolean;
};

const ROW_SEPARATOR = "/";

function segment(value: string): string {
  return encodeURIComponent(value);
}

export function databaseConnectionRowId(profileId: string): string {
  return ["connection", segment(profileId)].join(ROW_SEPARATOR);
}

export function databaseRowId(profileId: string, database: string): string {
  return [databaseConnectionRowId(profileId), "database", segment(database)].join(ROW_SEPARATOR);
}

export function schemasGroupRowId(profileId: string, database: string): string {
  return [databaseRowId(profileId, database), "schemas"].join(ROW_SEPARATOR);
}

export function schemaRowId(profileId: string, database: string, schema: string): string {
  return [schemasGroupRowId(profileId, database), segment(schema)].join(ROW_SEPARATOR);
}

export function objectGroupRowId(
  profileId: string,
  database: string,
  schema: string | null,
  group: DatabaseObjectGroupKind,
): string {
  return [
    schema === null ? databaseRowId(profileId, database) : schemaRowId(profileId, database, schema),
    "objects",
    group,
  ].join(ROW_SEPARATOR);
}

export function objectRowId(
  profileId: string,
  database: string,
  schema: string | null,
  group: DatabaseObjectGroupKind,
  name: string,
): string {
  return [objectGroupRowId(profileId, database, schema, group), segment(name)].join(ROW_SEPARATOR);
}

export function databaseExplorerSelectionForRow(
  row: DatabaseExplorerRow,
): DatabaseExplorerSelection | null {
  if (!row.selectable) {
    return null;
  }
  return {
    id: row.id,
    profileId: row.profileId,
    kind: row.kind,
    label: row.label,
    engine: row.engine,
    database: row.database,
    schema: row.schema,
    objectName: row.objectName,
  };
}

function connectedSnapshot(
  profile: DatabaseProfile,
  snapshots: Record<string, DatabaseConnectionSnapshot>,
): DatabaseConnectionSnapshot | undefined {
  const snapshot = snapshots[profile.id];
  return snapshot?.state === "connected" ? snapshot : undefined;
}

function activeDatabaseFor(
  profile: DatabaseProfile,
  snapshot: DatabaseConnectionSnapshot | undefined,
): string | null {
  return snapshot?.databaseName ?? profile.databaseName ?? null;
}

function rowMatches(row: DatabaseExplorerRow, query: string): boolean {
  return [
    row.label,
    row.profileName,
    row.database,
    row.schema,
    row.objectName,
    row.engine === "mysql" ? "mysql" : "postgresql",
  ]
    .filter((value): value is string => typeof value === "string" && value.length > 0)
    .join(" ")
    .toLowerCase()
    .includes(query);
}

function statusRow(
  parent: DatabaseExplorerRow,
  suffix: string,
  status: DatabaseExplorerStatusTone,
  options: {
    label?: string;
    detail?: string;
    action?: DatabaseExplorerRowAction;
    discoveryNode?: DatabaseExplorerNode | null;
  } = {},
): DatabaseExplorerRow {
  return {
    ...parent,
    id: [parent.id, "status", suffix].join(ROW_SEPARATOR),
    kind: "status",
    label: options.label ?? "",
    level: parent.level + 1,
    parentId: parent.id,
    discoveryNode: options.discoveryNode ?? parent.discoveryNode,
    expandable: false,
    expanded: false,
    selectable: false,
    status,
    detail: options.detail,
    action: options.action,
    busy: status === "loading",
    active: false,
    count: undefined,
    hasMore: false,
  };
}

function loadingRows(parent: DatabaseExplorerRow, subject: string): DatabaseExplorerRow[] {
  return [
    statusRow(parent, "loading-1", "loading", {
      label: `Loading ${subject}`,
    }),
    statusRow(parent, "loading-2", "loading"),
  ];
}

function errorRows(
  parent: DatabaseExplorerRow,
  subject: string,
  error: CommandError | undefined,
  errorMessage: string | undefined,
  action: DatabaseExplorerRowAction,
): DatabaseExplorerRow[] {
  const permission = error?.code === "permission";
  const capability = error?.code === "capability";
  const title = permission
    ? "Permission denied"
    : capability
      ? "Not supported"
      : `Could not load ${subject}`;
  const message =
    errorMessage ??
    error?.message ??
    (permission
      ? "The database account cannot read this object."
      : "The database did not return metadata for this node.");

  return [
    statusRow(parent, "error-title", permission ? "permission" : "error", {
      label: title,
      action,
      detail: error?.details,
    }),
    statusRow(parent, "error-message", permission ? "permission" : "error", {
      label: message,
      detail: error?.details,
    }),
  ];
}

function emptyRow(parent: DatabaseExplorerRow, label: string): DatabaseExplorerRow {
  return statusRow(parent, "empty", "empty", { label });
}

function pageSummary(entry: DatabaseExplorerNodeEntry | undefined): {
  count?: number;
  hasMore: boolean;
  loadingMore: boolean;
} {
  if (!entry?.payload || entry.payload.nodeKind === "objectCounts") {
    return { hasMore: false, loadingMore: false };
  }
  const payload = entry.payload.payload;
  return {
    count: payload.total ?? payload.items.length,
    hasMore: payload.hasMore,
    loadingMore: entry.loadingMore === true,
  };
}

function pagePayload<T>(payload: DatabaseExplorerNodePayload): DatabasePage<T> | null {
  return payload.nodeKind === "objectCounts" ? null : (payload.payload as DatabasePage<T>);
}

function objectGroupLabel(group: DatabaseObjectGroupKind): string {
  switch (group) {
    case "tables":
      return "Tables";
    case "views":
      return "Views";
    default:
      return "Sequences";
  }
}

function pushDatabaseChildren(
  rows: DatabaseExplorerRow[],
  databaseRow: DatabaseExplorerRow,
  profile: DatabaseProfile,
  snapshot: DatabaseConnectionSnapshot | undefined,
  input: BuildDatabaseExplorerRowsInput,
  shouldExpand: boolean,
): void {
  if (!shouldExpand) {
    return;
  }

  if (profile.engine === "postgresql") {
    const activeDatabase = activeDatabaseFor(profile, snapshot);
    if (activeDatabase && databaseRow.database !== activeDatabase) {
      rows.push(
        statusRow(databaseRow, "inactive-database", "info", {
          label: `Active database is ${activeDatabase}.`,
          detail:
            "Switch the active database before browsing its schemas. This connection shares one PostgreSQL catalog.",
        }),
      );
      return;
    }

    const groupId = schemasGroupRowId(profile.id, databaseRow.database ?? "");
    const groupExpanded =
      input.expandAll || input.expandedIds.has(groupId) || Boolean(input.query?.trim());
    const discoveryNode: DatabaseExplorerNode = { kind: "schemas" };
    const entry = input.nodes[databaseExplorerNodeKey(profile.id, discoveryNode)];
    const summary = pageSummary(entry);

    const groupRow: DatabaseExplorerRow = {
      ...databaseRow,
      id: groupId,
      kind: "schemas-group",
      label: "Schemas",
      level: databaseRow.level + 1,
      parentId: databaseRow.id,
      discoveryNode,
      expandable: true,
      expanded: groupExpanded,
      selectable: true,
      count: summary.count,
      hasMore: summary.hasMore,
      busy: entry?.status === "loading" || summary.loadingMore,
      active: false,
      system: false,
      detail: undefined,
    };
    rows.push(groupRow);

    if (!groupExpanded) {
      return;
    }

    if (!entry?.payload) {
      if (entry?.status === "error") {
        rows.push(...errorRows(groupRow, "schemas", entry.error, entry.errorMessage, "retry-node"));
      } else {
        rows.push(...loadingRows(groupRow, "schemas"));
      }
      return;
    }

    if (entry.status === "loading") {
      rows.push(statusRow(groupRow, "refreshing", "loading", { label: "Refreshing schemas" }));
    } else if (entry.status === "error") {
      rows.push(...errorRows(groupRow, "schemas", entry.error, entry.errorMessage, "retry-node"));
    }

    const schemas = pagePayload<DatabaseSchemaSummary>(entry.payload);
    if (!schemas) {
      return;
    }
    if (schemas.items.length === 0) {
      rows.push(emptyRow(groupRow, "No schemas found."));
    } else {
      schemas.items.forEach((schema) => {
        const schemaId = schemaRowId(profile.id, databaseRow.database ?? "", schema.name);
        const schemaExpanded =
          input.expandAll || input.expandedIds.has(schemaId) || Boolean(input.query?.trim());
        const schemaRow: DatabaseExplorerRow = {
          ...databaseRow,
          id: schemaId,
          kind: "schema",
          label: schema.name,
          level: groupRow.level + 1,
          parentId: groupRow.id,
          database: databaseRow.database,
          schema: schema.name,
          discoveryNode: null,
          expandable: true,
          expanded: schemaExpanded,
          selectable: true,
          system: schema.system,
          active: false,
          count: undefined,
          hasMore: false,
          busy: false,
          detail: undefined,
        };
        rows.push(schemaRow);
        pushObjectGroups(
          rows,
          schemaRow,
          profile,
          snapshot,
          input,
          schemaExpanded,
          databaseRow.database ?? "",
          schema.name,
        );
      });
    }

    if (summary.hasMore) {
      rows.push(loadMoreRow(groupRow, "schemas", summary.count));
    }
    return;
  }

  pushObjectGroups(
    rows,
    databaseRow,
    profile,
    snapshot,
    input,
    true,
    databaseRow.database ?? "",
    null,
  );
}

function supportsSequences(
  profile: DatabaseProfile,
  snapshot: DatabaseConnectionSnapshot | undefined,
): boolean {
  if (profile.engine === "postgresql") {
    return true;
  }
  return snapshot?.capabilities?.flags.sequences === true;
}

function pushObjectGroups(
  rows: DatabaseExplorerRow[],
  parent: DatabaseExplorerRow,
  profile: DatabaseProfile,
  snapshot: DatabaseConnectionSnapshot | undefined,
  input: BuildDatabaseExplorerRowsInput,
  parentExpanded: boolean,
  database: string,
  schema: string | null,
): void {
  const groups: DatabaseObjectGroupKind[] = ["tables", "views"];
  if (supportsSequences(profile, snapshot)) {
    groups.push("sequences");
  }

  if (!parentExpanded) {
    return;
  }

  groups.forEach((group) => {
    const groupId = objectGroupRowId(profile.id, database, schema, group);
    const groupExpanded =
      input.expandAll || input.expandedIds.has(groupId) || Boolean(input.query?.trim());
    const discoveryNode: DatabaseExplorerNode = {
      kind: group,
      database,
      ...(schema === null ? {} : { schema }),
    };
    const entry = input.nodes[databaseExplorerNodeKey(profile.id, discoveryNode)];
    const summary = pageSummary(entry);
    const groupRow: DatabaseExplorerRow = {
      ...parent,
      id: groupId,
      kind: "object-group",
      label: objectGroupLabel(group),
      level: parent.level + 1,
      parentId: parent.id,
      database,
      schema,
      discoveryNode,
      expandable: true,
      expanded: groupExpanded,
      selectable: true,
      count: summary.count,
      hasMore: summary.hasMore,
      busy: entry?.status === "loading" || summary.loadingMore,
      active: false,
      system: false,
      detail: undefined,
    };
    rows.push(groupRow);

    if (!groupExpanded) {
      return;
    }

    if (!entry?.payload) {
      if (entry?.status === "error") {
        rows.push(
          ...errorRows(
            groupRow,
            objectGroupLabel(group).toLowerCase(),
            entry.error,
            entry.errorMessage,
            "retry-node",
          ),
        );
      } else {
        rows.push(...loadingRows(groupRow, objectGroupLabel(group).toLowerCase()));
      }
      return;
    }

    if (entry.status === "loading") {
      rows.push(
        statusRow(groupRow, "refreshing", "loading", {
          label: `Refreshing ${objectGroupLabel(group).toLowerCase()}`,
        }),
      );
    } else if (entry.status === "error") {
      rows.push(
        ...errorRows(
          groupRow,
          objectGroupLabel(group).toLowerCase(),
          entry.error,
          entry.errorMessage,
          "retry-node",
        ),
      );
    }

    const page = pagePayload<DatabaseObjectSummary>(entry.payload);
    if (!page) {
      return;
    }
    if (page.items.length === 0) {
      rows.push(emptyRow(groupRow, `No ${objectGroupLabel(group).toLowerCase()} found.`));
    } else {
      page.items.forEach((item) => {
        const objectRow: DatabaseExplorerRow = {
          ...groupRow,
          id: objectRowId(profile.id, database, schema, group, item.name),
          kind: group === "tables" ? "table" : group === "views" ? "view" : "sequence",
          label: item.name,
          level: groupRow.level + 1,
          parentId: groupRow.id,
          objectName: item.name,
          discoveryNode: null,
          expandable: false,
          expanded: false,
          selectable: true,
          system: false,
          active: false,
          count: undefined,
          hasMore: false,
          busy: false,
          detail: objectDetail(group, item),
        };
        rows.push(objectRow);
      });
    }

    if (entry.loadMoreError) {
      rows.push(
        statusRow(groupRow, "load-more-error", "error", {
          label: entry.loadMoreError,
          action: "retry-load-more",
        }),
      );
    }
    if (summary.hasMore) {
      rows.push(loadMoreRow(groupRow, objectGroupLabel(group).toLowerCase(), summary.count));
    }
  });
}

function objectDetail(
  group: DatabaseObjectGroupKind,
  item: {
    estimatedRows?: { status: "available"; value: number } | { status: "unavailable" };
    kind?: string;
    dataType?: { status: "available"; value: string } | { status: "unavailable" };
  },
): string | undefined {
  if (group === "tables") {
    if (item.kind === "partitioned_table") {
      return "partitioned";
    }
    if (item.estimatedRows?.status === "available") {
      return `~${item.estimatedRows.value.toLocaleString()} rows`;
    }
    return undefined;
  }
  if (group === "views") {
    return item.kind === "materialized_view" ? "materialized" : "view";
  }
  return item.dataType?.status === "available" ? item.dataType.value : undefined;
}

function loadMoreRow(
  parent: DatabaseExplorerRow,
  subject: string,
  count: number | undefined,
): DatabaseExplorerRow {
  return {
    ...parent,
    id: [parent.id, "load-more"].join(ROW_SEPARATOR),
    kind: "load-more",
    label: count === undefined ? `Load more ${subject}` : `Load more ${subject} (${count})`,
    level: parent.level + 1,
    parentId: parent.id,
    discoveryNode: parent.discoveryNode,
    expandable: false,
    expanded: false,
    selectable: false,
    busy: parent.busy,
    action: "load-more",
    active: false,
    system: false,
    count: undefined,
    detail: undefined,
  };
}

function pushConnectionChildren(
  rows: DatabaseExplorerRow[],
  connectionRow: DatabaseExplorerRow,
  profile: DatabaseProfile,
  snapshot: DatabaseConnectionSnapshot | undefined,
  input: BuildDatabaseExplorerRowsInput,
  shouldExpand: boolean,
): void {
  if (!shouldExpand) {
    return;
  }

  if (!snapshot) {
    rows.push(
      statusRow(connectionRow, "disconnected", "disconnected", {
        label: "Connection required",
        detail: `Connect ${profile.name} over SSH to browse databases.`,
      }),
    );
    rows.push(
      statusRow(connectionRow, "connect", "disconnected", {
        label: `Connect ${profile.name}`,
        action: "connect",
      }),
    );
    return;
  }

  const discoveryNode: DatabaseExplorerNode = { kind: "databases" };
  const entry = input.nodes[databaseExplorerNodeKey(profile.id, discoveryNode)];

  if (!entry?.payload) {
    if (entry?.status === "error") {
      rows.push(
        ...errorRows(connectionRow, "databases", entry.error, entry.errorMessage, "retry-node"),
      );
    } else {
      rows.push(...loadingRows(connectionRow, "databases"));
    }
    return;
  }

  if (entry.status === "loading") {
    rows.push(statusRow(connectionRow, "refreshing", "loading", { label: "Refreshing databases" }));
  } else if (entry.status === "error") {
    rows.push(
      ...errorRows(connectionRow, "databases", entry.error, entry.errorMessage, "retry-node"),
    );
  }

  const databases = pagePayload<DatabaseSummary>(entry.payload);
  if (!databases) {
    return;
  }
  if (databases.items.length === 0) {
    rows.push(emptyRow(connectionRow, "No databases found."));
  } else {
    databases.items.forEach((database) => {
      const id = databaseRowId(profile.id, database.name);
      const databaseExpanded =
        input.expandAll || input.expandedIds.has(id) || Boolean(input.query?.trim());
      const activeDatabase = activeDatabaseFor(profile, snapshot);
      const databaseRow: DatabaseExplorerRow = {
        ...connectionRow,
        id,
        kind: "database",
        label: database.name,
        level: connectionRow.level + 1,
        parentId: connectionRow.id,
        database: database.name,
        active: activeDatabase === database.name,
        discoveryNode: null,
        expandable: true,
        expanded: databaseExpanded,
        selectable: true,
        system: database.system,
        busy: false,
      };
      rows.push(databaseRow);
      pushDatabaseChildren(rows, databaseRow, profile, snapshot, input, databaseExpanded);
    });
  }

  if (pageSummary(entry).hasMore) {
    rows.push(loadMoreRow(connectionRow, "databases", pageSummary(entry).count));
  }
}

export function buildDatabaseExplorerRows(
  input: BuildDatabaseExplorerRowsInput,
): DatabaseExplorerRow[] {
  const searchActive = Boolean(input.query?.trim()) || Boolean(input.expandAll);
  const rows: DatabaseExplorerRow[] = [];

  input.profiles.forEach((profile) => {
    const snapshot = connectedSnapshot(profile, input.snapshots);
    const id = databaseConnectionRowId(profile.id);
    const expanded = input.expandAll || input.expandedIds.has(id) || searchActive;
    const databaseEntry = input.nodes[databaseExplorerNodeKey(profile.id, { kind: "databases" })];
    const connectionRow: DatabaseExplorerRow = {
      id,
      kind: "connection",
      label: profile.name,
      level: 1,
      parentId: null,
      profileId: profile.id,
      profileName: profile.name,
      engine: profile.engine,
      database: null,
      schema: null,
      objectName: null,
      discoveryNode: { kind: "databases" },
      expandable: true,
      expanded,
      selectable: true,
      busy: databaseEntry?.status === "loading" || databaseEntry?.loadingMore === true,
    };
    rows.push(connectionRow);
    pushConnectionChildren(rows, connectionRow, profile, snapshot, input, expanded);
  });

  const query = input.query?.trim().toLowerCase() ?? "";
  if (!query) {
    return rows;
  }

  const byId = new Map(rows.map((row) => [row.id, row]));
  const included = new Set<string>();

  rows.forEach((row) => {
    if (row.kind === "status" || row.kind === "load-more" || !rowMatches(row, query)) {
      return;
    }
    let current: DatabaseExplorerRow | undefined = row;
    while (current) {
      included.add(current.id);
      current = current.parentId ? byId.get(current.parentId) : undefined;
    }
  });

  return rows
    .filter((row) => included.has(row.id))
    .map((row) => {
      if (!row.expandable) {
        return row;
      }
      return {
        ...row,
        expanded: included.has(row.id)
          ? rows.some((child) => child.parentId === row.id && included.has(child.id))
          : false,
      };
    });
}

/** Loaded discovery nodes used by the whole-tree refresh action. */
export function loadedDiscoveryNodes(
  input: BuildDatabaseExplorerRowsInput,
): Array<{ profileId: string; node: DatabaseExplorerNode }> {
  const rows = buildDatabaseExplorerRows({ ...input, expandAll: true, query: "" });
  const seen = new Set<string>();
  const nodes: Array<{ profileId: string; node: DatabaseExplorerNode }> = [];

  rows.forEach((row) => {
    if (!row.discoveryNode) {
      return;
    }
    const key = databaseExplorerNodeKey(row.profileId, row.discoveryNode);
    if (seen.has(key) || !input.nodes[key]) {
      return;
    }
    seen.add(key);
    nodes.push({ profileId: row.profileId, node: row.discoveryNode });
  });

  return nodes;
}

export function pageRequestForEntry(
  entry: DatabaseExplorerNodeEntry | undefined,
): { limit: number; offset: number } | undefined {
  if (!entry?.payload || entry.payload.nodeKind === "objectCounts") {
    return undefined;
  }
  const payload = entry.payload.payload;
  const limit = Math.min(500, Math.max(payload.limit, payload.items.length));
  return limit > 0 ? { limit, offset: 0 } : undefined;
}
