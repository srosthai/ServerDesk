import { useCallback, useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITheme } from "@xterm/xterm";
import { BroomIcon, ClipboardTextIcon, CopyIcon, SelectionAllIcon } from "@phosphor-icons/react";
import {
  readText as readSystemClipboard,
  writeText as writeSystemClipboard,
} from "@tauri-apps/plugin-clipboard-manager";
import "@xterm/xterm/css/xterm.css";
import { isTauriRuntime } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import type { TerminalSessionState } from "@/stores/terminalStore";
import type { TerminalSize } from "@/types/contracts";

const TERMINAL_THEME: ITheme = {
  background: "#000000",
  foreground: "#d7e4dc",
  cursor: "#f08a4b",
  cursorAccent: "#000000",
  selectionBackground: "#315843",
  selectionForeground: "#f4f8f5",
  selectionInactiveBackground: "#243c2f",
  black: "#151d19",
  red: "#e5484d",
  green: "#5bc46d",
  yellow: "#e7a33d",
  blue: "#6bb6e8",
  magenta: "#c678dd",
  cyan: "#56b6c2",
  white: "#d7e4dc",
  brightBlack: "#7d8d84",
  brightRed: "#ff7b72",
  brightGreen: "#7ee787",
  brightYellow: "#f2cc60",
  brightBlue: "#79c0ff",
  brightMagenta: "#d2a8ff",
  brightCyan: "#76e3ea",
  brightWhite: "#f1f5f2",
};

const TERMINAL_SCROLLBACK = 5_000;

const toolbarButtonClass =
  "inline-flex h-6 items-center gap-1 rounded-sm px-1.5 text-[11px] text-[#a2b0a7] transition-colors outline-none hover:bg-white/10 hover:text-[#f1f5f2] focus-visible:ring-2 focus-visible:ring-[#f08a4b]/60 disabled:pointer-events-none disabled:opacity-40";

async function copyToClipboard(value: string): Promise<boolean> {
  if (isTauriRuntime()) {
    try {
      await writeSystemClipboard(value);
      return true;
    } catch {
      // Fall through to browser APIs if the native clipboard is unavailable.
    }
  }

  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return true;
    } catch {
      // Fall through to the legacy copy path when the clipboard API is denied.
    }
  }

  try {
    const textarea = document.createElement("textarea");
    textarea.value = value;
    textarea.setAttribute("readonly", "");
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    const copied = document.execCommand("copy");
    textarea.remove();
    return copied;
  } catch {
    return false;
  }
}

async function readFromClipboard(): Promise<string | null> {
  if (isTauriRuntime()) {
    try {
      return await readSystemClipboard();
    } catch {
      // Fall through to browser APIs if the native clipboard is unavailable.
    }
  }

  if (!navigator.clipboard?.readText) {
    return null;
  }
  try {
    return await navigator.clipboard.readText();
  } catch {
    return null;
  }
}

type TerminalViewProps = {
  session: TerminalSessionState;
  active: boolean;
  inputEnabled: boolean;
  onInput: (data: string) => void;
  onResize: (size: TerminalSize) => void;
  id?: string;
  labelledBy?: string;
};

/**
 * One xterm.js instance bound to one PTY-backed SSH session.
 *
 * The component intentionally owns the xterm lifecycle: instances survive tab
 * switches so independent scrollback buffers are preserved, and every
 * disposable, observer, timer, and event listener is released on unmount.
 */
function TerminalView({
  session,
  active,
  inputEnabled,
  onInput,
  onResize,
  id,
  labelledBy,
}: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const writtenOutputRef = useRef("");
  const onInputRef = useRef(onInput);
  const onResizeRef = useRef(onResize);
  const lastSizeRef = useRef("");
  const actionRef = useRef<{ copy: () => void; paste: () => void } | null>(null);
  const [hasSelection, setHasSelection] = useState(false);
  const [dimensions, setDimensions] = useState({
    cols: session.info.cols,
    rows: session.info.rows,
  });
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    onInputRef.current = onInput;
  }, [onInput]);

  useEffect(() => {
    onResizeRef.current = onResize;
  }, [onResize]);

  useEffect(() => {
    if (!notice) {
      return;
    }
    const timeout = window.setTimeout(() => setNotice(null), 2_400);
    return () => window.clearTimeout(timeout);
  }, [notice]);

  const fitAndReport = useCallback(() => {
    const terminal = terminalRef.current;
    const fitAddon = fitAddonRef.current;
    const container = containerRef.current;
    if (!terminal || !fitAddon || !container) {
      return;
    }
    if (container.clientWidth <= 0 || container.clientHeight <= 0) {
      return;
    }

    try {
      fitAddon.fit();
    } catch {
      return;
    }

    const size: TerminalSize = {
      cols: terminal.cols,
      rows: terminal.rows,
      pixelWidth: Math.round(container.clientWidth),
      pixelHeight: Math.round(container.clientHeight),
    };
    setDimensions((current) =>
      current.cols === size.cols && current.rows === size.rows
        ? current
        : { cols: size.cols, rows: size.rows },
    );

    const key = `${size.cols}x${size.rows}:${size.pixelWidth}x${size.pixelHeight}`;
    if (key === lastSizeRef.current) {
      return;
    }
    lastSizeRef.current = key;
    onResizeRef.current(size);
  }, []);

  const copySelection = useCallback(async () => {
    const terminal = terminalRef.current;
    if (!terminal) {
      return;
    }
    const selection = terminal.getSelection();
    if (!selection) {
      setNotice("Select text in the terminal first.");
      return;
    }
    const copied = await copyToClipboard(selection);
    setNotice(copied ? `Copied ${selection.length.toLocaleString()} characters.` : "Copy failed.");
  }, []);

  const pasteClipboard = useCallback(async () => {
    const text = await readFromClipboard();
    if (text === null) {
      setNotice("Clipboard access failed. Use Ctrl+V to paste from the keyboard.");
      return;
    }
    if (!text) {
      setNotice("The clipboard is empty.");
      return;
    }
    onInputRef.current(text);
    terminalRef.current?.focus();
    setNotice(`Pasted ${text.length.toLocaleString()} characters.`);
  }, []);

  const clearScrollback = useCallback(() => {
    const terminal = terminalRef.current;
    if (!terminal) {
      return;
    }
    terminal.clear();
    terminal.focus();
    setHasSelection(false);
    setNotice("Local scrollback cleared.");
  }, []);

  const selectAll = useCallback(() => {
    const terminal = terminalRef.current;
    if (!terminal) {
      return;
    }
    terminal.selectAll();
    terminal.focus();
    setHasSelection(Boolean(terminal.getSelection()));
  }, []);

  useEffect(() => {
    actionRef.current = {
      copy: () => {
        void copySelection();
      },
      paste: () => {
        void pasteClipboard();
      },
    };
  }, [copySelection, pasteClipboard]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    const terminal = new Terminal({
      allowTransparency: false,
      convertEol: false,
      cursorBlink: true,
      cursorInactiveStyle: "outline",
      cursorStyle: "block",
      disableStdin: !inputEnabled,
      drawBoldTextInBrightColors: true,
      fontFamily: '"IBM Plex Mono", ui-monospace, SFMono-Regular, monospace',
      fontSize: 12,
      fontWeight: "400",
      fontWeightBold: "500",
      letterSpacing: 0,
      lineHeight: 1.35,
      macOptionIsMeta: true,
      rightClickSelectsWord: true,
      scrollback: TERMINAL_SCROLLBACK,
      theme: TERMINAL_THEME,
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(container);
    terminal.textarea?.setAttribute(
      "aria-label",
      `Terminal for ${session.info.term}, session ${session.info.cols} by ${session.info.rows}`,
    );

    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;
    writtenOutputRef.current = "";
    lastSizeRef.current = "";

    const dataDisposable = terminal.onData((data) => {
      onInputRef.current(data);
    });
    const selectionDisposable = terminal.onSelectionChange(() => {
      setHasSelection(Boolean(terminal.getSelection()));
    });
    terminal.attachCustomKeyEventHandler((event) => {
      const key = event.key.toLowerCase();
      const copyModifier = (event.ctrlKey || event.metaKey) && event.shiftKey;
      const pasteModifier = (event.ctrlKey || event.metaKey) && !event.shiftKey;
      const macCopy = event.metaKey && !event.ctrlKey && !event.shiftKey;

      if (event.type === "keydown" && (copyModifier || macCopy) && key === "c") {
        if (terminal.hasSelection()) {
          event.preventDefault();
          actionRef.current?.copy();
          return false;
        }
      }

      if (event.type === "keydown" && (copyModifier || pasteModifier) && key === "v") {
        event.preventDefault();
        actionRef.current?.paste();
        return false;
      }

      if (event.type === "keydown" && copyModifier && key === "a") {
        event.preventDefault();
        terminal.selectAll();
        return false;
      }

      return true;
    });

    const scheduleFit = () => {
      window.requestAnimationFrame(() => fitAndReport());
    };

    const observer = new ResizeObserver(scheduleFit);
    observer.observe(container);
    window.addEventListener("resize", scheduleFit);
    fitAndReport();

    let fontsCancelled = false;
    if (typeof document.fonts?.ready?.then === "function") {
      void document.fonts.ready.then(() => {
        if (!fontsCancelled) {
          scheduleFit();
        }
      });
    }

    return () => {
      fontsCancelled = true;
      observer.disconnect();
      window.removeEventListener("resize", scheduleFit);
      dataDisposable.dispose();
      selectionDisposable.dispose();
      terminal.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      writtenOutputRef.current = "";
      lastSizeRef.current = "";
    };
    // The xterm instance is created once per mounted tab. Session metadata is
    // immutable for the lifetime of that tab.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.info.sessionId]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) {
      return;
    }

    const nextOutput = session.output;
    const previousOutput = writtenOutputRef.current;
    if (nextOutput === previousOutput) {
      return;
    }

    if (previousOutput.length === 0 || nextOutput.startsWith(previousOutput)) {
      terminal.write(nextOutput.slice(previousOutput.length));
    } else {
      // The bounded store buffer was truncated (or reset), so replay the
      // remaining tail into a clean terminal rather than duplicating output.
      terminal.reset();
      terminal.write(nextOutput);
    }
    writtenOutputRef.current = nextOutput;
  }, [session.output]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) {
      return;
    }
    terminal.options.disableStdin = !inputEnabled;
  }, [inputEnabled]);

  useEffect(() => {
    if (!active) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      fitAndReport();
      terminalRef.current?.focus();
    });

    return () => window.cancelAnimationFrame(frame);
  }, [active, fitAndReport]);

  return (
    <div
      className={cn("flex h-full min-h-0 flex-col bg-[#000000]", active ? "flex" : "hidden")}
      data-terminal-session={session.info.sessionId}
      id={id}
      role="tabpanel"
      aria-labelledby={labelledBy}
      aria-label={
        labelledBy ? undefined : `Terminal shell ${session.info.cols} by ${session.info.rows}`
      }
    >
      <div className="flex items-center gap-1 border-b border-white/10 px-2 py-1">
        <button
          type="button"
          className={toolbarButtonClass}
          disabled={!hasSelection}
          onClick={() => {
            void copySelection();
          }}
          title="Copy selection (Ctrl+Shift+C)"
        >
          <CopyIcon aria-hidden="true" className="size-3.5" />
          Copy
        </button>
        <button
          type="button"
          className={toolbarButtonClass}
          disabled={!inputEnabled}
          onClick={() => {
            void pasteClipboard();
          }}
          title="Paste from clipboard (Ctrl+V or Ctrl+Shift+V)"
        >
          <ClipboardTextIcon aria-hidden="true" className="size-3.5" />
          Paste
        </button>
        <button
          type="button"
          className={toolbarButtonClass}
          onClick={selectAll}
          title="Select the entire buffer (Ctrl+Shift+A)"
        >
          <SelectionAllIcon aria-hidden="true" className="size-3.5" />
          Select all
        </button>
        <button
          type="button"
          className={toolbarButtonClass}
          onClick={clearScrollback}
          title="Clear the local scrollback buffer"
        >
          <BroomIcon aria-hidden="true" className="size-3.5" />
          Clear
        </button>
        <span className="ml-auto font-mono text-[10px] text-[#7d8d84]">
          {dimensions.cols}×{dimensions.rows} · {TERMINAL_SCROLLBACK.toLocaleString()} lines
        </span>
        <span className="min-w-[22ch] truncate text-right text-[10px] text-[#a2b0a7]" role="status">
          {notice ?? ""}
        </span>
      </div>
      <div
        ref={containerRef}
        className="min-h-0 flex-1 px-3 py-2 [&_.xterm]:h-full [&_.xterm-screen]:h-full"
      />
    </div>
  );
}

export { TERMINAL_SCROLLBACK, TerminalView };
