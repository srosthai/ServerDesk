import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OverviewPage } from "@/features/dashboard/OverviewPage";
import { getConnectionState } from "@/services/connections";
import { getSystemIdentity } from "@/services/identity";
import { getMonitoringSnapshot } from "@/services/monitoring";
import { runRemoteCommand } from "@/services/remote";
import { useConnectionStore } from "@/stores/connectionStore";
import { useMonitoringStore } from "@/stores/monitoringStore";
import { useRemoteCommandStore } from "@/stores/remoteCommandStore";
import { useServerStore } from "@/stores/serverStore";
import { useSystemIdentityStore } from "@/stores/systemIdentityStore";
import type {
  ConnectionSnapshot,
  MonitoringSnapshot,
  RemoteCommandResult,
  ServerProfile,
  SystemIdentity,
} from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    connectServer: vi.fn(),
    disconnectServer: vi.fn(),
    getConnectionState: vi.fn(),
  };
});

vi.mock("@/services/remote", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/remote")>();
  return {
    ...actual,
    runRemoteCommand: vi.fn(),
  };
});

vi.mock("@/services/identity", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/identity")>();
  return {
    ...actual,
    getSystemIdentity: vi.fn(),
  };
});

vi.mock("@/services/monitoring", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/monitoring")>();
  return {
    ...actual,
    getMonitoringSnapshot: vi.fn(),
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

const hostnameResult: RemoteCommandResult = {
  serverId: server.id,
  command: "hostname",
  template: "hostname",
  stdout: "production-1\n",
  stderr: "",
  exitCode: 0,
  succeeded: true,
  durationMs: 18,
  connectionState: "connected",
};

const uptimeResult: RemoteCommandResult = {
  serverId: server.id,
  command: "uptime",
  template: "uptime",
  stdout: "",
  stderr: "uptime: fixture failure\n",
  exitCode: 7,
  succeeded: false,
  durationMs: 22,
  connectionState: "connected",
};

const identity: SystemIdentity = {
  serverId: server.id,
  hostname: "prod-os-host",
  distribution: "Ubuntu",
  distributionVersion: "24.04",
  kernel: "6.8.0-45-generic",
  architecture: "x86_64",
  uptimeSeconds: 372_063,
  sampledAt: 1_762_665_720_000,
  durationMs: 18,
  connectionState: "connected",
};

const monitoringSnapshot: MonitoringSnapshot = {
  serverId: server.id,
  sampledAt: 1_762_665_720_000,
  durationMs: 42,
  connectionState: "connected",
  cpu: {
    status: "available",
    sampledAt: 1_762_665_720_000,
    detail: null,
    data: { load1m: 0.42, load5m: 0.31, load15m: 0.28, logicalCpus: 8 },
  },
  memory: {
    status: "available",
    sampledAt: 1_762_665_720_000,
    detail: null,
    data: {
      totalBytes: 8 * 1024 ** 3,
      usedBytes: 2 * 1024 ** 3,
      availableBytes: 6 * 1024 ** 3,
      usedPercent: 25,
    },
  },
  filesystems: {
    status: "available",
    sampledAt: 1_762_665_720_000,
    detail: null,
    data: [
      {
        filesystem: "/dev/vda1",
        mount: "/",
        totalBytes: 100 * 1024 ** 3,
        usedBytes: 40 * 1024 ** 3,
        availableBytes: 60 * 1024 ** 3,
        usedPercent: 40,
      },
    ],
  },
  network: {
    status: "available",
    sampledAt: 1_762_665_720_000,
    detail: null,
    data: [
      {
        interface: "eth0",
        receivedBytes: 1_500_000_000,
        transmittedBytes: 750_000_000,
        receivedPackets: 12_345,
        transmittedPackets: 6_789,
      },
    ],
  },
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function disableTauriRuntime() {
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
}

describe("OverviewPage", () => {
  beforeEach(() => {
    useServerStore.setState({ servers: [], status: "idle", error: null });
    useConnectionStore.setState({ connections: {}, busy: {}, error: null });
    useRemoteCommandStore.setState({ results: {}, pending: {}, error: null });
    useSystemIdentityStore.setState({
      identities: {},
      pending: {},
      stale: {},
      staleReason: {},
      lastAttemptAt: {},
      error: null,
    });
    useMonitoringStore.setState({ entries: {} });
    vi.mocked(getSystemIdentity).mockResolvedValue(identity);
    vi.mocked(getMonitoringSnapshot).mockResolvedValue(monitoringSnapshot);
  });

  it("adds a server profile from the first screen", async () => {
    disableTauriRuntime();
    useServerStore.setState({ servers: [], status: "ready", error: null });
    const user = userEvent.setup();

    render(
      <MemoryRouter>
        <OverviewPage />
      </MemoryRouter>,
    );

    expect(await screen.findByText("No server profiles yet")).toBeInTheDocument();
    await user.click(screen.getAllByRole("button", { name: "Add server" })[0]);
    await user.type(screen.getByLabelText("Name"), "Production");
    await user.type(screen.getByLabelText("Host"), "198.51.100.24");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.click(screen.getByRole("button", { name: "Save server" }));

    expect(await screen.findByRole("heading", { name: "Production" })).toBeInTheDocument();
    expect(screen.getByText("198.51.100.24:22")).toBeInTheDocument();
    expect(screen.getByText("Desktop only")).toBeInTheDocument();
  });

  it("runs hostname and uptime and shows stdout, stderr, exit codes, and connection state", async () => {
    enableTauriRuntime();
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(runRemoteCommand)
      .mockResolvedValueOnce(hostnameResult)
      .mockResolvedValueOnce(uptimeResult);
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    const user = userEvent.setup();

    render(
      <MemoryRouter>
        <OverviewPage />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Connected")).toBeInTheDocument();
    expect(
      screen.getByText("The authenticated SSH session is ready for fixed remote commands."),
    ).toBeInTheDocument();

    expect(await screen.findByText("prod-os-host")).toBeInTheDocument();
    expect(screen.getByText("Ubuntu")).toBeInTheDocument();
    expect(screen.getByText("24.04")).toBeInTheDocument();
    expect(screen.getByText("6.8.0-45-generic")).toBeInTheDocument();
    expect(screen.getByText("x86_64")).toBeInTheDocument();
    expect(screen.getByText("4 days, 7 hours")).toBeInTheDocument();
    expect(screen.getAllByText("Sampled").length).toBeGreaterThan(0);
    expect(screen.getByText(/Read-only sample completed in 18 ms/)).toBeInTheDocument();
    expect(getSystemIdentity).toHaveBeenCalledWith(server.id);
    expect(document.querySelector("time")).toHaveAttribute(
      "dateTime",
      new Date(identity.sampledAt).toISOString(),
    );

    expect(await screen.findByText("Resource monitoring")).toBeInTheDocument();
    expect(await screen.findByText("0.42")).toBeInTheDocument();
    expect(screen.getByText("25.0%")).toBeInTheDocument();
    expect(screen.getByText("Current")).toBeInTheDocument();
    expect(screen.getByText("Live")).toBeInTheDocument();
    expect(screen.getByText("Every 30 s")).toBeInTheDocument();
    expect(getMonitoringSnapshot).toHaveBeenCalledWith(server.id);
    expect(screen.getAllByText(/Sampled/).length).toBeGreaterThanOrEqual(4);

    await user.click(screen.getByRole("button", { name: "Run hostname" }));

    expect(await screen.findByText(/production-1/)).toBeInTheDocument();
    expect(screen.getByText("Exit 0")).toBeInTheDocument();
    expect(runRemoteCommand).toHaveBeenNthCalledWith(1, server.id, "hostname");

    await user.click(screen.getByRole("button", { name: "Run uptime" }));

    expect(await screen.findByText(/uptime: fixture failure/)).toBeInTheDocument();
    expect(screen.getByText("Exit 7")).toBeInTheDocument();
    expect(runRemoteCommand).toHaveBeenNthCalledWith(2, server.id, "uptime");
    expect(screen.getByText("Connected")).toBeInTheDocument();
  });

  it("shows the host-key verification prompt before a first connection", async () => {
    enableTauriRuntime();
    vi.mocked(getConnectionState).mockResolvedValue({
      serverId: server.id,
      state: "host-key-verification",
      changedAt: 2,
      hostKey: {
        keyType: "ssh-ed25519",
        fingerprint: "SHA256:firstseen",
      },
    });
    useServerStore.setState({ servers: [server], status: "ready", error: null });

    render(
      <MemoryRouter>
        <OverviewPage />
      </MemoryRouter>,
    );

    expect(await screen.findByRole("heading", { name: "Verify SSH host key" })).toBeInTheDocument();
    expect(screen.getAllByText("SHA256:firstseen").length).toBeGreaterThan(0);
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Trust and connect" })).toBeInTheDocument();
    });
  });

  it("shows unavailable identity fields and samples again on demand", async () => {
    enableTauriRuntime();
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(getSystemIdentity)
      .mockResolvedValueOnce({
        ...identity,
        distribution: null,
        distributionVersion: null,
        uptimeSeconds: null,
      })
      .mockResolvedValueOnce(identity);
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    const user = userEvent.setup();

    render(
      <MemoryRouter>
        <OverviewPage />
      </MemoryRouter>,
    );

    expect(await screen.findByText("prod-os-host")).toBeInTheDocument();
    expect(screen.getAllByText("Unavailable")).toHaveLength(3);
    expect(getSystemIdentity).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Refresh identity" }));

    expect(await screen.findByText("Ubuntu")).toBeInTheDocument();
    expect(screen.getByText("4 days, 7 hours")).toBeInTheDocument();
    expect(getSystemIdentity).toHaveBeenCalledTimes(2);
  });

  it("shows an honest unavailable dashboard for a disconnected server", () => {
    enableTauriRuntime();
    vi.mocked(getConnectionState).mockResolvedValue({
      serverId: server.id,
      state: "disconnected",
      changedAt: 1,
    });
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: { serverId: server.id, state: "disconnected", changedAt: 1 } },
      busy: {},
      error: null,
    });

    render(
      <MemoryRouter>
        <OverviewPage />
      </MemoryRouter>,
    );

    expect(screen.getByText("Disconnected")).toBeInTheDocument();
    expect(screen.getByText("Unavailable")).toBeInTheDocument();
    expect(screen.getByText("No successful sample yet")).toBeInTheDocument();
    expect(screen.getByText("Paused while disconnected")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Connect the server to collect CPU, memory, filesystem, and network snapshots. Polling never runs while the server is disconnected.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Refresh server overview" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Refresh identity" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Refresh monitoring" })).toBeDisabled();
    expect(screen.queryByText("0.42")).not.toBeInTheDocument();
    expect(screen.queryByText("25.0%")).not.toBeInTheDocument();
    expect(getSystemIdentity).not.toHaveBeenCalled();
    expect(getMonitoringSnapshot).not.toHaveBeenCalled();
  });

  it("refreshes identity and monitoring together without a page reload", async () => {
    enableTauriRuntime();
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(getSystemIdentity)
      .mockResolvedValueOnce(identity)
      .mockResolvedValueOnce({
        ...identity,
        hostname: "prod-os-host-2",
        uptimeSeconds: 372_125,
        sampledAt: identity.sampledAt + 60_000,
      });
    vi.mocked(getMonitoringSnapshot)
      .mockResolvedValueOnce(monitoringSnapshot)
      .mockResolvedValueOnce({
        ...monitoringSnapshot,
        sampledAt: monitoringSnapshot.sampledAt + 60_000,
        cpu: {
          ...monitoringSnapshot.cpu,
          data: { load1m: 1.24, load5m: 0.75, load15m: 0.5, logicalCpus: 8 },
        },
      });
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    const user = userEvent.setup();

    render(
      <MemoryRouter>
        <OverviewPage />
      </MemoryRouter>,
    );

    expect(await screen.findByText("prod-os-host")).toBeInTheDocument();
    expect(await screen.findByText("0.42")).toBeInTheDocument();
    expect(getSystemIdentity).toHaveBeenCalledTimes(1);
    expect(getMonitoringSnapshot).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Refresh server overview" }));

    expect(await screen.findByText("prod-os-host-2")).toBeInTheDocument();
    expect(await screen.findByText("1.24")).toBeInTheDocument();
    expect(getSystemIdentity).toHaveBeenCalledTimes(2);
    expect(getMonitoringSnapshot).toHaveBeenCalledTimes(2);
  });

  it("marks a retained identity sample stale with its last successful sample time", async () => {
    enableTauriRuntime();
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(getSystemIdentity).mockResolvedValueOnce(identity).mockRejectedValueOnce({
      code: "timeout",
      message: "The remote command timed out.",
    });
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    const user = userEvent.setup();

    render(
      <MemoryRouter>
        <OverviewPage />
      </MemoryRouter>,
    );

    expect(await screen.findByText("prod-os-host")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Refresh identity" }));

    expect(await screen.findByText("Stale sample")).toBeInTheDocument();
    expect(screen.getByText("Could not collect server identity")).toBeInTheDocument();
    expect(screen.getByText(/Showing the last successful sample from/)).toBeInTheDocument();
    expect(screen.getByText(/These values are stale and are not live/)).toBeInTheDocument();
    expect(screen.getByText("prod-os-host")).toBeInTheDocument();
    expect(screen.getAllByText(/Last successful sample/).length).toBeGreaterThan(0);
  });
});
