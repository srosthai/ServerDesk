import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  createDatabaseTable,
  databaseStructureCommands,
  dropDatabaseTable,
  duplicateDatabaseTable,
  previewDatabaseStructureAction,
  renameDatabaseTable,
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
});
