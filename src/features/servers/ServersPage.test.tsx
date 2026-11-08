import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ServersPage } from "@/features/servers/ServersPage";
import { useConnectionStore } from "@/stores/connectionStore";
import { useServerStore } from "@/stores/serverStore";
import type { ServerProfile } from "@/types/contracts";

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
    expect(screen.getByText("Connection error")).toBeInTheDocument();
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
});
