import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import {
  databaseStructureCommands,
  previewDatabaseStructureAction,
} from "@/services/databaseStructure";
import type {
  DatabaseStructureActionRequest,
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

describe("database structure service", () => {
  it("invokes the preview command with the typed request", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(preview);

    await expect(previewDatabaseStructureAction("profile-1", request)).resolves.toBe(preview);
    expect(invoke).toHaveBeenCalledWith(databaseStructureCommands.preview, {
      profileId: "profile-1",
      request,
    });
  });

  it("keeps the preview command preview-only", () => {
    expect(databaseStructureCommands).toEqual({
      preview: "preview_database_structure_action",
    });
  });
});
