import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseManagerPage } from "@/features/databases/DatabaseManagerPage";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type { ConnectionSnapshot, ServerProfile } from "@/types/contracts";

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

function renderPage() {
  return render(
    <MemoryRouter>
      <DatabaseManagerPage />
    </MemoryRouter>,
  );
}

function useConnectedServer() {
  useServerStore.setState({ servers: [server], status: "ready", error: null });
  useConnectionStore.setState({
    connections: { [server.id]: connected },
    busy: {},
    error: null,
  });
}

describe("DatabaseManagerPage", () => {
  beforeEach(() => {
    useServerStore.setState({
      servers: [],
      status: "ready",
      error: null,
      fetchServers: vi.fn(),
    });
    useConnectionStore.setState({ connections: {}, busy: {}, error: null });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES },
      status: "unavailable",
      error: null,
    });
  });

  it("renders the connected workspace empty state", () => {
    useConnectedServer();
    renderPage();

    expect(screen.getByRole("heading", { name: "Databases" })).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "No database connections yet" }),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Connected").length).toBeGreaterThan(0);
    expect(screen.getByText("Workspace ready. No database connections configured.")).toBeVisible();
    expect(screen.getByText("0 open connections")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add your first connection" })).toBeInTheDocument();
  });

  it("opens and closes the placeholder add-connection flow", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Add connection" }));

    const dialog = await screen.findByRole("dialog", { name: "Add database connection" });
    expect(within(dialog).getByText("Planned connection flow")).toBeInTheDocument();
    expect(
      within(dialog).getByText(/No database driver is loaded and no tunnel or connection/),
    ).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Back to workspace" }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
  });

  it("shows the disconnected placeholder when the active server is not connected", () => {
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    renderPage();

    expect(
      screen.getByRole("heading", { name: "Database workspace is disconnected" }),
    ).toBeInTheDocument();
    expect(screen.getByText("SSH disconnected from Production.")).toBeInTheDocument();
    expect(screen.queryByText("No database connections yet")).not.toBeInTheDocument();
  });

  it("shows a loading placeholder while server profiles load", () => {
    useServerStore.setState({ servers: [], status: "loading", error: null });
    renderPage();

    expect(screen.getByRole("heading", { name: "Loading database workspace" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Loading saved server profiles...");
  });

  it("shows an error placeholder and retries loading server profiles", async () => {
    const user = userEvent.setup();
    const fetchServers = vi.fn().mockResolvedValue(undefined);
    useServerStore.setState({
      servers: [],
      status: "error",
      error: "The profile store did not respond.",
      fetchServers,
    });
    renderPage();

    expect(screen.getByText("Could not load server profiles")).toBeInTheDocument();
    expect(screen.getByText("The profile store did not respond.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(fetchServers).toHaveBeenCalledTimes(1);
  });

  it("shows an empty placeholder when no server profiles exist", () => {
    renderPage();

    expect(screen.getByRole("heading", { name: "No server profiles yet" })).toBeInTheDocument();
    expect(screen.getAllByRole("link", { name: "Open servers" }).length).toBeGreaterThan(0);
  });

  it("supports keyboard navigation between workspace tabs", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    renderPage();

    await user.click(screen.getByRole("tab", { name: "Overview" }));
    await user.keyboard("{ArrowRight}");

    expect(screen.getByRole("tab", { name: "Structure" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { name: "No table selected" })).toBeInTheDocument();
  });

  it("reports refresh feedback instead of silently doing nothing", async () => {
    const user = userEvent.setup();
    useConnectedServer();
    renderPage();

    await user.click(screen.getByRole("button", { name: "Refresh connection status" }));

    expect(
      await screen.findByText(
        "Live SSH status requires the desktop build. No connection was opened.",
      ),
    ).toBeInTheDocument();
  });
});
