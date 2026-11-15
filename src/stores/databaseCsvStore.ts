import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  cancelDatabaseCsvTransfer,
  createDatabaseCsvRequestId,
  exportDatabaseCsv,
  importDatabaseCsv,
  previewDatabaseCsvImport,
} from "@/services/databaseCsv";
import type {
  CommandError,
  DatabaseCsvExportRequest,
  DatabaseCsvExportResult,
  DatabaseCsvImportOptions,
  DatabaseCsvImportPreview,
  DatabaseCsvImportRequest,
  DatabaseCsvImportResult,
  DatabaseCsvTransferDirection,
  DatabaseCsvTransferProgress,
  DatabaseCsvTransferStatus,
} from "@/types/contracts";

export type DatabaseCsvTransferStoreStatus =
  "idle" | "running" | DatabaseCsvTransferStatus | "error";

export type DatabaseCsvPreviewState = {
  status: "idle" | "loading" | "ready" | "error";
  preview?: DatabaseCsvImportPreview;
  error?: CommandError;
  errorMessage?: string;
};

export type DatabaseCsvTransferState = {
  transferId?: string;
  direction: DatabaseCsvTransferDirection;
  status: DatabaseCsvTransferStoreStatus;
  progress?: DatabaseCsvTransferProgress;
  exportResult?: DatabaseCsvExportResult;
  importResult?: DatabaseCsvImportResult;
  error?: CommandError;
  errorMessage?: string;
  cancelling: boolean;
};

type DatabaseCsvStore = {
  preview: DatabaseCsvPreviewState;
  transfer: DatabaseCsvTransferState;
  previewImport: (
    sourcePath: string,
    options: DatabaseCsvImportOptions,
    limit?: number,
  ) => Promise<DatabaseCsvImportPreview | null>;
  startExport: (
    profileId: string,
    request: DatabaseCsvExportRequest,
  ) => Promise<DatabaseCsvExportResult | null>;
  startImport: (
    profileId: string,
    request: DatabaseCsvImportRequest,
  ) => Promise<DatabaseCsvImportResult | null>;
  cancelTransfer: () => Promise<boolean>;
  applyTransferProgress: (progress: DatabaseCsvTransferProgress) => void;
  clearPreview: () => void;
  dismissTransfer: () => void;
};

const idleTransfer = (): DatabaseCsvTransferState => ({
  direction: "export",
  status: "idle",
  cancelling: false,
});

function commandErrorFor(error: unknown): CommandError | null {
  return isCommandError(error) ? error : null;
}

function failureState(
  direction: DatabaseCsvTransferDirection,
  error: unknown,
  transferId?: string,
): DatabaseCsvTransferState {
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

export const useDatabaseCsvStore = create<DatabaseCsvStore>((set, get) => ({
  preview: { status: "idle" },
  transfer: idleTransfer(),

  previewImport: async (sourcePath, options, limit) => {
    if (!isTauriRuntime()) {
      set({
        preview: {
          status: "error",
          errorMessage: "CSV preview is available in the desktop application.",
        },
      });
      return null;
    }
    set({ preview: { status: "loading" } });
    try {
      const preview = await previewDatabaseCsvImport(sourcePath, options, limit);
      set({ preview: { status: "ready", preview } });
      return preview;
    } catch (error) {
      const commandError = commandErrorFor(error);
      set({
        preview: {
          status: "error",
          error: commandError ?? undefined,
          errorMessage: getCommandErrorMessage(error),
        },
      });
      return null;
    }
  },

  startExport: async (profileId, request) => {
    const current = get().transfer;
    if (current.status === "running") {
      return null;
    }
    if (!isTauriRuntime()) {
      set({
        transfer: failureState(
          "export",
          new Error("CSV export is available in the desktop application."),
        ),
      });
      return null;
    }
    const transferId = createDatabaseCsvRequestId();
    set({
      transfer: {
        transferId,
        direction: "export",
        status: "running",
        cancelling: false,
      },
    });
    try {
      const result = await exportDatabaseCsv(profileId, request, transferId);
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

  startImport: async (profileId, request) => {
    const current = get().transfer;
    if (current.status === "running") {
      return null;
    }
    if (!isTauriRuntime()) {
      set({
        transfer: failureState(
          "import",
          new Error("CSV import is available in the desktop application."),
        ),
      });
      return null;
    }
    const transferId = createDatabaseCsvRequestId();
    set({
      transfer: {
        transferId,
        direction: "import",
        status: "running",
        cancelling: false,
      },
    });
    try {
      const result = await importDatabaseCsv(profileId, request, transferId);
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({
        transfer: {
          transferId,
          direction: "import",
          status: result.status,
          importResult: result,
          cancelling: false,
        },
      });
      return result;
    } catch (error) {
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({ transfer: failureState("import", error, transferId) });
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
      const result = await cancelDatabaseCsvTransfer(transferId);
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

  clearPreview: () => set({ preview: { status: "idle" } }),

  dismissTransfer: () => set({ transfer: idleTransfer() }),
}));
