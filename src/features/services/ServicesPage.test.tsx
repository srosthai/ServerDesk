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
  ServiceUnit,
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
const writeText = vi.fn<(value: string) => Promise<void>>();

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

const services: ServiceUnit[] = [
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
];

const discovery: ServiceDiscovery = {
  serverId: server.id,
  status: "available",
  services,
  detail: null,
  truncated: false,
  sampledAt,
  durationMs: 31,
  connectionState: "connected",
};

const stoppedDiscovery: ServiceDiscovery = {
  ...discovery,
  services: services.map((service) =>
    service.name === "nginx.service"
      ? {
          ...service,
          state: "inactive",
          activeState: "inactive",
          subState: "dead",
        }
      : service,
  ),
  sampledAt: sampledAt + 2_000,
  durationMs: 19,
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

const stopResult: ServiceActionResult = {
  ...restartResult,
  action: "stop",
  command: "/usr/bin/sudo -n -- /usr/bin/systemctl stop -- nginx.service",
  stdout: "stopped\n",
  durationMs: 28,
  verifiedStatus: {
    ...restartResult.verifiedStatus!,
    state: "inactive",
    activeState: "inactive",
    subState: "dead",
  },
};

const startResult: ServiceActionResult = {
  ...restartResult,
  action: "start",
  command: "/usr/bin/sudo -n -- /usr/bin/systemctl start -- nginx.service",
  stdout: "",
  stderr: "already active\n",
  durationMs: 18,
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

function rowFor(unit: string) {
  const unitCell = screen.getByText(unit);
  const row = unitCell.closest("tr");
  if (!row) {
    throw new Error(`No table row found for ${unit}`);
  }
  return row;
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
    vi.mocked(getSystemdServices).mockReset();
    vi.mocked(getSystemdServices).mockResolvedValue(discovery);
    vi.mocked(runServiceAction).mockReset();
    writeText.mockReset();
    writeText.mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
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
    expect(within(rowFor("nginx.service")).getByText("load loaded")).toBeInTheDocument();
    expect(getSystemdServices).toHaveBeenCalledWith(server.id);
  });

  it("filters by free text and by normalized state, then clears filters", async () => {
    const user = userEvent.setup();
    renderPage();

    await screen.findByText("Discovered systemd units");
    const filter = screen.getByLabelText("Filter units");

    await user.type(filter, "php");
    expect(screen.getByText("php8.3-fpm.service")).toBeInTheDocument();
    expect(screen.queryByText("nginx.service")).not.toBeInTheDocument();
    expect(screen.getByText("1 of 6 units")).toBeInTheDocument();

    await user.clear(filter);
    await user.click(screen.getByRole("button", { name: "Show failed units (1)" }));
    expect(screen.getByText("apache2.service")).toBeInTheDocument();
    expect(screen.queryByText("nginx.service")).not.toBeInTheDocument();
    expect(screen.getByText("1 of 6 units")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(screen.getByText("nginx.service")).toBeInTheDocument();
    expect(screen.getByText("6 of 6 units")).toBeInTheDocument();

    await user.type(filter, "does-not-exist");
    expect(screen.getByText("No units match this filter")).toBeInTheDocument();
    await user.click(screen.getAllByRole("button", { name: "Clear filters" })[0]!);
    expect(screen.getByText("nginx.service")).toBeInTheDocument();
  });

  it("sorts by unit name and by state severity", async () => {
    const user = userEvent.setup();
    renderPage();

    await screen.findByText("Discovered systemd units");
    const table = screen.getByRole("table");
    let rows = within(table).getAllByRole("row");
    expect(within(rows[1]).getByText("apache2.service")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Sort by state" }));
    rows = within(table).getAllByRole("row");
    expect(within(rows[1]).getByText("apache2.service")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "State" })).toHaveAttribute(
      "aria-sort",
      "ascending",
    );

    await user.click(screen.getByRole("button", { name: "Sort by state" }));
    rows = within(table).getAllByRole("row");
    expect(within(rows[1]).getByText("mysql.service")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "State" })).toHaveAttribute(
      "aria-sort",
      "descending",
    );

    await user.click(screen.getByRole("button", { name: "Sort by unit" }));
    rows = within(table).getAllByRole("row");
    expect(within(rows[1]).getByText("apache2.service")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Unit" })).toHaveAttribute(
      "aria-sort",
      "ascending",
    );
  });

  it("requires confirmation for stop and restart, naming the server and unit", async () => {
    const user = userEvent.setup();
    vi.mocked(runServiceAction).mockResolvedValue(restartResult);
    renderPage();

    await screen.findByText("Discovered systemd units");
    await user.click(screen.getByLabelText("Stop nginx.service"));

    const stopDialog = screen.getByRole("dialog", {
      name: "Stop nginx.service on Production?",
    });
    expect(
      within(stopDialog).getByText("Production (deploy@198.51.100.24:22)"),
    ).toBeInTheDocument();
    expect(within(stopDialog).getByText("nginx.service")).toBeInTheDocument();
    expect(within(stopDialog).getByText("systemctl stop -- nginx.service")).toBeInTheDocument();

    await user.click(within(stopDialog).getByRole("button", { name: "Cancel" }));
    expect(runServiceAction).not.toHaveBeenCalled();

    await user.click(screen.getByLabelText("Restart nginx.service"));
    const restartDialog = screen.getByRole("dialog", {
      name: "Restart nginx.service on Production?",
    });
    await user.click(within(restartDialog).getByRole("button", { name: "Restart unit" }));

    await waitFor(() => {
      expect(runServiceAction).toHaveBeenCalledWith(server.id, "nginx.service", "restart", true);
    });
  });

  it("runs inspect and start without a confirmation dialog", async () => {
    const user = userEvent.setup();
    vi.mocked(runServiceAction).mockResolvedValue(startResult);
    renderPage();

    await screen.findByText("Discovered systemd units");
    await user.click(screen.getByLabelText("Start nginx.service"));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(runServiceAction).toHaveBeenCalledWith(server.id, "nginx.service", "start", false);
    });

    await user.click(screen.getByLabelText("Inspect nginx.service"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(runServiceAction).toHaveBeenCalledWith(server.id, "nginx.service", "status", false);
    });
  });

  it("shows the verified state, command output, exit status, and copyable transcript", async () => {
    const user = userEvent.setup();
    vi.mocked(runServiceAction).mockResolvedValue(restartResult);
    renderPage();

    await screen.findByText("Discovered systemd units");
    await user.click(screen.getByLabelText("Restart nginx.service"));
    await user.click(screen.getByRole("button", { name: "Restart unit" }));

    const resultPanel = await screen.findByLabelText("Service action result");
    expect(within(resultPanel).getByText("Verified")).toBeInTheDocument();
    expect(within(resultPanel).getByText(/Verified state after the action/)).toBeInTheDocument();
    expect(
      within(resultPanel).getByText(/systemd reports nginx.service as active/),
    ).toBeInTheDocument();
    expect(within(resultPanel).getByText("Exit code 0")).toBeInTheDocument();
    expect(within(resultPanel).getByText("exit 0 · 42 ms")).toBeInTheDocument();

    const terminal = resultPanel.querySelector("pre");
    expect(terminal?.textContent).toContain(
      "$ /usr/bin/sudo -n -- /usr/bin/systemctl restart -- nginx.service",
    );
    expect(terminal?.textContent).toContain("restarted");
    expect(terminal?.textContent).toContain("No stderr.");

    const clipboardWrite = vi.spyOn(navigator.clipboard, "writeText");
    await user.click(within(resultPanel).getByRole("button", { name: "Copy" }));
    expect(clipboardWrite).toHaveBeenCalledTimes(1);
    const copied = clipboardWrite.mock.calls[0]?.[0] ?? "";
    expect(copied).toContain("/usr/bin/systemctl restart -- nginx.service");
    expect(copied).toContain("exit 0 - 42 ms");
    expect(copied).toContain("stdout:");
    expect(copied).toContain("restarted");
    expect(copied).toContain("stderr:");
    expect(copied).toContain("No stderr.");
  });

  it("refreshes the discovered unit list after an action", async () => {
    const user = userEvent.setup();
    vi.mocked(getSystemdServices)
      .mockReset()
      .mockResolvedValueOnce(discovery)
      .mockResolvedValue(stoppedDiscovery);
    vi.mocked(runServiceAction).mockResolvedValue(stopResult);
    renderPage();

    await screen.findByText("Discovered systemd units");
    expect(getSystemdServices).toHaveBeenCalledTimes(1);

    await user.click(screen.getByLabelText("Stop nginx.service"));
    await user.click(screen.getByRole("button", { name: "Stop unit" }));

    await waitFor(() => {
      expect(getSystemdServices).toHaveBeenCalledTimes(2);
    });

    const resultPanel = await screen.findByLabelText("Service action result");
    expect(
      within(resultPanel).getByText(/systemd reports nginx.service as inactive/),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(within(rowFor("nginx.service")).getByText("Inactive")).toBeInTheDocument();
    });
    expect(within(rowFor("nginx.service")).getByText("inactive / dead")).toBeInTheDocument();
  });

  it("shows sudo guidance and failed command output with the exit code", async () => {
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
    expect(within(resultPanel).getByText("Command failed")).toBeInTheDocument();
    expect(within(resultPanel).getByText("Sudo permission denied")).toBeInTheDocument();
    expect(within(resultPanel).getByText(/NOPASSWD/)).toBeInTheDocument();
    expect(within(resultPanel).getByText("Exit code 1")).toBeInTheDocument();

    const commandOutput = within(resultPanel).getByText("Command output").closest("section");
    const terminal = commandOutput?.querySelector("pre");
    expect(terminal?.textContent).toContain("sudo: a password is required");
    expect(terminal?.textContent).toContain("exit 1 - 42 ms");
  });

  it("paginates a server with many units and keeps filtering readable", async () => {
    const user = userEvent.setup();
    const manyServices: ServiceUnit[] = Array.from({ length: 60 }, (_, index) => {
      const state = (["failed", "active", "inactive", "unknown"] as const)[index % 4];
      return {
        name: `unit-${(index + 1).toString().padStart(2, "0")}.service`,
        state,
        activeState: state === "unknown" ? "activating" : state,
        subState: state === "active" ? "running" : "dead",
        loadState: "loaded",
        description: `Generated unit ${(index + 1).toString()}`,
      };
    });
    vi.mocked(getSystemdServices).mockResolvedValue({ ...discovery, services: manyServices });
    renderPage();

    expect(await screen.findByText(/60 units reported by systemctl/)).toBeInTheDocument();
    expect(screen.getByText(/Showing 1-25 of 60 units/)).toBeInTheDocument();
    const table = screen.getByRole("table");
    expect(within(table).getAllByRole("row")).toHaveLength(26);

    await user.click(screen.getByRole("button", { name: "Next page" }));
    expect(screen.getByText(/Showing 26-50 of 60 units/)).toBeInTheDocument();
    expect(within(table).getAllByRole("row")).toHaveLength(26);

    await user.click(screen.getByRole("button", { name: "Next page" }));
    expect(screen.getByText(/Showing 51-60 of 60 units/)).toBeInTheDocument();
    expect(within(table).getAllByRole("row")).toHaveLength(11);
    expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();

    await user.type(screen.getByLabelText("Filter units"), "unit-01");
    expect(
      screen.getByText(/Showing 1-1 of 1 filtered \(60 discovered\) units/),
    ).toBeInTheDocument();
    expect(screen.getByText("unit-01.service")).toBeInTheDocument();
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
    expect(screen.getByLabelText("Restart nginx.service")).toBeDisabled();
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
