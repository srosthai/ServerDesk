import { create } from "zustand";
import { getCommandErrorDetails, getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isRemotePathWithin, normalizeRemotePath, remoteParentPath } from "@/lib/remotePaths";
import { isTauriRuntime } from "@/lib/tauri";
import {
  browseRemoteFiles,
  cancelFileTransfer,
  createRemoteFolder,
  createTransferId,
  deleteRemoteEntry,
  downloadRemoteFile,
  renameRemoteEntry,
  uploadRemoteFile,
} from "@/services/files";
import type {
  ErrorCode,
  FileTransferDirection,
  FileTransferKind,
  FileTransferProgress,
  FileTransferResult,
  FileTransferStatus,
  RemoteFileListing,
  RemoteMutationResult,
} from "@/types/contracts";

export type FileBrowserEntry = {
  root: string | null;
  path: string | null;
  listing: RemoteFileListing | null;
  pending: boolean;
  error: string | null;
  errorDetails: string | null;
  lastAttemptAt: number | null;
};

export type FileTransferRequest =
  | {
      direction: "upload";
      serverId: string;
      root: string;
      localPath: string;
      remotePath: string;
      overwrite: boolean;
    }
  | {
      direction: "download";
      serverId: string;
      root: string;
      remotePath: string;
      localPath: string;
      overwrite: boolean;
    };

export type FileTransferState = {
  transferId: string;
  serverId: string;
  direction: FileTransferDirection;
  kind: FileTransferKind;
  status: FileTransferStatus;
  bytesTransferred: number;
  totalBytes: number | null;
  filesTransferred: number;
  filesTotal: number | null;
  currentPath: string | null;
  mayBeIncomplete: boolean;
  partialPaths: string[];
  warnings: string[];
  error: string | null;
  errorDetails: string | null;
  errorCode: ErrorCode | null;
  cancelling: boolean;
  startedAt: number;
  finishedAt: number | null;
  request: FileTransferRequest;
};

export type FileMutationState = {
  pending: boolean;
  result: RemoteMutationResult | null;
  error: string | null;
  errorDetails: string | null;
  errorCode: ErrorCode | null;
};

export type UploadStartInput = {
  serverId: string;
  root: string;
  localPath: string;
  remotePath: string;
  overwrite?: boolean;
};

export type DownloadStartInput = {
  serverId: string;
  root: string;
  remotePath: string;
  localPath: string;
  overwrite?: boolean;
};

type FileBrowserStore = {
  entries: Record<string, FileBrowserEntry>;
  transfers: Record<string, FileTransferState>;
  mutations: Record<string, FileMutationState>;
  browse: (serverId: string, root: string, path: string) => Promise<boolean>;
  openRoot: (serverId: string, path: string) => Promise<boolean>;
  refresh: (serverId: string) => Promise<boolean>;
  goUp: (serverId: string) => Promise<boolean>;
  clearError: (serverId: string) => void;
  clearServer: (serverId: string) => void;
  startUpload: (input: UploadStartInput) => Promise<string>;
  startDownload: (input: DownloadStartInput) => Promise<string>;
  cancelTransfer: (transferId: string) => Promise<boolean>;
  retryTransfer: (transferId: string) => Promise<string | null>;
  applyTransferProgress: (progress: FileTransferProgress) => void;
  dismissTransfer: (transferId: string) => void;
  createFolder: (serverId: string, root: string, parent: string, name: string) => Promise<boolean>;
  renameEntry: (serverId: string, root: string, path: string, newName: string) => Promise<boolean>;
  deleteEntry: (
    serverId: string,
    root: string,
    path: string,
    recursive: boolean,
  ) => Promise<boolean>;
  clearMutation: (serverId: string) => void;
};

export function emptyFileBrowserEntry(): FileBrowserEntry {
  return {
    root: null,
    path: null,
    listing: null,
    pending: false,
    error: null,
    errorDetails: null,
    lastAttemptAt: null,
  };
}

export function emptyFileMutationState(): FileMutationState {
  return {
    pending: false,
    result: null,
    error: null,
    errorDetails: null,
    errorCode: null,
  };
}

function commandErrorCode(error: unknown): ErrorCode | null {
  return isCommandError(error) ? error.code : null;
}

function transferFromResult(
  current: FileTransferState,
  result: FileTransferResult,
): FileTransferState {
  return {
    ...current,
    kind: result.kind,
    status: result.status,
    bytesTransferred: result.bytesTransferred,
    totalBytes: result.totalBytes,
    filesTransferred: result.filesTransferred,
    filesTotal: result.filesTotal,
    mayBeIncomplete: result.mayBeIncomplete,
    partialPaths: result.partialPaths,
    warnings: result.warnings,
    error: result.error ? result.error.message : null,
    errorDetails: result.error?.details ?? null,
    errorCode: result.error?.code ?? null,
    cancelling: false,
    finishedAt: result.finishedAt,
  };
}

export const useFileStore = create<FileBrowserStore>((set, get) => {
  let requestCounter = 0;
  const requestIds: Record<string, number> = {};

  function updateEntry(serverId: string, update: (entry: FileBrowserEntry) => FileBrowserEntry) {
    set((state) => ({
      entries: {
        ...state.entries,
        [serverId]: update(state.entries[serverId] ?? emptyFileBrowserEntry()),
      },
    }));
  }

  function updateMutation(
    serverId: string,
    update: (entry: FileMutationState) => FileMutationState,
  ) {
    set((state) => ({
      mutations: {
        ...state.mutations,
        [serverId]: update(state.mutations[serverId] ?? emptyFileMutationState()),
      },
    }));
  }

  function updateTransfer(
    transferId: string,
    update: (entry: FileTransferState) => FileTransferState,
  ) {
    set((state) => {
      const current = state.transfers[transferId];
      if (!current) {
        return state;
      }
      return {
        transfers: {
          ...state.transfers,
          [transferId]: update(current),
        },
      };
    });
  }

  async function browse(serverId: string, root: string, path: string): Promise<boolean> {
    const normalizedRoot = normalizeRemotePath(root);
    const normalizedPath = normalizeRemotePath(path);

    if (!normalizedRoot || !normalizedPath) {
      updateEntry(serverId, (entry) => ({
        ...entry,
        error:
          "Enter an absolute, normalized remote path such as /var/www. Relative paths, parent segments, and control characters are rejected.",
        errorDetails: null,
        lastAttemptAt: Date.now(),
      }));
      return false;
    }

    if (!isRemotePathWithin(normalizedRoot, normalizedPath)) {
      updateEntry(serverId, (entry) => ({
        ...entry,
        error: "The requested path is outside the selected root directory.",
        errorDetails: "Browsing is limited to the selected root and its descendants.",
        lastAttemptAt: Date.now(),
      }));
      return false;
    }

    if (!isTauriRuntime()) {
      return false;
    }

    const requestId = ++requestCounter;
    requestIds[serverId] = requestId;
    updateEntry(serverId, (entry) => ({
      ...entry,
      pending: true,
      error: null,
      errorDetails: null,
    }));

    try {
      const listing = await browseRemoteFiles(serverId, normalizedRoot, normalizedPath);
      if (requestIds[serverId] !== requestId) {
        return false;
      }
      updateEntry(serverId, (entry) => ({
        ...entry,
        root: listing.root,
        path: listing.path,
        listing,
        pending: false,
        error: null,
        errorDetails: null,
        lastAttemptAt: Date.now(),
      }));
      return true;
    } catch (error) {
      if (requestIds[serverId] === requestId) {
        updateEntry(serverId, (entry) => ({
          ...entry,
          // Keep the last successful listing visible with its original sample
          // time. A failed navigation never replaces real metadata with a
          // fabricated empty directory.
          pending: false,
          error: getCommandErrorMessage(error),
          errorDetails: getCommandErrorDetails(error),
          lastAttemptAt: Date.now(),
        }));
      }
      return false;
    }
  }

  async function runTransfer(request: FileTransferRequest): Promise<string> {
    const transferId = createTransferId();
    const startedAt = Date.now();
    const initialState: FileTransferState = {
      transferId,
      serverId: request.serverId,
      direction: request.direction,
      kind: "file",
      status: "running",
      bytesTransferred: 0,
      totalBytes: null,
      filesTransferred: 0,
      filesTotal: null,
      currentPath: null,
      mayBeIncomplete: false,
      partialPaths: [],
      warnings: [],
      error: null,
      errorDetails: null,
      errorCode: null,
      cancelling: false,
      startedAt,
      finishedAt: null,
      request,
    };

    if (!isTauriRuntime()) {
      set((state) => ({
        transfers: {
          ...state.transfers,
          [transferId]: {
            ...initialState,
            status: "failed",
            error: "File transfers are available in the desktop application.",
            finishedAt: Date.now(),
          },
        },
      }));
      return transferId;
    }

    set((state) => ({
      transfers: { ...state.transfers, [transferId]: initialState },
    }));

    try {
      const result =
        request.direction === "upload"
          ? await uploadRemoteFile({
              transferId,
              serverId: request.serverId,
              root: request.root,
              localPath: request.localPath,
              remotePath: request.remotePath,
              overwrite: request.overwrite,
            })
          : await downloadRemoteFile({
              transferId,
              serverId: request.serverId,
              root: request.root,
              remotePath: request.remotePath,
              localPath: request.localPath,
              overwrite: request.overwrite,
            });
      updateTransfer(transferId, (current) => transferFromResult(current, result));
    } catch (error) {
      updateTransfer(transferId, (current) => ({
        ...current,
        status: "failed",
        error: getCommandErrorMessage(error),
        errorDetails: getCommandErrorDetails(error),
        errorCode: commandErrorCode(error),
        mayBeIncomplete: false,
        cancelling: false,
        finishedAt: Date.now(),
      }));
    }

    return transferId;
  }

  async function refreshCurrent(serverId: string) {
    const entry = get().entries[serverId];
    if (!entry?.root || !entry.path) {
      return;
    }
    await browse(serverId, entry.root, entry.path);
  }

  async function runMutation(
    serverId: string,
    operation: () => Promise<RemoteMutationResult>,
  ): Promise<boolean> {
    updateMutation(serverId, () => ({ ...emptyFileMutationState(), pending: true }));
    try {
      const result = await operation();
      updateMutation(serverId, () => ({
        ...emptyFileMutationState(),
        result,
      }));
      await refreshCurrent(serverId);
      return true;
    } catch (error) {
      updateMutation(serverId, () => ({
        ...emptyFileMutationState(),
        error: getCommandErrorMessage(error),
        errorDetails: getCommandErrorDetails(error),
        errorCode: commandErrorCode(error),
      }));
      return false;
    }
  }

  return {
    entries: {},
    transfers: {},
    mutations: {},

    browse,

    openRoot: (serverId, path) => browse(serverId, path, path),

    refresh: (serverId) => {
      const entry = get().entries[serverId];
      if (!entry?.root || !entry.path) {
        return Promise.resolve(false);
      }
      return browse(serverId, entry.root, entry.path);
    },

    goUp: (serverId) => {
      const entry = get().entries[serverId];
      if (!entry?.root || !entry.path) {
        return Promise.resolve(false);
      }
      const parent = remoteParentPath(entry.root, entry.path);
      if (!parent) {
        return Promise.resolve(false);
      }
      return browse(serverId, entry.root, parent);
    },

    clearError: (serverId) =>
      updateEntry(serverId, (entry) => ({
        ...entry,
        error: null,
        errorDetails: null,
      })),

    clearServer: (serverId) => {
      requestIds[serverId] = ++requestCounter;
      set((state) => {
        const entries = { ...state.entries };
        delete entries[serverId];
        const mutations = { ...state.mutations };
        delete mutations[serverId];
        const transfers = Object.fromEntries(
          Object.entries(state.transfers).filter(([, transfer]) => transfer.serverId !== serverId),
        );
        return { entries, mutations, transfers };
      });
    },

    startUpload: (input) =>
      runTransfer({
        direction: "upload",
        serverId: input.serverId,
        root: input.root,
        localPath: input.localPath,
        remotePath: input.remotePath,
        overwrite: input.overwrite ?? false,
      }),

    startDownload: (input) =>
      runTransfer({
        direction: "download",
        serverId: input.serverId,
        root: input.root,
        remotePath: input.remotePath,
        localPath: input.localPath,
        overwrite: input.overwrite ?? false,
      }),

    cancelTransfer: async (transferId) => {
      const transfer = get().transfers[transferId];
      if (!transfer || transfer.status !== "running") {
        return false;
      }
      try {
        const accepted = await cancelFileTransfer(transferId);
        if (accepted) {
          updateTransfer(transferId, (current) => ({ ...current, cancelling: true }));
        }
        return accepted;
      } catch (error) {
        updateTransfer(transferId, (current) => ({
          ...current,
          error: getCommandErrorMessage(error),
          errorDetails: getCommandErrorDetails(error),
        }));
        return false;
      }
    },

    retryTransfer: async (transferId) => {
      const transfer = get().transfers[transferId];
      if (!transfer) {
        return null;
      }
      return runTransfer({ ...transfer.request, overwrite: false });
    },

    applyTransferProgress: (progress) => {
      updateTransfer(progress.transferId, (current) => {
        // The final command result is authoritative. Once it has been applied,
        // late progress events must not downgrade or duplicate the outcome.
        if (current.status !== "running") {
          return current;
        }
        return {
          ...current,
          kind: progress.kind,
          status: progress.status,
          bytesTransferred: Math.max(current.bytesTransferred, progress.bytesTransferred),
          totalBytes: progress.totalBytes ?? current.totalBytes,
          filesTransferred: Math.max(current.filesTransferred, progress.filesTransferred),
          filesTotal: progress.filesTotal ?? current.filesTotal,
          currentPath: progress.currentPath ?? current.currentPath,
          mayBeIncomplete: current.mayBeIncomplete || progress.mayBeIncomplete,
        };
      });
    },

    dismissTransfer: (transferId) =>
      set((state) => {
        const transfers = { ...state.transfers };
        delete transfers[transferId];
        return { transfers };
      }),

    createFolder: (serverId, root, parent, name) =>
      runMutation(serverId, () => createRemoteFolder({ serverId, root, parent, name })),

    renameEntry: (serverId, root, path, newName) =>
      runMutation(serverId, () =>
        renameRemoteEntry({ serverId, root, path, newName, confirmed: true }),
      ),

    deleteEntry: (serverId, root, path, recursive) =>
      runMutation(serverId, () =>
        deleteRemoteEntry({ serverId, root, path, confirmed: true, recursive }),
      ),

    clearMutation: (serverId) =>
      set((state) => {
        const mutations = { ...state.mutations };
        delete mutations[serverId];
        return { mutations };
      }),
  };
});
