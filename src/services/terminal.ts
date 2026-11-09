import { invoke } from "@tauri-apps/api/core";
import type { TerminalSessionInfo, TerminalSize } from "@/types/contracts";

export const TERMINAL_EVENT = "terminal-event";

export const terminalCommands = {
  open: "open_terminal",
  input: "terminal_input",
  resize: "resize_terminal",
  close: "close_terminal",
  list: "list_terminal_sessions",
} as const;

export type OpenTerminalInput = {
  serverId: string;
  term?: string;
  size?: TerminalSize;
};

export function openTerminal(input: OpenTerminalInput): Promise<TerminalSessionInfo> {
  return invoke<TerminalSessionInfo>(terminalCommands.open, { input });
}

export function writeTerminal(sessionId: string, data: string): Promise<void> {
  return invoke<void>(terminalCommands.input, {
    input: { sessionId, data },
  });
}

export function resizeTerminal(sessionId: string, size: TerminalSize): Promise<void> {
  return invoke<void>(terminalCommands.resize, {
    input: { sessionId, size },
  });
}

export function closeTerminal(sessionId: string): Promise<void> {
  return invoke<void>(terminalCommands.close, {
    input: { sessionId },
  });
}

export function listTerminalSessions(): Promise<TerminalSessionInfo[]> {
  return invoke<TerminalSessionInfo[]>(terminalCommands.list);
}
