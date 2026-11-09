import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  browseRemoteFiles,
  cancelFileTransfer,
  createRemoteFolder,
  createTransferId,
  deleteRemoteEntry,
  renameRemoteEntry,
  uploadRemoteFile,
} from "@/services/files";
import {
  canNavigateBack,
  canNavigateForward,
  emptyFileBrowserEntry,
  useFileStore,
} from "@/stores/fileStore";
import type {
  FileTransferResult,
  RemoteFileListing,
  RemoteMutationResult,
} from "@/types/contracts";

vi.mock("@/services/files", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/files")>();
  return {
    ...actual,
    browseRemoteFiles: vi.fn(),
    uploadRemoteFile: vi.fn(),
    downloadRemoteFile: vi.fn(),
    cancelFileTransfer: vi.fn(),
    createRemoteFolder: vi.fn(),
    renameRemoteEntry: vi.fn(),
    deleteRemoteEntry: vi.fn(),
    createTransferId: vi.fn(() => "transfer-test-1"),
  };
});

function listing(overrides: Partial<RemoteFileListing> = {}): RemoteFileListing {
  return {
    serverId: "server-1",
    root: "/var/www",
    path: "/var/www",
    resolvedPath: "/var/www",
    entries: [
      {
        name: "app",
        path: "/var/www/app",
        entryType: "directory",
        size: 4096,
        permissions: "drwxr-xr-x",
        mode: 0o040755,
        modifiedAt: 1_762_665_720,
        brokenSymlink: false,
      },
    ],
    truncated: false,
    sampledAt: 1_762_665_720_000,
    durationMs: 12,
    connectionState: "connected",
    ...overrides,
  };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function transferResult(overrides: Partial<FileTransferResult> = {}): FileTransferResult {
  return {
    transferId: "transfer-test-1",
    serverId: "server-1",
    direction: "upload",
    kind: "file",
    status: "completed",
    sourcePath: "/home/user/file.txt",
    destinationPath: "/var/www/file.txt",
    bytesTransferred: 8,
    totalBytes: 8,
    filesTransferred: 1,
    filesTotal: 1,
    directoriesCreated: 0,
    skippedEntries: 0,
    warnings: [],
    partialPaths: [],
    mayBeIncomplete: false,
    overwrite: false,
    startedAt: 1_762_665_720_000,
    finishedAt: 1_762_665_721_000,
    durationMs: 1_000,
    connectionState: "connected",
    ...overrides,
  };
}

function mutationResult(overrides: Partial<RemoteMutationResult> = {}): RemoteMutationResult {
  return {
    serverId: "server-1",
    action: "create_folder",
    root: "/var/www",
    path: "/var/www",
    targetPath: "/var/www/backups",
    recursive: false,
    deletedEntries: 0,
    verified: true,
    completedAt: 1_762_665_720_000,
    connectionState: "connected",
    ...overrides,
  };
}

describe("file browser store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useFileStore.setState({ entries: {}, transfers: {}, mutations: {} });
    vi.mocked(browseRemoteFiles).mockReset();
    vi.mocked(uploadRemoteFile).mockReset();
    vi.mocked(cancelFileTransfer).mockReset();
    vi.mocked(createRemoteFolder).mockReset();
    vi.mocked(renameRemoteEntry).mockReset();
    vi.mocked(deleteRemoteEntry).mockReset();
    vi.mocked(createTransferId).mockReturnValue("transfer-test-1");
  });

  it("opens a root and stores the listing with the canonical root", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());

    await expect(useFileStore.getState().openRoot("server-1", "/var//www/")).resolves.toBe(true);

    expect(browseRemoteFiles).toHaveBeenCalledWith("server-1", "/var/www", "/var/www");
    const entry = useFileStore.getState().entries["server-1"];
    expect(entry?.root).toBe("/var/www");
    expect(entry?.path).toBe("/var/www");
    expect(entry?.listing).toEqual(listing());
    expect(entry?.pending).toBe(false);
    expect(entry?.error).toBeNull();
  });

  it("navigates to nested paths without changing the selected root", async () => {
    vi.mocked(browseRemoteFiles)
      .mockResolvedValueOnce(listing())
      .mockResolvedValueOnce(
        listing({
          path: "/var/www/app",
          resolvedPath: "/var/www/app",
          entries: [],
        }),
      );

    await useFileStore.getState().openRoot("server-1", "/var/www");
    await useFileStore.getState().browse("server-1", "/var/www", "/var/www/app");

    expect(browseRemoteFiles).toHaveBeenLastCalledWith("server-1", "/var/www", "/var/www/app");
    const entry = useFileStore.getState().entries["server-1"];
    expect(entry?.root).toBe("/var/www");
    expect(entry?.path).toBe("/var/www/app");
    expect(entry?.listing?.entries).toEqual([]);
  });

  it("rejects traversal and paths outside the root before calling the service", async () => {
    await expect(
      useFileStore.getState().browse("server-1", "/var/www", "/var/www/../etc"),
    ).resolves.toBe(false);
    await expect(
      useFileStore.getState().browse("server-1", "/var/www", "/etc/passwd"),
    ).resolves.toBe(false);

    expect(browseRemoteFiles).not.toHaveBeenCalled();
    const entry = useFileStore.getState().entries["server-1"];
    expect(entry?.error).toContain("outside the selected root");
  });

  it("keeps the last successful listing when a refresh fails", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValueOnce(listing()).mockRejectedValueOnce({
      code: "permission",
      message: "The SSH account does not have permission to read this remote location.",
      details: "ServerDesk does not elevate with sudo.",
    });

    await useFileStore.getState().openRoot("server-1", "/var/www");
    await expect(useFileStore.getState().refresh("server-1")).resolves.toBe(false);

    const entry = useFileStore.getState().entries["server-1"];
    expect(entry?.listing).toEqual(listing());
    expect(entry?.path).toBe("/var/www");
    expect(entry?.pending).toBe(false);
    expect(entry?.error).toContain("permission");
    expect(entry?.errorDetails).toContain("sudo");
  });

  it("records navigation history, moves back and forward, and truncates forward history", async () => {
    vi.mocked(browseRemoteFiles).mockImplementation(async (_serverId, _root, path) =>
      listing({ path, resolvedPath: path, entries: [] }),
    );

    await useFileStore.getState().openRoot("server-1", "/var/www");
    let entry = useFileStore.getState().entries["server-1"];
    expect(entry?.history).toEqual(["/var/www"]);
    expect(entry?.historyIndex).toBe(0);
    expect(canNavigateBack(entry)).toBe(false);
    expect(canNavigateForward(entry)).toBe(false);

    await useFileStore.getState().browse("server-1", "/var/www", "/var/www/app");
    await useFileStore.getState().browse("server-1", "/var/www", "/var/www/app/public");
    entry = useFileStore.getState().entries["server-1"];
    expect(entry?.history).toEqual(["/var/www", "/var/www/app", "/var/www/app/public"]);
    expect(entry?.historyIndex).toBe(2);
    expect(canNavigateBack(entry)).toBe(true);
    expect(canNavigateForward(entry)).toBe(false);

    await expect(useFileStore.getState().goBack("server-1")).resolves.toBe(true);
    entry = useFileStore.getState().entries["server-1"];
    expect(entry?.path).toBe("/var/www/app");
    expect(entry?.historyIndex).toBe(1);
    expect(canNavigateForward(entry)).toBe(true);
    expect(browseRemoteFiles).toHaveBeenLastCalledWith("server-1", "/var/www", "/var/www/app");

    await expect(useFileStore.getState().goForward("server-1")).resolves.toBe(true);
    entry = useFileStore.getState().entries["server-1"];
    expect(entry?.path).toBe("/var/www/app/public");
    expect(entry?.historyIndex).toBe(2);

    await useFileStore.getState().goBack("server-1");
    await useFileStore.getState().browse("server-1", "/var/www", "/var/www/app/other");
    entry = useFileStore.getState().entries["server-1"];
    expect(entry?.history).toEqual(["/var/www", "/var/www/app", "/var/www/app/other"]);
    expect(entry?.historyIndex).toBe(2);
    expect(canNavigateForward(entry)).toBe(false);
  });

  it("does not add failed or refreshed navigation to history", async () => {
    vi.mocked(browseRemoteFiles)
      .mockResolvedValueOnce(listing())
      .mockRejectedValueOnce({
        code: "permission",
        message: "Permission denied.",
      })
      .mockResolvedValueOnce(listing());

    await useFileStore.getState().openRoot("server-1", "/var/www");
    await expect(
      useFileStore.getState().browse("server-1", "/var/www", "/var/www/private"),
    ).resolves.toBe(false);

    let entry = useFileStore.getState().entries["server-1"];
    expect(entry?.history).toEqual(["/var/www"]);
    expect(entry?.errorCode).toBe("permission");
    expect(entry?.errorPath).toBe("/var/www/private");

    await expect(useFileStore.getState().refresh("server-1")).resolves.toBe(true);
    entry = useFileStore.getState().entries["server-1"];
    expect(entry?.history).toEqual(["/var/www"]);
    expect(entry?.historyIndex).toBe(0);
  });

  it("moves up only within the selected root and clears server state", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(
      listing({ path: "/var/www/app", resolvedPath: "/var/www/app" }),
    );

    await useFileStore.getState().openRoot("server-1", "/var/www");
    useFileStore.setState({
      entries: {
        "server-1": {
          ...emptyFileBrowserEntry(),
          root: "/var/www",
          path: "/var/www/app",
          listing: listing(),
        },
      },
    });

    await expect(useFileStore.getState().goUp("server-1")).resolves.toBe(true);
    expect(browseRemoteFiles).toHaveBeenLastCalledWith("server-1", "/var/www", "/var/www");

    useFileStore.setState({
      entries: {
        "server-1": {
          ...emptyFileBrowserEntry(),
          root: "/var/www",
          path: "/var/www",
          listing: listing(),
        },
      },
    });
    await expect(useFileStore.getState().goUp("server-1")).resolves.toBe(false);

    useFileStore.getState().clearServer("server-1");
    expect(useFileStore.getState().entries["server-1"]).toBeUndefined();
  });

  it("stores upload progress and the completed transfer result", async () => {
    vi.mocked(uploadRemoteFile).mockResolvedValue(transferResult());

    const transferId = await useFileStore.getState().startUpload({
      serverId: "server-1",
      root: "/var/www",
      localPath: "/home/user/file.txt",
      remotePath: "/var/www/file.txt",
    });

    expect(uploadRemoteFile).toHaveBeenCalledWith(
      expect.objectContaining({
        transferId: "transfer-test-1",
        overwrite: false,
      }),
    );
    const transfer = useFileStore.getState().transfers[transferId];
    expect(transfer?.status).toBe("completed");
    expect(transfer?.bytesTransferred).toBe(8);
    expect(transfer?.mayBeIncomplete).toBe(false);
    expect(transfer?.request).toEqual({
      direction: "upload",
      serverId: "server-1",
      root: "/var/www",
      localPath: "/home/user/file.txt",
      remotePath: "/var/www/file.txt",
      overwrite: false,
    });
  });

  it("keeps a conflict failure so the UI can ask for overwrite confirmation", async () => {
    vi.mocked(uploadRemoteFile).mockRejectedValue({
      code: "conflict",
      message: "1 existing destination file would be overwritten.",
      details: "Overwrite was not confirmed.",
    });

    const transferId = await useFileStore.getState().startUpload({
      serverId: "server-1",
      root: "/var/www",
      localPath: "/home/user/file.txt",
      remotePath: "/var/www/file.txt",
    });

    const transfer = useFileStore.getState().transfers[transferId];
    expect(transfer?.status).toBe("failed");
    expect(transfer?.errorCode).toBe("conflict");
    expect(transfer?.error).toContain("overwritten");
    expect(transfer?.request.overwrite).toBe(false);
  });

  it("tracks live progress, cancellation, and partial paths", async () => {
    let resolveUpload: (result: FileTransferResult) => void = () => undefined;
    vi.mocked(uploadRemoteFile).mockImplementation(
      () =>
        new Promise<FileTransferResult>((resolve) => {
          resolveUpload = resolve;
        }),
    );
    vi.mocked(cancelFileTransfer).mockResolvedValue(true);

    const start = useFileStore.getState().startUpload({
      serverId: "server-1",
      root: "/var/www",
      localPath: "/home/user/large.bin",
      remotePath: "/var/www/large.bin",
    });
    const transferId = Object.keys(useFileStore.getState().transfers)[0];

    useFileStore.getState().applyTransferProgress({
      transferId,
      serverId: "server-1",
      direction: "upload",
      status: "running",
      kind: "file",
      bytesTransferred: 4_096,
      totalBytes: 8_192,
      filesTransferred: 0,
      filesTotal: 1,
      currentPath: "/var/www/large.bin",
      mayBeIncomplete: false,
      emittedAt: 1_762_665_720_500,
    });

    let transfer = useFileStore.getState().transfers[transferId];
    expect(transfer?.bytesTransferred).toBe(4_096);
    expect(transfer?.totalBytes).toBe(8_192);

    await expect(useFileStore.getState().cancelTransfer(transferId)).resolves.toBe(true);
    expect(cancelFileTransfer).toHaveBeenCalledWith(transferId);
    expect(useFileStore.getState().transfers[transferId]?.cancelling).toBe(true);

    resolveUpload(
      transferResult({
        status: "cancelled",
        bytesTransferred: 4_096,
        totalBytes: 8_192,
        filesTransferred: 0,
        mayBeIncomplete: true,
        partialPaths: ["/var/www/large.bin.serverdesk-test.part"],
      }),
    );
    await start;

    transfer = useFileStore.getState().transfers[transferId];
    expect(transfer?.status).toBe("cancelled");
    expect(transfer?.mayBeIncomplete).toBe(true);
    expect(transfer?.partialPaths).toEqual(["/var/www/large.bin.serverdesk-test.part"]);
    expect(transfer?.cancelling).toBe(false);
  });

  it("retries a failed transfer with overwrite unconfirmed", async () => {
    vi.mocked(uploadRemoteFile)
      .mockRejectedValueOnce({
        code: "network",
        message: "The SSH connection was lost.",
      })
      .mockResolvedValueOnce(transferResult());
    vi.mocked(createTransferId)
      .mockReturnValueOnce("transfer-test-1")
      .mockReturnValueOnce("transfer-test-2");

    const firstId = await useFileStore.getState().startUpload({
      serverId: "server-1",
      root: "/var/www",
      localPath: "/home/user/file.txt",
      remotePath: "/var/www/file.txt",
      overwrite: true,
    });
    expect(useFileStore.getState().transfers[firstId]?.status).toBe("failed");

    const retryId = await useFileStore.getState().retryTransfer(firstId);
    expect(retryId).toBe("transfer-test-2");
    expect(vi.mocked(uploadRemoteFile).mock.calls[1][0].overwrite).toBe(false);
    expect(useFileStore.getState().transfers[retryId ?? ""]?.status).toBe("completed");
  });

  it("deletes selected entries one at a time, aggregates the result, and refreshes once", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(deleteRemoteEntry).mockImplementation(async (input) =>
      mutationResult({
        action: "delete",
        path: input.path,
        targetPath: undefined,
        recursive: input.recursive,
        deletedEntries: input.recursive ? 3 : 1,
      }),
    );
    useFileStore.setState({
      entries: {
        "server-1": {
          ...emptyFileBrowserEntry(),
          root: "/var/www",
          path: "/var/www",
          listing: listing(),
        },
      },
    });

    await expect(
      useFileStore.getState().deleteEntries("server-1", "/var/www", [
        { path: "/var/www/readme.txt", recursive: false },
        { path: "/var/www/backups", recursive: true },
      ]),
    ).resolves.toBe(true);

    expect(deleteRemoteEntry).toHaveBeenNthCalledWith(1, {
      serverId: "server-1",
      root: "/var/www",
      path: "/var/www/readme.txt",
      confirmed: true,
      recursive: false,
    });
    expect(deleteRemoteEntry).toHaveBeenNthCalledWith(2, {
      serverId: "server-1",
      root: "/var/www",
      path: "/var/www/backups",
      confirmed: true,
      recursive: true,
    });
    const result = useFileStore.getState().mutations["server-1"]?.result;
    expect(result?.action).toBe("delete");
    expect(result?.targetPath).toBeUndefined();
    expect(result?.deletedEntries).toBe(4);
    expect(result?.recursive).toBe(true);
    expect(result?.verified).toBe(true);
    expect(browseRemoteFiles).toHaveBeenCalledWith("server-1", "/var/www", "/var/www");
  });

  it("refreshes the listing after a batch delete fails partway through", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(deleteRemoteEntry)
      .mockResolvedValueOnce(
        mutationResult({
          action: "delete",
          path: "/var/www/readme.txt",
          targetPath: undefined,
          deletedEntries: 1,
        }),
      )
      .mockRejectedValueOnce({
        code: "permission",
        message: "Permission denied.",
        details: "The SSH account cannot remove this entry.",
      });
    useFileStore.setState({
      entries: {
        "server-1": {
          ...emptyFileBrowserEntry(),
          root: "/var/www",
          path: "/var/www",
          listing: listing(),
        },
      },
    });

    await expect(
      useFileStore.getState().deleteEntries("server-1", "/var/www", [
        { path: "/var/www/readme.txt", recursive: false },
        { path: "/var/www/backups", recursive: true },
      ]),
    ).resolves.toBe(false);

    const mutation = useFileStore.getState().mutations["server-1"];
    expect(mutation?.pending).toBe(false);
    expect(mutation?.error).toBe("Permission denied.");
    expect(mutation?.errorCode).toBe("permission");
    expect(browseRemoteFiles).toHaveBeenCalledWith("server-1", "/var/www", "/var/www");
  });

  it("runs confirmed mutations and refreshes the current listing", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(createRemoteFolder).mockResolvedValue(mutationResult());
    vi.mocked(renameRemoteEntry).mockResolvedValue(
      mutationResult({
        action: "rename",
        path: "/var/www/readme.txt",
        targetPath: "/var/www/renamed.txt",
      }),
    );
    vi.mocked(deleteRemoteEntry).mockResolvedValue(
      mutationResult({
        action: "delete",
        path: "/var/www/renamed.txt",
        targetPath: undefined,
        deletedEntries: 1,
      }),
    );
    useFileStore.setState({
      entries: {
        "server-1": {
          ...emptyFileBrowserEntry(),
          root: "/var/www",
          path: "/var/www",
          listing: listing(),
        },
      },
    });

    await expect(
      useFileStore.getState().createFolder("server-1", "/var/www", "/var/www", "backups"),
    ).resolves.toBe(true);
    expect(createRemoteFolder).toHaveBeenCalledWith({
      serverId: "server-1",
      root: "/var/www",
      parent: "/var/www",
      name: "backups",
    });

    await expect(
      useFileStore
        .getState()
        .renameEntry("server-1", "/var/www", "/var/www/readme.txt", "renamed.txt"),
    ).resolves.toBe(true);
    expect(renameRemoteEntry).toHaveBeenCalledWith({
      serverId: "server-1",
      root: "/var/www",
      path: "/var/www/readme.txt",
      newName: "renamed.txt",
      confirmed: true,
    });

    await expect(
      useFileStore.getState().deleteEntry("server-1", "/var/www", "/var/www/renamed.txt", false),
    ).resolves.toBe(true);
    expect(deleteRemoteEntry).toHaveBeenCalledWith({
      serverId: "server-1",
      root: "/var/www",
      path: "/var/www/renamed.txt",
      confirmed: true,
      recursive: false,
    });

    expect(useFileStore.getState().mutations["server-1"]?.result?.verified).toBe(true);
    expect(vi.mocked(browseRemoteFiles).mock.calls.length).toBeGreaterThanOrEqual(3);
  });
});
