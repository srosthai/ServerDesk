import { objectRowId } from "@/features/databases/databaseExplorerTreeModel";
import type { DatabaseExplorerSelection } from "@/features/databases/databaseExplorerTreeModel";
import type {
  DatabaseConstraintSummary,
  DatabaseMetadataValue,
  DatabaseProfile,
  DatabaseTableSummary,
} from "@/types/contracts";

/** The graph never asks the backend for more than one bounded catalog page. */
export const ER_DIAGRAM_CATALOG_LIMIT = 500;

/** Layout and rendering limits keep a large schema responsive. */
export const ER_DIAGRAM_DEFAULT_TABLE_LIMIT = 50;
export const ER_DIAGRAM_TABLE_LIMIT_OPTIONS = [25, 50, 100] as const;
export const ER_DIAGRAM_MAX_TABLE_LIMIT = 100;
export const ER_DIAGRAM_MAX_RELATIONSHIPS = 300;

export type ErDiagramCardinality = "one-to-one" | "many-to-one";

export type ErDiagramTable = {
  key: string;
  database: string | null;
  schema: string | null;
  name: string;
  qualifiedName: string;
  kind: DatabaseTableSummary["kind"];
  estimatedRows: number | null;
  incoming: number;
  outgoing: number;
};

export type ErDiagramRelationship = {
  id: string;
  name: string;
  sourceKey: string;
  targetKey: string;
  sourceTable: string;
  targetTable: string;
  sourceColumns: string[];
  targetColumns: string[];
  cardinality: ErDiagramCardinality;
  onUpdate: string | null;
  onDelete: string | null;
};

export type ErDiagramStats = {
  /** Table count reported by the catalog page, or the number loaded if unknown. */
  totalTables: number;
  totalTablesKnown: boolean;
  loadedTables: number;
  matchedTables: number;
  visibleTables: number;
  hiddenBySearch: number;
  hiddenByLimit: number;
  unloadedTables: number;
  catalogTruncated: boolean;
  duplicateTables: number;
  totalRelationships: number;
  visibleRelationships: number;
  hiddenRelationships: number;
  truncatedRelationships: boolean;
  unresolvedForeignKeys: number;
  isolatedTables: number;
};

export type ErDiagramGraph = {
  tables: ErDiagramTable[];
  relationships: ErDiagramRelationship[];
  stats: ErDiagramStats;
};

export type NormalizedErDiagramTables = {
  tables: DatabaseTableSummary[];
  duplicateTables: number;
};

export type ErDiagramTableSelection = {
  all: DatabaseTableSummary[];
  matched: DatabaseTableSummary[];
  visible: DatabaseTableSummary[];
  hiddenBySearch: number;
  hiddenByLimit: number;
};

type BuildErDiagramInput = {
  tables: DatabaseTableSummary[];
  /** Constraints keyed by [`erDiagramTableKey`]. Missing keys mean "not loaded". */
  constraintsByTable: Record<string, DatabaseConstraintSummary[]>;
  query?: string;
  limit?: number;
  totalTableCount?: number | null;
  catalogTruncated?: boolean;
};

function metadataValue<T>(value: DatabaseMetadataValue<T>): T | null {
  return value.status === "available" ? value.value : null;
}

export function erDiagramTableKey(
  database: string | null,
  schema: string | null,
  table: string,
): string {
  return [database ?? "", schema ?? "", table].map(encodeURIComponent).join("::");
}

export function erDiagramQualifiedName(schema: string | null, table: string): string {
  return schema ? `${schema}.${table}` : table;
}

/** Deduplicates a catalog page and returns a stable schema/table order. */
export function normalizeErDiagramTables(
  tables: readonly DatabaseTableSummary[],
): NormalizedErDiagramTables {
  const seen = new Set<string>();
  const unique: DatabaseTableSummary[] = [];
  let duplicateTables = 0;

  for (const table of tables) {
    const key = erDiagramTableKey(table.database, table.schema, table.name);
    if (seen.has(key)) {
      duplicateTables += 1;
      continue;
    }
    seen.add(key);
    unique.push(table);
  }

  unique.sort((left, right) => {
    const schemaOrder = (left.schema ?? "").localeCompare(right.schema ?? "");
    return schemaOrder === 0 ? left.name.localeCompare(right.name) : schemaOrder;
  });

  return { tables: unique, duplicateTables };
}

function queryTokens(query: string): string[] {
  return query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
}

/**
 * Applies the table search and the layout limit. All tokens must appear in the
 * qualified name, so `public ord` narrows a `public.orders` table without
 * requiring an exact prefix.
 */
export function selectErDiagramTables(
  tables: readonly DatabaseTableSummary[],
  query = "",
  limit = ER_DIAGRAM_DEFAULT_TABLE_LIMIT,
): ErDiagramTableSelection {
  const normalized = normalizeErDiagramTables(tables).tables;
  const tokens = queryTokens(query);
  const matched = tokens.length
    ? normalized.filter((table) => {
        const haystack = [erDiagramQualifiedName(table.schema, table.name), table.database ?? ""]
          .join(" ")
          .toLocaleLowerCase();
        return tokens.every((token) => haystack.includes(token));
      })
    : normalized;
  const boundedLimit = Math.max(1, Math.min(Math.floor(limit), ER_DIAGRAM_MAX_TABLE_LIMIT));

  return {
    all: normalized,
    matched,
    visible: matched.slice(0, boundedLimit),
    hiddenBySearch: normalized.length - matched.length,
    hiddenByLimit: Math.max(0, matched.length - boundedLimit),
  };
}

function normalizedColumns(columns: readonly string[]): string {
  return columns
    .map((column) => column.toLocaleLowerCase())
    .sort()
    .join("\u0000");
}

/**
 * A foreign key is one-to-one when the referencing columns are also covered by
 * a primary key or unique constraint. Otherwise one parent row can be
 * referenced by many child rows.
 */
export function relationshipCardinality(
  sourceConstraints: readonly DatabaseConstraintSummary[],
  constraint: DatabaseConstraintSummary,
): ErDiagramCardinality {
  if (constraint.columns.length === 0) {
    return "many-to-one";
  }
  const foreignKeyColumns = normalizedColumns(constraint.columns);
  const unique = sourceConstraints.some(
    (candidate) =>
      (candidate.kind === "primary_key" || candidate.kind === "unique") &&
      candidate.columns.length === constraint.columns.length &&
      normalizedColumns(candidate.columns) === foreignKeyColumns,
  );
  return unique ? "one-to-one" : "many-to-one";
}

export function erDiagramCardinalityLabel(cardinality: ErDiagramCardinality): string {
  return cardinality === "one-to-one" ? "1:1" : "N:1";
}

export function erDiagramCardinalityDescription(cardinality: ErDiagramCardinality): string {
  return cardinality === "one-to-one"
    ? "one child row references one parent row"
    : "many child rows reference one parent row";
}

function constraintTarget(
  table: DatabaseTableSummary,
  constraint: DatabaseConstraintSummary,
): { key: string; tableName: string; schema: string | null } | null {
  const referencedTable = metadataValue(constraint.referencedTable);
  if (!referencedTable) {
    return null;
  }
  // PostgreSQL always reports the referenced schema. MySQL reports the
  // referenced database as the schema, which matches its table summaries.
  const referencedSchema = metadataValue(constraint.referencedSchema) ?? table.schema;
  return {
    key: erDiagramTableKey(table.database, referencedSchema, referencedTable),
    tableName: referencedTable,
    schema: referencedSchema,
  };
}

function buildRelationship(
  table: DatabaseTableSummary,
  constraint: DatabaseConstraintSummary,
  target: { key: string; tableName: string; schema: string | null },
  constraints: readonly DatabaseConstraintSummary[],
): ErDiagramRelationship {
  return {
    id: `${erDiagramTableKey(table.database, table.schema, table.name)}::${constraint.name}`,
    name: constraint.name,
    sourceKey: erDiagramTableKey(table.database, table.schema, table.name),
    targetKey: target.key,
    sourceTable: erDiagramQualifiedName(table.schema, table.name),
    targetTable: erDiagramQualifiedName(target.schema, target.tableName),
    sourceColumns: [...constraint.columns],
    targetColumns: [...constraint.referencedColumns],
    cardinality: relationshipCardinality(constraints, constraint),
    onUpdate: metadataValue(constraint.onUpdate),
    onDelete: metadataValue(constraint.onDelete),
  };
}

/**
 * Builds a read-only diagram from a table catalog and the foreign-key
 * constraints that have been loaded for those tables. Relationships that point
 * outside the loaded catalog are counted, not invented. Layout and rendering
 * limits apply after filtering so a large schema stays responsive.
 */
export function buildErDiagram({
  tables,
  constraintsByTable,
  query = "",
  limit = ER_DIAGRAM_DEFAULT_TABLE_LIMIT,
  totalTableCount = null,
  catalogTruncated = false,
}: BuildErDiagramInput): ErDiagramGraph {
  const normalized = normalizeErDiagramTables(tables);
  const selection = selectErDiagramTables(normalized.tables, query, limit);
  const allKeys = new Set(
    selection.all.map((table) => erDiagramTableKey(table.database, table.schema, table.name)),
  );
  const visibleKeys = new Set(
    selection.visible.map((table) => erDiagramTableKey(table.database, table.schema, table.name)),
  );

  const allRelationships: ErDiagramRelationship[] = [];
  let unresolvedForeignKeys = 0;

  for (const table of selection.all) {
    const key = erDiagramTableKey(table.database, table.schema, table.name);
    const constraints = constraintsByTable[key] ?? [];
    for (const constraint of constraints) {
      if (constraint.kind !== "foreign_key") {
        continue;
      }
      const target = constraintTarget(table, constraint);
      if (!target || !allKeys.has(target.key)) {
        unresolvedForeignKeys += 1;
        continue;
      }
      allRelationships.push(buildRelationship(table, constraint, target, constraints));
    }
  }

  allRelationships.sort((left, right) => {
    const sourceOrder = left.sourceTable.localeCompare(right.sourceTable);
    if (sourceOrder !== 0) {
      return sourceOrder;
    }
    const targetOrder = left.targetTable.localeCompare(right.targetTable);
    return targetOrder === 0 ? left.name.localeCompare(right.name) : targetOrder;
  });

  const visibleBeforeLimit = allRelationships.filter(
    (relationship) =>
      visibleKeys.has(relationship.sourceKey) && visibleKeys.has(relationship.targetKey),
  );
  const relationships = visibleBeforeLimit.slice(0, ER_DIAGRAM_MAX_RELATIONSHIPS);
  const truncatedRelationships = visibleBeforeLimit.length > relationships.length;

  const incoming = new Map<string, number>();
  const outgoing = new Map<string, number>();
  for (const relationship of relationships) {
    outgoing.set(relationship.sourceKey, (outgoing.get(relationship.sourceKey) ?? 0) + 1);
    incoming.set(relationship.targetKey, (incoming.get(relationship.targetKey) ?? 0) + 1);
  }

  const visibleTables: ErDiagramTable[] = selection.visible.map((table) => {
    const key = erDiagramTableKey(table.database, table.schema, table.name);
    return {
      key,
      database: table.database,
      schema: table.schema,
      name: table.name,
      qualifiedName: erDiagramQualifiedName(table.schema, table.name),
      kind: table.kind,
      estimatedRows: metadataValue(table.estimatedRows),
      incoming: incoming.get(key) ?? 0,
      outgoing: outgoing.get(key) ?? 0,
    };
  });

  const loadedTables = selection.all.length;
  const reportedTotal = totalTableCount ?? loadedTables;
  const totalTables = Math.max(reportedTotal, loadedTables);

  return {
    tables: visibleTables,
    relationships,
    stats: {
      totalTables,
      totalTablesKnown: totalTableCount !== null,
      loadedTables,
      matchedTables: selection.matched.length,
      visibleTables: visibleTables.length,
      hiddenBySearch: selection.hiddenBySearch,
      hiddenByLimit: selection.hiddenByLimit,
      unloadedTables: Math.max(0, totalTables - loadedTables),
      catalogTruncated,
      duplicateTables: normalized.duplicateTables,
      totalRelationships: allRelationships.length,
      visibleRelationships: relationships.length,
      hiddenRelationships: allRelationships.length - relationships.length,
      truncatedRelationships,
      unresolvedForeignKeys,
      isolatedTables: visibleTables.filter((table) => table.incoming === 0 && table.outgoing === 0)
        .length,
    },
  };
}

/** Builds the same explorer selection a tree click would produce. */
export function erDiagramSelectionForTable(
  profile: DatabaseProfile,
  table: Pick<ErDiagramTable, "database" | "schema" | "name">,
): DatabaseExplorerSelection | null {
  if (!table.database) {
    return null;
  }
  const schema = profile.engine === "mysql" ? null : table.schema;
  return {
    id: objectRowId(profile.id, table.database, schema, "tables", table.name),
    profileId: profile.id,
    kind: "table",
    label: table.name,
    engine: profile.engine,
    database: table.database,
    schema,
    objectName: table.name,
  };
}

export function erDiagramTableCountLabel(stats: ErDiagramStats): string {
  if (stats.catalogTruncated && !stats.totalTablesKnown) {
    return `Showing ${stats.visibleTables.toLocaleString()} of ${stats.loadedTables.toLocaleString()}+ tables`;
  }
  return `Showing ${stats.visibleTables.toLocaleString()} of ${stats.totalTables.toLocaleString()} tables`;
}

export function erDiagramRelationshipColumns(
  relationship: Pick<ErDiagramRelationship, "sourceColumns" | "targetColumns">,
): string {
  const source = relationship.sourceColumns.join(", ") || "—";
  const target = relationship.targetColumns.join(", ") || "—";
  return `${source} → ${target}`;
}
