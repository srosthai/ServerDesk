import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  connectDatabaseProfile,
  createDatabaseProfile,
  deleteDatabaseProfile,
  disconnectDatabaseProfile,
  getDatabaseConnectionState,
  getDatabaseProfile,
  listDatabaseProfiles,
  reconnectDatabaseProfile,
  testDatabaseProfile,
  updateDatabaseProfile,
} from "@/services/databaseProfiles";
import type {
  CommandError,
  DatabaseConnectionSnapshot,
  DatabaseConnectionState,
  DatabaseProfile,
  DatabaseProfileInput,
  DatabaseTestResult,
} from "@/types/contracts";

type DatabaseProfileStatus = "idle" | "loading" | "ready" | "error";

type DatabaseProfileStore = {
  profiles: Record<string, DatabaseProfile[]>;
  status: Record<string, DatabaseProfileStatus>;
  snapshots: Record<string, DatabaseConnectionSnapshot>;
  connectionStates: Record<string, DatabaseConnectionState>;
  testResults: Record<string, DatabaseTestResult>;
  busy: Record<string, boolean>;
  error: string | null;
  commandError: CommandError | null;
  fetchProfiles: (serverId: string) => Promise<void>;
  fetchProfile: (id: string) => Promise<DatabaseProfile | null>;
  createProfile: (input: DatabaseProfileInput) => Promise<DatabaseProfile | null>;
  updateProfile: (id: string, input: DatabaseProfileInput) => Promise<DatabaseProfile | null>;
  deleteProfile: (id: string) => Promise<boolean>;
  testProfile: (id: string) => Promise<DatabaseTestResult | null>;
  connectProfile: (id: string) => Promise<boolean>;
  reconnectProfile: (id: string) => Promise<boolean>;
  disconnectProfile: (id: string) => Promise<boolean>;
  refreshConnectionState: (id: string) => Promise<void>;
  applySnapshot: (snapshot: DatabaseConnectionSnapshot) => void;
  clearError: () => void;
};

function sortProfiles(profiles: DatabaseProfile[]) {
  return [...profiles].sort((left, right) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: "base" }),
  );
}

function findProfile(
  profiles: Record<string, DatabaseProfile[]>,
  id: string,
): DatabaseProfile | undefined {
  return Object.values(profiles)
    .flat()
    .find((profile) => profile.id === id);
}

function cacheProfile(
  profiles: Record<string, DatabaseProfile[]>,
  profile: DatabaseProfile,
): Record<string, DatabaseProfile[]> {
  const next: Record<string, DatabaseProfile[]> = {};
  for (const [serverId, entries] of Object.entries(profiles)) {
    const remaining = entries.filter((entry) => entry.id !== profile.id);
    if (remaining.length > 0) {
      next[serverId] = remaining;
    }
  }
  next[profile.serverId] = sortProfiles([...(next[profile.serverId] ?? []), profile]);
  return next;
}

function removeCachedProfile(
  profiles: Record<string, DatabaseProfile[]>,
  id: string,
): Record<string, DatabaseProfile[]> {
  const next: Record<string, DatabaseProfile[]> = {};
  for (const [serverId, entries] of Object.entries(profiles)) {
    const remaining = entries.filter((entry) => entry.id !== id);
    if (remaining.length > 0) {
      next[serverId] = remaining;
    }
  }
  return next;
}

function mergeSnapshot(
  snapshots: Record<string, DatabaseConnectionSnapshot>,
  snapshot: DatabaseConnectionSnapshot,
): Record<string, DatabaseConnectionSnapshot> {
  const current = snapshots[snapshot.profileId];
  if (current && current.changedAt > snapshot.changedAt) {
    return snapshots;
  }
  return { ...snapshots, [snapshot.profileId]: snapshot };
}

function pendingSnapshot(
  profile: DatabaseProfile,
  state: DatabaseConnectionState,
): DatabaseConnectionSnapshot {
  return {
    profileId: profile.id,
    serverId: profile.serverId,
    name: profile.name,
    engine: profile.engine,
    state,
    readOnly: profile.readOnly,
    databaseName: profile.databaseName,
    // Pending snapshots are always superseded by the command result.
    changedAt: 0,
  };
}

function connectionDetailsChanged(left: DatabaseProfile, right: DatabaseProfile) {
  return (
    left.serverId !== right.serverId ||
    left.engine !== right.engine ||
    left.host !== right.host ||
    left.port !== right.port ||
    left.socket !== right.socket ||
    left.databaseName !== right.databaseName ||
    left.username !== right.username ||
    left.readOnly !== right.readOnly ||
    left.sslMode !== right.sslMode
  );
}

function commandErrorFor(error: unknown): CommandError | null {
  return isCommandError(error) ? error : null;
}

function omitKey<T>(record: Record<string, T>, id: string): Record<string, T> {
  const next = { ...record };
  delete next[id];
  return next;
}

export const useDatabaseProfileStore = create<DatabaseProfileStore>((set, get) => ({
  profiles: {},
  status: {},
  snapshots: {},
  connectionStates: {},
  testResults: {},
  busy: {},
  error: null,
  commandError: null,

  fetchProfiles: async (serverId) => {
    if (!isTauriRuntime()) {
      set((state) => ({
        profiles: { ...state.profiles, [serverId]: [] },
        status: { ...state.status, [serverId]: "ready" },
        error: null,
        commandError: null,
      }));
      return;
    }

    set((state) => ({
      status: { ...state.status, [serverId]: "loading" },
      error: null,
      commandError: null,
    }));

    try {
      const profiles = await listDatabaseProfiles(serverId);
      set((state) => ({
        profiles: { ...state.profiles, [serverId]: sortProfiles(profiles) },
        status: { ...state.status, [serverId]: "ready" },
        error: null,
        commandError: null,
      }));
    } catch (error) {
      set((state) => ({
        status: { ...state.status, [serverId]: "error" },
        error: getCommandErrorMessage(error),
        commandError: commandErrorFor(error),
      }));
    }
  },

  fetchProfile: async (id) => {
    if (!isTauriRuntime()) {
      return findProfile(get().profiles, id) ?? null;
    }

    try {
      const profile = await getDatabaseProfile(id);
      if (profile) {
        set((state) => ({ profiles: cacheProfile(state.profiles, profile) }));
      }
      return profile;
    } catch (error) {
      set({ error: getCommandErrorMessage(error), commandError: commandErrorFor(error) });
      return null;
    }
  },

  createProfile: async (input) => {
    if (!isTauriRuntime()) {
      set({ error: "Database profiles are available in the desktop application." });
      return null;
    }

    try {
      const profile = await createDatabaseProfile(input);
      set((state) => ({
        profiles: cacheProfile(state.profiles, profile),
        status: { ...state.status, [profile.serverId]: "ready" },
        error: null,
        commandError: null,
      }));
      return profile;
    } catch (error) {
      set({ error: getCommandErrorMessage(error), commandError: commandErrorFor(error) });
      return null;
    }
  },

  updateProfile: async (id, input) => {
    if (!isTauriRuntime()) {
      set({ error: "Database profiles are available in the desktop application." });
      return null;
    }

    try {
      const profile = await updateDatabaseProfile(id, input);
      set((state) => {
        const previous = findProfile(state.profiles, id);
        const connectionChanged =
          previous !== undefined && connectionDetailsChanged(previous, profile);
        const snapshot = state.snapshots[id];

        return {
          profiles: cacheProfile(state.profiles, profile),
          status: {
            ...state.status,
            [profile.serverId]: "ready",
          },
          snapshots:
            connectionChanged && snapshot
              ? {
                  ...state.snapshots,
                  [id]: pendingSnapshot(profile, "disconnected"),
                }
              : snapshot && !connectionChanged
                ? {
                    ...state.snapshots,
                    [id]: {
                      ...snapshot,
                      serverId: profile.serverId,
                      name: profile.name,
                      engine: profile.engine,
                      readOnly: profile.readOnly,
                      databaseName: profile.databaseName,
                    },
                  }
                : state.snapshots,
          connectionStates: connectionChanged
            ? { ...state.connectionStates, [id]: "disconnected" }
            : state.connectionStates,
          error: null,
          commandError: null,
        };
      });
      return profile;
    } catch (error) {
      set({ error: getCommandErrorMessage(error), commandError: commandErrorFor(error) });
      return null;
    }
  },

  deleteProfile: async (id) => {
    if (!isTauriRuntime()) {
      set({ error: "Database profiles are available in the desktop application." });
      return false;
    }

    set((state) => ({
      busy: { ...state.busy, [id]: true },
      error: null,
      commandError: null,
    }));

    try {
      await deleteDatabaseProfile(id);
      set((state) => ({
        profiles: removeCachedProfile(state.profiles, id),
        snapshots: omitKey(state.snapshots, id),
        testResults: omitKey(state.testResults, id),
        connectionStates: omitKey(state.connectionStates, id),
        busy: omitKey(state.busy, id),
        error: null,
        commandError: null,
      }));
      return true;
    } catch (error) {
      set((state) => ({
        busy: { ...state.busy, [id]: false },
        error: getCommandErrorMessage(error),
        commandError: commandErrorFor(error),
      }));
      return false;
    }
  },

  testProfile: async (id) => {
    if (!isTauriRuntime()) {
      set({ error: "Database profiles are available in the desktop application." });
      return null;
    }

    set((state) => ({
      busy: { ...state.busy, [id]: true },
      error: null,
      commandError: null,
    }));

    try {
      const result = await testDatabaseProfile(id);
      set((state) => ({
        testResults: { ...state.testResults, [id]: result },
        busy: { ...state.busy, [id]: false },
        error: null,
        commandError: null,
      }));
      return result;
    } catch (error) {
      set((state) => ({
        testResults: omitKey(state.testResults, id),
        busy: { ...state.busy, [id]: false },
        error: getCommandErrorMessage(error),
        commandError: commandErrorFor(error),
      }));
      return null;
    }
  },

  connectProfile: async (id) => {
    return runConnectionTransition(set, get, id, "connecting", () => connectDatabaseProfile(id));
  },

  reconnectProfile: async (id) => {
    return runConnectionTransition(set, get, id, "reconnecting", () =>
      reconnectDatabaseProfile(id),
    );
  },

  disconnectProfile: async (id) => {
    if (!isTauriRuntime()) {
      set({ error: "Database profiles are available in the desktop application." });
      return false;
    }

    set((state) => ({
      busy: { ...state.busy, [id]: true },
      error: null,
      commandError: null,
    }));

    try {
      const snapshot = await disconnectDatabaseProfile(id);
      get().applySnapshot(snapshot);
      return true;
    } catch (error) {
      const commandError = commandErrorFor(error);
      set((state) => ({
        connectionStates: { ...state.connectionStates, [id]: "error" },
        snapshots: state.snapshots[id]
          ? {
              ...state.snapshots,
              [id]: {
                ...state.snapshots[id],
                state: "error",
                error: commandError ?? undefined,
              },
            }
          : state.snapshots,
        error: getCommandErrorMessage(error),
        commandError,
      }));
      return false;
    } finally {
      set((state) => ({
        busy: { ...state.busy, [id]: false },
      }));
    }
  },

  refreshConnectionState: async (id) => {
    if (!isTauriRuntime()) {
      return;
    }

    try {
      const snapshot = await getDatabaseConnectionState(id);
      get().applySnapshot(snapshot);
    } catch (error) {
      set({ error: getCommandErrorMessage(error), commandError: commandErrorFor(error) });
    }
  },

  applySnapshot: (snapshot) => {
    set((state) => {
      const snapshots = mergeSnapshot(state.snapshots, snapshot);
      const current = snapshots[snapshot.profileId] ?? snapshot;

      return {
        snapshots,
        connectionStates: {
          ...state.connectionStates,
          [snapshot.profileId]: current.state,
        },
      };
    });
  },

  clearError: () => set({ error: null, commandError: null }),
}));

async function runConnectionTransition(
  set: (
    partial:
      | Partial<DatabaseProfileStore>
      | ((state: DatabaseProfileStore) => Partial<DatabaseProfileStore>),
  ) => void,
  get: () => DatabaseProfileStore,
  id: string,
  pendingState: DatabaseConnectionState,
  command: () => Promise<DatabaseConnectionSnapshot>,
): Promise<boolean> {
  if (!isTauriRuntime()) {
    set({ error: "Database profiles are available in the desktop application." });
    return false;
  }

  const profile = findProfile(get().profiles, id);
  set((state) => ({
    busy: { ...state.busy, [id]: true },
    connectionStates: { ...state.connectionStates, [id]: pendingState },
    snapshots: profile
      ? {
          ...state.snapshots,
          [id]: pendingSnapshot(profile, pendingState),
        }
      : state.snapshots,
    error: null,
    commandError: null,
  }));

  try {
    const snapshot = await command();
    get().applySnapshot(snapshot);
    return true;
  } catch (error) {
    const commandError = commandErrorFor(error);
    set((state) => ({
      connectionStates: { ...state.connectionStates, [id]: "error" },
      snapshots: state.snapshots[id]
        ? {
            ...state.snapshots,
            [id]: {
              ...state.snapshots[id],
              state: "error",
              error: commandError ?? undefined,
            },
          }
        : profile
          ? {
              ...state.snapshots,
              [id]: { ...pendingSnapshot(profile, "error"), error: commandError ?? undefined },
            }
          : state.snapshots,
      error: getCommandErrorMessage(error),
      commandError,
    }));
    return false;
  } finally {
    set((state) => ({
      busy: { ...state.busy, [id]: false },
    }));
  }
}
