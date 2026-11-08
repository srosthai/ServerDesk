import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProjectsPage } from "@/features/projects/ProjectsPage";
import { getConnectionState } from "@/services/connections";
import { browseRemoteDirectories, listProjects, scanRemoteProjects } from "@/services/projects";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useProjectStore } from "@/stores/projectStore";
import { useServerStore } from "@/stores/serverStore";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import type {
  ConnectionSnapshot,
  ProjectRecord,
  ProjectScanResult,
  RemoteDirectoryListing,
  ServerProfile,
} from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    getConnectionState: vi.fn(),
  };
});

vi.mock("@/services/projects", () => ({
  listProjects: vi.fn(),
  browseRemoteDirectories: vi.fn(),
  scanRemoteProjects: vi.fn(),
  deleteProject: vi.fn(),
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

const project: ProjectRecord = {
  id: "project-1",
  serverId: server.id,
  name: "storefront",
  remotePath: "/var/www/storefront",
  projectType: "laravel",
  laravelVersion: "v11.30.0",
  phpVersion: "8.3.14",
  queueHint: "Queue connection: redis; Laravel Horizon is installed",
  schedulerHint: "A cron entry runs schedule:run for this path",
  detectedAt: 1_762_665_720,
  logPath: "/var/www/storefront/storage/logs/laravel.log",
  recentOperations: [
    {
      id: "operation-1",
      operation: "artisan-about",
      safeSummary: "Read Artisan about",
      status: "succeeded",
      exitCode: 0,
      startedAt: 1_762_665_700,
      durationMs: 42,
    },
    {
      id: "operation-2",
      operation: "queue-restart",
      safeSummary: "Restarted queue workers",
      status: "failed",
      exitCode: 1,
      startedAt: 1_762_665_600,
      durationMs: 17,
    },
  ],
};

const listing: RemoteDirectoryListing = {
  serverId: server.id,
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
    serverId: server.id,
    path: "/var/www",
    includeChildren: true,
    scannedPaths: 3,
    rejectedPaths: 2,
    truncated: false,
    projects: [project],
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

function renderPage() {
  return render(
    <MemoryRouter>
      <ProjectsPage />
    </MemoryRouter>,
  );
}

describe("ProjectsPage", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    useProjectStore.setState({ entries: {} });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES, lastActiveServerId: server.id },
    });
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(listProjects).mockReset();
    vi.mocked(listProjects).mockResolvedValue([]);
    vi.mocked(browseRemoteDirectories).mockReset();
    vi.mocked(scanRemoteProjects).mockReset();
  });

  it("loads saved projects without scanning automatically", async () => {
    vi.mocked(listProjects).mockResolvedValue([project]);
    renderPage();

    await waitFor(() => expect(listProjects).toHaveBeenCalledWith(server.id));
    expect(await screen.findByLabelText("Project storefront")).toBeInTheDocument();
    expect(scanRemoteProjects).not.toHaveBeenCalled();
    expect(browseRemoteDirectories).not.toHaveBeenCalled();
  });

  it("shows project versions, hints, recent operation status, and links", async () => {
    vi.mocked(listProjects).mockResolvedValue([project]);
    renderPage();

    const card = await screen.findByLabelText("Project storefront");
    expect(within(card).getByText("/var/www/storefront")).toBeInTheDocument();
    expect(within(card).getByText("v11.30.0")).toBeInTheDocument();
    expect(within(card).getByText("8.3.14")).toBeInTheDocument();
    expect(within(card).getByText(/Laravel Horizon/)).toBeInTheDocument();
    expect(within(card).getByText(/schedule:run/)).toBeInTheDocument();
    expect(within(card).getByText("Recent operation status")).toBeInTheDocument();
    expect(within(card).getByText("Succeeded")).toBeInTheDocument();
    expect(within(card).getByText("Failed")).toBeInTheDocument();
    expect(within(card).getByText("artisan-about")).toBeInTheDocument();
    expect(
      within(card).getByText("/var/www/storefront/storage/logs/laravel.log"),
    ).toBeInTheDocument();

    const terminalLink = within(card).getByRole("link", { name: /Open terminal/i });
    const terminalUrl = new URL(terminalLink.getAttribute("href") ?? "", "http://localhost");
    expect(terminalUrl.pathname).toBe("/terminal");
    expect(terminalUrl.searchParams.get("server")).toBe(server.id);
    expect(terminalUrl.searchParams.get("path")).toBe("/var/www/storefront");

    const logLink = within(card).getByRole("link", { name: /Open Laravel log/i });
    const logUrl = new URL(logLink.getAttribute("href") ?? "", "http://localhost");
    expect(logUrl.pathname).toBe("/logs");
    expect(logUrl.searchParams.get("path")).toBe("/var/www/storefront/storage/logs/laravel.log");
  });

  it("normalizes the displayed path before scanning and reports a non-Laravel directory", async () => {
    vi.mocked(scanRemoteProjects).mockResolvedValue(
      scanResult({
        path: "/var/www/storefront",
        includeChildren: false,
        scannedPaths: 1,
        rejectedPaths: 1,
        projects: [],
      }),
    );
    const user = userEvent.setup();
    renderPage();

    const input = screen.getByLabelText("Remote directory");
    await user.clear(input);
    await user.type(input, "/var/www//storefront/");
    await user.click(screen.getByRole("button", { name: "Scan directory" }));

    await waitFor(() =>
      expect(scanRemoteProjects).toHaveBeenCalledWith(server.id, "/var/www/storefront", false),
    );
    expect(
      await screen.findByText("No Laravel project found", {
        selector: "[data-slot='alert-title']",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/rejected as non-Laravel/)).toBeInTheDocument();
    expect(input).toHaveValue("/var/www/storefront");
  });

  it("browses only the selected directory and lets the user use a child path", async () => {
    vi.mocked(browseRemoteDirectories).mockResolvedValue(listing);
    const user = userEvent.setup();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Browse" }));

    await waitFor(() =>
      expect(browseRemoteDirectories).toHaveBeenCalledWith(server.id, "/var/www"),
    );
    expect(await screen.findByText("Immediate subdirectories")).toBeInTheDocument();
    expect(screen.getByText("/var/www/blog")).toBeInTheDocument();
    expect(screen.queryByText("/var/www/blog/secret")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Use path /var/www/blog" }));

    expect(screen.getByLabelText("Remote directory")).toHaveValue("/var/www/blog");
    expect(scanRemoteProjects).not.toHaveBeenCalled();
  });

  it("requires an absolute normalized path before scanning", async () => {
    const user = userEvent.setup();
    renderPage();

    const input = screen.getByLabelText("Remote directory");
    await user.clear(input);
    await user.type(input, "var/www/storefront");
    await user.click(screen.getByRole("button", { name: "Scan directory" }));

    expect(
      await screen.findByText(/Enter an absolute, normalized remote path/),
    ).toBeInTheDocument();
    expect(scanRemoteProjects).not.toHaveBeenCalled();
  });

  it("keeps saved projects visible when the server is disconnected", async () => {
    const disconnected: ConnectionSnapshot = {
      serverId: server.id,
      state: "disconnected",
      changedAt: 1,
    };
    useConnectionStore.setState({
      connections: { [server.id]: disconnected },
      busy: {},
      error: null,
    });
    vi.mocked(getConnectionState).mockResolvedValue(disconnected);
    vi.mocked(listProjects).mockResolvedValue([project]);
    renderPage();

    expect(await screen.findByLabelText("Project storefront")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Browse" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Scan directory" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Scan subdirectories" })).toBeDisabled();
  });
});
