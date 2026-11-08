import { useEffect, useState } from "react";
import { getCommandErrorMessage } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { getAppInfo } from "@/services/system";
import type { AppInfo } from "@/types/contracts";

export type AppInfoState =
  | { status: "loading" }
  | { status: "ready"; info: AppInfo }
  | { status: "unavailable" }
  | { status: "error"; message: string };

export function useAppInfo(): AppInfoState {
  const [state, setState] = useState<AppInfoState>(() =>
    isTauriRuntime() ? { status: "loading" } : { status: "unavailable" },
  );

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }

    let cancelled = false;

    getAppInfo()
      .then((info) => {
        if (!cancelled) {
          setState({ status: "ready", info });
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setState({ status: "error", message: getCommandErrorMessage(error) });
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}
