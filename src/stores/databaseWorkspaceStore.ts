import { create } from "zustand";
import type { DatabaseExplorerSelection } from "@/features/databases/databaseExplorerTreeModel";
import {
  DEFAULT_DATA_GRID_STATE,
  type DatabaseDataGridState,
} from "@/features/databases/databaseDataGridModel";
import type { DatabaseEngine, DatabaseProfile } from "@/types/contracts";

export const DATABASE_WORKSPACE_VIEWS = [
  "overview",
  "structure",
  "diagram",
  "data",
  "sql",
  "indexes",
  "operations",
  "access",
  "sessions",
] as const;

export type DatabaseWorkspaceView = (typeof DATABASE_WORKSPACE_VIEWS)[number];

export type DatabaseSqlExecutionSource = "selection" | "statement" | "all";

export type DatabaseSqlLastRun = {
  sql: string;
  source: DatabaseSqlExecutionSource;
  statementNumber: number;
  statementCount: number;
  requestedAt: number;
};

/**
 * One query tab inside a connection workspace. The draft, selection, and
 * execution options stay with this tab while the user switches SQL or
 * connection tabs. Runtime status and results live in the query store keyed by
 * this tab's id.
 */
export type DatabaseSqlQueryTab = {
  id: string;
  title: string;
  sql: string;
  selectionStart: number;
  selectionEnd: number;
  maxRows: number;
  timeoutMs: number;
  lastRun: DatabaseSqlLastRun | null;
};

export type DatabaseWorkspaceTaskStatus = "running" | "succeeded" | "failed" | "cancelled";

export type DatabaseWorkspaceTask = {
  id: string;
  label: string;
  status: DatabaseWorkspaceTaskStatus;
  detail?: string | null;
  /** True when the backend result says the target may be incomplete. */
  partial?: boolean;
  startedAt: number;
};

/**
 * One open database workspace. Every mutable field is owned by this tab so
 * switching connections never copies selection, SQL, or results between them.
 */
export type DatabaseWorkspaceTab = {
  profileId: string;
  serverId: string;
  name: string;
  engine: DatabaseEngine;
  host: string;
  port: number;
  socket: string | null;
  databaseName: string | null;
  readOnly: boolean;
  activeView: DatabaseWorkspaceView;
  selection: DatabaseExplorerSelection | null;
  /** Client-side guard for the SQL workspace, on top of the profile flag. */
  sqlReadOnly: boolean;
  sqlTabs: DatabaseSqlQueryTab[];
  activeSqlTabId: string;
  sqlQuerySequence: number;
  /** Data-grid state is keyed by table and owned by this connection tab. */
  dataGrid: Record<string, DatabaseDataGridState>;
  tasks: DatabaseWorkspaceTask[];
  openedAt: number;
};

export const DEFAULT_SQL_QUERY_LIMIT = 500;
export const DEFAULT_SQL_QUERY_TIMEOUT_MS = 10_000;

type DatabaseWorkspaceStore = {
  tabs: Record<string, DatabaseWorkspaceTab>;
  order: string[];
  activeTabId: string | null;
  openTab: (profile: DatabaseProfile) => void;
  closeTab: (profileId: string) => void;
  activateTab: (profileId: string) => void;
  setActiveView: (profileId: string, view: DatabaseWorkspaceView) => void;
  setSelection: (profileId: string, selection: DatabaseExplorerSelection | null) => void;
  setSqlReadOnly: (profileId: string, readOnly: boolean) => void;
  addSqlQueryTab: (profileId: string) => string | null;
  closeSqlQueryTab: (profileId: string, queryTabId: string) => void;
  activateSqlQueryTab: (profileId: string, queryTabId: string) => void;
  setSqlQueryText: (profileId: string, queryTabId: string, sql: string) => void;
  setSqlQuerySelection: (profileId: string, queryTabId: string, start: number, end: number) => void;
  setSqlQueryOptions: (
    profileId: string,
    queryTabId: string,
    options: { maxRows?: number; timeoutMs?: number },
  ) => void;
  setSqlQueryLastRun: (
    profileId: string,
    queryTabId: string,
    lastRun: DatabaseSqlLastRun | null,
  ) => void;
  setDataGridState: (
    profileId: string,
    tableKey: string,
    patch: Partial<DatabaseDataGridState>,
  ) => void;
  resetDataGridState: (profileId: string, tableKey: string) => void;
  addTask: (profileId: string, task: DatabaseWorkspaceTask) => void;
  updateTask: (
    profileId: string,
    taskId: string,
    patch: Partial<Omit<DatabaseWorkspaceTask, "id" | "startedAt">>,
  ) => void;
  removeTask: (profileId: string, taskId: string) => void;
  clearTasks: (profileId: string) => void;
};

/** Creates a stable id used only inside the workspace's query-tab strip. */
export function createDatabaseSqlQueryTabId(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return `sql-${cryptoApi.randomUUID()}`;
  }
  return `sql-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function createDatabaseSqlQueryTab(sequence: number): DatabaseSqlQueryTab {
  return {
    id: createDatabaseSqlQueryTabId(),
    title: `Query ${sequence}`,
    sql: "",
    selectionStart: 0,
    selectionEnd: 0,
    maxRows: DEFAULT_SQL_QUERY_LIMIT,
    timeoutMs: DEFAULT_SQL_QUERY_TIMEOUT_MS,
    lastRun: null,
  };
}

function tabFromProfile(profile: DatabaseProfile): DatabaseWorkspaceTab {
  const firstQueryTab = createDatabaseSqlQueryTab(1);
  return {
    profileId: profile.id,
    serverId: profile.serverId,
    name: profile.name,
    engine: profile.engine,
    host: profile.host,
    port: profile.port,
    socket: profile.socket,
    databaseName: profile.databaseName,
    readOnly: profile.readOnly,
    activeView: "overview",
    selection: null,
    sqlReadOnly: profile.readOnly,
    sqlTabs: [firstQueryTab],
    activeSqlTabId: firstQueryTab.id,
    sqlQuerySequence: 1,
    dataGrid: {},
    tasks: [],
    openedAt: Date.now(),
  };
}

/** Updates identity fields without touching the tab's workspace context. */
function refreshTabIdentity(
  tab: DatabaseWorkspaceTab,
  profile: DatabaseProfile,
): DatabaseWorkspaceTab {
  const readOnly = profile.readOnly;
  if (
    tab.serverId === profile.serverId &&
    tab.name === profile.name &&
    tab.engine === profile.engine &&
    tab.host === profile.host &&
    tab.port === profile.port &&
    tab.socket === profile.socket &&
    tab.databaseName === profile.databaseName &&
    tab.readOnly === readOnly &&
    (!readOnly || tab.sqlReadOnly)
  ) {
    return tab;
  }

  return {
    ...tab,
    serverId: profile.serverId,
    name: profile.name,
    engine: profile.engine,
    host: profile.host,
    port: profile.port,
    socket: profile.socket,
    databaseName: profile.databaseName,
    readOnly,
    // A read-only profile cannot be unlocked in the SQL workspace.
    sqlReadOnly: readOnly ? true : tab.sqlReadOnly,
  };
}

export function nextActiveTabId(
  order: readonly string[],
  activeTabId: string | null,
  closingTabId: string,
): string | null {
  if (activeTabId !== closingTabId) {
    return activeTabId;
  }
  const index = order.indexOf(closingTabId);
  if (index < 0) {
    return activeTabId;
  }
  return order[index + 1] ?? order[index - 1] ?? null;
}

function updateTab(
  state: Pick<DatabaseWorkspaceStore, "tabs">,
  profileId: string,
  update: (tab: DatabaseWorkspaceTab) => DatabaseWorkspaceTab,
): Partial<DatabaseWorkspaceStore> {
  const tab = state.tabs[profileId];
  if (!tab) {
    return {};
  }
  return { tabs: { ...state.tabs, [profileId]: update(tab) } };
}

function updateSqlQueryTab(
  state: Pick<DatabaseWorkspaceStore, "tabs">,
  profileId: string,
  queryTabId: string,
  update: (queryTab: DatabaseSqlQueryTab) => DatabaseSqlQueryTab,
): Partial<DatabaseWorkspaceStore> {
  return updateTab(state, profileId, (tab) => {
    if (!tab.sqlTabs.some((queryTab) => queryTab.id === queryTabId)) {
      return tab;
    }
    return {
      ...tab,
      sqlTabs: tab.sqlTabs.map((queryTab) =>
        queryTab.id === queryTabId ? update(queryTab) : queryTab,
      ),
    };
  });
}

export const useDatabaseWorkspaceStore = create<DatabaseWorkspaceStore>((set) => ({
  tabs: {},
  order: [],
  activeTabId: null,

  openTab: (profile) => {
    set((state) => {
      const existing = state.tabs[profile.id];
      if (existing) {
        return {
          tabs: {
            ...state.tabs,
            [profile.id]: refreshTabIdentity(existing, profile),
          },
          activeTabId: profile.id,
        };
      }
      return {
        tabs: { ...state.tabs, [profile.id]: tabFromProfile(profile) },
        order: [...state.order, profile.id],
        activeTabId: profile.id,
      };
    });
  },

  closeTab: (profileId) => {
    set((state) => {
      if (!state.tabs[profileId]) {
        return {};
      }
      const tabs = { ...state.tabs };
      delete tabs[profileId];
      return {
        tabs,
        order: state.order.filter((id) => id !== profileId),
        activeTabId: nextActiveTabId(state.order, state.activeTabId, profileId),
      };
    });
  },

  activateTab: (profileId) => {
    set((state) => (state.tabs[profileId] ? { activeTabId: profileId } : {}));
  },

  setActiveView: (profileId, view) => {
    set((state) => updateTab(state, profileId, (tab) => ({ ...tab, activeView: view })));
  },

  setSelection: (profileId, selection) => {
    set((state) => updateTab(state, profileId, (tab) => ({ ...tab, selection })));
  },

  setSqlReadOnly: (profileId, readOnly) => {
    set((state) =>
      updateTab(state, profileId, (tab) => {
        // A read-only connection profile is enforced by the adapter. The
        // workspace toggle can add a guard but must never remove that one.
        if (tab.readOnly && !readOnly) {
          return tab;
        }
        return tab.sqlReadOnly === readOnly ? tab : { ...tab, sqlReadOnly: readOnly };
      }),
    );
  },

  addSqlQueryTab: (profileId) => {
    let createdId: string | null = null;
    set((state) =>
      updateTab(state, profileId, (tab) => {
        const sequence = tab.sqlQuerySequence + 1;
        const queryTab = createDatabaseSqlQueryTab(sequence);
        createdId = queryTab.id;
        return {
          ...tab,
          sqlTabs: [...tab.sqlTabs, queryTab],
          activeSqlTabId: queryTab.id,
          sqlQuerySequence: sequence,
        };
      }),
    );
    return createdId;
  },

  closeSqlQueryTab: (profileId, queryTabId) => {
    set((state) =>
      updateTab(state, profileId, (tab) => {
        const index = tab.sqlTabs.findIndex((queryTab) => queryTab.id === queryTabId);
        if (index < 0) {
          return tab;
        }
        const remaining = tab.sqlTabs.filter((queryTab) => queryTab.id !== queryTabId);
        if (remaining.length === 0) {
          const sequence = tab.sqlQuerySequence + 1;
          const replacement = createDatabaseSqlQueryTab(sequence);
          return {
            ...tab,
            sqlTabs: [replacement],
            activeSqlTabId: replacement.id,
            sqlQuerySequence: sequence,
          };
        }
        const nextActive =
          tab.activeSqlTabId === queryTabId
            ? (remaining[index] ?? remaining[index - 1] ?? remaining[0]).id
            : tab.activeSqlTabId;
        return { ...tab, sqlTabs: remaining, activeSqlTabId: nextActive };
      }),
    );
  },

  activateSqlQueryTab: (profileId, queryTabId) => {
    set((state) =>
      updateTab(state, profileId, (tab) =>
        tab.sqlTabs.some((queryTab) => queryTab.id === queryTabId)
          ? { ...tab, activeSqlTabId: queryTabId }
          : tab,
      ),
    );
  },

  setSqlQueryText: (profileId, queryTabId, sql) => {
    set((state) => updateSqlQueryTab(state, profileId, queryTabId, (tab) => ({ ...tab, sql })));
  },

  setSqlQuerySelection: (profileId, queryTabId, start, end) => {
    set((state) =>
      updateSqlQueryTab(state, profileId, queryTabId, (tab) => {
        const selectionStart = Math.max(0, Math.min(start, tab.sql.length));
        const selectionEnd = Math.max(selectionStart, Math.min(end, tab.sql.length));
        if (tab.selectionStart === selectionStart && tab.selectionEnd === selectionEnd) {
          return tab;
        }
        return { ...tab, selectionStart, selectionEnd };
      }),
    );
  },

  setSqlQueryOptions: (profileId, queryTabId, options) => {
    set((state) =>
      updateSqlQueryTab(state, profileId, queryTabId, (tab) => ({
        ...tab,
        maxRows: options.maxRows ?? tab.maxRows,
        timeoutMs: options.timeoutMs ?? tab.timeoutMs,
      })),
    );
  },

  setSqlQueryLastRun: (profileId, queryTabId, lastRun) => {
    set((state) => updateSqlQueryTab(state, profileId, queryTabId, (tab) => ({ ...tab, lastRun })));
  },

  setDataGridState: (profileId, tableKey, patch) => {
    set((state) =>
      updateTab(state, profileId, (tab) => {
        const current = tab.dataGrid[tableKey] ?? DEFAULT_DATA_GRID_STATE;
        return {
          ...tab,
          dataGrid: {
            ...tab.dataGrid,
            [tableKey]: { ...current, ...patch },
          },
        };
      }),
    );
  },

  resetDataGridState: (profileId, tableKey) => {
    set((state) =>
      updateTab(state, profileId, (tab) => {
        const dataGrid = { ...tab.dataGrid };
        delete dataGrid[tableKey];
        return { ...tab, dataGrid };
      }),
    );
  },

  addTask: (profileId, task) => {
    set((state) =>
      updateTab(state, profileId, (tab) => ({
        ...tab,
        tasks: [...tab.tasks.filter((entry) => entry.id !== task.id), task],
      })),
    );
  },

  updateTask: (profileId, taskId, patch) => {
    set((state) =>
      updateTab(state, profileId, (tab) => ({
        ...tab,
        tasks: tab.tasks.map((task) => (task.id === taskId ? { ...task, ...patch } : task)),
      })),
    );
  },

  removeTask: (profileId, taskId) => {
    set((state) =>
      updateTab(state, profileId, (tab) => ({
        ...tab,
        tasks: tab.tasks.filter((task) => task.id !== taskId),
      })),
    );
  },

  clearTasks: (profileId) => {
    set((state) => updateTab(state, profileId, (tab) => ({ ...tab, tasks: [] })));
  },
}));
