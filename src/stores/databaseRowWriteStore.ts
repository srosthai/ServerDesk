import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { deleteTableRows, insertTableRow, updateTableRow } from "@/services/databaseRowWrites";
import { databaseRowKey } from "@/stores/databaseRowStore";
import type {
  CommandError,
  DatabaseRowIdentityValue,
  DatabaseRowTable,
  DatabaseRowWriteResult,
  DatabaseRowWriteValue,
} from "@/types/contracts";

export type DatabaseRowWriteStatus = "idle" | "running" | "succeeded" | "failed";

export type DatabaseRowWriteEntry = {
  status: DatabaseRowWriteStatus;
  result?: DatabaseRowWriteResult;
  error?: CommandError;
  errorMessage?: string;
  /** Local time of the last completed attempt. */
  updatedAt?: number;
};

type DatabaseRowWriteStore = {
  entries: Record<string, DatabaseRowWriteEntry>;
  insertRow: (
    profileId: string,
    table: DatabaseRowTable,
    values: DatabaseRowWriteValue[],
  ) => Promise<DatabaseRowWriteResult | null>;
  updateRow: (
    profileId: string,
    table: DatabaseRowTable,
    identity: DatabaseRowIdentityValue[],
    values: DatabaseRowWriteValue[],
    expected: DatabaseRowIdentityValue[],
  ) => Promise<DatabaseRowWriteResult | null>;
  deleteRows: (
    profileId: string,
    table: DatabaseRowTable,
    targets: DatabaseRowIdentityValue[][],
    confirmed: boolean,
  ) => Promise<DatabaseRowWriteResult | null>;
  clearEntry: (profileId: string, table: DatabaseRowTable) => void;
  resetProfile: (profileId: string) => void;
};

function commandErrorFor(error: unknown): CommandError | null {
  return isCommandError(error) ? error : null;
}

function failedEntry(error: unknown): DatabaseRowWriteEntry {
  const commandError = commandErrorFor(error);
  return {
    status: "failed",
    error: commandError ?? undefined,
    errorMessage: commandError?.message ?? getCommandErrorMessage(error),
    updatedAt: Date.now(),
  };
}

async function runWrite(
  profileId: string,
  table: DatabaseRowTable,
  set: (updater: (state: DatabaseRowWriteStore) => Partial<DatabaseRowWriteStore>) => void,
  write: () => Promise<DatabaseRowWriteResult>,
): Promise<DatabaseRowWriteResult | null> {
  const key = databaseRowKey(profileId, table);
  if (!isTauriRuntime()) {
    set((state) => ({
      entries: {
        ...state.entries,
        [key]: failedEntry({
          code: "connection",
          message: "Row editing is available in the desktop application.",
        }),
      },
    }));
    return null;
  }

  set((state) => ({
    entries: {
      ...state.entries,
      [key]: { status: "running" },
    },
  }));

  try {
    const result = await write();
    set((state) => ({
      entries: {
        ...state.entries,
        [key]: {
          status: "succeeded",
          result,
          updatedAt: Date.now(),
        },
      },
    }));
    return result;
  } catch (error) {
    set((state) => ({
      entries: {
        ...state.entries,
        [key]: failedEntry(error),
      },
    }));
    return null;
  }
}

export const useDatabaseRowWriteStore = create<DatabaseRowWriteStore>((set) => ({
  entries: {},

  insertRow: (profileId, table, values) =>
    runWrite(profileId, table, set, () => insertTableRow(profileId, table, values)),

  updateRow: (profileId, table, identity, values, expected) =>
    runWrite(profileId, table, set, () =>
      updateTableRow(profileId, table, identity, values, expected),
    ),

  deleteRows: (profileId, table, targets, confirmed) =>
    runWrite(profileId, table, set, () => deleteTableRows(profileId, table, targets, confirmed)),

  clearEntry: (profileId, table) => {
    const key = databaseRowKey(profileId, table);
    set((state) => {
      const entries = { ...state.entries };
      delete entries[key];
      return { entries };
    });
  },

  resetProfile: (profileId) => {
    const prefix = `${profileId}::`;
    set((state) => ({
      entries: Object.fromEntries(
        Object.entries(state.entries).filter(([key]) => !key.startsWith(prefix)),
      ),
    }));
  },
}));
