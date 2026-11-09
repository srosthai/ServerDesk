import { useTauriEvent } from "@/hooks/useTauriEvent";
import { TERMINAL_EVENT } from "@/services/terminal";
import { useTerminalStore } from "@/stores/terminalStore";
import type { TerminalEvent } from "@/types/contracts";

export function useTerminalEvents() {
  const applyEvent = useTerminalStore((state) => state.applyEvent);

  useTauriEvent<TerminalEvent>(TERMINAL_EVENT, applyEvent);
}
