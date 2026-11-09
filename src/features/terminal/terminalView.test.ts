import { describe, expect, it } from "vitest";
import {
  measureTerminalSize,
  stripAnsi,
  terminalKeySequence,
} from "@/features/terminal/terminalView";

function key(key: string, overrides: Partial<Parameters<typeof terminalKeySequence>[0]> = {}) {
  return terminalKeySequence({
    key,
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    ...overrides,
  });
}

describe("terminal view helpers", () => {
  it("maps printable keys, control keys, and navigation sequences", () => {
    expect(key("a")).toBe("a");
    expect(key("Enter")).toBe("\r");
    expect(key("Backspace")).toBe("\x7f");
    expect(key("Tab")).toBe("\t");
    expect(key("ArrowUp")).toBe("\x1b[A");
    expect(key("PageDown")).toBe("\x1b[6~");
    expect(key("F5")).toBe("\x1b[15~");
    expect(key("c", { ctrlKey: true })).toBe("\x03");
    expect(key("d", { ctrlKey: true })).toBe("\x04");
    expect(key("x", { altKey: true })).toBe("\x1bx");
    expect(key("v", { metaKey: true })).toBeNull();
    expect(key("Shift")).toBeNull();
  });

  it("strips ANSI color and OSC sequences for the plain-text preview", () => {
    const input = "\x1b[32mgreen\x1b[0m\r\n\x1b]0;title\x07plain";

    expect(stripAnsi(input)).toBe("green\r\nplain");
  });

  it("measures terminal dimensions with safe minimums and maximums", () => {
    expect(measureTerminalSize(0, 0)).toEqual({
      cols: 20,
      rows: 5,
      pixelWidth: 0,
      pixelHeight: 0,
    });
    expect(measureTerminalSize(840, 432)).toEqual({
      cols: 100,
      rows: 24,
      pixelWidth: 840,
      pixelHeight: 432,
    });
    expect(measureTerminalSize(20_000, 20_000)).toEqual({
      cols: 1_000,
      rows: 1_000,
      pixelWidth: 20_000,
      pixelHeight: 20_000,
    });
  });
});
