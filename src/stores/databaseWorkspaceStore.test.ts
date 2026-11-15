import { beforeEach, describe, expect, it } from "vitest";
import { dataGridTableKey } from "@/features/databases/databaseDataGridModel";
import {
  DEFAULT_SQL_QUERY_LIMIT,
  DEFAULT_SQL_QUERY_TIMEOUT_MS,
  nextActiveTabId,
  useDatabaseWorkspaceStore,
  type DatabaseSqlLastRun,
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

const lastRun: DatabaseSqlLastRun = {
  sql: "select * from users",
  source: "statement",
  statementNumber: 1,
  statementCount: 1,
  requestedAt: 10,
};

function firstQueryTab(profileId: string) {
  return useDatabaseWorkspaceStore.getState().tabs[profileId].sqlTabs[0];
}

describe("database workspace store", () => {
  beforeEach(() => {
    useDatabaseWorkspaceStore.setState({
      tabs: {},
      order: [],
      activeTabId: null,
    });
  });

  it("creates a workspace tab with one query tab, overview, and a scoped read-only mode", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);

    const state = useDatabaseWorkspaceStore.getState();
    const tab = state.tabs[primary.id];
    expect(state.order).toEqual([primary.id]);
    expect(state.activeTabId).toBe(primary.id);
    expect(tab).toMatchObject({
      profileId: primary.id,
      serverId: primary.serverId,
      name: "Primary",
      engine: "postgresql",
      activeView: "overview",
      selection: null,
      sqlReadOnly: true,
      sqlQuerySequence: 1,
      tasks: [],
    });
    expect(tab.sqlTabs).toHaveLength(1);
    expect(tab.activeSqlTabId).toBe(tab.sqlTabs[0].id);
    expect(tab.sqlTabs[0]).toMatchObject({
      title: "Query 1",
      sql: "",
      selectionStart: 0,
      selectionEnd: 0,
      maxRows: DEFAULT_SQL_QUERY_LIMIT,
      timeoutMs: DEFAULT_SQL_QUERY_TIMEOUT_MS,
      lastRun: null,
    });
  });

  it("keeps two connections open and scopes every workspace field per tab", () => {
    const workspace = useDatabaseWorkspaceStore.getState();
    workspace.openTab(primary);
    workspace.openTab(analytics);
    const primaryQuery = firstQueryTab(primary.id);

    useDatabaseWorkspaceStore.getState().setActiveView(primary.id, "sql");
    useDatabaseWorkspaceStore.getState().setSelection(primary.id, appSelection);
    useDatabaseWorkspaceStore
      .getState()
      .setSqlQueryText(primary.id, primaryQuery.id, "select * from users");
    useDatabaseWorkspaceStore.getState().setSqlQuerySelection(primary.id, primaryQuery.id, 5, 12);
    useDatabaseWorkspaceStore.getState().setSqlQueryLastRun(primary.id, primaryQuery.id, lastRun);
    useDatabaseWorkspaceStore
      .getState()
      .setSqlQueryOptions(primary.id, primaryQuery.id, { maxRows: 100, timeoutMs: 5_000 });
    useDatabaseWorkspaceStore.getState().addTask(primary.id, {
      id: "task-1",
      label: "Export users",
      status: "running",
      startedAt: 1,
    });

    useDatabaseWorkspaceStore.getState().setActiveView(analytics.id, "structure");
    useDatabaseWorkspaceStore.getState().setSelection(analytics.id, metricsSelection);
    useDatabaseWorkspaceStore.getState().setSqlReadOnly(analytics.id, true);

    const state = useDatabaseWorkspaceStore.getState();
    expect(Object.keys(state.tabs).sort()).toEqual([primary.id, analytics.id].sort());
    expect(state.activeTabId).toBe(analytics.id);
    expect(state.tabs[primary.id]).toMatchObject({
      activeView: "sql",
      selection: appSelection,
      sqlReadOnly: true,
    });
    expect(state.tabs[primary.id].sqlTabs[0]).toMatchObject({
      sql: "select * from users",
      selectionStart: 5,
      selectionEnd: 12,
      maxRows: 100,
      timeoutMs: 5_000,
      lastRun,
    });
    expect(state.tabs[primary.id].tasks).toHaveLength(1);
    expect(state.tabs[analytics.id]).toMatchObject({
      activeView: "structure",
      selection: metricsSelection,
      sqlReadOnly: true,
    });
    expect(state.tabs[analytics.id].sqlTabs[0]).toMatchObject({
      sql: "",
      lastRun: null,
    });
    expect(state.tabs[analytics.id].tasks).toEqual([]);
  });

  it("adds, activates, and closes query tabs without touching sibling drafts", () => {
    useDatabaseWorkspaceStore.getState().openTab(analytics);
    const first = firstQueryTab(analytics.id);
    const secondId = useDatabaseWorkspaceStore.getState().addSqlQueryTab(analytics.id);
    expect(secondId).not.toBeNull();

    useDatabaseWorkspaceStore.getState().setSqlQueryText(analytics.id, first.id, "select 1");
    useDatabaseWorkspaceStore
      .getState()
      .setSqlQueryText(analytics.id, secondId as string, "select 2");

    let tab = useDatabaseWorkspaceStore.getState().tabs[analytics.id];
    expect(tab.sqlTabs.map((queryTab) => queryTab.title)).toEqual(["Query 1", "Query 2"]);
    expect(tab.activeSqlTabId).toBe(secondId);

    useDatabaseWorkspaceStore.getState().activateSqlQueryTab(analytics.id, first.id);
    tab = useDatabaseWorkspaceStore.getState().tabs[analytics.id];
    expect(tab.activeSqlTabId).toBe(first.id);

    useDatabaseWorkspaceStore.getState().closeSqlQueryTab(analytics.id, secondId as string);
    tab = useDatabaseWorkspaceStore.getState().tabs[analytics.id];
    expect(tab.sqlTabs).toHaveLength(1);
    expect(tab.sqlTabs[0]).toMatchObject({ title: "Query 1", sql: "select 1" });
    expect(tab.activeSqlTabId).toBe(first.id);
  });

  it("selects a neighbour and replaces the last query tab when it closes", () => {
    useDatabaseWorkspaceStore.getState().openTab(analytics);
    const first = firstQueryTab(analytics.id);
    const secondId = useDatabaseWorkspaceStore.getState().addSqlQueryTab(analytics.id) as string;
    const thirdId = useDatabaseWorkspaceStore.getState().addSqlQueryTab(analytics.id) as string;

    useDatabaseWorkspaceStore.getState().activateSqlQueryTab(analytics.id, thirdId);
    useDatabaseWorkspaceStore.getState().closeSqlQueryTab(analytics.id, thirdId);
    let tab = useDatabaseWorkspaceStore.getState().tabs[analytics.id];
    expect(tab.activeSqlTabId).toBe(secondId);

    useDatabaseWorkspaceStore.getState().closeSqlQueryTab(analytics.id, secondId);
    useDatabaseWorkspaceStore.getState().closeSqlQueryTab(analytics.id, first.id);
    tab = useDatabaseWorkspaceStore.getState().tabs[analytics.id];
    expect(tab.sqlTabs).toHaveLength(1);
    expect(tab.sqlTabs[0].title).toBe("Query 4");
    expect(tab.sqlTabs[0].sql).toBe("");
    expect(tab.activeSqlTabId).toBe(tab.sqlTabs[0].id);
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
    const primaryQuery = firstQueryTab(primary.id);
    useDatabaseWorkspaceStore.getState().setSqlQueryText(primary.id, primaryQuery.id, "select 1");

    useDatabaseWorkspaceStore.getState().activateTab(primary.id);

    const state = useDatabaseWorkspaceStore.getState();
    expect(state.activeTabId).toBe(primary.id);
    expect(state.tabs[primary.id].sqlTabs[0].sql).toBe("select 1");
    expect(state.tabs[analytics.id].sqlTabs[0].sql).toBe("");
  });

  it("closes only the requested tab and selects a neighbour", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);
    useDatabaseWorkspaceStore.getState().openTab(analytics);
    const analyticsQuery = firstQueryTab(analytics.id);
    useDatabaseWorkspaceStore
      .getState()
      .setSqlQueryText(analytics.id, analyticsQuery.id, "select 2");

    useDatabaseWorkspaceStore.getState().closeTab(primary.id);

    const state = useDatabaseWorkspaceStore.getState();
    expect(state.tabs[primary.id]).toBeUndefined();
    expect(state.tabs[analytics.id].sqlTabs[0].sql).toBe("select 2");
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
    const primaryQuery = firstQueryTab(primary.id);
    useDatabaseWorkspaceStore.getState().setSqlQueryText(primary.id, primaryQuery.id, "select 3");
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
    expect(reopened.sqlTabs[0].sql).toBe("");
    expect(reopened.selection).toBeNull();
    expect(reopened.tasks).toEqual([]);
  });

  it("refreshes identity on reopen without resetting an open tab context", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);
    const primaryQuery = firstQueryTab(primary.id);
    useDatabaseWorkspaceStore.getState().setSqlQueryText(primary.id, primaryQuery.id, "select 4");
    useDatabaseWorkspaceStore.getState().activateTab(primary.id);

    useDatabaseWorkspaceStore.getState().openTab({
      ...primary,
      name: "Primary renamed",
      databaseName: "new_app",
    });

    const tab = useDatabaseWorkspaceStore.getState().tabs[primary.id];
    expect(tab.name).toBe("Primary renamed");
    expect(tab.databaseName).toBe("new_app");
    expect(tab.sqlTabs[0].sql).toBe("select 4");
  });

  it("locks the SQL read-only guard for read-only profiles while allowing it otherwise", () => {
    useDatabaseWorkspaceStore.getState().openTab(primary);
    useDatabaseWorkspaceStore.getState().openTab(analytics);

    expect(useDatabaseWorkspaceStore.getState().tabs[primary.id].sqlReadOnly).toBe(true);
    useDatabaseWorkspaceStore.getState().setSqlReadOnly(primary.id, false);
    expect(useDatabaseWorkspaceStore.getState().tabs[primary.id].sqlReadOnly).toBe(true);

    expect(useDatabaseWorkspaceStore.getState().tabs[analytics.id].sqlReadOnly).toBe(false);
    useDatabaseWorkspaceStore.getState().setSqlReadOnly(analytics.id, true);
    expect(useDatabaseWorkspaceStore.getState().tabs[analytics.id].sqlReadOnly).toBe(true);
    useDatabaseWorkspaceStore.getState().setSqlReadOnly(analytics.id, false);
    expect(useDatabaseWorkspaceStore.getState().tabs[analytics.id].sqlReadOnly).toBe(false);

    // Reopening the same profile with a read-only flag forces the guard on.
    useDatabaseWorkspaceStore.getState().openTab({ ...analytics, readOnly: true });
    expect(useDatabaseWorkspaceStore.getState().tabs[analytics.id].sqlReadOnly).toBe(true);
  });

  it("chooses the previous tab when the last tab closes", () => {
    expect(nextActiveTabId(["a", "b"], "b", "b")).toBe("a");
    expect(nextActiveTabId(["a", "b"], "a", "a")).toBe("b");
    expect(nextActiveTabId(["a", "b"], null, "b")).toBeNull();
    expect(nextActiveTabId(["a", "b"], "b", "a")).toBe("b");
  });
});
