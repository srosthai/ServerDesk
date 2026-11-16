import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DatabaseObjectActionHost,
  type DatabaseObjectIntent,
} from "@/features/databases/DatabaseObjectDialogs";
import { previewDatabaseObjectAction, runDatabaseObjectAction } from "@/services/databaseObjects";
import type {
  DatabaseObjectActionResult,
  DatabaseObjectDefinition,
  DatabaseObjectDdlPreview,
} from "@/types/contracts";

vi.mock("@/services/databaseObjects", () => ({
  inspectDatabaseObject: vi.fn(),
  previewDatabaseObjectAction: vi.fn(),
  runDatabaseObjectAction: vi.fn(),
}));

const definition: DatabaseObjectDefinition = {
  kind: "view",
  database: "app",
  schema: "public",
  name: "active_users",
  owner: { status: "available", value: "app" },
  language: { status: "unavailable" },
  routineKind: { status: "unavailable" },
  arguments: { status: "unavailable" },
  returnType: { status: "unavailable" },
  viewQuery: { status: "available", value: "SELECT id FROM users" },
  routineBody: { status: "unavailable" },
  definition: {
    status: "available",
    value: "CREATE VIEW public.active_users AS SELECT id FROM users",
  },
  trigger: null,
  sequence: null,
  truncated: false,
};

const preview: DatabaseObjectDdlPreview = {
  action: "create_view",
  engine: "postgresql",
  object: { kind: "view", database: "app", schema: "public", name: "active_users" },
  statements: ['CREATE VIEW "public"."active_users" AS\nSELECT id FROM users'],
  ddl: 'CREATE VIEW "public"."active_users" AS\nSELECT id FROM users',
  warnings: [],
  destructive: false,
  requiresConfirmation: false,
};

const dropPreview: DatabaseObjectDdlPreview = {
  ...preview,
  action: "drop_view",
  statements: ['DROP VIEW "public"."active_users"'],
  ddl: 'DROP VIEW "public"."active_users"',
  destructive: true,
  requiresConfirmation: true,
};

const result: DatabaseObjectActionResult = {
  action: "create_view",
  engine: "postgresql",
  object: { kind: "view", database: "app", schema: "public", name: "active_users" },
  statements: preview.statements,
  warnings: [],
  verified: true,
  verification: "Metadata re-read confirms the view matches the requested change.",
  verificationError: null,
  definition,
  durationMs: 4,
};

function renderHost(intent: DatabaseObjectIntent, readOnly = false) {
  const onCompleted = vi.fn();
  const onClose = vi.fn();
  render(
    <DatabaseObjectActionHost
      profileId="profile-1"
      profileName="Primary"
      engine="postgresql"
      connected
      readOnly={readOnly}
      intent={intent}
      onClose={onClose}
      onCompleted={onCompleted}
    />,
  );
  return { onCompleted, onClose };
}

describe("DatabaseObjectActionHost", () => {
  beforeEach(() => {
    vi.mocked(previewDatabaseObjectAction).mockReset();
    vi.mocked(previewDatabaseObjectAction).mockResolvedValue(preview);
    vi.mocked(runDatabaseObjectAction).mockReset();
    vi.mocked(runDatabaseObjectAction).mockResolvedValue(result);
  });

  it("previews create-view DDL before executing it", async () => {
    const user = userEvent.setup();
    const { onCompleted } = renderHost({
      type: "create",
      kind: "view",
      database: "app",
      schema: "public",
    });

    await user.type(screen.getByLabelText("Name"), "active_users");
    await user.clear(screen.getByLabelText("SELECT query"));
    await user.type(screen.getByLabelText("SELECT query"), "SELECT id FROM users");
    await user.click(screen.getByRole("button", { name: "Preview DDL" }));

    expect(
      await screen.findByText((content) => content.includes("CREATE VIEW")),
    ).toBeInTheDocument();
    expect(previewDatabaseObjectAction).toHaveBeenCalledWith("profile-1", {
      kind: "create_view",
      name: "active_users",
      query: "SELECT id FROM users",
      orReplace: false,
      database: "app",
      schema: "public",
    });

    await user.click(screen.getByRole("button", { name: "Run DDL" }));
    await waitFor(() => expect(runDatabaseObjectAction).toHaveBeenCalled());
    expect(runDatabaseObjectAction).toHaveBeenCalledWith(
      "profile-1",
      expect.objectContaining({ kind: "create_view", name: "active_users" }),
      false,
    );
    expect(onCompleted).toHaveBeenCalledWith(result);
  });

  it("requires a second confirmation for a drop", async () => {
    const user = userEvent.setup();
    vi.mocked(previewDatabaseObjectAction).mockResolvedValue(dropPreview);
    renderHost({ type: "drop", definition });

    expect(await screen.findByText((content) => content.includes("DROP VIEW"))).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Run drop" }));

    expect(screen.getByText("Confirm drop view")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Drop object" }));

    await waitFor(() =>
      expect(runDatabaseObjectAction).toHaveBeenCalledWith(
        "profile-1",
        expect.objectContaining({ kind: "drop_view", name: "active_users" }),
        true,
      ),
    );
  });

  it("disables execution for a read-only profile", async () => {
    renderHost({ type: "create", kind: "view", database: "app", schema: "public" }, true);

    expect(screen.getByText("Action unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Preview DDL" })).toBeDisabled();
  });

  it("shows a typed error when the preview is rejected", async () => {
    const user = userEvent.setup();
    vi.mocked(previewDatabaseObjectAction).mockRejectedValue({
      code: "capability",
      message: "create_sequence is not supported by MySQL 8.0.36.",
    });
    renderHost({
      type: "create",
      kind: "sequence",
      database: "app",
      schema: null,
    });

    await user.type(screen.getByLabelText("Name"), "orders_id_seq");
    await user.click(screen.getByRole("button", { name: "Preview DDL" }));

    expect(await screen.findByText("Object action failed")).toBeInTheDocument();
    expect(
      screen.getByText("create_sequence is not supported by MySQL 8.0.36."),
    ).toBeInTheDocument();
  });
});
