import { invoke } from "@tauri-apps/api/core";
import type {
  DatabaseConstraintInput,
  DatabaseConstraintKind,
  DatabaseStructureActionRequest,
  DatabaseStructureActionResult,
  DatabaseStructureDdlPreview,
  DatabaseTableOptionsInput,
  DatabaseUniqueKeyInput,
  DatabaseColumnDefinitionInput,
  DatabaseColumnDefaultInput,
  DatabaseColumnPositionInput,
} from "@/types/contracts";

export const databaseStructureCommands = {
  preview: "preview_database_structure_action",
  run: "run_database_structure_action",
} as const;

/**
 * Requests generated DDL for a table, column, index, or constraint change.
 * This never executes the DDL.
 */
export function previewDatabaseStructureAction(
  profileId: string,
  request: DatabaseStructureActionRequest,
): Promise<DatabaseStructureDdlPreview> {
  return invoke<DatabaseStructureDdlPreview>(databaseStructureCommands.preview, {
    profileId,
    request,
  });
}

/**
 * Executes one validated structure action. The backend regenerates the same
 * preview first, rejects an unconfirmed drop/truncate, and re-reads metadata
 * before reporting a verified result.
 */
export function runDatabaseStructureAction(
  profileId: string,
  request: DatabaseStructureActionRequest,
  confirmed = false,
): Promise<DatabaseStructureActionResult> {
  return invoke<DatabaseStructureActionResult>(databaseStructureCommands.run, {
    profileId,
    request,
    confirmed,
  });
}

type TableScope = { database?: string | null; schema?: string | null };

export function createDatabaseTable(
  profileId: string,
  input: {
    name: string;
    columns: DatabaseColumnDefinitionInput[];
    primaryKey?: string[];
    uniqueKeys?: DatabaseUniqueKeyInput[];
    options?: DatabaseTableOptionsInput | null;
  } & TableScope,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "create_table", ...input });
}

export function renameDatabaseTable(
  profileId: string,
  input: { name: string; newName: string } & TableScope,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "rename_table", ...input });
}

export function duplicateDatabaseTable(
  profileId: string,
  input: { name: string; newName: string } & TableScope,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "duplicate_table", ...input });
}

/**
 * Destructive actions require an explicit confirmation argument. Keeping it
 * required in the service signature prevents an accidental drop from a caller
 * that forgets to opt in.
 */
export function truncateDatabaseTable(
  profileId: string,
  input: { name: string } & TableScope,
  confirmed: boolean,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "truncate_table", ...input }, confirmed);
}

export function dropDatabaseTable(
  profileId: string,
  input: { name: string } & TableScope,
  confirmed: boolean,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "drop_table", ...input }, confirmed);
}

type TableColumnScope = TableScope & { table: string };

/**
 * Adds one column with a type, optional length/precision, nullability,
 * default, generated/identity option, and comment. MySQL can place the column
 * with `position`; PostgreSQL appends and rejects a position.
 */
export function addDatabaseColumn(
  profileId: string,
  input: {
    column: DatabaseColumnDefinitionInput;
    position?: DatabaseColumnPositionInput | null;
  } & TableColumnScope,
  confirmed = false,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "add_column", ...input }, confirmed);
}

/**
 * Edits supported column properties. MySQL needs the complete type and
 * nullability because MODIFY COLUMN replaces the whole definition; the backend
 * rejects a partial request with a capability explanation.
 */
export function alterDatabaseColumn(
  profileId: string,
  input: {
    name: string;
    dataType?: string | null;
    nullable?: boolean | null;
    default?: DatabaseColumnDefaultInput | null;
    dropDefault?: boolean;
    comment?: string | null;
    dropComment?: boolean;
  } & TableColumnScope,
  confirmed = false,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "alter_column", ...input }, confirmed);
}

export function renameDatabaseColumn(
  profileId: string,
  input: { name: string; newName: string } & TableColumnScope,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "rename_column", ...input });
}

/**
 * Dropping a column is destructive and always requires an explicit
 * confirmation argument.
 */
export function dropDatabaseColumn(
  profileId: string,
  input: { name: string } & TableColumnScope,
  confirmed: boolean,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "drop_column", ...input }, confirmed);
}

/**
 * Reorders one column where the engine supports a safe move. PostgreSQL
 * rejects this action; callers should rely on the capability snapshot and the
 * preview error instead of hiding the limitation in the client.
 */
export function reorderDatabaseColumn(
  profileId: string,
  input: {
    column: DatabaseColumnDefinitionInput;
    position: DatabaseColumnPositionInput;
  } & TableColumnScope,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "reorder_column", ...input });
}

/**
 * Creates a secondary or unique index. The backend previews the exact DDL and
 * rejects partial indexes when the selected engine/version cannot express
 * them. Rename is available on MySQL 5.7+ and PostgreSQL 9.2+; a caller that
 * targets an older server should disable the action and show the capability
 * explanation instead of offering a no-op.
 */
export function createDatabaseIndex(
  profileId: string,
  input: {
    table: string;
    name: string;
    columns: string[];
    unique?: boolean;
    method?: string | null;
    predicate?: string | null;
  } & TableScope,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "create_index", ...input });
}

export function renameDatabaseIndex(
  profileId: string,
  input: { table: string; name: string; newName: string } & TableScope,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "rename_index", ...input });
}

/**
 * Dropping an index removes a structural optimization and requires an explicit
 * confirmation. The preview also names a blocking constraint or incoming
 * foreign key before the drop is attempted.
 */
export function dropDatabaseIndex(
  profileId: string,
  input: { table: string; name: string } & TableScope,
  confirmed: boolean,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "drop_index", ...input }, confirmed);
}

/**
 * Adds a primary-key, unique, foreign-key, or check constraint. The backend
 * verifies the constraint kind, ordered columns, referenced table/columns, and
 * update/delete actions after execution.
 */
export function addDatabaseConstraint(
  profileId: string,
  input: { table: string; constraint: DatabaseConstraintInput } & TableScope,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "add_constraint", ...input });
}

/**
 * Drops one inspected constraint by name and kind. The caller must pass the
 * kind shown by metadata so the backend can verify the exact object and avoid
 * dropping a different constraint with the same name.
 */
export function dropDatabaseConstraint(
  profileId: string,
  input: { table: string; name: string; constraintKind: DatabaseConstraintKind } & TableScope,
  confirmed: boolean,
): Promise<DatabaseStructureActionResult> {
  return runDatabaseStructureAction(profileId, { kind: "drop_constraint", ...input }, confirmed);
}
