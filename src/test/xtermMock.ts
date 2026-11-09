import { vi } from "vitest";

type Handler<T> = ((value: T) => void) | null;

export const xtermInstances: MockTerminal[] = [];

export function resetXtermMock() {
  xtermInstances.length = 0;
}

/**
 * Test double for the subset of the xterm.js API used by TerminalView.
 * The real browser renderer is exercised in headless Chrome instead.
 */
export class MockTerminal {
  cols = 80;
  rows = 24;
  options: Record<string, unknown> = {};
  textarea: HTMLTextAreaElement | undefined;
  container: HTMLElement | null = null;
  writes: string[] = [];
  disposed = false;
  dataHandler: Handler<string> = null;
  selectionHandler: (() => void) | null = null;
  keyHandler: ((event: KeyboardEvent) => boolean) | null = null;
  dataDisposable = { dispose: vi.fn() };
  selectionDisposable = { dispose: vi.fn() };

  constructor() {
    xtermInstances.push(this);
  }

  open = vi.fn((container: HTMLElement) => {
    this.container = container;
    Object.defineProperty(container, "clientWidth", { configurable: true, value: 840 });
    Object.defineProperty(container, "clientHeight", { configurable: true, value: 432 });
    this.textarea = document.createElement("textarea");
    container.appendChild(this.textarea);
  });

  loadAddon = vi.fn((addon: unknown) => {
    void addon;
  });

  write = vi.fn((data: string) => {
    this.writes.push(data);
  });

  reset = vi.fn();

  dispose = vi.fn(() => {
    this.disposed = true;
  });

  focus = vi.fn();

  clear = vi.fn();

  selectAll = vi.fn();

  getSelection = vi.fn(() => "");

  hasSelection = vi.fn(() => false);

  onData = vi.fn((handler: (data: string) => void) => {
    this.dataHandler = handler;
    return this.dataDisposable;
  });

  onSelectionChange = vi.fn((handler: () => void) => {
    this.selectionHandler = handler;
    return this.selectionDisposable;
  });

  attachCustomKeyEventHandler = vi.fn((handler: (event: KeyboardEvent) => boolean) => {
    this.keyHandler = handler;
  });
}

export class MockFitAddon {
  fit = vi.fn();
}
