import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  cancelDatabaseDump,
  createDatabaseDumpRequestId,
  exportDatabaseDump,
  getDatabaseDumpTools,
  importDatabaseDump,
} from "@/services/databaseDump";
import type {
  CommandError,
  DatabaseDumpExportRequest,
  DatabaseDumpExportResult,
  DatabaseDumpProgress,
  DatabaseDumpRestoreRequest,
  DatabaseDumpRestoreResult,
  DatabaseDumpStatus,
  DatabaseDumpToolReport,
} from "@/types/contracts";

export type DatabaseDumpStoreStatus = "idle" | "running" | DatabaseDumpStatus | "error";

export type DatabaseDumpToolsState = {
  status: "idle" | "loading" | "ready" | "error";
  report?: DatabaseDumpToolReport;
  error?: CommandError;
  errorMessage?: string;
};

export type DatabaseDumpTransferState = {
  transferId?: string;
  direction: "export" | "restore";
  status: DatabaseDumpStoreStatus;
  progress?: DatabaseDumpProgress;
  exportResult?: DatabaseDumpExportResult;
  restoreResult?: DatabaseDumpRestoreResult;
  error?: CommandError;
  errorMessage?: string;
  cancelling: boolean;
};

type DatabaseDumpStore = {
  tools: DatabaseDumpToolsState;
  transfer: DatabaseDumpTransferState;
  detectTools: (profileId: string) => Promise<DatabaseDumpToolReport | null>;
  startExport: (
    profileId: string,
    request: DatabaseDumpExportRequest,
  ) => Promise<DatabaseDumpExportResult | null>;
  startRestore: (
    profileId: string,
    request: DatabaseDumpRestoreRequest,
  ) => Promise<DatabaseDumpRestoreResult | null>;
  cancelTransfer: () => Promise<boolean>;
  applyTransferProgress: (progress: DatabaseDumpProgress) => void;
  clearTools: () => void;
  dismissTransfer: () => void;
};

const idleTransfer = (): DatabaseDumpTransferState => ({
  direction: "export",
  status: "idle",
  cancelling: false,
});

function commandErrorFor(error: unknown): CommandError | null {
  return isCommandError(error) ? error : null;
}

function failureState(
  direction: "export" | "restore",
  error: unknown,
  transferId?: string,
): DatabaseDumpTransferState {
  const commandError = commandErrorFor(error);
  return {
    transferId,
    direction,
    status: "error",
    error: commandError ?? undefined,
    errorMessage: getCommandErrorMessage(error),
    cancelling: false,
  };
}

export const useDatabaseDumpStore = create<DatabaseDumpStore>((set, get) => ({
  tools: { status: "idle" },
  transfer: idleTransfer(),

  detectTools: async (profileId) => {
    if (!isTauriRuntime()) {
      set({
        tools: {
          status: "error",
          errorMessage: "SQL dump tools are available in the desktop application.",
        },
      });
      return null;
    }
    set({ tools: { status: "loading" } });
    try {
      const report = await getDatabaseDumpTools(profileId);
      set({ tools: { status: "ready", report } });
      return report;
    } catch (error) {
      const commandError = commandErrorFor(error);
      set({
        tools: {
          status: "error",
          error: commandError ?? undefined,
          errorMessage: getCommandErrorMessage(error),
        },
      });
      return null;
    }
  },

  startExport: async (profileId, request) => {
    if (get().transfer.status === "running") {
      return null;
    }
    if (!isTauriRuntime()) {
      set({
        transfer: failureState(
          "export",
          new Error("SQL dump export is available in the desktop application."),
        ),
      });
      return null;
    }
    const transferId = createDatabaseDumpRequestId();
    set({
      transfer: {
        transferId,
        direction: "export",
        status: "running",
        cancelling: false,
      },
    });
    try {
      const result = await exportDatabaseDump(profileId, request, transferId);
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({
        transfer: {
          transferId,
          direction: "export",
          status: result.status,
          exportResult: result,
          cancelling: false,
        },
      });
      return result;
    } catch (error) {
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({ transfer: failureState("export", error, transferId) });
      return null;
    }
  },

  startRestore: async (profileId, request) => {
    if (get().transfer.status === "running") {
      return null;
    }
    if (!isTauriRuntime()) {
      set({
        transfer: failureState(
          "restore",
          new Error("SQL dump restore is available in the desktop application."),
        ),
      });
      return null;
    }
    const transferId = createDatabaseDumpRequestId();
    set({
      transfer: {
        transferId,
        direction: "restore",
        status: "running",
        cancelling: false,
      },
    });
    try {
      const result = await importDatabaseDump(profileId, request, transferId);
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({
        transfer: {
          transferId,
          direction: "restore",
          status: result.status,
          restoreResult: result,
          cancelling: false,
        },
      });
      return result;
    } catch (error) {
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({ transfer: failureState("restore", error, transferId) });
      return null;
    }
  },

  cancelTransfer: async () => {
    const transferId = get().transfer.transferId;
    if (!transferId || get().transfer.status !== "running") {
      return false;
    }
    set((state) => ({
      transfer: { ...state.transfer, cancelling: true },
    }));
    try {
      const result = await cancelDatabaseDump(transferId);
      return result.cancelled;
    } catch {
      set((state) => ({
        transfer: { ...state.transfer, cancelling: false },
      }));
      return false;
    }
  },

  applyTransferProgress: (progress) => {
    if (get().transfer.transferId !== progress.transferId) {
      return;
    }
    set((state) => ({
      transfer: {
        ...state.transfer,
        status: progress.status,
        progress,
      },
    }));
  },

  clearTools: () => set({ tools: { status: "idle" } }),

  dismissTransfer: () => set({ transfer: idleTransfer() }),
}));
