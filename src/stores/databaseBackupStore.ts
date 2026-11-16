import { create } from "zustand";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  cancelDatabaseBackup,
  createDatabaseBackup,
  createDatabaseBackupRequestId,
  getDatabaseBackupTools,
  previewDatabaseRestore,
  restoreDatabaseBackup,
} from "@/services/databaseBackup";
import type {
  CommandError,
  DatabaseBackupRequest,
  DatabaseBackupRestorePreview,
  DatabaseBackupRestorePreviewRequest,
  DatabaseBackupRestoreRequest,
  DatabaseBackupResult,
  DatabaseBackupRestoreResult,
  DatabaseDumpProgress,
  DatabaseDumpStatus,
  DatabaseDumpToolReport,
} from "@/types/contracts";

export type DatabaseBackupStoreStatus = "idle" | "running" | DatabaseDumpStatus | "error";

export type DatabaseBackupToolsState = {
  status: "idle" | "loading" | "ready" | "error";
  profileId?: string;
  report?: DatabaseDumpToolReport;
  error?: CommandError;
  errorMessage?: string;
};

export type DatabaseBackupPreviewState = {
  status: "idle" | "loading" | "ready" | "error";
  profileId?: string;
  preview?: DatabaseBackupRestorePreview;
  error?: CommandError;
  errorMessage?: string;
};

export type DatabaseBackupTransferState = {
  transferId?: string;
  /** Keeps an active transfer attached to the connection tab that started it. */
  profileId?: string;
  direction: "backup" | "restore";
  status: DatabaseBackupStoreStatus;
  progress?: DatabaseDumpProgress;
  backupResult?: DatabaseBackupResult;
  restoreResult?: DatabaseBackupRestoreResult;
  error?: CommandError;
  errorMessage?: string;
  cancelling: boolean;
  startedAt?: number;
};

type DatabaseBackupStore = {
  tools: DatabaseBackupToolsState;
  preview: DatabaseBackupPreviewState;
  transfer: DatabaseBackupTransferState;
  detectTools: (profileId: string) => Promise<DatabaseDumpToolReport | null>;
  previewRestore: (
    profileId: string,
    request: DatabaseBackupRestorePreviewRequest,
  ) => Promise<DatabaseBackupRestorePreview | null>;
  startBackup: (
    profileId: string,
    request: DatabaseBackupRequest,
  ) => Promise<DatabaseBackupResult | null>;
  startRestore: (
    profileId: string,
    request: DatabaseBackupRestoreRequest,
  ) => Promise<DatabaseBackupRestoreResult | null>;
  cancelTransfer: () => Promise<boolean>;
  applyTransferProgress: (progress: DatabaseDumpProgress) => void;
  clearPreview: () => void;
  dismissTransfer: () => void;
};

const idleTransfer = (): DatabaseBackupTransferState => ({
  direction: "backup",
  status: "idle",
  cancelling: false,
});

function commandErrorFor(error: unknown): CommandError | null {
  return isCommandError(error) ? error : null;
}

function failureState(
  direction: "backup" | "restore",
  error: unknown,
  transferId?: string,
  profileId?: string,
  startedAt?: number,
): DatabaseBackupTransferState {
  const commandError = commandErrorFor(error);
  return {
    transferId,
    profileId,
    direction,
    status: "error",
    error: commandError ?? undefined,
    errorMessage: getCommandErrorMessage(error),
    cancelling: false,
    startedAt,
  };
}

function confirmationMissing(): CommandError {
  return {
    code: "validation",
    message:
      "Restoring a backup requires an acknowledged confirmation that names the server, target database, and replaced data.",
  };
}

export const useDatabaseBackupStore = create<DatabaseBackupStore>((set, get) => ({
  tools: { status: "idle" },
  preview: { status: "idle" },
  transfer: idleTransfer(),

  detectTools: async (profileId) => {
    if (!isTauriRuntime()) {
      set({
        tools: {
          status: "error",
          profileId,
          errorMessage: "Backup tools are available in the desktop application.",
        },
      });
      return null;
    }
    set({ tools: { status: "loading", profileId } });
    try {
      const report = await getDatabaseBackupTools(profileId);
      set({ tools: { status: "ready", profileId, report } });
      return report;
    } catch (error) {
      const commandError = commandErrorFor(error);
      set({
        tools: {
          status: "error",
          profileId,
          error: commandError ?? undefined,
          errorMessage: getCommandErrorMessage(error),
        },
      });
      return null;
    }
  },

  previewRestore: async (profileId, request) => {
    if (!isTauriRuntime()) {
      set({
        preview: {
          status: "error",
          profileId,
          errorMessage: "Backup restore previews are available in the desktop application.",
        },
      });
      return null;
    }
    set({ preview: { status: "loading", profileId } });
    try {
      const preview = await previewDatabaseRestore(profileId, request);
      set({ preview: { status: "ready", profileId, preview } });
      return preview;
    } catch (error) {
      const commandError = commandErrorFor(error);
      set({
        preview: {
          status: "error",
          profileId,
          error: commandError ?? undefined,
          errorMessage: getCommandErrorMessage(error),
        },
      });
      return null;
    }
  },

  startBackup: async (profileId, request) => {
    if (get().transfer.status === "running") {
      return null;
    }
    if (!isTauriRuntime()) {
      set({
        transfer: failureState(
          "backup",
          new Error("Backup creation is available in the desktop application."),
          undefined,
          profileId,
        ),
      });
      return null;
    }
    const transferId = createDatabaseBackupRequestId();
    const startedAt = Date.now();
    set({
      transfer: {
        transferId,
        profileId,
        direction: "backup",
        status: "running",
        cancelling: false,
        startedAt,
      },
    });
    try {
      const result = await createDatabaseBackup(profileId, request, transferId);
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({
        transfer: {
          transferId,
          profileId,
          direction: "backup",
          status: result.status,
          backupResult: result,
          cancelling: false,
          startedAt,
        },
      });
      return result;
    } catch (error) {
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({ transfer: failureState("backup", error, transferId, profileId, startedAt) });
      return null;
    }
  },

  startRestore: async (profileId, request) => {
    if (get().transfer.status === "running") {
      return null;
    }
    if (!request.confirmed || !request.confirmation?.acknowledged) {
      set({ transfer: failureState("restore", confirmationMissing()) });
      return null;
    }
    if (!isTauriRuntime()) {
      set({
        transfer: failureState(
          "restore",
          new Error("Backup restore is available in the desktop application."),
          undefined,
          profileId,
        ),
      });
      return null;
    }
    const transferId = createDatabaseBackupRequestId();
    const startedAt = Date.now();
    set({
      transfer: {
        transferId,
        profileId,
        direction: "restore",
        status: "running",
        cancelling: false,
        startedAt,
      },
    });
    try {
      const result = await restoreDatabaseBackup(profileId, request, transferId);
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({
        transfer: {
          transferId,
          profileId,
          direction: "restore",
          status: result.status,
          restoreResult: result,
          cancelling: false,
          startedAt,
        },
      });
      return result;
    } catch (error) {
      if (get().transfer.transferId !== transferId) {
        return null;
      }
      set({ transfer: failureState("restore", error, transferId, profileId, startedAt) });
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
      const result = await cancelDatabaseBackup(transferId);
      if (!result.cancelled) {
        set((state) => ({
          transfer: { ...state.transfer, cancelling: false },
        }));
      }
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
