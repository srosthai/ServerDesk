import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalPage } from "@/features/terminal/TerminalPage";
import { connectServer, getConnectionState } from "@/services/connections";
import { closeTerminal, openTerminal, resizeTerminal, writeTerminal } from "@/services/terminal";
import { useConnectionStore } from "@/stores/connectionStore";
import { useServerStore } from "@/stores/serverStore";
import { useTerminalStore, type TerminalSessionState } from "@/stores/terminalStore";
import { resetXtermMock, xtermInstances } from "@/test/xtermMock";
import type { ConnectionSnapshot, ServerProfile, TerminalSessionInfo } from "@/types/contracts";

vi.mock("@xterm/xterm", async () => {
  const { MockTerminal } = await import("@/test/xtermMock");
  return { Terminal: MockTerminal };
});

vi.mock("@xterm/addon-fit", async () => {
  const { MockFitAddon } = await import("@/test/xtermMock");
  return { FitAddon: MockFitAddon };
});

vi.mock("@/services/terminal", () => ({
  openTerminal: vi.fn(),
  writeTerminal: vi.fn(),
  resizeTerminal: vi.fn(),
  closeTerminal: vi.fn(),
}));

vi.mock("@/services/connections", () => ({
  connectServer: vi.fn(),
  disconnectServer: vi.fn(),
  getConnectionState: vi.fn(),
}));

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

const disconnected: ConnectionSnapshot = {
  serverId: server.id,
  state: "disconnected",
  changedAt: 2,
};

function terminalInfo(sessionId: string): TerminalSessionInfo {
  return {
    sessionId,
    serverId: server.id,
    term: "xterm-256color",
    cols: 80,
    rows: 24,
    openedAt: Number(sessionId.replace(/\D/g, "")) || 1,
  };
}

function sessionState(info: TerminalSessionInfo): TerminalSessionState {
  return {
    info,
    status: "open",
    output: "\x1b[32mready\x1b[0m\r\n",
    outputTruncated: false,
    lastSequence: 1,
    exitReason: null,
    exitCode: null,
    message: null,
    error: null,
  };
}

function renderPage() {
  return render(
    <MemoryRouter>
      <TerminalPage />
    </MemoryRouter>,
  );
}

describe("TerminalPage", () => {
  beforeEach(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    resetXtermMock();
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 1;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});

    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    useTerminalStore.setState({
      sessions: {},
      openingByServer: {},
      activeByServer: {},
      error: null,
    });

    vi.mocked(openTerminal).mockReset();
    vi.mocked(writeTerminal).mockReset();
    vi.mocked(resizeTerminal).mockReset();
    vi.mocked(closeTerminal).mockReset();
    vi.mocked(connectServer).mockReset();
    vi.mocked(getConnectionState).mockReset();
    vi.mocked(writeTerminal).mockResolvedValue();
    vi.mocked(resizeTerminal).mockResolvedValue();
    vi.mocked(closeTerminal).mockResolvedValue();
    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(connectServer).mockResolvedValue(connected);
  });

  it("shows the server and SSH user in the terminal header", async () => {
    renderPage();

    expect(await screen.findByRole("heading", { name: "Production" })).toBeInTheDocument();
    expect(screen.getByText(/deploy@198\.51\.100\.24:22/)).toBeInTheDocument();
    expect(screen.getByText("Connected")).toBeInTheDocument();
  });

  it("opens independent tabs, switches between them, and cleans up a closed tab", async () => {
    const user = userEvent.setup();
    const first = terminalInfo("terminal-1");
    const second = terminalInfo("terminal-2");
    vi.mocked(openTerminal).mockResolvedValueOnce(first).mockResolvedValueOnce(second);

    renderPage();
    await user.click(screen.getAllByRole("button", { name: "Open terminal" })[0]);

    const firstTab = await screen.findByRole("tab", { name: /^Shell 1/ });
    expect(firstTab).toHaveAttribute("aria-selected", "true");
    expect(xtermInstances).toHaveLength(1);

    await user.click(screen.getByRole("button", { name: "New shell" }));
    const secondTab = await screen.findByRole("tab", { name: /^Shell 2/ });
    expect(secondTab).toHaveAttribute("aria-selected", "true");
    expect(xtermInstances).toHaveLength(2);

    const panels = document.querySelectorAll<HTMLElement>("[data-terminal-session]");
    expect(panels).toHaveLength(2);
    expect(panels[0]).toHaveClass("hidden");
    expect(panels[1]).not.toHaveClass("hidden");

    await user.click(firstTab);
    expect(firstTab).toHaveAttribute("aria-selected", "true");
    expect(secondTab).toHaveAttribute("aria-selected", "false");

    await user.click(screen.getByRole("button", { name: "Close Shell 1" }));

    await waitFor(() => {
      expect(screen.getAllByRole("tab")).toHaveLength(1);
    });
    const remainingTab = screen.getByRole("tab");
    expect(remainingTab).toHaveAttribute("aria-controls", "terminal-panel-terminal-2");
    expect(remainingTab).toHaveAttribute("aria-selected", "true");
    expect(closeTerminal).toHaveBeenCalledWith(first.sessionId);
    expect(xtermInstances[0].dispose).toHaveBeenCalledTimes(1);
  });

  it("surfaces a lost connection and requires a deliberate reconnect", async () => {
    const user = userEvent.setup();
    const info = terminalInfo("terminal-1");
    useConnectionStore.setState({
      connections: { [server.id]: disconnected },
      busy: {},
      error: null,
    });
    useTerminalStore.setState({
      sessions: { [info.sessionId]: sessionState(info) },
      activeByServer: { [server.id]: info.sessionId },
      openingByServer: {},
      error: null,
    });

    renderPage();

    expect(screen.getByText("Connection lost")).toBeInTheDocument();
    expect(screen.getByText(/Existing shells can no longer receive input/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(connectServer).toHaveBeenCalledWith(server.id);
  });

  it("shows session termination, allows dismissal, and reports the exit code", async () => {
    const user = userEvent.setup();
    const info = terminalInfo("terminal-1");
    useTerminalStore.setState({
      sessions: { [info.sessionId]: sessionState(info) },
      activeByServer: { [server.id]: info.sessionId },
      openingByServer: {},
      error: null,
    });

    renderPage();
    const panel = screen.getByRole("tabpanel");
    expect(within(panel).getByRole("button", { name: "Copy" })).toBeInTheDocument();

    act(() => {
      useTerminalStore.getState().applyEvent({
        type: "exited",
        sessionId: info.sessionId,
        sequence: 2,
        reason: "remote-exit",
        exitCode: 7,
        message: null,
      });
    });

    expect(await screen.findByText("Remote process exited with exit code 7.")).toBeInTheDocument();
    expect(screen.getByText("Remote process exited · code 7")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => {
      expect(screen.queryByRole("tab", { name: /^Shell 1/ })).not.toBeInTheDocument();
    });
    expect(xtermInstances[0].dispose).toHaveBeenCalledTimes(1);
  });
});
