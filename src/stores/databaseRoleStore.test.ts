import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDatabaseRoleStore } from "@/stores/databaseRoleStore";
import { getDatabaseRolePrivileges, listDatabaseRoles } from "@/services/databaseRoles";
import type { DatabaseRolePrivilegeSnapshot, DatabaseRoleSummary } from "@/types/contracts";

vi.mock("@/services/databaseRoles", () => ({
  listDatabaseRoles: vi.fn(),
  getDatabaseRolePrivileges: vi.fn(),
  previewDatabaseRoleAction: vi.fn(),
  runDatabaseRoleAction: vi.fn(),
}));

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

const snapshot: DatabaseRolePrivilegeSnapshot = {
  role: { name: "reporting", host: null },
  direct: [],
  memberOf: [],
  effective: [],
};

describe("databaseRoleStore", () => {
  beforeEach(() => {
    useDatabaseRoleStore.setState({ entries: {} });
    vi.mocked(listDatabaseRoles).mockReset();
    vi.mocked(getDatabaseRolePrivileges).mockReset();
  });

  it("loads roles, selects the first account, and loads its privileges", async () => {
    vi.mocked(listDatabaseRoles).mockResolvedValue({
      items: [reporting],
      limit: 100,
      offset: 0,
      hasMore: false,
      total: 1,
    });
    vi.mocked(getDatabaseRolePrivileges).mockResolvedValue(snapshot);

    await useDatabaseRoleStore.getState().fetchRoles("profile-1");
    expect(useDatabaseRoleStore.getState().entries["profile-1"]).toMatchObject({
      status: "ready",
      selectedKey: "reporting",
      roles: [reporting],
    });

    await useDatabaseRoleStore
      .getState()
      .fetchSnapshot("profile-1", { name: "reporting", host: null });
    expect(useDatabaseRoleStore.getState().entries["profile-1"]?.snapshot).toEqual(snapshot);
  });

  it("keeps a permission error typed and does not retain stale roles", async () => {
    vi.mocked(listDatabaseRoles).mockRejectedValue({
      code: "permission",
      message: "The database account does not have permission for this operation.",
    });

    await useDatabaseRoleStore.getState().fetchRoles("profile-1");
    expect(useDatabaseRoleStore.getState().entries["profile-1"]).toMatchObject({
      status: "error",
      roles: [],
      error: "The database account does not have permission for this operation.",
    });
  });

  it("resets one profile without clearing other connections", async () => {
    vi.mocked(listDatabaseRoles).mockResolvedValue({
      items: [reporting],
      limit: 100,
      offset: 0,
      hasMore: false,
      total: 1,
    });
    await useDatabaseRoleStore.getState().fetchRoles("profile-1");
    await useDatabaseRoleStore.getState().fetchRoles("profile-2");

    useDatabaseRoleStore.getState().reset("profile-1");
    expect(useDatabaseRoleStore.getState().entries["profile-1"]).toBeUndefined();
    expect(useDatabaseRoleStore.getState().entries["profile-2"]).toBeDefined();
  });
});
