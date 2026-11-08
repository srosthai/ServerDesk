import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MonitoringCard } from "@/features/dashboard/MonitoringCard";
import { getMonitoringSnapshot } from "@/services/monitoring";
import { useMonitoringStore } from "@/stores/monitoringStore";
import type { MonitoringSnapshot } from "@/types/contracts";

vi.mock("@/services/monitoring", () => ({
  getMonitoringSnapshot: vi.fn(),
}));

const sampledAt = 1_762_665_720_000;

const snapshot: MonitoringSnapshot = {
  serverId: "server-1",
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

function storeSnapshot(value: MonitoringSnapshot = snapshot) {
  useMonitoringStore.setState({
    entries: {
      "server-1": {
        snapshot: value,
        stale: false,
        staleReason: null,
        error: null,
        pending: false,
        lastAttemptAt: sampledAt,
      },
    },
  });
}

describe("MonitoringCard", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useMonitoringStore.setState({ entries: {} });
    vi.mocked(getMonitoringSnapshot).mockResolvedValue(snapshot);
  });

  it("shows units and per-group sample times for a current snapshot", async () => {
    storeSnapshot();

    render(<MonitoringCard serverId="server-1" connected />);

    expect(await screen.findByText("Current")).toBeInTheDocument();
    expect(screen.getByText("0.42")).toBeInTheDocument();
    expect(screen.getByText("5.3%")).toBeInTheDocument();
    expect(screen.getByText("8 logical CPUs available.")).toBeInTheDocument();
    expect(screen.getByText("25.0%")).toBeInTheDocument();
    expect(screen.getByText("8.00 GiB")).toBeInTheDocument();
    expect(screen.getByText("eth0")).toBeInTheDocument();
    expect(screen.getByText("1.40 GiB")).toBeInTheDocument();
    expect(screen.getByText("12,345 RX packets · 6,789 TX packets")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Disk used at /" })).toHaveAttribute(
      "aria-valuenow",
      "40",
    );
    expect(screen.getAllByText(/Sampled/).length).toBeGreaterThanOrEqual(4);
    expect(screen.getByText(/Refreshes every 30 s while connected/)).toBeInTheDocument();
  });

  it("keeps the last snapshot visible and marks it stale after a failed refresh", async () => {
    storeSnapshot();
    vi.mocked(getMonitoringSnapshot).mockRejectedValueOnce({
      code: "timeout",
      message: "The remote command timed out.",
    });

    render(<MonitoringCard serverId="server-1" connected />);

    expect(await screen.findByText("Stale")).toBeInTheDocument();
    expect(screen.getByText("Could not refresh monitoring")).toBeInTheDocument();
    expect(screen.getByText(/The last snapshot is kept but marked stale/)).toBeInTheDocument();
    expect(screen.getByText("0.42")).toBeInTheDocument();
    expect(screen.getByText(/Last successful sample/)).toBeInTheDocument();
  });

  it("shows unsupported and unavailable groups with explicit reasons", () => {
    storeSnapshot({
      ...snapshot,
      memory: {
        status: "unavailable",
        sampledAt,
        data: null,
        detail: "The memory snapshot did not include a usable MemAvailable value.",
      },
      network: {
        status: "unsupported",
        sampledAt,
        data: null,
        detail: "The remote system does not expose /proc/net/dev.",
      },
    });

    render(<MonitoringCard serverId="server-1" connected={false} />);

    expect(screen.getByText("Unavailable")).toBeInTheDocument();
    expect(screen.getByText("Unsupported")).toBeInTheDocument();
    expect(
      screen.getByText("The memory snapshot did not include a usable MemAvailable value."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("The remote system does not expose /proc/net/dev."),
    ).toBeInTheDocument();
  });

  it("stops polling and labels retained values as paused when disconnected", () => {
    storeSnapshot();

    render(<MonitoringCard serverId="server-1" connected={false} />);

    expect(screen.getByText("Paused")).toBeInTheDocument();
    expect(screen.getByText("Monitoring paused")).toBeInTheDocument();
    expect(screen.getByText("0.42")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Refresh monitoring" })).toBeDisabled();
    expect(getMonitoringSnapshot).not.toHaveBeenCalled();
  });

  it("refreshes on demand after an initial failure", async () => {
    vi.mocked(getMonitoringSnapshot).mockRejectedValueOnce({
      code: "not_found",
      message: "The server is not connected.",
    });
    const user = userEvent.setup();

    render(<MonitoringCard serverId="server-1" connected />);

    expect(await screen.findByText("Could not refresh monitoring")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Refresh monitoring" }));

    expect(await screen.findByText("Current")).toBeInTheDocument();
    expect(screen.getByText("0.42")).toBeInTheDocument();
    expect(getMonitoringSnapshot).toHaveBeenCalledTimes(2);
  });

  it("never fabricates metrics before the first successful sample", () => {
    render(<MonitoringCard serverId="server-1" connected={false} />);

    expect(screen.getByText("No sample")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Connect the server to collect CPU, memory, filesystem, and network snapshots. Polling never runs while the server is disconnected.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("0.42")).not.toBeInTheDocument();
    expect(screen.queryByText("25.0%")).not.toBeInTheDocument();
    expect(screen.queryByText("eth0")).not.toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(getMonitoringSnapshot).not.toHaveBeenCalled();
  });
});
