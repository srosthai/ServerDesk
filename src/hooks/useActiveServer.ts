import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";

export function useActiveServer() {
  const servers = useServerStore((state) => state.servers);
  const lastActiveServerId = usePreferencesStore((state) => state.preferences.lastActiveServerId);
  const connections = useConnectionStore((state) => state.connections);

  const connectedServerId = Object.values(connections).find(
    (connection) => connection.state === "connected",
  )?.serverId;

  const activeServer =
    servers.find((server) => server.id === lastActiveServerId) ??
    servers.find((server) => server.id === connectedServerId) ??
    (servers.length === 1 ? servers[0] : null);

  return {
    activeServer,
    connection: activeServer ? (connections[activeServer.id] ?? null) : null,
  };
}
