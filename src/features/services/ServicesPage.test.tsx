import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServicesPage } from "@/features/services/ServicesPage";
import { getConnectionState } from "@/services/connections";
import { getSystemdServices, runServiceAction } from "@/services/services";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import { useServiceStore } from "@/stores/serviceStore";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import type {
  ConnectionSnapshot,
  ServerProfile,
  ServiceActionResult,
  ServiceDiscovery,
} from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    getConnectionState: vi.fn(),
  };
});

vi.mock("@/services/services", () => ({
  getSystemdServices: vi.fn(),
  runServiceAction: vi.fn(),
}));

const sampledAt = Date.now();

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

const discovery: ServiceDiscovery = {
  serverId: server.id,
  status: "available",
  services: [
    {
      name: "nginx.service",
      state: "active",
      activeState: "active",
      subState: "running",
      loadState: "loaded",
      description: "A high performance web server",
    },
    {
      name: "apache2.service",
      state: "failed",
      activeState: "failed",
      subState: "failed",
      loadState: "loaded",
      description: "The Apache HTTP Server",
    },
    {
      name: "mysql.service",
      state: "inactive",
      activeState: "inactive",
      subState: "dead",
      loadState: "loaded",
      description: "MySQL Community Server",
    },
    {
      name: "redis-server.service",
      state: "unknown",
      activeState: "activating",
      subState: "start",
      loadState: "loaded",
      description: "Redis persistent key-value database",
    },
    {
      name: "php8.3-fpm.service",
      state: "active",
      activeState: "active",
      subState: "running",
      loadState: "loaded",
      description: "The PHP 8.3 FastCGI Process Manager",
    },
    {
      name: "custom-worker.service",
      state: "active",
      activeState: "active",
      subState: "running",
      loadState: "loaded",
      description: "A service that is not in any common list",
    },
  ],
  detail: null,
  truncated: false,
  sampledAt,
  durationMs: 31,
  connectionState: "connected",
};

const restartResult: ServiceActionResult = {
  serverId: server.id,
  unit: "nginx.service",
  action: "restart",
  command: "/usr/bin/sudo -n -- /usr/bin/systemctl restart -- nginx.service",
  succeeded: true,
  stdout: "restarted\n",
  stderr: "",
  exitCode: 0,
  durationMs: 42,
  permissionDenied: false,
  sudoGuidance: null,
  verifiedStatus: {
    serverId: server.id,
    unit: "nginx.service",
    state: "active",
    activeState: "active",
    subState: "running",
    loadState: "loaded",
    unitFileState: "enabled",
    description: "A high performance web server",
    sampledAt: sampledAt + 1_000,
    durationMs: 7,
  },
  stateVerified: true,
  verificationError: null,
  error: null,
  connectionState: "connected",
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function renderPage() {
  return render(
    <MemoryRouter>
      <ServicesPage />
    </MemoryRouter>,
  );
}

describe("ServicesPage", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    useServiceStore.setState({ entries: {}, actions: {} });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES, lastActiveServerId: server.id },
    });
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(getSystemdServices).mockResolvedValue(discovery);
    vi.mocked(runServiceAction).mockReset();
  });

  it("lists discovered units with distinct active, failed, inactive, and unknown badges", async () => {
    renderPage();

    expect(screen.getByRole("heading", { name: "Services" })).toBeInTheDocument();
    expect(await screen.findByText("Discovered systemd units")).toBeInTheDocument();
    expect(screen.getByText("nginx.service")).toBeInTheDocument();
    expect(screen.getByText("php8.3-fpm.service")).toBeInTheDocument();
    expect(screen.getByText("custom-worker.service")).toBeInTheDocument();
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Inactive")).toBeInTheDocument();
    expect(screen.getByText("Unknown")).toBeInTheDocument();
    expect(screen.getAllByText("Active").length).toBeGreaterThan(0);
    expect(screen.getByText("activating / start")).toBeInTheDocument();
    expect(getSystemdServices).toHaveBeenCalledWith(server.id);
  });

  it("refreshes discovery on demand", async () => {
    const user = userEvent.setup();
    renderPage();

    await screen.findByText("Discovered systemd units");
    expect(getSystemdServices).toHaveBeenCalledTimes(1);

    vi.mocked(getSystemdServices).mockResolvedValueOnce({
      ...discovery,
      sampledAt: sampledAt + 30_000,
    });
    await user.click(screen.getByLabelText("Refresh discovered services"));

    await waitFor(() => {
      expect(getSystemdServices).toHaveBeenCalledTimes(2);
    });
  });

  it("confirms a restart and shows the verified state instead of an optimistic result", async () => {
    const user = userEvent.setup();
    vi.mocked(runServiceAction).mockResolvedValue(restartResult);
    renderPage();

    await screen.findByText("Discovered systemd units");
    await user.click(screen.getByLabelText("Restart nginx.service"));

    expect(screen.getByRole("dialog", { name: "Restart nginx.service?" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Restart unit" }));

    await waitFor(() => {
      expect(runServiceAction).toHaveBeenCalledWith(server.id, "nginx.service", "restart");
    });

    const resultPanel = await screen.findByLabelText("Service action result");
    expect(within(resultPanel).getByText(/Verified state after the action/)).toBeInTheDocument();
    expect(
      within(resultPanel).getByText(/systemd reports nginx.service as active/),
    ).toBeInTheDocument();
    expect(within(resultPanel).getByText("restarted")).toBeInTheDocument();
    expect(within(resultPanel).getByText("No stderr.")).toBeInTheDocument();
  });

  it("shows sudo guidance and remote output when an action is denied", async () => {
    const user = userEvent.setup();
    vi.mocked(runServiceAction).mockResolvedValue({
      ...restartResult,
      action: "stop",
      command: "/usr/bin/sudo -n -- /usr/bin/systemctl stop -- nginx.service",
      succeeded: false,
      stdout: "",
      stderr: "sudo: a password is required\n",
      exitCode: 1,
      permissionDenied: true,
      sudoGuidance:
        "<serverdesk-ssh-user> ALL=(root) NOPASSWD: /usr/bin/systemctl stop -- nginx.service",
      verifiedStatus: {
        ...restartResult.verifiedStatus!,
        state: "inactive",
        activeState: "inactive",
        subState: "dead",
      },
      stateVerified: false,
      error: {
        code: "permission",
        message: "The SSH account is not allowed to stop nginx.service.",
        details: "Never grant unrestricted sudo.",
      },
    });
    renderPage();

    await screen.findByText("Discovered systemd units");
    await user.click(screen.getByLabelText("Stop nginx.service"));
    await user.click(screen.getByRole("button", { name: "Stop unit" }));

    const resultPanel = await screen.findByLabelText("Service action result");
    expect(within(resultPanel).getByText("Sudo permission denied")).toBeInTheDocument();
    expect(within(resultPanel).getByText(/NOPASSWD/)).toBeInTheDocument();
    expect(within(resultPanel).getByText("sudo: a password is required")).toBeInTheDocument();
    expect(within(resultPanel).getByText("Exit code 1")).toBeInTheDocument();
  });

  it("shows a typed unsupported state without inventing services", async () => {
    vi.mocked(getSystemdServices).mockResolvedValue({
      ...discovery,
      status: "unsupported",
      services: [],
      detail: "Systemd is not available on this server (exit code 127).",
    });

    renderPage();

    expect(await screen.findByText("Systemd is not available")).toBeInTheDocument();
    expect(screen.getByText(/exit code 127/)).toBeInTheDocument();
    expect(screen.queryByText("Discovered systemd units")).not.toBeInTheDocument();
    expect(screen.queryByText("nginx.service")).not.toBeInTheDocument();
  });

  it("keeps the last discovery visible and disables refresh while disconnected", async () => {
    useServiceStore.setState({
      entries: {
        [server.id]: {
          discovery,
          pending: false,
          error: null,
          lastAttemptAt: sampledAt,
        },
      },
    });
    useConnectionStore.setState({
      connections: {
        [server.id]: { serverId: server.id, state: "disconnected", changedAt: 2 },
      },
      busy: {},
      error: null,
    });

    renderPage();

    expect(await screen.findByText("Discovered systemd units")).toBeInTheDocument();
    expect(screen.getByLabelText("Refresh discovered services")).toBeDisabled();
    expect(screen.getByText("Read-only")).toBeInTheDocument();
    expect(getSystemdServices).not.toHaveBeenCalled();
  });

  it("shows an empty profile state without requesting discovery", () => {
    useServerStore.setState({ servers: [], status: "ready", error: null });
    useConnectionStore.setState({ connections: {}, busy: {}, error: null });

    renderPage();

    expect(screen.getByText("No server selected")).toBeInTheDocument();
    expect(screen.getAllByRole("link", { name: "Open servers" })[0]).toHaveAttribute(
      "href",
      "/servers",
    );
    expect(getSystemdServices).not.toHaveBeenCalled();
  });
});
