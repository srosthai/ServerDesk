import { create } from "zustand";
import { getCommandErrorDetails, getCommandErrorMessage } from "@/lib/errors";
import { isRemotePathWithin, normalizeRemotePath, remoteParentPath } from "@/lib/remotePaths";
import { isTauriRuntime } from "@/lib/tauri";
import { browseRemoteFiles } from "@/services/files";
import type { RemoteFileListing } from "@/types/contracts";

export type FileBrowserEntry = {
  root: string | null;
  path: string | null;
  listing: RemoteFileListing | null;
  pending: boolean;
  error: string | null;
  errorDetails: string | null;
  lastAttemptAt: number | null;
};

type FileBrowserStore = {
  entries: Record<string, FileBrowserEntry>;
  browse: (serverId: string, root: string, path: string) => Promise<boolean>;
  openRoot: (serverId: string, path: string) => Promise<boolean>;
  refresh: (serverId: string) => Promise<boolean>;
  goUp: (serverId: string) => Promise<boolean>;
  clearError: (serverId: string) => void;
  clearServer: (serverId: string) => void;
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

  return {
    entries: {},

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
        return { entries };
      });
    },
  };
});
