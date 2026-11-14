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
  "data",
  "sql",
  "indexes",
  "operations",
] as const;

export type DatabaseWorkspaceView = (typeof DATABASE_WORKSPACE_VIEWS)[number];

export type DatabaseWorkspaceQueryResult = {
  statement: string;
  columns: string[];
  rows: string[][];
  affectedRows: number | null;
  durationMs: number;
  error: string | null;
  executedAt: number;
};

export type DatabaseWorkspaceTaskStatus = "running" | "succeeded" | "failed" | "cancelled";

export type DatabaseWorkspaceTask = {
  id: string;
  label: string;
  status: DatabaseWorkspaceTaskStatus;
  detail?: string | null;
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
  sqlDraft: string;
  sqlResult: DatabaseWorkspaceQueryResult | null;
  /** Data-grid state is keyed by table and owned by this connection tab. */
  dataGrid: Record<string, DatabaseDataGridState>;
  tasks: DatabaseWorkspaceTask[];
  openedAt: number;
};

type DatabaseWorkspaceStore = {
  tabs: Record<string, DatabaseWorkspaceTab>;
  order: string[];
  activeTabId: string | null;
  openTab: (profile: DatabaseProfile) => void;
  closeTab: (profileId: string) => void;
  activateTab: (profileId: string) => void;
  setActiveView: (profileId: string, view: DatabaseWorkspaceView) => void;
  setSelection: (profileId: string, selection: DatabaseExplorerSelection | null) => void;
  setSqlDraft: (profileId: string, draft: string) => void;
  setSqlResult: (profileId: string, result: DatabaseWorkspaceQueryResult | null) => void;
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

function tabFromProfile(profile: DatabaseProfile): DatabaseWorkspaceTab {
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
    sqlDraft: "",
    sqlResult: null,
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
  if (
    tab.serverId === profile.serverId &&
    tab.name === profile.name &&
    tab.engine === profile.engine &&
    tab.host === profile.host &&
    tab.port === profile.port &&
    tab.socket === profile.socket &&
    tab.databaseName === profile.databaseName &&
    tab.readOnly === profile.readOnly
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
    readOnly: profile.readOnly,
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

  setSqlDraft: (profileId, draft) => {
    set((state) => updateTab(state, profileId, (tab) => ({ ...tab, sqlDraft: draft })));
  },

  setSqlResult: (profileId, result) => {
    set((state) => updateTab(state, profileId, (tab) => ({ ...tab, sqlResult: result })));
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
