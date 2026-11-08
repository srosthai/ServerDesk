import { useEffect } from "react";
import { MIN_MONITORING_INTERVAL_SECONDS } from "@/lib/preferences";
import { useMonitoringStore } from "@/stores/monitoringStore";
import { usePreferencesStore } from "@/stores/preferencesStore";

/**
 * Polls monitoring snapshots only while a server is selected and connected.
 *
 * The configured interval comes from preferences, but it is clamped to the
 * safe minimum so a corrupt value can never spin the remote host.
 */
export function useMonitoringPolling(serverId: string | null, connected: boolean) {
  const intervalSeconds = usePreferencesStore(
    (state) => state.preferences.monitoringIntervalSeconds,
  );
  const refresh = useMonitoringStore((state) => state.refresh);
  const markStale = useMonitoringStore((state) => state.markStale);
  const intervalMs = Math.max(MIN_MONITORING_INTERVAL_SECONDS, intervalSeconds) * 1000;

  useEffect(() => {
    if (!serverId) {
      return;
    }

    if (!connected) {
      markStale(serverId, "Polling is paused because the server is disconnected.");
      return;
    }

    void refresh(serverId);
    const timer = window.setInterval(() => {
      void refresh(serverId);
    }, intervalMs);

    return () => {
      window.clearInterval(timer);
    };
  }, [connected, intervalMs, markStale, refresh, serverId]);
}
