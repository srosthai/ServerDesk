import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OverviewPage } from "@/features/dashboard/OverviewPage";
import { getConnectionState } from "@/services/connections";
import { runRemoteCommand } from "@/services/remote";
import { useConnectionStore } from "@/stores/connectionStore";
import { useRemoteCommandStore } from "@/stores/remoteCommandStore";
import { useServerStore } from "@/stores/serverStore";
import type { ConnectionSnapshot, RemoteCommandResult, ServerProfile } from "@/types/contracts";

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
});
