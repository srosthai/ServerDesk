import { beforeEach, describe, expect, it, vi } from "vitest";
import { browseRemoteDirectories, listProjects, scanRemoteProjects } from "@/services/projects";
import { useProjectStore } from "@/stores/projectStore";
import type { ProjectRecord, ProjectScanResult, RemoteDirectoryListing } from "@/types/contracts";

vi.mock("@/services/projects", () => ({
  listProjects: vi.fn(),
  browseRemoteDirectories: vi.fn(),
  scanRemoteProjects: vi.fn(),
  deleteProject: vi.fn(),
}));

function project(overrides: Partial<ProjectRecord> = {}): ProjectRecord {
  return {
    id: "project-1",
    serverId: "server-1",
    name: "storefront",
    remotePath: "/var/www/storefront",
    projectType: "laravel",
    laravelVersion: "v11.30.0",
    phpVersion: "8.3.14",
    queueHint: "Queue connection: redis",
    schedulerHint: "A cron entry runs schedule:run for this path",
    detectedAt: 1_762_665_720,
    logPath: "/var/www/storefront/storage/logs/laravel.log",
    recentOperations: [],
    ...overrides,
  };
}

const listing: RemoteDirectoryListing = {
  serverId: "server-1",
  path: "/var/www",
  directories: [
    { name: "storefront", path: "/var/www/storefront" },
    { name: "blog", path: "/var/www/blog" },
  ],
  truncated: false,
  sampledAt: 1_762_665_720_000,
  durationMs: 12,
  connectionState: "connected",
};

function scanResult(overrides: Partial<ProjectScanResult> = {}): ProjectScanResult {
  return {
    serverId: "server-1",
    path: "/var/www",
    includeChildren: true,
    scannedPaths: 3,
    rejectedPaths: 2,
    truncated: false,
    projects: [project()],
    durationMs: 31,
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

describe("project store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useProjectStore.setState({ entries: {} });
    vi.mocked(listProjects).mockReset();
    vi.mocked(browseRemoteDirectories).mockReset();
    vi.mocked(scanRemoteProjects).mockReset();
  });

  it("loads saved projects for one server without scanning", async () => {
    vi.mocked(listProjects).mockResolvedValue([project()]);

    await expect(useProjectStore.getState().fetchProjects("server-1")).resolves.toBe(true);

    expect(listProjects).toHaveBeenCalledWith("server-1");
    expect(scanRemoteProjects).not.toHaveBeenCalled();
    const entry = useProjectStore.getState().entries["server-1"];
    expect(entry?.projects).toEqual([project()]);
    expect(entry?.loading).toBe(false);
    expect(entry?.error).toBeNull();
  });

  it("browses the selected directory and stores the typed listing", async () => {
    vi.mocked(browseRemoteDirectories).mockResolvedValue(listing);

    await expect(useProjectStore.getState().browse("server-1", "/var/www")).resolves.toBe(true);

    expect(browseRemoteDirectories).toHaveBeenCalledWith("server-1", "/var/www");
    const entry = useProjectStore.getState().entries["server-1"];
    expect(entry?.listing).toEqual(listing);
    expect(entry?.browsing).toBe(false);
    expect(entry?.browseError).toBeNull();
  });

  it("merges a rescan into the existing record instead of duplicating it", async () => {
    vi.mocked(scanRemoteProjects)
      .mockResolvedValueOnce(scanResult())
      .mockResolvedValueOnce(
        scanResult({
          projects: [
            project({
              laravelVersion: "v11.31.0",
              detectedAt: 1_762_665_900,
            }),
          ],
          rejectedPaths: 0,
        }),
      );

    await useProjectStore.getState().scan("server-1", "/var/www", true);
    await useProjectStore.getState().scan("server-1", "/var/www", true);

    const entry = useProjectStore.getState().entries["server-1"];
    expect(entry?.projects).toHaveLength(1);
    expect(entry?.projects[0].id).toBe("project-1");
    expect(entry?.projects[0].laravelVersion).toBe("v11.31.0");
    expect(entry?.scan?.rejectedPaths).toBe(0);
  });

  it("keeps a failed scan visible without discarding saved projects", async () => {
    useProjectStore.setState({
      entries: {
        "server-1": {
          projects: [project()],
          loading: false,
          error: null,
          listing: null,
          browsing: false,
          browseError: null,
          scan: scanResult(),
          scanning: false,
          scanError: null,
        },
      },
    });
    vi.mocked(scanRemoteProjects).mockRejectedValueOnce({
      code: "not_found",
      message: "The remote path does not exist.",
    });

    await expect(
      useProjectStore.getState().scan("server-1", "/var/www/missing", false),
    ).resolves.toBe(false);

    const entry = useProjectStore.getState().entries["server-1"];
    expect(entry?.projects).toEqual([project()]);
    expect(entry?.scan?.projects).toEqual([project()]);
    expect(entry?.scanError).toBe("The remote path does not exist.");
    expect(entry?.scanning).toBe(false);
  });

  it("reports browse errors without inventing a listing", async () => {
    vi.mocked(browseRemoteDirectories).mockRejectedValueOnce({
      code: "permission",
      message: "Permission denied.",
    });

    await expect(useProjectStore.getState().browse("server-1", "/root")).resolves.toBe(false);

    const entry = useProjectStore.getState().entries["server-1"];
    expect(entry?.listing).toBeNull();
    expect(entry?.browseError).toBe("Permission denied.");
  });
});
