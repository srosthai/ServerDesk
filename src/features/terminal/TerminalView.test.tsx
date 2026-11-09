import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalView } from "@/features/terminal/TerminalView";
import type { TerminalSessionState } from "@/stores/terminalStore";
import { resetXtermMock, xtermInstances } from "@/test/xtermMock";

vi.mock("@xterm/xterm", async () => {
  const { MockTerminal } = await import("@/test/xtermMock");
  return { Terminal: MockTerminal };
});

vi.mock("@xterm/addon-fit", async () => {
  const { MockFitAddon } = await import("@/test/xtermMock");
  return { FitAddon: MockFitAddon };
});

const session: TerminalSessionState = {
  info: {
    sessionId: "terminal-1",
    serverId: "server-1",
    term: "xterm-256color",
    cols: 80,
    rows: 24,
    openedAt: 1,
  },
  status: "open",
  output: "\x1b[32mready\x1b[0m\r\n",
  outputTruncated: false,
  lastSequence: 1,
  exitReason: null,
  exitCode: null,
  message: null,
  error: null,
};

let writeText: ReturnType<typeof vi.fn>;
let readText: ReturnType<typeof vi.fn>;
let observerCallbacks: Array<() => void>;
let observerDisconnects: Array<ReturnType<typeof vi.fn>>;

function stubAnimationFrame() {
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    callback(0);
    return 1;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
}

function renderView(overrides: Partial<TerminalSessionState> = {}) {
  const onInput = vi.fn();
  const onResize = vi.fn();
  const view = render(
    <TerminalView
      session={{ ...session, ...overrides }}
      active
      inputEnabled
      onInput={onInput}
      onResize={onResize}
    />,
  );
  return { ...view, onInput, onResize };
}

describe("TerminalView", () => {
  beforeEach(() => {
    resetXtermMock();
    stubAnimationFrame();
    observerCallbacks = [];
    observerDisconnects = [];
    class CapturingResizeObserver {
      disconnect = vi.fn();
      constructor(callback: ResizeObserverCallback) {
        observerCallbacks.push(() => callback([], this as unknown as ResizeObserver));
        observerDisconnects.push(this.disconnect);
      }
      observe() {}
      unobserve() {}
    }
    vi.stubGlobal("ResizeObserver", CapturingResizeObserver);

    writeText = vi.fn().mockResolvedValue(undefined);
    readText = vi.fn().mockResolvedValue("");
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText, readText },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("creates one xterm, replays output, writes only deltas, and forwards input", async () => {
    const { rerender, onInput } = renderView();
    const terminal = xtermInstances[0];

    expect(xtermInstances).toHaveLength(1);
    expect(terminal.writes).toEqual(["\x1b[32mready\x1b[0m\r\n"]);
    expect(screen.getByText(/80×24/)).toBeInTheDocument();

    act(() => terminal.dataHandler?.("ls -la\r"));
    expect(onInput).toHaveBeenCalledWith("ls -la\r");

    rerender(
      <TerminalView
        session={{ ...session, output: `${session.output}total 0\r\n`, lastSequence: 2 }}
        active
        inputEnabled
        onInput={onInput}
        onResize={vi.fn()}
      />,
    );
    expect(terminal.writes).toEqual(["\x1b[32mready\x1b[0m\r\n", "total 0\r\n"]);

    rerender(
      <TerminalView
        session={{ ...session, output: "replacement tail" }}
        active
        inputEnabled
        onInput={onInput}
        onResize={vi.fn()}
      />,
    );
    expect(terminal.reset).toHaveBeenCalledTimes(1);
    expect(terminal.writes.at(-1)).toBe("replacement tail");
  });

  it("reports resize after container and window changes", async () => {
    const { onResize } = renderView();
    const terminal = xtermInstances[0];

    await waitFor(() => {
      expect(onResize).toHaveBeenCalledWith({
        cols: 80,
        rows: 24,
        pixelWidth: 840,
        pixelHeight: 432,
      });
    });

    onResize.mockClear();
    terminal.cols = 100;
    terminal.rows = 30;
    act(() => observerCallbacks.forEach((callback) => callback()));
    expect(onResize).toHaveBeenCalledWith({
      cols: 100,
      rows: 30,
      pixelWidth: 840,
      pixelHeight: 432,
    });

    onResize.mockClear();
    terminal.cols = 120;
    terminal.rows = 36;
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(onResize).toHaveBeenCalledWith({
      cols: 120,
      rows: 36,
      pixelWidth: 840,
      pixelHeight: 432,
    });
  });

  it("supports copy, paste, select all, and clear actions", async () => {
    const { onInput } = renderView();
    const terminal = xtermInstances[0];
    terminal.getSelection.mockReturnValue("selected text");
    terminal.hasSelection.mockReturnValue(true);

    act(() => terminal.selectionHandler?.());
    const copyButton = screen.getByRole("button", { name: "Copy" });
    await waitFor(() => expect(copyButton).toBeEnabled());
    fireEvent.click(copyButton);
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("selected text"));

    readText.mockResolvedValue("pasted command");
    fireEvent.click(screen.getByRole("button", { name: "Paste" }));
    await waitFor(() => expect(onInput).toHaveBeenCalledWith("pasted command"));

    fireEvent.click(screen.getByRole("button", { name: "Select all" }));
    expect(terminal.selectAll).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(terminal.clear).toHaveBeenCalledTimes(1);
  });

  it("disposes the xterm instance and every subscription on unmount", () => {
    const { unmount } = renderView();
    const terminal = xtermInstances[0];
    const removeListener = vi.spyOn(window, "removeEventListener");

    unmount();

    expect(terminal.dataDisposable.dispose).toHaveBeenCalledTimes(1);
    expect(terminal.selectionDisposable.dispose).toHaveBeenCalledTimes(1);
    expect(terminal.dispose).toHaveBeenCalledTimes(1);
    expect(terminal.disposed).toBe(true);
    expect(observerDisconnects[0]).toHaveBeenCalledTimes(1);
    expect(removeListener).toHaveBeenCalledWith("resize", expect.any(Function));
  });
});
