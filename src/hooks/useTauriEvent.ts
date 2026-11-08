import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauriRuntime } from "@/lib/tauri";

export function useTauriEvent<T>(event: string, handler: (payload: T) => void) {
  const handlerRef = useRef(handler);

  useEffect(() => {
    handlerRef.current = handler;
  }, [handler]);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }

    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<T>(event, (payload) => {
      handlerRef.current(payload.payload);
    })
      .then((dispose) => {
        if (cancelled) {
          dispose();
        } else {
          unlisten = dispose;
        }
      })
      .catch(() => {
        // A missing listener is not fatal; the feature can surface its own state.
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [event]);
}
