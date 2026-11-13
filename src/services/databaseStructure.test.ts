import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  addDatabaseColumn,
  addDatabaseConstraint,
  alterDatabaseColumn,
  createDatabaseIndex,
  createDatabaseTable,
  databaseStructureCommands,
  dropDatabaseColumn,
  dropDatabaseConstraint,
  dropDatabaseIndex,
  dropDatabaseTable,
  duplicateDatabaseTable,
  previewDatabaseStructureAction,
  renameDatabaseColumn,
  renameDatabaseIndex,
  renameDatabaseTable,
  reorderDatabaseColumn,
  runDatabaseStructureAction,
  truncateDatabaseTable,
} from "@/services/databaseStructure";
import type {
  DatabaseStructureActionRequest,
  DatabaseStructureActionResult,
  DatabaseStructureDdlPreview,
} from "@/types/contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const request: DatabaseStructureActionRequest = {
  kind: "add_column",
  database: "app",
  schema: null,
  table: "Users",
  column: {
    name: "Display Name",
    dataType: "varchar(120)",
    nullable: false,
    default: { kind: "string", value: "n/a" },
  },
  position: { kind: "after", column: "Email" },
};

const preview: DatabaseStructureDdlPreview = {
  action: "add_column",
  engine: "mysql",
  object: {
    kind: "column",
    database: "app",
    table: "Users",
    name: "Display Name",
  },
  statements: [
    "ALTER TABLE `app`.`Users` ADD COLUMN `Display Name` varchar(120) NOT NULL DEFAULT 'n/a' AFTER `Email`",
  ],
  ddl: "ALTER TABLE `app`.`Users` ADD COLUMN `Display Name` varchar(120) NOT NULL DEFAULT 'n/a' AFTER `Email`",
  warnings: [],
  destructive: false,
  requiresConfirmation: false,
};

const columns = [{ name: "id", dataType: "bigint", nullable: false }];

const result: DatabaseStructureActionResult = {
  action: "create_table",
  engine: "postgresql",
  object: { kind: "table", database: "app", schema: "public", table: "users" },
  statements: ['CREATE TABLE "public"."users" (\n  "id" bigint NOT NULL\n)'],
  warnings: [],
  verified: true,
  verification: "Metadata re-read confirms table users exists with the expected 1 column(s).",
  durationMs: 4,
};

describe("database structure service", () => {
  it("invokes the preview command with the typed request", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(preview);

    await expect(previewDatabaseStructureAction("profile-1", request)).resolves.toBe(preview);
    expect(invoke).toHaveBeenCalledWith(databaseStructureCommands.preview, {
      profileId: "profile-1",
      request,
    });
  });

  it("keeps the preview command separate from execution", () => {
    expect(databaseStructureCommands).toEqual({
      preview: "preview_database_structure_action",
      run: "run_database_structure_action",
    });
  });

  it("exposes run as a typed invocation", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(result);

    await expect(
      runDatabaseStructureAction("profile-1", {
        kind: "create_table",
        database: "app",
        schema: "public",
        name: "users",
        columns,
      }),
    ).resolves.toEqual(result);

    expect(invoke).toHaveBeenCalledWith(databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "create_table",
        database: "app",
        schema: "public",
        name: "users",
        columns,
      },
      confirmed: false,
    });
  });

  it("builds engine-aware create requests with keys and options", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(result);

    await createDatabaseTable("profile-1", {
      database: "app",
      schema: "public",
      name: "users",
      columns,
      primaryKey: ["id"],
      uniqueKeys: [{ name: "users_email_key", columns: ["email"] }],
      options: { unlogged: true, tablespace: "fast" },
    });

    expect(invoke).toHaveBeenCalledWith(databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "create_table",
        database: "app",
        schema: "public",
        name: "users",
        columns,
        primaryKey: ["id"],
        uniqueKeys: [{ name: "users_email_key", columns: ["email"] }],
        options: { unlogged: true, tablespace: "fast" },
      },
      confirmed: false,
    });
  });

  it("sends rename and duplicate requests without a destructive confirmation", async () => {
    vi.mocked(invoke).mockResolvedValue(result);

    await renameDatabaseTable("profile-1", {
      database: "app",
      schema: "public",
      name: "users",
      newName: "accounts",
    });
    await duplicateDatabaseTable("profile-1", {
      database: "app",
      schema: "public",
      name: "users",
      newName: "users_copy",
    });

    expect(invoke).toHaveBeenNthCalledWith(1, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "rename_table",
        database: "app",
        schema: "public",
        name: "users",
        newName: "accounts",
      },
      confirmed: false,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "duplicate_table",
        database: "app",
        schema: "public",
        name: "users",
        newName: "users_copy",
      },
      confirmed: false,
    });
  });

  it("requires an explicit confirmation value for truncate and drop", async () => {
    vi.mocked(invoke).mockResolvedValue(result);

    await truncateDatabaseTable("profile-1", { database: "app", name: "users" }, true);
    await dropDatabaseTable("profile-1", { database: "app", name: "users" }, false);

    expect(invoke).toHaveBeenNthCalledWith(1, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: { kind: "truncate_table", database: "app", name: "users" },
      confirmed: true,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: { kind: "drop_table", database: "app", name: "users" },
      confirmed: false,
    });
  });

  it("builds typed column add, edit, rename, drop, and reorder requests", async () => {
    vi.mocked(invoke).mockResolvedValue(result);

    await addDatabaseColumn("profile-1", {
      database: "app",
      schema: null,
      table: "Users",
      column: {
        name: "Display Name",
        dataType: "varchar(120)",
        nullable: false,
        default: { kind: "string", value: "n/a" },
        comment: "Shown in lists",
      },
      position: { kind: "after", column: "Email" },
    });
    await alterDatabaseColumn(
      "profile-1",
      {
        database: "app",
        schema: null,
        table: "Users",
        name: "Email",
        dataType: "varchar(500)",
        nullable: false,
        comment: "Primary address",
      },
      true,
    );
    await renameDatabaseColumn("profile-1", {
      database: "app",
      schema: null,
      table: "Users",
      name: "Display Name",
      newName: "Nickname",
    });
    await dropDatabaseColumn(
      "profile-1",
      { database: "app", schema: null, table: "Users", name: "Nickname" },
      true,
    );
    await reorderDatabaseColumn("profile-1", {
      database: "app",
      schema: null,
      table: "Users",
      column: { name: "id", dataType: "bigint", nullable: false },
      position: { kind: "first" },
    });

    expect(invoke).toHaveBeenNthCalledWith(1, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "add_column",
        database: "app",
        schema: null,
        table: "Users",
        column: {
          name: "Display Name",
          dataType: "varchar(120)",
          nullable: false,
          default: { kind: "string", value: "n/a" },
          comment: "Shown in lists",
        },
        position: { kind: "after", column: "Email" },
      },
      confirmed: false,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "alter_column",
        database: "app",
        schema: null,
        table: "Users",
        name: "Email",
        dataType: "varchar(500)",
        nullable: false,
        comment: "Primary address",
      },
      confirmed: true,
    });
    expect(invoke).toHaveBeenNthCalledWith(3, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "rename_column",
        database: "app",
        schema: null,
        table: "Users",
        name: "Display Name",
        newName: "Nickname",
      },
      confirmed: false,
    });
    expect(invoke).toHaveBeenNthCalledWith(4, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "drop_column",
        database: "app",
        schema: null,
        table: "Users",
        name: "Nickname",
      },
      confirmed: true,
    });
    expect(invoke).toHaveBeenNthCalledWith(5, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "reorder_column",
        database: "app",
        schema: null,
        table: "Users",
        column: { name: "id", dataType: "bigint", nullable: false },
        position: { kind: "first" },
      },
      confirmed: false,
    });
  });

  it("builds typed index and constraint requests with confirmation where required", async () => {
    vi.mocked(invoke).mockResolvedValue(result);

    await createDatabaseIndex("profile-1", {
      database: "app",
      schema: null,
      table: "Users",
      name: "users_email_idx",
      columns: ["email"],
      unique: true,
    });
    await renameDatabaseIndex("profile-1", {
      database: "app",
      schema: null,
      table: "Users",
      name: "users_email_idx",
      newName: "users_email_lookup_idx",
    });
    await dropDatabaseIndex(
      "profile-1",
      { database: "app", schema: null, table: "Users", name: "users_email_idx" },
      true,
    );
    await addDatabaseConstraint("profile-1", {
      database: "app",
      schema: null,
      table: "Orders",
      constraint: {
        kind: "foreign_key",
        name: "orders_user_fk",
        columns: ["user_id"],
        referencedTable: "Users",
        referencedColumns: ["id"],
        onDelete: "cascade",
      },
    });
    await dropDatabaseConstraint(
      "profile-1",
      {
        database: "app",
        schema: null,
        table: "Orders",
        name: "orders_user_fk",
        constraintKind: "foreign_key",
      },
      false,
    );

    expect(invoke).toHaveBeenNthCalledWith(1, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "create_index",
        database: "app",
        schema: null,
        table: "Users",
        name: "users_email_idx",
        columns: ["email"],
        unique: true,
      },
      confirmed: false,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "rename_index",
        database: "app",
        schema: null,
        table: "Users",
        name: "users_email_idx",
        newName: "users_email_lookup_idx",
      },
      confirmed: false,
    });
    expect(invoke).toHaveBeenNthCalledWith(3, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "drop_index",
        database: "app",
        schema: null,
        table: "Users",
        name: "users_email_idx",
      },
      confirmed: true,
    });
    expect(invoke).toHaveBeenNthCalledWith(4, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "add_constraint",
        database: "app",
        schema: null,
        table: "Orders",
        constraint: {
          kind: "foreign_key",
          name: "orders_user_fk",
          columns: ["user_id"],
          referencedTable: "Users",
          referencedColumns: ["id"],
          onDelete: "cascade",
        },
      },
      confirmed: false,
    });
    expect(invoke).toHaveBeenNthCalledWith(5, databaseStructureCommands.run, {
      profileId: "profile-1",
      request: {
        kind: "drop_constraint",
        database: "app",
        schema: null,
        table: "Orders",
        name: "orders_user_fk",
        constraintKind: "foreign_key",
      },
      confirmed: false,
    });
  });
});
