import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServicesPage } from "@/features/services/ServicesPage";
import { getConnectionState } from "@/services/connections";
import { getSystemdServices } from "@/services/services";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import { useServiceStore } from "@/stores/serviceStore";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import type { ConnectionSnapshot, ServerProfile, ServiceDiscovery } from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    getConnectionState: vi.fn(),
  };
});

vi.mock("@/services/services", () => ({
  getSystemdServices: vi.fn(),
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
    useServiceStore.setState({ entries: {} });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES, lastActiveServerId: server.id },
    });
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(getSystemdServices).mockResolvedValue(discovery);
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
