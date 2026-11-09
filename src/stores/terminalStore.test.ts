import { beforeEach, describe, expect, it, vi } from "vitest";
import { closeTerminal, openTerminal, resizeTerminal, writeTerminal } from "@/services/terminal";
import { MAX_TERMINAL_OUTPUT_CHARS, useTerminalStore } from "@/stores/terminalStore";
import type { TerminalSessionInfo } from "@/types/contracts";

vi.mock("@/services/terminal", () => ({
  closeTerminal: vi.fn(),
  openTerminal: vi.fn(),
  resizeTerminal: vi.fn(),
  writeTerminal: vi.fn(),
}));

const info: TerminalSessionInfo = {
  sessionId: "terminal-1",
  serverId: "server-1",
  term: "xterm-256color",
  cols: 80,
  rows: 24,
  openedAt: 1_762_665_720_000,
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function seedOpenSession(): string {
  useTerminalStore.setState({
    sessions: {
      [info.sessionId]: {
        info,
        status: "open",
        output: "",
        outputTruncated: false,
        lastSequence: 0,
        exitReason: null,
        exitCode: null,
        message: null,
        error: null,
      },
    },
    activeByServer: { "server-1": info.sessionId },
  });
  return info.sessionId;
}

describe("terminal store", () => {
  beforeEach(() => {
    enableTauriRuntime();
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
  });

  it("opens a session and remembers it as the active session for the server", async () => {
    vi.mocked(openTerminal).mockResolvedValue(info);
    const size = { cols: 100, rows: 40, pixelWidth: 840, pixelHeight: 720 };

    await expect(useTerminalStore.getState().open("server-1", size)).resolves.toBe(info.sessionId);

    expect(openTerminal).toHaveBeenCalledWith({ serverId: "server-1", size });
    const session = useTerminalStore.getState().sessions[info.sessionId];
    expect(session?.status).toBe("open");
    expect(session?.info).toEqual(info);
    expect(useTerminalStore.getState().activeByServer["server-1"]).toBe(info.sessionId);
    expect(useTerminalStore.getState().openingByServer["server-1"]).toBe(false);
  });

  it("surfaces typed open failures without creating a session", async () => {
    vi.mocked(openTerminal).mockRejectedValue({
      code: "not_found",
      message: "The server is not connected.",
    });

    await expect(useTerminalStore.getState().open("server-1")).resolves.toBeNull();

    expect(useTerminalStore.getState().sessions).toEqual({});
    expect(useTerminalStore.getState().error).toBe("The server is not connected.");
    expect(useTerminalStore.getState().openingByServer["server-1"]).toBe(false);
  });

  it("appends base64 output byte-for-byte and ignores stale sequences", () => {
    const sessionId = seedOpenSession();
    const data = btoa("\x1b[32mgreen\x1b[0m\r\n");

    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 2,
      data,
    });
    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 2,
      data: btoa("duplicate"),
    });
    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 1,
      data: btoa("stale"),
    });

    const session = useTerminalStore.getState().sessions[sessionId];
    expect(session?.output).toBe("\x1b[32mgreen\x1b[0m\r\n");
    expect(session?.lastSequence).toBe(2);
  });

  it("keeps multi-byte UTF-8 intact across output packets", () => {
    const sessionId = seedOpenSession();
    const euro = new TextEncoder().encode("€");
    const encode = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes));

    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 1,
      data: encode(euro.slice(0, 2)),
    });
    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 2,
      data: encode(euro.slice(2)),
    });

    expect(useTerminalStore.getState().sessions[sessionId]?.output).toBe("€");
  });

  it("marks remote exits and ignores later output", () => {
    const sessionId = seedOpenSession();

    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 1,
      data: btoa("bye\r\n"),
    });
    useTerminalStore.getState().applyEvent({
      type: "exited",
      sessionId,
      sequence: 2,
      reason: "remote-exit",
      exitCode: 7,
      message: null,
    });
    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 3,
      data: btoa("too late"),
    });

    const session = useTerminalStore.getState().sessions[sessionId];
    expect(session?.status).toBe("exited");
    expect(session?.exitReason).toBe("remote-exit");
    expect(session?.exitCode).toBe(7);
    expect(session?.output).toBe("bye\r\n");
    expect(session?.lastSequence).toBe(2);
  });

  it("forwards input without storing it in application state", async () => {
    const sessionId = seedOpenSession();
    vi.mocked(writeTerminal).mockResolvedValue();

    await expect(useTerminalStore.getState().write(sessionId, "echo super-secret\r")).resolves.toBe(
      true,
    );

    expect(writeTerminal).toHaveBeenCalledWith(sessionId, "echo super-secret\r");
    expect(JSON.stringify(useTerminalStore.getState().sessions)).not.toContain("super-secret");
    expect(useTerminalStore.getState().sessions[sessionId]?.output).toBe("");
  });

  it("does not write input to a closed session", async () => {
    const sessionId = seedOpenSession();
    useTerminalStore.setState({
      sessions: {
        [sessionId]: {
          ...useTerminalStore.getState().sessions[sessionId],
          status: "exited",
        },
      },
    });

    await expect(useTerminalStore.getState().write(sessionId, "ls\r")).resolves.toBe(false);
    expect(writeTerminal).not.toHaveBeenCalled();
  });

  it("surfaces input failures on the session", async () => {
    const sessionId = seedOpenSession();
    vi.mocked(writeTerminal).mockRejectedValue({
      code: "network",
      message: "The SSH connection was lost.",
    });

    await expect(useTerminalStore.getState().write(sessionId, "ls\r")).resolves.toBe(false);

    expect(useTerminalStore.getState().sessions[sessionId]?.error).toBe(
      "The SSH connection was lost.",
    );
  });

  it("forwards resize events while the session is open", async () => {
    const sessionId = seedOpenSession();
    vi.mocked(resizeTerminal).mockResolvedValue();
    const size = { cols: 132, rows: 43, pixelWidth: 1_320, pixelHeight: 860 };

    await expect(useTerminalStore.getState().resize(sessionId, size)).resolves.toBe(true);

    expect(resizeTerminal).toHaveBeenCalledWith(sessionId, size);
  });

  it("closes a session once and keeps an existing exit reason", async () => {
    const sessionId = seedOpenSession();
    vi.mocked(closeTerminal).mockResolvedValue();

    await expect(useTerminalStore.getState().close(sessionId)).resolves.toBe(true);
    await expect(useTerminalStore.getState().close(sessionId)).resolves.toBe(true);

    expect(closeTerminal).toHaveBeenCalledTimes(2);
    expect(useTerminalStore.getState().sessions[sessionId]?.status).toBe("exited");
    expect(useTerminalStore.getState().sessions[sessionId]?.exitReason).toBe("closed");
    await expect(useTerminalStore.getState().close("unknown")).resolves.toBe(true);
    expect(closeTerminal).toHaveBeenCalledTimes(2);
  });

  it("dismisses a session and clears its active server selection", async () => {
    const sessionId = seedOpenSession();

    useTerminalStore.getState().dismiss(sessionId);

    expect(useTerminalStore.getState().sessions[sessionId]).toBeUndefined();
    expect(useTerminalStore.getState().activeByServer["server-1"]).toBeNull();
  });

  it("clears every session for one server without touching another server", () => {
    const otherInfo = { ...info, sessionId: "terminal-2", serverId: "server-2" };
    useTerminalStore.setState({
      sessions: {
        [info.sessionId]: {
          info,
          status: "open",
          output: "",
          outputTruncated: false,
          lastSequence: 0,
          exitReason: null,
          exitCode: null,
          message: null,
          error: null,
        },
        [otherInfo.sessionId]: {
          info: otherInfo,
          status: "open",
          output: "",
          outputTruncated: false,
          lastSequence: 0,
          exitReason: null,
          exitCode: null,
          message: null,
          error: null,
        },
      },
      activeByServer: { "server-1": info.sessionId, "server-2": otherInfo.sessionId },
      openingByServer: { "server-1": true },
    });

    useTerminalStore.getState().clearServer("server-1");

    expect(useTerminalStore.getState().sessions[info.sessionId]).toBeUndefined();
    expect(useTerminalStore.getState().sessions[otherInfo.sessionId]).toBeDefined();
    expect(useTerminalStore.getState().activeByServer["server-1"]).toBeUndefined();
    expect(useTerminalStore.getState().activeByServer["server-2"]).toBe(otherInfo.sessionId);
  });

  it("bounds the preview buffer and keeps the truncation marker", () => {
    const sessionId = seedOpenSession();
    const large = "x".repeat(MAX_TERMINAL_OUTPUT_CHARS + 50);

    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 1,
      data: btoa(large),
    });
    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 2,
      data: btoa("tail"),
    });

    const session = useTerminalStore.getState().sessions[sessionId];
    expect(session?.output.length).toBe(MAX_TERMINAL_OUTPUT_CHARS);
    expect(session?.output.endsWith("tail")).toBe(true);
    expect(session?.outputTruncated).toBe(true);
  });

  it("records undecodable output as a session error without crashing", () => {
    const sessionId = seedOpenSession();

    useTerminalStore.getState().applyEvent({
      type: "output",
      sessionId,
      sequence: 1,
      data: "not-base64%%%",
    });

    expect(useTerminalStore.getState().sessions[sessionId]?.error).toBe(
      "The terminal output could not be decoded.",
    );
  });
});
