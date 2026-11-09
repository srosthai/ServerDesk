import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FileManagerPage } from "@/features/files/FileManagerPage";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { getConnectionState } from "@/services/connections";
import { browseRemoteFiles } from "@/services/files";
import { useConnectionStore } from "@/stores/connectionStore";
import { emptyFileBrowserEntry, useFileStore } from "@/stores/fileStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ConnectionSnapshot,
  RemoteFileEntry,
  RemoteFileListing,
  ServerProfile,
} from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    getConnectionState: vi.fn(),
  };
});

vi.mock("@/services/files", () => ({
  browseRemoteFiles: vi.fn(),
}));

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
});
