import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseAccessPanel } from "@/features/databases/DatabaseAccessPanel";
import { roleCapabilitiesFor } from "@/features/databases/databaseRolesModel";
import {
  getDatabaseRolePrivileges,
  listDatabaseRoles,
  previewDatabaseRoleAction,
  runDatabaseRoleAction,
} from "@/services/databaseRoles";
import { useDatabaseRoleStore } from "@/stores/databaseRoleStore";
import type {
  CapabilityFlags,
  DatabasePage,
  DatabaseRoleActionResult,
  DatabaseRoleDdlPreview,
  DatabaseRolePrivilegeSnapshot,
  DatabaseRoleSummary,
} from "@/types/contracts";

vi.mock("@/services/databaseRoles", () => ({
  listDatabaseRoles: vi.fn(),
  getDatabaseRolePrivileges: vi.fn(),
  previewDatabaseRoleAction: vi.fn(),
  runDatabaseRoleAction: vi.fn(),
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

const reporting: DatabaseRoleSummary = {
  name: "reporting",
  host: null,
  kind: "user",
  canLogin: { status: "available", value: true },
  superuser: { status: "available", value: false },
  createDatabase: { status: "available", value: false },
  createRole: { status: "available", value: false },
  inherit: { status: "available", value: true },
};

const reader: DatabaseRoleSummary = {
  name: "reporting_reader",
  host: null,
  kind: "role",
  canLogin: { status: "available", value: false },
  superuser: { status: "available", value: false },
  createDatabase: { status: "available", value: false },
  createRole: { status: "available", value: false },
  inherit: { status: "available", value: true },
};

const rolesPage: DatabasePage<DatabaseRoleSummary> = {
  items: [reporting, reader],
  limit: 100,
  offset: 0,
  hasMore: false,
  total: 2,
};

const snapshot: DatabaseRolePrivilegeSnapshot = {
  role: { name: "reporting", host: null },
  direct: [
    {
      privilege: "SELECT",
      target: { kind: "table", schema: "public", object: "orders" },
      grantable: false,
    },
  ],
  memberOf: [{ name: "reporting_reader", host: null }],
  effective: [
    {
      privilege: "SELECT",
      target: { kind: "table", schema: "public", object: "orders" },
      grantable: false,
      sources: ["direct"],
    },
    {
      privilege: "SELECT",
      target: { kind: "table", schema: "public", object: "users" },
      grantable: false,
      sources: ["reporting_reader"],
    },
  ],
};

const preview: DatabaseRoleDdlPreview = {
  action: "grant_privileges",
  engine: "postgresql",
  role: { name: "reporting", host: null },
  target: { kind: "table", schema: "public", object: "orders" },
  statements: ['GRANT SELECT ON TABLE "public"."orders" TO "reporting"'],
  ddl: 'GRANT SELECT ON TABLE "public"."orders" TO "reporting"',
  warnings: ["PostgreSQL applies granted privileges according to INHERIT."],
  destructive: false,
  requiresConfirmation: true,
  privilegeDiff: {
    before: [],
    after: [
      {
        privilege: "SELECT",
        target: { kind: "table", schema: "public", object: "orders" },
        grantable: false,
        sources: ["direct"],
      },
    ],
    added: [
      {
        privilege: "SELECT",
        target: { kind: "table", schema: "public", object: "orders" },
        grantable: false,
        sources: ["direct"],
      },
    ],
    removed: [],
  },
  roleChanges: [],
};

const result: DatabaseRoleActionResult = {
  action: "grant_privileges",
  engine: "postgresql",
  role: { name: "reporting", host: null },
  target: { kind: "table", schema: "public", object: "orders" },
  statements: preview.statements,
  warnings: preview.warnings,
  verified: true,
  verification: "Privilege metadata re-read confirms the requested effective privileges.",
  privilegeDiff: preview.privilegeDiff,
  roleChanges: [],
  durationMs: 4,
};

function capabilities(overrides: { readOnly?: boolean; roles?: boolean } = {}) {
  return roleCapabilitiesFor({
    engine: "postgresql",
    version: "16.2.0",
    connected: true,
    readOnly: overrides.readOnly ?? false,
    flags: { ...flags, roles: overrides.roles ?? true },
  });
}

function renderPanel(overrides: { readOnly?: boolean; roles?: boolean } = {}) {
  return render(
    <DatabaseAccessPanel
      profileId="profile-1"
      profileName="Primary"
      engine="postgresql"
      connected
      readOnly={overrides.readOnly ?? false}
      capabilities={capabilities(overrides)}
      scope={{ database: "app", schema: "public", table: "orders" }}
    />,
  );
}

describe("DatabaseAccessPanel", () => {
  beforeEach(() => {
    useDatabaseRoleStore.setState({ entries: {} });
    vi.mocked(listDatabaseRoles).mockReset();
    vi.mocked(listDatabaseRoles).mockResolvedValue(rolesPage);
    vi.mocked(getDatabaseRolePrivileges).mockReset();
    vi.mocked(getDatabaseRolePrivileges).mockResolvedValue(snapshot);
    vi.mocked(previewDatabaseRoleAction).mockReset();
    vi.mocked(previewDatabaseRoleAction).mockResolvedValue(preview);
    vi.mocked(runDatabaseRoleAction).mockReset();
    vi.mocked(runDatabaseRoleAction).mockResolvedValue(result);
  });

  it("lists users and roles with direct and effective privileges", async () => {
    renderPanel();

    expect((await screen.findAllByText("reporting")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("reporting_reader").length).toBeGreaterThan(0);
    expect(screen.getByText("Member of")).toBeInTheDocument();
    expect(screen.getByText("Direct grants")).toBeInTheDocument();
    expect(screen.getByText("Effective privileges")).toBeInTheDocument();
    expect(screen.getAllByText("public.orders").length).toBeGreaterThan(0);
    expect(screen.getByText("public.users")).toBeInTheDocument();
    expect(screen.getAllByText("reporting_reader").length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toMatch(
      /authentication_string|rolpassword|IDENTIFIED BY/i,
    );
  });

  it("disables every mutation with a reason for a read-only profile", async () => {
    renderPanel({ readOnly: true });

    expect((await screen.findAllByText("reporting")).length).toBeGreaterThan(0);
    expect(screen.getByText("Read-only access")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New user or role" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Modify" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Grant" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Revoke" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Modify" })).toHaveAttribute(
      "title",
      expect.stringContaining("read-only"),
    );
  });

  it("shows a capability explanation without querying roles", async () => {
    renderPanel({ roles: false });

    expect(await screen.findByText("Users and roles are unavailable")).toBeInTheDocument();
    expect(screen.getByText(/not supported by PostgreSQL/)).toBeInTheDocument();
    expect(listDatabaseRoles).not.toHaveBeenCalled();
  });

  it("previews exact grant SQL and only executes after confirmation", async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole("button", { name: "Grant" }));
    await user.click(screen.getByRole("button", { name: "Preview exact changes" }));

    expect(
      await screen.findByText('GRANT SELECT ON TABLE "public"."orders" TO "reporting"'),
    ).toBeInTheDocument();
    expect(screen.getByText("Added effective privileges")).toBeInTheDocument();
    expect(runDatabaseRoleAction).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Confirm and apply" }));
    await user.click(screen.getByRole("button", { name: "Apply privilege change" }));

    await waitFor(() => {
      expect(runDatabaseRoleAction).toHaveBeenCalledWith(
        "profile-1",
        expect.objectContaining({ kind: "grant_privileges" }),
        true,
      );
    });
    expect(await screen.findByText("Privilege change verified")).toBeInTheDocument();
  });

  it("previews create role statements and never renders secrets", async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole("button", { name: "New user or role" }));
    await user.type(screen.getByLabelText("Name"), "deploy");
    await user.click(screen.getByRole("button", { name: "Preview exact changes" }));

    expect(
      await screen.findByText('GRANT SELECT ON TABLE "public"."orders" TO "reporting"'),
    ).toBeInTheDocument();
    expect(screen.getByText(/No password is set/)).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/hunter2|authentication_string/i);

    await user.click(screen.getByRole("button", { name: "Confirm and apply" }));
    await user.click(screen.getByRole("button", { name: "Apply privilege change" }));
    await waitFor(() => expect(runDatabaseRoleAction).toHaveBeenCalled());
  });

  it("reports a permission denial clearly", async () => {
    vi.mocked(getDatabaseRolePrivileges).mockRejectedValue({
      code: "permission",
      message: "The database account does not have permission for this operation.",
    });
    renderPanel();

    expect(await screen.findByText("Could not inspect privileges")).toBeInTheDocument();
    expect(
      screen.getByText("The database account does not have permission for this operation."),
    ).toBeInTheDocument();
  });
});
