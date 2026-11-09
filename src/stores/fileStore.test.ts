import { beforeEach, describe, expect, it, vi } from "vitest";
import { browseRemoteFiles } from "@/services/files";
import { emptyFileBrowserEntry, useFileStore } from "@/stores/fileStore";
import type { RemoteFileListing } from "@/types/contracts";

vi.mock("@/services/files", () => ({
  browseRemoteFiles: vi.fn(),
}));

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

describe("file browser store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useFileStore.setState({ entries: {} });
    vi.mocked(browseRemoteFiles).mockReset();
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
});
