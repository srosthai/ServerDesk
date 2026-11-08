import { create } from "zustand";
import { getCommandErrorDetails, getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  browseRemoteDirectories,
  listProjects,
  runProjectArtisanAction,
  scanRemoteProjects,
} from "@/services/projects";
import type {
  ArtisanActionName,
  ProjectArtisanRun,
  ProjectRecord,
  ProjectScanResult,
  RemoteDirectoryListing,
} from "@/types/contracts";

export type ProjectEntry = {
  projects: ProjectRecord[];
  loading: boolean;
  error: string | null;
  listing: RemoteDirectoryListing | null;
  browsing: boolean;
  browseError: string | null;
  scan: ProjectScanResult | null;
  scanning: boolean;
  scanError: string | null;
};

export type ProjectActionEntry = {
  pending: ArtisanActionName | null;
  run: ProjectArtisanRun | null;
  error: string | null;
  errorDetails: string | null;
};

export type ProjectActionKey = `${string}:${string}`;

type ProjectStore = {
  entries: Record<string, ProjectEntry>;
  actions: Record<string, ProjectActionEntry>;
  fetchProjects: (serverId: string) => Promise<boolean>;
  browse: (serverId: string, path: string) => Promise<boolean>;
  scan: (serverId: string, path: string, includeChildren: boolean) => Promise<boolean>;
  runAction: (
    serverId: string,
    projectId: string,
    action: ArtisanActionName,
    confirmed: boolean,
  ) => Promise<boolean>;
  clearActionResult: (serverId: string, projectId: string) => void;
  clearScan: (serverId: string) => void;
  clearBrowse: (serverId: string) => void;
  clearError: (serverId: string) => void;
  clearServer: (serverId: string) => void;
};

export function projectActionKey(serverId: string, projectId: string): ProjectActionKey {
  return `${serverId}:${projectId}`;
}

export function emptyProjectEntry(): ProjectEntry {
  return {
    projects: [],
    loading: false,
    error: null,
    listing: null,
    browsing: false,
    browseError: null,
    scan: null,
    scanning: false,
    scanError: null,
  };
}

function mergeProjects(current: ProjectRecord[], incoming: ProjectRecord[]): ProjectRecord[] {
  const byId = new Map(current.map((project) => [project.id, project]));
  const byPath = new Map(current.map((project) => [project.remotePath, project]));

  for (const project of incoming) {
    const existing = byId.get(project.id) ?? byPath.get(project.remotePath);
    if (existing) {
      byId.delete(existing.id);
      if (existing.remotePath !== project.remotePath) {
        byPath.delete(existing.remotePath);
      }
    }
    byId.set(project.id, project);
    byPath.set(project.remotePath, project);
  }

  return [...byId.values()].sort((left, right) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: "base" }),
  );
}

export const useProjectStore = create<ProjectStore>((set, get) => {
  let requestCounter = 0;
  const fetchIds: Record<string, number> = {};
  const browseIds: Record<string, number> = {};
  const scanIds: Record<string, number> = {};
  const actionIds: Record<string, number> = {};

  function updateEntry(serverId: string, update: (entry: ProjectEntry) => ProjectEntry) {
    set((state) => ({
      entries: {
        ...state.entries,
        [serverId]: update(state.entries[serverId] ?? emptyProjectEntry()),
      },
    }));
  }

  return {
    entries: {},
    actions: {},

    fetchProjects: async (serverId) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++requestCounter;
      fetchIds[serverId] = requestId;
      updateEntry(serverId, (entry) => ({ ...entry, loading: true, error: null }));

      try {
        const projects = await listProjects(serverId);
        if (fetchIds[serverId] !== requestId) {
          return false;
        }
        updateEntry(serverId, (entry) => ({
          ...entry,
          projects,
          loading: false,
          error: null,
        }));
        return true;
      } catch (error) {
        if (fetchIds[serverId] === requestId) {
          updateEntry(serverId, (entry) => ({
            ...entry,
            loading: false,
            error: getCommandErrorMessage(error),
          }));
        }
        return false;
      }
    },

    browse: async (serverId, path) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++requestCounter;
      browseIds[serverId] = requestId;
      updateEntry(serverId, (entry) => ({
        ...entry,
        browsing: true,
        browseError: null,
      }));

      try {
        const listing = await browseRemoteDirectories(serverId, path);
        if (browseIds[serverId] !== requestId) {
          return false;
        }
        updateEntry(serverId, (entry) => ({
          ...entry,
          listing,
          browsing: false,
          browseError: null,
        }));
        return true;
      } catch (error) {
        if (browseIds[serverId] === requestId) {
          updateEntry(serverId, (entry) => ({
            ...entry,
            browsing: false,
            browseError: getCommandErrorMessage(error),
          }));
        }
        return false;
      }
    },

    scan: async (serverId, path, includeChildren) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const requestId = ++requestCounter;
      scanIds[serverId] = requestId;
      updateEntry(serverId, (entry) => ({
        ...entry,
        scanning: true,
        scanError: null,
      }));

      try {
        const scan = await scanRemoteProjects(serverId, path, includeChildren);
        if (scanIds[serverId] !== requestId) {
          return false;
        }
        updateEntry(serverId, (entry) => ({
          ...entry,
          projects: mergeProjects(entry.projects, scan.projects),
          scan,
          scanning: false,
          scanError: null,
        }));
        return true;
      } catch (error) {
        if (scanIds[serverId] === requestId) {
          updateEntry(serverId, (entry) => ({
            ...entry,
            scanning: false,
            scanError: getCommandErrorMessage(error),
          }));
        }
        return false;
      }
    },

    runAction: async (serverId, projectId, action, confirmed) => {
      if (!isTauriRuntime()) {
        return false;
      }

      const key = projectActionKey(serverId, projectId);
      const requestId = ++requestCounter;
      actionIds[key] = requestId;
      set((state) => ({
        actions: {
          ...state.actions,
          [key]: {
            pending: action,
            run: null,
            error: null,
            errorDetails: null,
          },
        },
      }));

      try {
        const run = await runProjectArtisanAction(serverId, projectId, action, confirmed);
        if (actionIds[key] !== requestId) {
          return false;
        }
        set((state) => ({
          actions: {
            ...state.actions,
            [key]: {
              pending: null,
              run,
              error: null,
              errorDetails: null,
            },
          },
        }));

        // The backend records every executed action locally. Refresh the saved
        // project so the dashboard shows its new recent-operation status.
        await get().fetchProjects(serverId);
        return run.result.succeeded;
      } catch (error) {
        if (actionIds[key] === requestId) {
          set((state) => ({
            actions: {
              ...state.actions,
              [key]: {
                pending: null,
                run: null,
                error: getCommandErrorMessage(error),
                errorDetails: getCommandErrorDetails(error),
              },
            },
          }));
        }
        return false;
      }
    },

    clearActionResult: (serverId, projectId) => {
      const key = projectActionKey(serverId, projectId);
      actionIds[key] = ++requestCounter;
      set((state) => {
        const actions = { ...state.actions };
        delete actions[key];
        return { actions };
      });
    },

    clearScan: (serverId) =>
      updateEntry(serverId, (entry) => ({
        ...entry,
        scan: null,
        scanError: null,
      })),

    clearBrowse: (serverId) =>
      updateEntry(serverId, (entry) => ({
        ...entry,
        listing: null,
        browseError: null,
      })),

    clearError: (serverId) => updateEntry(serverId, (entry) => ({ ...entry, error: null })),

    clearServer: (serverId) => {
      fetchIds[serverId] = ++requestCounter;
      browseIds[serverId] = ++requestCounter;
      scanIds[serverId] = ++requestCounter;
      const prefix = `${serverId}:`;
      for (const key of Object.keys(actionIds)) {
        if (key.startsWith(prefix)) {
          actionIds[key] = ++requestCounter;
        }
      }
      set((state) => {
        const entries = { ...state.entries };
        delete entries[serverId];
        const actions = Object.fromEntries(
          Object.entries(state.actions).filter(([key]) => !key.startsWith(prefix)),
        );
        return { entries, actions };
      });
    },
  };
});
