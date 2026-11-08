import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MonitoringPage } from "@/features/monitoring/MonitoringPage";
import { getConnectionState } from "@/services/connections";
import { getMonitoringSnapshot } from "@/services/monitoring";
import { useConnectionStore } from "@/stores/connectionStore";
import { useMonitoringStore } from "@/stores/monitoringStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import type { ConnectionSnapshot, MonitoringSnapshot, ServerProfile } from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    getConnectionState: vi.fn(),
  };
});

vi.mock("@/services/monitoring", () => ({
  getMonitoringSnapshot: vi.fn(),
}));

vi.mock("@/features/monitoring/MonitoringCharts", () => ({
  default: (props: { history: unknown[]; intervalSeconds: number; stale: boolean }) => (
    <div
      data-testid="monitoring-charts"
      data-samples={props.history.length}
      data-interval={props.intervalSeconds}
      data-stale={String(props.stale)}
    />
  ),
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

const snapshot: MonitoringSnapshot = {
  serverId: server.id,
  sampledAt,
  durationMs: 42,
  connectionState: "connected",
  cpu: {
    status: "available",
    sampledAt,
    detail: null,
    data: { load1m: 0.42, load5m: 0.31, load15m: 0.28, logicalCpus: 8 },
  },
  memory: {
    status: "available",
    sampledAt,
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
    sampledAt,
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
    sampledAt,
    detail: null,
    data: [],
  },
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
      <MonitoringPage />
    </MemoryRouter>,
  );
}

describe("MonitoringPage", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    useMonitoringStore.setState({ entries: {} });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES, monitoringIntervalSeconds: 30 },
    });
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(getMonitoringSnapshot).mockResolvedValue(snapshot);
  });

  it("renders charts from polling samples and refreshes on demand", async () => {
    const user = userEvent.setup();

    renderPage();

    expect(screen.getByRole("heading", { name: "Monitoring" })).toBeInTheDocument();
    expect(await screen.findByTestId("monitoring-charts")).toHaveAttribute("data-samples", "1");
    expect(await screen.findByText("Live")).toBeInTheDocument();
    expect(screen.getByLabelText("Refresh monitoring charts")).toBeEnabled();
    expect(getMonitoringSnapshot).toHaveBeenCalledWith(server.id);

    vi.mocked(getMonitoringSnapshot).mockResolvedValueOnce({
      ...snapshot,
      sampledAt: sampledAt + 30_000,
      cpu: {
        ...snapshot.cpu,
        data: { load1m: 1.25, load5m: 0.75, load15m: 0.5, logicalCpus: 8 },
      },
    });

    await user.click(screen.getByLabelText("Refresh monitoring charts"));

    await waitFor(() => {
      expect(screen.getByTestId("monitoring-charts")).toHaveAttribute("data-samples", "2");
    });
    expect(getMonitoringSnapshot).toHaveBeenCalledTimes(2);
  });

  it("shows an honest no-sample state and never fabricates chart data", async () => {
    vi.mocked(getMonitoringSnapshot).mockRejectedValue({
      code: "timeout",
      message: "The remote command timed out.",
    });

    renderPage();

    expect(await screen.findByText("Could not refresh monitoring")).toBeInTheDocument();
    expect(screen.getByText("No chart samples yet")).toBeInTheDocument();
    expect(screen.queryByTestId("monitoring-charts")).not.toBeInTheDocument();
    expect(screen.getByText("No successful sample yet")).toBeInTheDocument();
  });

  it("keeps retained chart samples visible and marks them stale when disconnected", async () => {
    useMonitoringStore.setState({
      entries: {
        [server.id]: {
          snapshot,
          history: [{ attemptedAt: sampledAt, snapshot }],
          stale: true,
          staleReason: "Polling is paused because the server is disconnected.",
          error: null,
          pending: false,
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

    expect(await screen.findByText("Monitoring paused")).toBeInTheDocument();
    expect(screen.getByText("Paused · stale")).toBeInTheDocument();
    expect(screen.getByTestId("monitoring-charts")).toHaveAttribute("data-stale", "true");
    expect(screen.getByLabelText("Refresh monitoring charts")).toBeDisabled();
    expect(getMonitoringSnapshot).not.toHaveBeenCalled();
  });

  it("shows an empty profile state without requesting monitoring", () => {
    useServerStore.setState({ servers: [], status: "ready", error: null });
    useConnectionStore.setState({ connections: {}, busy: {}, error: null });

    renderPage();

    expect(screen.getByText("No server selected")).toBeInTheDocument();
    expect(screen.getAllByRole("link", { name: "Open servers" })[0]).toHaveAttribute(
      "href",
      "/servers",
    );
    expect(getMonitoringSnapshot).not.toHaveBeenCalled();
  });
});
