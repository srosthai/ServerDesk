import type {
  CapabilityFlags,
  DatabaseAction,
  DatabaseColumnSummary,
  DatabaseConstraintKind,
  DatabaseConstraintSummary,
  DatabaseDependencySummary,
  DatabaseEngine,
  DatabaseIndexSummary,
  DatabaseMetadataValue,
  DatabaseTableDependencies,
  DatabaseTableSummary,
} from "@/types/contracts";

/** One table, schema, and database selection from the explorer. */
export type DatabaseTableScope = {
  database: string | null;
  schema: string | null;
  table: string;
};

export type StructureDecision = {
  enabled: boolean;
  reason: string | null;
};

/** A user request to open one structure editor dialog. */
export type DatabaseStructureIntent =
  | { type: "create_table" }
  | { type: "rename_table" }
  | { type: "duplicate_table" }
  | { type: "truncate_table" }
  | { type: "drop_table" }
  | { type: "add_column" }
  | { type: "edit_column"; column: DatabaseColumnSummary }
  | { type: "rename_column"; column: DatabaseColumnSummary }
  | { type: "drop_column"; column: DatabaseColumnSummary }
  | { type: "create_index" }
  | { type: "rename_index"; index: DatabaseIndexSummary }
  | { type: "drop_index"; index: DatabaseIndexSummary }
  | { type: "add_constraint" }
  | { type: "drop_constraint"; constraint: DatabaseConstraintSummary };

export type StructureCapabilities = {
  createTable: StructureDecision;
  renameTable: StructureDecision;
  duplicateTable: StructureDecision;
  truncateTable: StructureDecision;
  dropTable: StructureDecision;
  addColumn: StructureDecision;
  editColumn: StructureDecision;
  renameColumn: StructureDecision;
  dropColumn: StructureDecision;
  reorderColumns: StructureDecision;
  createIndex: StructureDecision;
  renameIndex: StructureDecision;
  dropIndex: StructureDecision;
  addConstraint: StructureDecision;
  addCheckConstraint: StructureDecision;
  generatedColumns: StructureDecision;
  identityColumns: StructureDecision;
  autoIncrementColumns: StructureDecision;
  partialIndexes: StructureDecision;
};

export type TableRelationship = {
  id: string;
  direction: "outgoing" | "incoming";
  name: string;
  sourceTable: string;
  sourceColumns: string[];
  targetTable: string;
  targetColumns: string[];
  onUpdate: string | null;
  onDelete: string | null;
};

export type DependencyGroup = {
  key: "incomingForeignKeys" | "dependentViews" | "ownedSequences" | "triggers";
  label: string;
  state: "available" | "unavailable";
  items: DatabaseDependencySummary[];
  truncated: boolean;
};

export type TableSummaryRow = {
  label: string;
  value: string;
  mono?: boolean;
};

export function engineLabel(engine: DatabaseEngine): string {
  return engine === "mysql" ? "MySQL" : "PostgreSQL";
}

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

/**
 * Derives UI action availability from the capability document the backend
 * returned for this connection. Baseline actions stay available when an older
 * capability payload has no matching action entry; version-gated actions are
 * conservative and require an explicit capability.
 */
export function structureCapabilitiesFor(input: {
  engine: DatabaseEngine;
  version?: string | null;
  connected: boolean;
  readOnly: boolean;
  actions?: DatabaseAction[];
  flags?: CapabilityFlags;
}): StructureCapabilities {
  const { actions, flags } = input;
  const actionSet = actions ? new Set(actions) : null;
  const globalReason = !input.connected
    ? "Connect this database profile before changing table structure."
    : input.readOnly
      ? "This database profile is read-only. Save a writable profile before changing table structure."
      : null;

  function decide(
    action: DatabaseAction | null,
    label: string,
    flag?: keyof CapabilityFlags,
  ): StructureDecision {
    if (globalReason) {
      return disabled(globalReason);
    }
    if (actionSet && action && !actionSet.has(action)) {
      return disabled(unsupportedReason(label, input.engine, input.version));
    }
    if (!actionSet && flag) {
      if (!flags) {
        return disabled(
          "Capability details are unavailable for this connection. Reconnect to refresh them before using this action.",
        );
      }
      if (!flags[flag]) {
        return disabled(unsupportedReason(label, input.engine, input.version));
      }
    }
    return enabled();
  }

  return {
    createTable: decide("create_table", "Table creation"),
    renameTable: decide("rename_table", "Table renaming", "tableRename"),
    duplicateTable: decide("duplicate_table", "Table duplication", "tableDuplicate"),
    truncateTable: decide("truncate_table", "Table truncation"),
    dropTable: decide("drop_table", "Table drop"),
    addColumn: decide("add_column", "Column addition"),
    editColumn: decide("edit_column", "Column editing", "columnTypeChange"),
    renameColumn: decide("rename_column", "Column renaming", "columnRename"),
    dropColumn: decide("drop_column", "Column drop"),
    reorderColumns: decide("reorder_columns", "Column reordering", "columnReorder"),
    createIndex: decide("create_index", "Index creation"),
    renameIndex: decide("rename_index", "Index renaming", "indexRename"),
    dropIndex: decide("drop_index", "Index drop"),
    addConstraint: decide(null, "Constraint creation"),
    addCheckConstraint: decide(
      "create_check_constraint",
      "Check constraint creation",
      "checkConstraints",
    ),
    generatedColumns: decide(
      "create_generated_column",
      "Generated column creation",
      "generatedColumns",
    ),
    identityColumns: decide(
      "create_identity_column",
      "Identity column creation",
      "identityColumns",
    ),
    autoIncrementColumns: decide(
      "create_auto_increment_column",
      "Auto-increment column creation",
      "autoIncrementColumns",
    ),
    partialIndexes: decide("create_partial_index", "Partial index creation", "partialIndexes"),
  };
}

export function formatMetadataValue<T>(
  value: DatabaseMetadataValue<T>,
  format: (value: T) => string,
  unavailable = "Unavailable",
): string {
  return value.status === "available" ? format(value.value) : unavailable;
}

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) {
    return "Unavailable";
  }
  if (value < 1024) {
    return `${value.toLocaleString()} B`;
  }
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let amount = value / 1024;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit += 1;
  }
  return `${amount.toLocaleString(undefined, { maximumFractionDigits: 1 })} ${units[unit]}`;
}

export function formatRowEstimate(value: DatabaseMetadataValue<number>): string {
  return formatMetadataValue(value, (rows) => rows.toLocaleString());
}

export function columnTypeLabel(column: DatabaseColumnSummary): string {
  const type = column.dataType;
  if (type.includes("(")) {
    return type;
  }
  if (column.length.status === "available" && column.length.value !== null) {
    return `${type}(${column.length.value})`;
  }
  if (column.precision.status === "available" && column.precision.value !== null) {
    if (column.scale.status === "available" && column.scale.value !== null) {
      return `${type}(${column.precision.value},${column.scale.value})`;
    }
    return `${type}(${column.precision.value})`;
  }
  return type;
}

export function columnDefaultLabel(column: DatabaseColumnSummary): string {
  return formatMetadataValue(column.defaultValue, (value) =>
    value === null || value.trim() === "" ? "—" : value,
  );
}

export function columnIdentityLabel(column: DatabaseColumnSummary): string {
  return formatMetadataValue(column.identity, (identity) => {
    switch (identity) {
      case "auto_increment":
        return "Auto increment";
      case "always":
        return "Identity (always)";
      case "by_default":
        return "Identity (by default)";
      default:
        return "—";
    }
  });
}

export function columnGeneratedLabel(column: DatabaseColumnSummary): string {
  if (column.generated.status === "unavailable") {
    return "Unavailable";
  }
  if (!column.generated.value) {
    return "—";
  }
  if (column.generationExpression.status === "available" && column.generationExpression.value) {
    return column.generationExpression.value;
  }
  return "Generated";
}

export function tableKindLabel(kind: DatabaseTableSummary["kind"]): string {
  switch (kind) {
    case "partitioned_table":
      return "Partitioned table";
    case "foreign_table":
      return "Foreign table";
    case "view":
      return "View";
    case "other":
      return "Other";
    default:
      return "Table";
  }
}

export function constraintKindLabel(kind: DatabaseConstraintKind): string {
  switch (kind) {
    case "primary_key":
      return "Primary key";
    case "unique":
      return "Unique";
    case "foreign_key":
      return "Foreign key";
    case "check":
      return "Check";
    default:
      return "Other";
  }
}

export function qualifiedTableName(schema: string | null, table: string): string {
  return schema ? `${schema}.${table}` : table;
}

export function tableSummaryKey(profileId: string, scope: DatabaseTableScope): string {
  return [profileId, scope.database ?? "", scope.schema ?? "", scope.table].join("::");
}

/**
 * Resolves the table summary the explorer already loaded. Metadata discovery
 * has no single-table read command, so the tree page is the authoritative
 * summary for a selected row.
 */
export function findTableSummary(
  nodes: Record<string, { payload?: { nodeKind: string; payload: unknown } }>,
  profileId: string,
  scope: DatabaseTableScope,
): DatabaseTableSummary | null {
  for (const [key, entry] of Object.entries(nodes)) {
    if (!key.startsWith(`${profileId}::tables::`)) {
      continue;
    }
    const payload = entry.payload;
    if (!payload || payload.nodeKind !== "tables") {
      continue;
    }
    const page = payload.payload as { items?: DatabaseTableSummary[] };
    const summary = page.items?.find((item) => {
      if (item.name !== scope.table) {
        return false;
      }
      const databaseMatches =
        scope.database === null || item.database === null || item.database === scope.database;
      const schemaMatches =
        scope.schema === null || item.schema === null || item.schema === scope.schema;
      return databaseMatches && schemaMatches;
    });
    if (summary) {
      return summary;
    }
  }
  return null;
}

export function relationshipTarget(schema: string | null, table: string | null): string | null {
  if (!table) {
    return null;
  }
  return qualifiedTableName(schema, table);
}

export function buildTableRelationships(
  scope: DatabaseTableScope,
  constraints: DatabaseConstraintSummary[],
  dependencies: DatabaseTableDependencies | null,
): TableRelationship[] {
  const relationships: TableRelationship[] = [];
  const currentTable = qualifiedTableName(scope.schema, scope.table);

  constraints
    .filter((constraint) => constraint.kind === "foreign_key")
    .forEach((constraint) => {
      const referencedSchema =
        constraint.referencedSchema.status === "available"
          ? constraint.referencedSchema.value
          : null;
      const referencedTable =
        constraint.referencedTable.status === "available" ? constraint.referencedTable.value : null;
      if (!referencedTable) {
        return;
      }
      relationships.push({
        id: `outgoing:${constraint.name}`,
        direction: "outgoing",
        name: constraint.name,
        sourceTable: currentTable,
        sourceColumns: constraint.columns,
        targetTable: qualifiedTableName(referencedSchema, referencedTable),
        targetColumns: constraint.referencedColumns,
        onUpdate: constraint.onUpdate.status === "available" ? constraint.onUpdate.value : null,
        onDelete: constraint.onDelete.status === "available" ? constraint.onDelete.value : null,
      });
    });

  if (dependencies?.incomingForeignKeys.status === "available") {
    dependencies.incomingForeignKeys.value.items.forEach((dependency) => {
      const schema = dependency.schema;
      const table = dependency.table;
      if (!table) {
        return;
      }
      relationships.push({
        id: `incoming:${schema ?? ""}:${table}:${dependency.name}`,
        direction: "incoming",
        name: dependency.name,
        sourceTable: qualifiedTableName(schema, table),
        sourceColumns: dependency.columns,
        targetTable: currentTable,
        targetColumns: dependency.referencedColumns,
        onUpdate: dependency.onUpdate.status === "available" ? dependency.onUpdate.value : null,
        onDelete: dependency.onDelete.status === "available" ? dependency.onDelete.value : null,
      });
    });
  }

  return relationships;
}

export function dependencyObjectLabel(dependency: DatabaseDependencySummary): string {
  const schema = dependency.schema;
  const table = dependency.table;
  return [schema, table, dependency.name].filter(Boolean).join(".");
}

export function dependencyGroups(
  dependencies: DatabaseTableDependencies | null,
): DependencyGroup[] {
  if (!dependencies) {
    return [];
  }
  return [
    {
      key: "incomingForeignKeys",
      label: "Incoming foreign keys",
      ...boundedGroup(dependencies.incomingForeignKeys),
    },
    {
      key: "dependentViews",
      label: "Dependent views",
      ...boundedGroup(dependencies.dependentViews),
    },
    {
      key: "ownedSequences",
      label: "Owned sequences",
      ...boundedGroup(dependencies.ownedSequences),
    },
    {
      key: "triggers",
      label: "Triggers",
      ...boundedGroup(dependencies.triggers),
    },
  ];
}

function boundedGroup(
  value: DatabaseTableDependencies["incomingForeignKeys"],
): Pick<DependencyGroup, "state" | "items" | "truncated"> {
  if (value.status === "unavailable") {
    return { state: "unavailable", items: [], truncated: false };
  }
  return {
    state: "available",
    items: value.value.items,
    truncated: value.value.truncated,
  };
}

export function tableSummaryRows(
  table: DatabaseTableSummary | null,
  scope: DatabaseTableScope,
): TableSummaryRow[] {
  const database = table?.database ?? scope.database;
  const schema = table?.schema ?? scope.schema;
  const rows: TableSummaryRow[] = [
    { label: "Name", value: table?.name ?? scope.table, mono: true },
    { label: "Database", value: database ?? "Active database", mono: true },
    { label: "Schema", value: schema ?? "Default schema", mono: true },
    { label: "Kind", value: table ? tableKindLabel(table.kind) : "Unavailable" },
  ];

  if (!table) {
    return rows;
  }

  rows.push(
    { label: "Estimated rows", value: formatRowEstimate(table.estimatedRows) },
    {
      label: "Size",
      value:
        table.sizeBytes.status === "available" ? formatBytes(table.sizeBytes.value) : "Unavailable",
    },
    {
      label: "Owner",
      value: formatMetadataValue(table.owner, (value) => value),
    },
    {
      label: "Storage engine",
      value: formatMetadataValue(table.engine, (value) => value),
    },
    {
      label: "Encoding",
      value: formatMetadataValue(table.encoding, (value) => value),
    },
    {
      label: "Collation",
      value: formatMetadataValue(table.collation, (value) => value),
    },
    {
      label: "Statistics refreshed",
      value: formatMetadataValue(table.metadataRefreshedAt, (value) => value),
    },
  );

  return rows;
}

/** Actions that should be listed when they are unavailable. */
export function unavailableDecisions(
  entries: Array<{ label: string; decision: StructureDecision }>,
): Array<{ label: string; reason: string }> {
  const seen = new Set<string>();
  const result: Array<{ label: string; reason: string }> = [];
  entries.forEach(({ label, decision }) => {
    if (decision.enabled || !decision.reason || seen.has(decision.reason)) {
      return;
    }
    seen.add(decision.reason);
    result.push({ label, reason: decision.reason });
  });
  return result;
}

export function indexActionDecisions(
  index: DatabaseIndexSummary,
  capabilities: StructureCapabilities,
): { rename: StructureDecision; drop: StructureDecision } {
  if (index.primary) {
    const reason =
      "The primary key index is managed by its primary-key constraint. Rename or drop the constraint instead.";
    return { rename: disabled(reason), drop: disabled(reason) };
  }
  return { rename: capabilities.renameIndex, drop: capabilities.dropIndex };
}

export function constraintActionDecision(
  constraint: DatabaseConstraintSummary,
  capabilities: StructureCapabilities,
): StructureDecision {
  if (constraint.kind === "check" && !capabilities.addCheckConstraint.enabled) {
    return capabilities.addCheckConstraint;
  }
  return capabilities.addConstraint;
}
