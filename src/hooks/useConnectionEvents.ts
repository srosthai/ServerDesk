import { useTauriEvent } from "@/hooks/useTauriEvent";
import { CONNECTION_STATE_EVENT } from "@/services/connections";
import { useConnectionStore } from "@/stores/connectionStore";
import type { ConnectionSnapshot } from "@/types/contracts";

export function useConnectionEvents() {
  const applySnapshot = useConnectionStore((state) => state.applySnapshot);

  useTauriEvent<ConnectionSnapshot>(CONNECTION_STATE_EVENT, applySnapshot);
}
