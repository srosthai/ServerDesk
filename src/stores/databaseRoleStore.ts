import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { getDatabaseRolePrivileges, listDatabaseRoles } from "@/services/databaseRoles";
import type {
  DatabaseRolePrivilegeSnapshot,
  DatabaseRoleRef,
  DatabaseRoleSummary,
} from "@/types/contracts";

export type DatabaseRoleEntry = {
  status: "idle" | "loading" | "ready" | "error";
  roles: DatabaseRoleSummary[];
  selectedKey: string | null;
  hasMore: boolean;
  error: string | null;
  snapshotStatus: "idle" | "loading" | "ready" | "error";
  snapshot: DatabaseRolePrivilegeSnapshot | null;
  snapshotError: string | null;
};

const EMPTY_ENTRY: DatabaseRoleEntry = {
  status: "idle",
  roles: [],
  selectedKey: null,
  hasMore: false,
  error: null,
  snapshotStatus: "idle",
  snapshot: null,
  snapshotError: null,
};

function roleKey(role: DatabaseRoleRef): string {
  return role.host ? `${role.name}@${role.host}` : role.name;
}

function entryFor(
  entries: Record<string, DatabaseRoleEntry>,
  profileId: string,
): DatabaseRoleEntry {
  return entries[profileId] ?? EMPTY_ENTRY;
}

function updateEntry(
  entries: Record<string, DatabaseRoleEntry>,
  profileId: string,
  update: Partial<DatabaseRoleEntry>,
): Record<string, DatabaseRoleEntry> {
  return {
    ...entries,
    [profileId]: { ...entryFor(entries, profileId), ...update },
  };
}

type DatabaseRoleStore = {
  entries: Record<string, DatabaseRoleEntry>;
  fetchRoles: (profileId: string) => Promise<void>;
  selectRole: (profileId: string, key: string | null) => void;
  fetchSnapshot: (profileId: string, role: DatabaseRoleRef) => Promise<void>;
  refresh: (profileId: string) => Promise<void>;
  reset: (profileId?: string) => void;
};

export const useDatabaseRoleStore = create<DatabaseRoleStore>((set, get) => ({
  entries: {},

  async fetchRoles(profileId) {
    set((state) => ({
      entries: updateEntry(state.entries, profileId, {
        status: "loading",
        error: null,
      }),
    }));
    try {
      const page = await listDatabaseRoles(profileId, { limit: 100, offset: 0 });
      set((state) => {
        const current = entryFor(state.entries, profileId);
        const selectedKey =
          current.selectedKey &&
          page.items.some(
            (role) => roleKey({ name: role.name, host: role.host }) === current.selectedKey,
          )
            ? current.selectedKey
            : page.items[0]
              ? roleKey({ name: page.items[0].name, host: page.items[0].host })
              : null;
        return {
          entries: updateEntry(state.entries, profileId, {
            status: "ready",
            roles: page.items,
            selectedKey,
            hasMore: page.hasMore,
            error: null,
          }),
        };
      });
    } catch (error) {
      set((state) => ({
        entries: updateEntry(state.entries, profileId, {
          status: "error",
          roles: [],
          selectedKey: null,
          hasMore: false,
          error: getCommandErrorMessage(error),
          snapshot: null,
          snapshotStatus: "idle",
          snapshotError: null,
        }),
      }));
    }
  },

  selectRole(profileId, key) {
    set((state) => ({
      entries: updateEntry(state.entries, profileId, {
        selectedKey: key,
        snapshot: null,
        snapshotStatus: "idle",
        snapshotError: null,
      }),
    }));
  },

  async fetchSnapshot(profileId, role) {
    set((state) => ({
      entries: updateEntry(state.entries, profileId, {
        snapshotStatus: "loading",
        snapshotError: null,
      }),
    }));
    try {
      const snapshot = await getDatabaseRolePrivileges(profileId, role);
      set((state) => ({
        entries: updateEntry(state.entries, profileId, {
          snapshotStatus: "ready",
          snapshot,
          snapshotError: null,
        }),
      }));
    } catch (error) {
      set((state) => ({
        entries: updateEntry(state.entries, profileId, {
          snapshotStatus: "error",
          snapshot: null,
          snapshotError: getCommandErrorMessage(error),
        }),
      }));
    }
  },

  async refresh(profileId) {
    await get().fetchRoles(profileId);
    const entry = entryFor(get().entries, profileId);
    const selected = entry.roles.find(
      (role) => roleKey({ name: role.name, host: role.host }) === entry.selectedKey,
    );
    if (selected) {
      await get().fetchSnapshot(profileId, {
        name: selected.name,
        host: selected.host ?? null,
      });
    }
  },

  reset(profileId) {
    if (!profileId) {
      set({ entries: {} });
      return;
    }
    set((state) => {
      const entries = { ...state.entries };
      delete entries[profileId];
      return { entries };
    });
  },
}));
