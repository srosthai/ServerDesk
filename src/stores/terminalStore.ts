import { create } from "zustand";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  closeTerminal as closeTerminalService,
  openTerminal as openTerminalService,
  resizeTerminal as resizeTerminalService,
  writeTerminal as writeTerminalService,
} from "@/services/terminal";
import type {
  TerminalEvent,
  TerminalExitReason,
  TerminalSessionInfo,
  TerminalSize,
} from "@/types/contracts";

export const MAX_TERMINAL_OUTPUT_CHARS = 200_000;

export type TerminalSessionState = {
  info: TerminalSessionInfo;
  status: "open" | "exited";
  output: string;
  outputTruncated: boolean;
  lastSequence: number;
  exitReason: TerminalExitReason | null;
  exitCode: number | null;
  message: string | null;
  error: string | null;
};

type TerminalStore = {
  sessions: Record<string, TerminalSessionState>;
  openingByServer: Record<string, boolean>;
  activeByServer: Record<string, string | null>;
  error: string | null;
  open: (serverId: string, size?: TerminalSize) => Promise<string | null>;
  write: (sessionId: string, data: string) => Promise<boolean>;
  resize: (sessionId: string, size: TerminalSize) => Promise<boolean>;
  close: (sessionId: string) => Promise<boolean>;
  select: (serverId: string, sessionId: string | null) => void;
  applyEvent: (event: TerminalEvent) => void;
  dismiss: (sessionId: string) => void;
  clearServer: (serverId: string) => void;
  clearError: () => void;
};

export function decodeTerminalBytes(data: string): Uint8Array {
  const binary = atob(data);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

export function decodeTerminalData(data: string): string {
  return new TextDecoder().decode(decodeTerminalBytes(data));
}

export function appendTerminalOutput(current: string, chunk: string) {
  const combined = current + chunk;
  if (combined.length <= MAX_TERMINAL_OUTPUT_CHARS) {
    return { output: combined, truncated: false };
  }

  return {
    output: combined.slice(combined.length - MAX_TERMINAL_OUTPUT_CHARS),
    truncated: true,
  };
}

export const useTerminalStore = create<TerminalStore>((set, get) => {
  const decoders = new Map<string, TextDecoder>();

  return {
    sessions: {},
    openingByServer: {},
    activeByServer: {},
    error: null,

    open: async (serverId, size) => {
      if (!isTauriRuntime()) {
        set({ error: "The interactive terminal is available in the desktop application." });
        return null;
      }

      set((state) => ({
        openingByServer: { ...state.openingByServer, [serverId]: true },
        error: null,
      }));

      try {
        const info = await openTerminalService({ serverId, size });
        set((state) => ({
          sessions: {
            ...state.sessions,
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
          activeByServer: { ...state.activeByServer, [serverId]: info.sessionId },
        }));
        return info.sessionId;
      } catch (error) {
        set({ error: getCommandErrorMessage(error) });
        return null;
      } finally {
        set((state) => ({
          openingByServer: { ...state.openingByServer, [serverId]: false },
        }));
      }
    },

    write: async (sessionId, data) => {
      const session = get().sessions[sessionId];
      if (!session || session.status !== "open" || data.length === 0) {
        return false;
      }
      if (!isTauriRuntime()) {
        return false;
      }

      try {
        // Terminal input is forwarded only. It is deliberately never stored in
        // frontend state, errors, or logs.
        await writeTerminalService(sessionId, data);
        return true;
      } catch (error) {
        set((state) => ({
          sessions: {
            ...state.sessions,
            [sessionId]: {
              ...state.sessions[sessionId],
              error: getCommandErrorMessage(error),
            },
          },
        }));
        return false;
      }
    },

    resize: async (sessionId, size) => {
      const session = get().sessions[sessionId];
      if (!session || session.status !== "open" || !isTauriRuntime()) {
        return false;
      }

      try {
        await resizeTerminalService(sessionId, size);
        set((state) => {
          const current = state.sessions[sessionId];
          if (!current || (current.info.cols === size.cols && current.info.rows === size.rows)) {
            return state;
          }
          return {
            sessions: {
              ...state.sessions,
              [sessionId]: {
                ...current,
                info: { ...current.info, cols: size.cols, rows: size.rows },
              },
            },
          };
        });
        return true;
      } catch (error) {
        set((state) => ({
          sessions: {
            ...state.sessions,
            [sessionId]: {
              ...state.sessions[sessionId],
              error: getCommandErrorMessage(error),
            },
          },
        }));
        return false;
      }
    },

    close: async (sessionId) => {
      const session = get().sessions[sessionId];
      if (!session) {
        return true;
      }

      try {
        await closeTerminalService(sessionId);
        set((state) => ({
          sessions: {
            ...state.sessions,
            [sessionId]: {
              ...state.sessions[sessionId],
              status: "exited",
              exitReason: state.sessions[sessionId].exitReason ?? ("closed" as TerminalExitReason),
              exitCode: state.sessions[sessionId].exitCode ?? null,
              message: state.sessions[sessionId].message ?? null,
            },
          },
        }));
        return true;
      } catch (error) {
        set((state) => ({
          sessions: {
            ...state.sessions,
            [sessionId]: {
              ...state.sessions[sessionId],
              error: getCommandErrorMessage(error),
            },
          },
        }));
        return false;
      }
    },

    select: (serverId, sessionId) =>
      set((state) => ({
        activeByServer: { ...state.activeByServer, [serverId]: sessionId },
      })),

    applyEvent: (event) => {
      set((state) => {
        const current = state.sessions[event.sessionId];
        if (!current || event.sequence <= current.lastSequence) {
          return state;
        }

        if (event.type === "output") {
          if (current.status !== "open") {
            return state;
          }

          let text: string;
          try {
            const decoder =
              decoders.get(event.sessionId) ??
              (() => {
                const created = new TextDecoder();
                decoders.set(event.sessionId, created);
                return created;
              })();
            text = decoder.decode(decodeTerminalBytes(event.data), { stream: true });
          } catch {
            return {
              sessions: {
                ...state.sessions,
                [event.sessionId]: {
                  ...current,
                  error: "The terminal output could not be decoded.",
                },
              },
            };
          }

          const appended = appendTerminalOutput(current.output, text);
          return {
            sessions: {
              ...state.sessions,
              [event.sessionId]: {
                ...current,
                output: appended.output,
                outputTruncated: current.outputTruncated || appended.truncated,
                lastSequence: event.sequence,
              },
            },
          };
        }

        decoders.delete(event.sessionId);
        return {
          sessions: {
            ...state.sessions,
            [event.sessionId]: {
              ...current,
              status: "exited",
              lastSequence: event.sequence,
              exitReason: event.reason,
              exitCode: event.exitCode,
              message: event.message,
            },
          },
        };
      });
    },

    dismiss: (sessionId) =>
      set((state) => {
        const session = state.sessions[sessionId];
        if (!session) {
          return state;
        }
        decoders.delete(sessionId);
        const sessions = { ...state.sessions };
        delete sessions[sessionId];
        return {
          sessions,
          activeByServer: {
            ...state.activeByServer,
            [session.info.serverId]:
              state.activeByServer[session.info.serverId] === sessionId
                ? null
                : state.activeByServer[session.info.serverId],
          },
        };
      }),

    clearServer: (serverId) =>
      set((state) => {
        for (const [sessionId, session] of Object.entries(state.sessions)) {
          if (session.info.serverId === serverId) {
            decoders.delete(sessionId);
          }
        }
        const sessions = Object.fromEntries(
          Object.entries(state.sessions).filter(
            ([, session]) => session.info.serverId !== serverId,
          ),
        );
        const activeByServer = { ...state.activeByServer };
        delete activeByServer[serverId];
        const openingByServer = { ...state.openingByServer };
        delete openingByServer[serverId];
        return { sessions, activeByServer, openingByServer };
      }),

    clearError: () => set({ error: null }),
  };
});
