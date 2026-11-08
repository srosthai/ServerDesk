import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServersPage } from "@/features/servers/ServersPage";
import { useConnectionStore } from "@/stores/connectionStore";
import { useServerStore } from "@/stores/serverStore";
import type { ServerProfile } from "@/types/contracts";

const originalTrustHostKey = useConnectionStore.getState().trustHostKey;
const originalRejectHostKey = useConnectionStore.getState().rejectHostKey;

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

describe("ServersPage", () => {
  beforeEach(() => {
    useServerStore.setState({
      servers: [],
      status: "idle",
      error: null,
    });
    useConnectionStore.setState({
      connections: {},
      busy: {},
      error: null,
      trustHostKey: originalTrustHostKey,
      rejectHostKey: originalRejectHostKey,
    });
  });

  it("creates, searches, and deletes a server profile", async () => {
    const user = userEvent.setup();
    render(<ServersPage />);

    expect(await screen.findByText("No server profiles yet")).toBeInTheDocument();
    await user.click(screen.getAllByRole("button", { name: "Add server" })[0]);

    await user.type(screen.getByLabelText("Name"), "Production");
    await user.type(screen.getByLabelText("Host"), "198.51.100.24");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.click(screen.getByRole("button", { name: "Save server" }));

    expect(await screen.findByText("Production")).toBeInTheDocument();
    expect(screen.getByText("198.51.100.24:22")).toBeInTheDocument();

    await user.type(screen.getByLabelText("Search servers"), "staging");
    expect(await screen.findByText("No servers match")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(await screen.findByText("Production")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete Production" }));
    expect(screen.getByText("Delete Production?")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Delete server" }));

    await waitFor(() => {
      expect(screen.queryByText("Production")).not.toBeInTheDocument();
    });
  });

  it("shows connection state and typed errors from the connection store", async () => {
    useServerStore.setState({
      servers: [server],
      status: "ready",
      error: null,
    });
    useConnectionStore.setState({
      connections: {
        [server.id]: {
          serverId: server.id,
          state: "connected",
          changedAt: 1,
        },
      },
      busy: {},
      error: null,
    });

    render(<ServersPage />);
    expect(screen.getByText("Connected")).toBeInTheDocument();

    useConnectionStore.getState().applySnapshot({
      serverId: server.id,
      state: "error",
      changedAt: 2,
      error: { code: "timeout", message: "Connection timed out after 15 seconds." },
    });

    expect(await screen.findByText("Connection timed out after 15 seconds.")).toBeInTheDocument();
    expect(screen.getByText("Connection timed out")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Check that the host is reachable, the SSH port is correct, and a firewall is not blocking the connection.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Connection error")).toBeInTheDocument();
  });

  it.each([
    ["disconnected", "Disconnected"],
    ["connecting", "Connecting"],
    ["host-key-verification", "Verify host key"],
    ["reconnecting", "Reconnecting"],
  ] as const)("renders the %s connection state", (state, label) => {
    useServerStore.setState({
      servers: [server],
      status: "ready",
      error: null,
    });
    useConnectionStore.setState({
      connections: {
        [server.id]: {
          serverId: server.id,
          state,
          changedAt: 1,
        },
      },
      busy: {},
      error: null,
    });

    render(<ServersPage />);

    expect(screen.getAllByText(label).length).toBeGreaterThan(0);
  });

  it("shows a distinct authentication error instead of a generic failure", async () => {
    useServerStore.setState({
      servers: [server],
      status: "ready",
      error: null,
    });
    useConnectionStore.setState({
      connections: {
        [server.id]: {
          serverId: server.id,
          state: "error",
          changedAt: 3,
          error: {
            code: "authentication",
            message: "SSH authentication failed. Check the username and credentials.",
          },
        },
      },
      busy: {},
      error: null,
    });

    render(<ServersPage />);

    expect(
      screen.getByText("SSH authentication failed. Check the username and credentials."),
    ).toBeInTheDocument();
  });

  it("requires explicit acceptance before trusting a first-seen host key", async () => {
    const user = userEvent.setup();
    const trust = vi.fn().mockResolvedValue(true);
    const reject = vi.fn().mockResolvedValue(true);
    useServerStore.setState({
      servers: [server],
      status: "ready",
      error: null,
    });
    useConnectionStore.setState({
      connections: {
        [server.id]: {
          serverId: server.id,
          state: "host-key-verification",
          changedAt: 1,
          hostKey: {
            keyType: "ssh-ed25519",
            fingerprint: "SHA256:firstseen",
          },
        },
      },
      busy: {},
      error: null,
      trustHostKey: trust,
      rejectHostKey: reject,
    });

    render(<ServersPage />);

    expect(screen.getByRole("heading", { name: "Verify SSH host key" })).toBeInTheDocument();
    expect(screen.getAllByText("SHA256:firstseen").length).toBeGreaterThan(0);

    await user.click(screen.getByRole("button", { name: "Trust and connect" }));
    expect(trust).toHaveBeenCalledWith(server.id, "ssh-ed25519", "SHA256:firstseen");
    expect(reject).not.toHaveBeenCalled();
  });

  it("shows a blocking security warning when the host key changed", async () => {
    const user = userEvent.setup();
    useServerStore.setState({
      servers: [server],
      status: "ready",
      error: null,
    });
    useConnectionStore.setState({
      connections: {
        [server.id]: {
          serverId: server.id,
          state: "error",
          changedAt: 1,
          error: {
            code: "host_key",
            message: "The server host key has changed. The connection was blocked.",
          },
          hostKey: {
            keyType: "ssh-ed25519",
            fingerprint: "SHA256:received",
            expectedFingerprint: "SHA256:saved",
          },
        },
      },
      busy: {},
      error: null,
    });

    render(<ServersPage />);

    expect(screen.getByRole("heading", { name: "SSH host key changed" })).toBeInTheDocument();
    expect(screen.getByText("SHA256:saved")).toBeInTheDocument();
    expect(screen.getAllByText("SHA256:received").length).toBeGreaterThan(0);
    expect(screen.queryByRole("button", { name: "Trust and connect" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Dismiss warning" }));
    await waitFor(() => {
      expect(
        screen.queryByRole("heading", { name: "SSH host key changed" }),
      ).not.toBeInTheDocument();
    });
  });
});
