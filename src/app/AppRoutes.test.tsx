import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import App from "@/App";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import type { ServerProfile } from "@/types/contracts";

const server: ServerProfile = {
  id: "server-1",
  name: "RentMee",
  host: "188.166.215.167",
  port: 22,
  username: "root",
  authType: "password",
  credentialRef: "serverdesk:ssh:test",
  groupName: null,
  tags: ["Web"],
  notes: null,
  createdAt: 1,
  updatedAt: 1,
};

describe("application shell", () => {
  beforeEach(() => {
    useServerStore.setState({ servers: [], status: "idle", error: null });
    useConnectionStore.setState({ connections: {}, busy: {}, error: null });
    usePreferencesStore.setState({
      preferences: DEFAULT_PREFERENCES,
      status: "unavailable",
      error: null,
    });
  });

  it("renders the overview and navigates between routes", async () => {
    const user = userEvent.setup();
    render(<App />);

    expect(screen.getByRole("heading", { name: "Overview" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Skip to content" })).toBeInTheDocument();

    await user.click(screen.getByRole("link", { name: "Servers" }));

    expect(await screen.findByRole("heading", { name: "Servers" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Servers" })).toHaveAttribute("aria-current", "page");
    expect(await screen.findByText("No server profiles yet")).toBeInTheDocument();
  });

  it("collapses and expands the sidebar", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "Collapse sidebar" }));

    expect(screen.getByRole("button", { name: "Expand sidebar" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(localStorage.getItem("serverdesk-sidebar-collapsed")).toBe("true");

    await user.click(screen.getByRole("button", { name: "Expand sidebar" }));

    expect(screen.getByRole("button", { name: "Collapse sidebar" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("shows the active server and its live connection state", async () => {
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: {
        [server.id]: { serverId: server.id, state: "connected", changedAt: 1 },
      },
      busy: {},
      error: null,
    });
    render(<App />);

    expect((await screen.findAllByText("RentMee")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Connected").length).toBeGreaterThan(0);
    expect(screen.getByText("ServerDesk v0.1.0")).toBeInTheDocument();
  });

  it("opens settings from navigation", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("link", { name: "Settings" }));

    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getAllByText("Web preview").length).toBeGreaterThan(0);
  });
});
