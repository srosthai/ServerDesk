import type { TerminalSize } from "@/types/contracts";

const CELL_WIDTH = 8.4;
const CELL_HEIGHT = 18;

const ESCAPE_KEY_SEQUENCES: Record<string, string> = {
  Enter: "\r",
  Backspace: "\x7f",
  Tab: "\t",
  Escape: "\x1b",
  ArrowUp: "\x1b[A",
  ArrowDown: "\x1b[B",
  ArrowRight: "\x1b[C",
  ArrowLeft: "\x1b[D",
  Home: "\x1b[H",
  End: "\x1b[F",
  Insert: "\x1b[2~",
  Delete: "\x1b[3~",
  PageUp: "\x1b[5~",
  PageDown: "\x1b[6~",
  F1: "\x1bOP",
  F2: "\x1bOQ",
  F3: "\x1bOR",
  F4: "\x1bOS",
  F5: "\x1b[15~",
  F6: "\x1b[17~",
  F7: "\x1b[18~",
  F8: "\x1b[19~",
  F9: "\x1b[20~",
  F10: "\x1b[21~",
  F11: "\x1b[23~",
  F12: "\x1b[24~",
};

type TerminalKeyEvent = {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
};

/**
 * Maps a browser key event to the byte sequence an xterm-compatible terminal
 * sends. Returning null leaves the key to the browser.
 */
export function terminalKeySequence(event: TerminalKeyEvent): string | null {
  if (event.metaKey) {
    return null;
  }

  if (event.ctrlKey) {
    if (event.key.length !== 1) {
      return null;
    }
    const character = event.key.toLowerCase();
    if (character >= "a" && character <= "z") {
      return String.fromCharCode(character.charCodeAt(0) - 96);
    }
    return null;
  }

  if (event.altKey) {
    return event.key.length === 1 ? `\x1b${event.key}` : null;
  }

  const mapped = ESCAPE_KEY_SEQUENCES[event.key];
  if (mapped) {
    return mapped;
  }

  return event.key.length === 1 ? event.key : null;
}

/* eslint-disable no-control-regex -- ANSI control sequences are the target. */
const OSC_SEQUENCE = new RegExp("\\u001b\\][^\\u0007]*(?:\\u0007|\\u001b\\\\)", "g");
const CSI_SEQUENCE = new RegExp("\\u001b\\[[0-?]*[ -/]*[@-~]", "g");
const ESCAPE_SEQUENCE = new RegExp("\\u001b[@-Z\\\\-_]", "g");
/* eslint-enable no-control-regex */

/** Removes ANSI escapes for the plain-text preview used before #27's xterm.js view. */
export function stripAnsi(value: string): string {
  return value.replace(OSC_SEQUENCE, "").replace(CSI_SEQUENCE, "").replace(ESCAPE_SEQUENCE, "");
}

export function measureTerminalSize(width: number, height: number): TerminalSize {
  return {
    cols: Math.min(1_000, Math.max(20, Math.floor(width / CELL_WIDTH))),
    rows: Math.min(1_000, Math.max(5, Math.floor(height / CELL_HEIGHT))),
    pixelWidth: Math.max(0, Math.round(width)),
    pixelHeight: Math.max(0, Math.round(height)),
  };
}
