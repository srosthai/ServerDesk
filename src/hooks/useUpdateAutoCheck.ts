import { useEffect } from "react";
import { isTauriRuntime } from "@/lib/tauri";
import { useUpdateStore } from "@/stores/updateStore";

export function useUpdateAutoCheck() {
  const checkForUpdate = useUpdateStore((state) => state.checkForUpdate);

  useEffect(() => {
    if (!isTauriRuntime() || import.meta.env.DEV) {
      return;
    }

    void checkForUpdate({ trigger: "startup" });
  }, [checkForUpdate]);
}
