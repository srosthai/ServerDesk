import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseObjectPanel } from "@/features/databases/DatabaseObjectPanel";
import { objectCapabilitiesFor } from "@/features/databases/databaseObjectModel";
import { inspectDatabaseObject } from "@/services/databaseObjects";
import type { CapabilityFlags, DatabaseObjectDefinition, DatabaseProfile } from "@/types/contracts";

vi.mock("@/services/databaseObjects", () => ({
  inspectDatabaseObject: vi.fn(),
  previewDatabaseObjectAction: vi.fn(),
  runDatabaseObjectAction: vi.fn(),
}));

const flags: CapabilityFlags = {
  schemas: true,
  sequences: true,
  materializedViews: true,
  routines: true,
  routineEdit: true,
  triggers: true,
  triggerEdit: true,
  viewManagement: true,
  roles: true,
  sessions: true,
  transactions: true,
  savepoints: true,
  cancelQuery: true,
  explain: true,
  checkConstraints: true,
  generatedColumns: true,
  identityColumns: true,
  autoIncrementColumns: false,
  columnTypeChange: true,
  columnRename: true,
  tableRename: true,
  tableDuplicate: true,
  columnReorder: false,
  databaseRename: true,
  indexRename: true,
  partialIndexes: true,
  queryTimeout: true,
};

const profile: DatabaseProfile = {
  id: "profile-1",
  serverId: "server-1",
  name: "Primary",
  engine: "postgresql",
  host: "127.0.0.1",
  port: 5432,
  socket: null,
  databaseName: "app",
  username: "app",
  hasCredential: true,
  readOnly: false,
  sslMode: "required",
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: 1,
};

const definition: DatabaseObjectDefinition = {
  kind: "trigger",
  database: "app",
  schema: "public",
  name: "users_audit",
  owner: { status: "available", value: "app" },
  language: { status: "unavailable" },
  routineKind: { status: "unavailable" },
  arguments: { status: "unavailable" },
  returnType: { status: "unavailable" },
  viewQuery: { status: "unavailable" },
  routineBody: { status: "unavailable" },
  definition: {
    status: "available",
    value:
      "CREATE FUNCTION public.audit_users() RETURNS trigger AS $$ BEGIN RETURN NEW; END $$ LANGUAGE plpgsql",
  },
  trigger: {
    table: "users",
    timing: "after",
    events: ["insert"],
    statement: { status: "available", value: "CREATE TRIGGER users_audit AFTER INSERT ON users" },
    functionName: { status: "available", value: "audit_users" },
    function: { status: "available", value: "CREATE FUNCTION public.audit_users()" },
  },
  sequence: null,
  truncated: false,
};

describe("DatabaseObjectPanel", () => {
  beforeEach(() => {
    vi.mocked(inspectDatabaseObject).mockReset();
    vi.mocked(inspectDatabaseObject).mockResolvedValue(definition);
  });

  it("shows trigger metadata and a bounded definition without rendering HTML", async () => {
    const capabilities = objectCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: false,
      flags,
    });
    render(
      <DatabaseObjectPanel
        profile={profile}
        profileName="Primary"
        connected
        object={{ kind: "trigger", database: "app", schema: "public", name: "users_audit" }}
        capabilities={capabilities}
        onIntent={vi.fn()}
        onRefreshExplorer={vi.fn()}
      />,
    );

    expect(await screen.findByText("Trigger users_audit")).toBeInTheDocument();
    expect(screen.getByText("users")).toBeInTheDocument();
    expect(screen.getByText("After")).toBeInTheDocument();
    expect(screen.getByText("Insert")).toBeInTheDocument();
    expect(screen.getByText("Trigger statement")).toBeInTheDocument();
    expect(screen.getAllByText(/CREATE FUNCTION public.audit_users/).length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "Edit" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Drop" })).toBeEnabled();
  });

  it("disables actions with an explanation for a read-only profile", async () => {
    const capabilities = objectCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: true,
      flags,
    });
    render(
      <DatabaseObjectPanel
        profile={profile}
        profileName="Primary"
        connected
        object={{ kind: "trigger", database: "app", schema: "public", name: "users_audit" }}
        capabilities={capabilities}
        onIntent={vi.fn()}
        onRefreshExplorer={vi.fn()}
      />,
    );

    const edit = await screen.findByRole("button", { name: "Edit" });
    expect(edit).toBeDisabled();
    expect(edit).toHaveAttribute("title", expect.stringContaining("read-only"));
  });
});
