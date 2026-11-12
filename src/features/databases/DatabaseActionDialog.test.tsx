import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseActionDialog } from "@/features/databases/DatabaseActionDialog";
import { previewDatabaseAction, createDatabase } from "@/services/databaseActions";
import type { DatabaseProfile } from "@/types/contracts";

vi.mock("@/services/databaseActions", () => ({
  previewDatabaseAction: vi.fn(),
  createDatabase: vi.fn(),
  renameDatabase: vi.fn(),
  dropDatabase: vi.fn(),
  setActiveDatabase: vi.fn(),
  runDatabaseAction: vi.fn(),
}));

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

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

const preview = {
  action: "create" as const,
  engine: "postgresql" as const,
  database: "app_next",
  targetDatabase: null,
  statements: [
    'CREATE DATABASE "app_next" WITH OWNER = "app" TEMPLATE = "template0" ENCODING = \'UTF8\'',
  ],
  ddl: 'CREATE DATABASE "app_next" WITH OWNER = "app" TEMPLATE = "template0" ENCODING = \'UTF8\'',
  warnings: ["PostgreSQL records encoding and locale at creation time."],
  destructive: false,
  requiresConfirmation: false,
};

const result = {
  action: "create" as const,
  engine: "postgresql" as const,
  database: "app_next",
  previousName: null,
  statements: preview.statements,
  verified: true,
  verification: "Metadata re-read confirms database app_next exists (encoding UTF8).",
  verificationError: null,
  metadata: null,
  activeDatabase: null,
  durationMs: 5,
};

describe("DatabaseActionDialog", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.mocked(previewDatabaseAction).mockReset();
    vi.mocked(previewDatabaseAction).mockResolvedValue(preview);
    vi.mocked(createDatabase).mockReset();
    vi.mocked(createDatabase).mockResolvedValue(result);
  });

  it("requires a DDL preview before creating the database", async () => {
    const user = userEvent.setup();
    const onCompleted = vi.fn();
    render(
      <DatabaseActionDialog
        open
        mode="create"
        profile={profile}
        onOpenChange={vi.fn()}
        onCompleted={onCompleted}
      />,
    );

    const apply = screen.getByRole("button", { name: "Create database" });
    expect(apply).toBeDisabled();

    await user.type(screen.getByLabelText("Database name"), "app_next");
    await user.type(screen.getByLabelText("Encoding"), "UTF8");
    await user.click(screen.getByRole("button", { name: "Preview DDL" }));

    expect(await screen.findByText(preview.ddl)).toBeInTheDocument();
    expect(previewDatabaseAction).toHaveBeenCalledWith("profile-1", {
      kind: "create",
      name: "app_next",
      encoding: "UTF8",
      collation: null,
      owner: null,
      template: null,
    });
    expect(apply).toBeEnabled();

    await user.click(apply);

    await waitFor(() => {
      expect(createDatabase).toHaveBeenCalledWith("profile-1", {
        name: "app_next",
        encoding: "UTF8",
        collation: null,
        owner: null,
        template: null,
      });
    });
    expect(onCompleted).toHaveBeenCalledWith(result);
  });

  it("clears the preview when the form changes so stale DDL cannot be applied", async () => {
    const user = userEvent.setup();
    render(
      <DatabaseActionDialog
        open
        mode="create"
        profile={profile}
        onOpenChange={vi.fn()}
        onCompleted={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Database name"), "app_next");
    await user.click(screen.getByRole("button", { name: "Preview DDL" }));
    expect(await screen.findByText(preview.ddl)).toBeInTheDocument();

    await user.type(screen.getByLabelText("Database name"), "_changed");
    expect(screen.queryByText(preview.ddl)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create database" })).toBeDisabled();
  });

  it("shows typed permission denial from the database account", async () => {
    const user = userEvent.setup();
    vi.mocked(createDatabase).mockRejectedValue({
      code: "permission",
      message: "The database account does not have permission for this operation.",
    });
    render(
      <DatabaseActionDialog
        open
        mode="create"
        profile={profile}
        onOpenChange={vi.fn()}
        onCompleted={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Database name"), "app_next");
    await user.click(screen.getByRole("button", { name: "Preview DDL" }));
    await screen.findByText(preview.ddl);
    await user.click(screen.getByRole("button", { name: "Create database" }));

    expect(
      await screen.findByText("The database account does not have permission for this operation."),
    ).toBeInTheDocument();
  });
});
