import { beforeEach, describe, expect, it } from "vitest";
import { dataGridTableKey } from "@/features/databases/databaseDataGridModel";
import {
  nextActiveTabId,
  useDatabaseWorkspaceStore,
  type DatabaseWorkspaceQueryResult,
} from "@/stores/databaseWorkspaceStore";
import type { DatabaseExplorerSelection } from "@/features/databases/databaseExplorerTreeModel";
import type { DatabaseProfile } from "@/types/contracts";

const primary: DatabaseProfile = {
  id: "profile-primary",
  serverId: "server-1",
  name: "Primary",
  engine: "postgresql",
  host: "127.0.0.1",
  port: 5432,
  socket: null,
  databaseName: "app",
  username: "app",
  hasCredential: true,
  readOnly: true,
  sslMode: "required",
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: null,
};

const analytics: DatabaseProfile = {
  ...primary,
  id: "profile-analytics",
  name: "Analytics",
  engine: "mysql",
  port: 3306,
  databaseName: "metrics",
  readOnly: false,
};

const appSelection: DatabaseExplorerSelection = {
  id: "connection/profile-primary/database/app",
  profileId: primary.id,
  kind: "database",
  label: "app",
  engine: "postgresql",
  database: "app",
  schema: null,
  objectName: null,
};

const metricsSelection: DatabaseExplorerSelection = {
  id: "connection/profile-analytics/database/metrics",
  profileId: analytics.id,
  kind: "database",
  label: "metrics",
  engine: "mysql",
  database: "metrics",
  schema: null,
  objectName: null,
};

const result: DatabaseWorkspaceQueryResult = {
  statement: "select 1",
  columns: ["?column?"],
  rows: [["1"]],
  affectedRows: null,
  durationMs: 3,
  error: null,
  executedAt: 10,
};

describe("database workspace store", () => {
  beforeEach(() => {
    useDatabaseWorkspaceStore.setState({
      tabs: {},
      order: [],
      activeTabId: null,
    });
  });

  it("creates a workspace tab with overview and selects it", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);

    const state = useDatabaseWorkspaceStore.getState();
    expect(state.order).toEqual([primary.id]);
    expect(state.activeTabId).toBe(primary.id);
    expect(state.tabs[primary.id]).toMatchObject({
      profileId: primary.id,
      serverId: primary.serverId,
      name: "Primary",
      engine: "postgresql",
      activeView: "overview",
      selection: null,
      sqlDraft: "",
      sqlResult: null,
      tasks: [],
    });
  });

  it("keeps two connections open and scopes every workspace field per tab", () => {
    const workspace = useDatabaseWorkspaceStore.getState();
    workspace.openTab(primary);
    workspace.openTab(analytics);

    useDatabaseWorkspaceStore.getState().setActiveView(primary.id, "sql");
    useDatabaseWorkspaceStore.getState().setSelection(primary.id, appSelection);
    useDatabaseWorkspaceStore.getState().setSqlDraft(primary.id, "select * from users");
    useDatabaseWorkspaceStore.getState().setSqlResult(primary.id, result);
    useDatabaseWorkspaceStore.getState().addTask(primary.id, {
      id: "task-1",
      label: "Export users",
      status: "running",
      startedAt: 1,
    });

    useDatabaseWorkspaceStore.getState().setActiveView(analytics.id, "structure");
    useDatabaseWorkspaceStore.getState().setSelection(analytics.id, metricsSelection);

    const state = useDatabaseWorkspaceStore.getState();
    expect(Object.keys(state.tabs).sort()).toEqual([primary.id, analytics.id].sort());
    expect(state.activeTabId).toBe(analytics.id);
    expect(state.tabs[primary.id]).toMatchObject({
      activeView: "sql",
      selection: appSelection,
      sqlDraft: "select * from users",
      sqlResult: result,
    });
    expect(state.tabs[primary.id].tasks).toHaveLength(1);
    expect(state.tabs[analytics.id]).toMatchObject({
      activeView: "structure",
      selection: metricsSelection,
      sqlDraft: "",
      sqlResult: null,
      tasks: [],
    });
  });

  it("keeps data-grid query and column state scoped to each connection tab", () => {
    const workspace = useDatabaseWorkspaceStore.getState();
    workspace.openTab(primary);
    workspace.openTab(analytics);
    const usersKey = dataGridTableKey({ database: "app", schema: "public", table: "users" });
    const ordersKey = dataGridTableKey({ database: "app", schema: "public", table: "orders" });

    useDatabaseWorkspaceStore.getState().setDataGridState(primary.id, usersKey, {
      offset: 100,
      limit: 25,
      sort: [{ column: "created_at", direction: "desc" }],
      filters: [
        {
          column: "email",
          operator: "contains",
          value: { kind: "text", value: "example.com" },
        },
      ],
      hiddenColumns: ["notes"],
      columnWidths: { email: 240 },
      includeExactCount: true,
    });
    useDatabaseWorkspaceStore.getState().setDataGridState(primary.id, ordersKey, {
      offset: 50,
    });
    useDatabaseWorkspaceStore.getState().setDataGridState(analytics.id, usersKey, {
      offset: 25,
    });

    expect(useDatabaseWorkspaceStore.getState().tabs[primary.id].dataGrid[usersKey]).toMatchObject({
      offset: 100,
      limit: 25,
      sort: [{ column: "created_at", direction: "desc" }],
      hiddenColumns: ["notes"],
      columnWidths: { email: 240 },
      includeExactCount: true,
    });
    expect(useDatabaseWorkspaceStore.getState().tabs[primary.id].dataGrid[ordersKey].offset).toBe(
      50,
    );
    expect(useDatabaseWorkspaceStore.getState().tabs[analytics.id].dataGrid[usersKey].offset).toBe(
      25,
    );

    useDatabaseWorkspaceStore.getState().resetDataGridState(primary.id, usersKey);
    expect(
      useDatabaseWorkspaceStore.getState().tabs[primary.id].dataGrid[usersKey],
    ).toBeUndefined();
    expect(useDatabaseWorkspaceStore.getState().tabs[analytics.id].dataGrid[usersKey].offset).toBe(
      25,
    );
  });

  it("switches the active tab without mutating either connection", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);
    useDatabaseWorkspaceStore.getState().openTab(analytics);
    useDatabaseWorkspaceStore.getState().setSqlDraft(primary.id, "select 1");

    useDatabaseWorkspaceStore.getState().activateTab(primary.id);

    const state = useDatabaseWorkspaceStore.getState();
    expect(state.activeTabId).toBe(primary.id);
    expect(state.tabs[primary.id].sqlDraft).toBe("select 1");
    expect(state.tabs[analytics.id].sqlDraft).toBe("");
  });

  it("closes only the requested tab and selects a neighbour", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);
    useDatabaseWorkspaceStore.getState().openTab(analytics);
    useDatabaseWorkspaceStore.getState().setSqlDraft(analytics.id, "select 2");

    useDatabaseWorkspaceStore.getState().closeTab(primary.id);

    const state = useDatabaseWorkspaceStore.getState();
    expect(state.tabs[primary.id]).toBeUndefined();
    expect(state.tabs[analytics.id].sqlDraft).toBe("select 2");
    expect(state.order).toEqual([analytics.id]);
    expect(state.activeTabId).toBe(analytics.id);
  });

  it("leaves the active tab alone when closing another connection", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);
    useDatabaseWorkspaceStore.getState().openTab(analytics);
    useDatabaseWorkspaceStore.getState().activateTab(primary.id);

    useDatabaseWorkspaceStore.getState().closeTab(analytics.id);

    expect(useDatabaseWorkspaceStore.getState().activeTabId).toBe(primary.id);
  });

  it("drops tasks and context when a tab closes and starts fresh on reopen", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);
    useDatabaseWorkspaceStore.getState().setSqlDraft(primary.id, "select 3");
    useDatabaseWorkspaceStore.getState().addTask(primary.id, {
      id: "task-1",
      label: "Backup",
      status: "running",
      startedAt: 1,
    });

    useDatabaseWorkspaceStore.getState().closeTab(primary.id);
    expect(useDatabaseWorkspaceStore.getState().tabs[primary.id]).toBeUndefined();
    expect(useDatabaseWorkspaceStore.getState().activeTabId).toBeNull();

    useDatabaseWorkspaceStore.getState().openTab(primary);
    const reopened = useDatabaseWorkspaceStore.getState().tabs[primary.id];
    expect(reopened.sqlDraft).toBe("");
    expect(reopened.selection).toBeNull();
    expect(reopened.tasks).toEqual([]);
  });

  it("refreshes identity on reopen without resetting an open tab context", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);
    useDatabaseWorkspaceStore.getState().setSqlDraft(primary.id, "select 4");
    useDatabaseWorkspaceStore.getState().activateTab(primary.id);

    useDatabaseWorkspaceStore.getState().openTab({
      ...primary,
      name: "Primary renamed",
      databaseName: "new_app",
    });

    const tab = useDatabaseWorkspaceStore.getState().tabs[primary.id];
    expect(tab.name).toBe("Primary renamed");
    expect(tab.databaseName).toBe("new_app");
    expect(tab.sqlDraft).toBe("select 4");
  });

  it("chooses the previous tab when the last tab closes", () => {
    expect(nextActiveTabId(["a", "b"], "b", "b")).toBe("a");
    expect(nextActiveTabId(["a", "b"], "a", "a")).toBe("b");
    expect(nextActiveTabId(["a", "b"], null, "b")).toBeNull();
    expect(nextActiveTabId(["a", "b"], "b", "a")).toBe("b");
  });
});
