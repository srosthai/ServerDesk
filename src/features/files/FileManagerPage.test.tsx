import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FileManagerPage } from "@/features/files/FileManagerPage";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { getConnectionState } from "@/services/connections";
import {
  browseRemoteFiles,
  cancelFileTransfer,
  createRemoteFolder,
  deleteRemoteEntry,
  downloadRemoteFile,
  renameRemoteEntry,
  uploadRemoteFile,
} from "@/services/files";
import { useConnectionStore } from "@/stores/connectionStore";
import { emptyFileBrowserEntry, useFileStore } from "@/stores/fileStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ConnectionSnapshot,
  FileTransferResult,
  RemoteFileEntry,
  RemoteFileListing,
  RemoteMutationResult,
  ServerProfile,
} from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    getConnectionState: vi.fn(),
  };
});

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
  };
});

const server: ServerProfile = {
  id: "server-1",
  name: "Production",
  host: "198.51.100.24",
  port: 22,
  username: "deploy",
  authType: "password",
  credentialRef: null,
  groupName: null,
  tags: [],
  notes: null,
  createdAt: 1,
  updatedAt: 1,
};

const connected: ConnectionSnapshot = {
  serverId: server.id,
  state: "connected",
  changedAt: 1,
};

function entry(overrides: Partial<RemoteFileEntry> = {}): RemoteFileEntry {
  return {
    name: "readme.txt",
    path: "/var/www/readme.txt",
    entryType: "file",
    size: 12,
    permissions: "-rw-r--r--",
    mode: 0o100644,
    modifiedAt: 1_762_665_720,
    brokenSymlink: false,
    ...overrides,
  };
}

function listing(overrides: Partial<RemoteFileListing> = {}): RemoteFileListing {
  return {
    serverId: server.id,
    root: "/var/www",
    path: "/var/www",
    resolvedPath: "/var/www",
    entries: [
      entry({
        name: "nested",
        path: "/var/www/nested",
        entryType: "directory",
        size: 4096,
        permissions: "drwxr-xr-x",
        mode: 0o040755,
      }),
      entry(),
      entry({
        name: "escape-link",
        path: "/var/www/escape-link",
        entryType: "symlink",
        size: 4,
        permissions: "lrwxrwxrwx",
        mode: 0o120777,
        symlinkTarget: "/etc",
        symlinkEscapesRoot: true,
      }),
      entry({
        name: "inside-link",
        path: "/var/www/inside-link",
        entryType: "symlink",
        size: 6,
        permissions: "lrwxrwxrwx",
        mode: 0o120777,
        symlinkTarget: "nested",
        resolvedPath: "/var/www/nested",
        symlinkEscapesRoot: false,
      }),
    ],
    truncated: false,
    sampledAt: 1_762_665_720_000,
    durationMs: 18,
    connectionState: "connected",
    ...overrides,
  };
}

function transferResult(overrides: Partial<FileTransferResult> = {}): FileTransferResult {
  const transferId = overrides.transferId ?? "transfer-1";
  return {
    transferId,
    serverId: server.id,
    direction: "upload",
    kind: "file",
    status: "completed",
    sourcePath: "/home/user/readme.txt",
    destinationPath: "/var/www/readme.txt",
    bytesTransferred: 12,
    totalBytes: 12,
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
    serverId: server.id,
    action: "delete",
    root: "/var/www",
    path: "/var/www/readme.txt",
    recursive: false,
    deletedEntries: 1,
    verified: true,
    completedAt: 1_762_665_720_000,
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

function renderPage() {
  return render(
    <MemoryRouter>
      <FileManagerPage />
    </MemoryRouter>,
  );
}

describe("FileManagerPage", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES, lastActiveServerId: server.id },
    });
    useFileStore.setState({ entries: {} });
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(browseRemoteFiles).mockReset();
    vi.mocked(uploadRemoteFile).mockReset();
    vi.mocked(downloadRemoteFile).mockReset();
    vi.mocked(cancelFileTransfer).mockReset();
    vi.mocked(createRemoteFolder).mockReset();
    vi.mocked(renameRemoteEntry).mockReset();
    vi.mocked(deleteRemoteEntry).mockReset();
  });

  async function openRoot(path = "/var/www") {
    const user = userEvent.setup();
    const input = screen.getByLabelText("Selected root directory");
    await user.clear(input);
    await user.type(input, path);
    await user.click(screen.getByRole("button", { name: "Browse" }));
    return user;
  }

  it("browses the selected root and shows name, type, size, permissions, and modified time", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    renderPage();

    await openRoot();

    await waitFor(() =>
      expect(browseRemoteFiles).toHaveBeenCalledWith(server.id, "/var/www", "/var/www"),
    );
    expect(await screen.findByText("readme.txt")).toBeInTheDocument();
    expect(screen.getByText("nested")).toBeInTheDocument();
    expect(screen.getByText("12 B")).toBeInTheDocument();
    expect(screen.getAllByText("-rw-r--r--").length).toBeGreaterThan(0);
    expect(screen.getAllByText("drwxr-xr-x").length).toBeGreaterThan(0);
    expect(screen.getByText(/Listed in 18 ms/)).toBeInTheDocument();
    expect(screen.getByText("4 entries")).toBeInTheDocument();
  });

  it("navigates into nested directories while keeping the selected root", async () => {
    vi.mocked(browseRemoteFiles)
      .mockResolvedValueOnce(listing())
      .mockResolvedValueOnce(
        listing({
          path: "/var/www/nested",
          resolvedPath: "/var/www/nested",
          entries: [
            entry({
              name: "deeper",
              path: "/var/www/nested/deeper",
              entryType: "directory",
            }),
          ],
        }),
      );
    renderPage();

    const user = await openRoot();
    await screen.findByText("readme.txt");
    await user.click(screen.getByRole("button", { name: "Open directory nested" }));

    await waitFor(() =>
      expect(browseRemoteFiles).toHaveBeenLastCalledWith(server.id, "/var/www", "/var/www/nested"),
    );
    expect(await screen.findByText("deeper")).toBeInTheDocument();
    expect(screen.getByText("/var/www/nested")).toBeInTheDocument();
  });

  it("rejects path traversal in the browser before any remote request", async () => {
    const user = userEvent.setup();
    renderPage();

    const input = screen.getByLabelText("Selected root directory");
    await user.clear(input);
    await user.type(input, "/var/www/../etc");
    await user.click(screen.getByRole("button", { name: "Browse" }));

    expect(browseRemoteFiles).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("absolute, normalized remote path");
  });

  it("clearly marks escaping symlinks and blocks navigating through them", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    renderPage();

    await openRoot();
    await screen.findByText("escape-link");

    const row = screen.getByText("escape-link").closest("tr");
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getByText("Outside root")).toBeInTheDocument();
    expect(within(row as HTMLElement).getByText("→ /etc")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Open symlink escape-link" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open symlink inside-link" })).toBeInTheDocument();
  });

  it("reports permission errors and keeps the last listing visible", async () => {
    vi.mocked(browseRemoteFiles).mockRejectedValueOnce({
      code: "permission",
      message: "The SSH account does not have permission to read this remote location.",
      details: "ServerDesk does not elevate with sudo.",
    });
    renderPage();

    await openRoot("/var/private");

    expect(
      await screen.findByText(
        "The SSH account does not have permission to read this remote location.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("ServerDesk does not elevate with sudo.")).toBeInTheDocument();
    expect(screen.getByText("Could not browse this remote path")).toBeInTheDocument();
  });

  it("keeps the last listing visible with a stale warning when disconnected", async () => {
    useConnectionStore.setState({
      connections: {
        [server.id]: { serverId: server.id, state: "disconnected", changedAt: 2 },
      },
      busy: {},
      error: null,
    });
    useFileStore.setState({
      entries: {
        [server.id]: {
          ...emptyFileBrowserEntry(),
          root: "/var/www",
          path: "/var/www",
          listing: listing(),
          lastAttemptAt: 1_762_665_720_000,
        },
      },
    });
    renderPage();

    expect(screen.getByText("Showing the last directory listing")).toBeInTheDocument();
    expect(screen.getByText("Disconnected")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Browse new root" })).toBeDisabled();
    expect(screen.getByText("readme.txt")).toBeInTheDocument();
  });

  it("uploads a file with live progress and reports completion", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(uploadRemoteFile).mockImplementation(async (input) => {
      useFileStore.getState().applyTransferProgress({
        transferId: input.transferId,
        serverId: input.serverId,
        direction: "upload",
        status: "running",
        kind: "file",
        bytesTransferred: 6,
        totalBytes: 12,
        filesTransferred: 0,
        filesTotal: 1,
        currentPath: input.remotePath,
        mayBeIncomplete: false,
        emittedAt: 1_762_665_720_500,
      });
      return transferResult({
        transferId: input.transferId,
        sourcePath: input.localPath,
        destinationPath: input.remotePath,
      });
    });
    renderPage();

    const user = await openRoot();
    await screen.findByText("readme.txt");
    await user.click(screen.getByRole("button", { name: "Upload" }));
    await user.type(screen.getByLabelText("Local source path"), "/home/user/readme.txt");
    expect(screen.getByText("/var/www/readme.txt")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Start upload" }));

    await waitFor(() =>
      expect(uploadRemoteFile).toHaveBeenCalledWith(
        expect.objectContaining({
          serverId: server.id,
          root: "/var/www",
          localPath: "/home/user/readme.txt",
          remotePath: "/var/www/readme.txt",
          overwrite: false,
        }),
      ),
    );
    expect(await screen.findByText("Completed")).toBeInTheDocument();
    expect(
      screen.getByRole("progressbar", { name: /upload progress for readme.txt/i }),
    ).toBeInTheDocument();
    expect(screen.getByText("12 B of 12 B")).toBeInTheDocument();
  });

  it("downloads an entry to an explicit local destination", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(downloadRemoteFile).mockResolvedValue(
      transferResult({
        direction: "download",
        sourcePath: "/var/www/readme.txt",
        destinationPath: "/home/user/readme.txt",
      }),
    );
    renderPage();

    const user = await openRoot();
    await screen.findByText("readme.txt");
    await user.click(screen.getByRole("button", { name: "Download readme.txt" }));
    await user.type(screen.getByLabelText("Local destination path"), "/home/user/readme.txt");
    await user.click(screen.getByRole("button", { name: "Start download" }));

    await waitFor(() =>
      expect(downloadRemoteFile).toHaveBeenCalledWith(
        expect.objectContaining({
          remotePath: "/var/www/readme.txt",
          localPath: "/home/user/readme.txt",
          overwrite: false,
        }),
      ),
    );
  });

  it("requires explicit confirmation before overwriting an existing destination", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(uploadRemoteFile)
      .mockRejectedValueOnce({
        code: "conflict",
        message: "1 existing destination file would be overwritten.",
        details: "Overwrite was not confirmed.",
      })
      .mockResolvedValueOnce(
        transferResult({
          overwrite: true,
          sourcePath: "/home/user/readme.txt",
          destinationPath: "/var/www/readme.txt",
        }),
      );
    renderPage();

    const user = await openRoot();
    await screen.findByText("readme.txt");
    await user.click(screen.getByRole("button", { name: "Upload" }));
    await user.type(screen.getByLabelText("Local source path"), "/home/user/readme.txt");
    await user.click(screen.getByRole("button", { name: "Start upload" }));

    expect(await screen.findByText("Overwrite the existing destination?")).toBeInTheDocument();
    expect(uploadRemoteFile).toHaveBeenCalledTimes(1);
    expect(vi.mocked(uploadRemoteFile).mock.calls[0][0].overwrite).toBe(false);

    await user.click(screen.getByRole("button", { name: "Overwrite and transfer" }));

    await waitFor(() => expect(uploadRemoteFile).toHaveBeenCalledTimes(2));
    expect(vi.mocked(uploadRemoteFile).mock.calls[1][0].overwrite).toBe(true);
  });

  it("cancels a running transfer and reports the incomplete partial state", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    let resolveUpload: (result: FileTransferResult) => void = () => undefined;
    vi.mocked(uploadRemoteFile).mockImplementation(
      (input) =>
        new Promise<FileTransferResult>((resolve) => {
          resolveUpload = (result) => resolve({ ...result, transferId: input.transferId });
        }),
    );
    vi.mocked(cancelFileTransfer).mockResolvedValue(true);
    renderPage();

    const user = await openRoot();
    await screen.findByText("readme.txt");
    await user.click(screen.getByRole("button", { name: "Upload" }));
    await user.type(screen.getByLabelText("Local source path"), "/home/user/readme.txt");
    await user.click(screen.getByRole("button", { name: "Start upload" }));

    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(cancelFileTransfer).toHaveBeenCalledTimes(1));

    const transferId = vi.mocked(uploadRemoteFile).mock.calls[0][0].transferId;
    expect(cancelFileTransfer).toHaveBeenCalledWith(transferId);
    resolveUpload(
      transferResult({
        status: "cancelled",
        bytesTransferred: 4,
        totalBytes: 12,
        filesTransferred: 0,
        mayBeIncomplete: true,
        partialPaths: ["/var/www/readme.txt.serverdesk-abc.part"],
      }),
    );

    expect(await screen.findByText("This transfer may be incomplete")).toBeInTheDocument();
    expect(screen.getByText(/readme\.txt\.serverdesk-abc\.part/)).toBeInTheDocument();
    expect(screen.getByText("Cancelled")).toBeInTheDocument();
  });

  it("requires confirmation before deleting a remote entry and reports verification", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(deleteRemoteEntry).mockResolvedValue(mutationResult());
    renderPage();

    const user = await openRoot();
    await screen.findByText("readme.txt");
    await user.click(screen.getByRole("button", { name: "Delete readme.txt" }));

    expect(await screen.findByText("Delete readme.txt?")).toBeInTheDocument();
    expect(deleteRemoteEntry).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Delete file" }));

    await waitFor(() =>
      expect(deleteRemoteEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          path: "/var/www/readme.txt",
          confirmed: true,
          recursive: false,
        }),
      ),
    );
    expect(await screen.findByText("Entry deleted")).toBeInTheDocument();
    expect(screen.getAllByText("Verified by re-reading the remote path.").length).toBeGreaterThan(
      0,
    );
  });

  it("renames an entry only after the confirmation dialog is submitted", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(renameRemoteEntry).mockResolvedValue(
      mutationResult({
        action: "rename",
        targetPath: "/var/www/renamed.txt",
      }),
    );
    renderPage();

    const user = await openRoot();
    await screen.findByText("readme.txt");
    await user.click(screen.getByRole("button", { name: "Rename readme.txt" }));

    const input = await screen.findByLabelText("New name");
    await user.clear(input);
    await user.type(input, "renamed.txt");
    expect(renameRemoteEntry).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Rename" }));

    await waitFor(() =>
      expect(renameRemoteEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          path: "/var/www/readme.txt",
          newName: "renamed.txt",
          confirmed: true,
        }),
      ),
    );
    expect(await screen.findByText("Entry renamed")).toBeInTheDocument();
  });

  it("creates a folder in the current directory after validating the name", async () => {
    vi.mocked(browseRemoteFiles).mockResolvedValue(listing());
    vi.mocked(createRemoteFolder).mockResolvedValue(
      mutationResult({
        action: "create_folder",
        path: "/var/www",
        targetPath: "/var/www/backups",
      }),
    );
    renderPage();

    const user = await openRoot();
    await screen.findByText("readme.txt");
    await user.click(screen.getByRole("button", { name: "New folder" }));

    const input = await screen.findByLabelText("Folder name");
    await user.type(input, "backups");
    await user.click(screen.getByRole("button", { name: "Create folder" }));

    await waitFor(() =>
      expect(createRemoteFolder).toHaveBeenCalledWith(
        expect.objectContaining({
          root: "/var/www",
          parent: "/var/www",
          name: "backups",
        }),
      ),
    );
    expect(await screen.findByText("Folder created")).toBeInTheDocument();
  });
});
