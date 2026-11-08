import { beforeEach, describe, expect, it, vi } from "vitest";
import { connectServer, disconnectServer, getConnectionState } from "@/services/connections";
import { useConnectionStore } from "@/stores/connectionStore";
import type { ConnectionSnapshot } from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    connectServer: vi.fn(),
    disconnectServer: vi.fn(),
    getConnectionState: vi.fn(),
  };
});

function snapshot(
  serverId: string,
  state: ConnectionSnapshot["state"],
  changedAt = 1,
): ConnectionSnapshot {
  return { serverId, state, changedAt };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("connection store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useConnectionStore.setState({
      connections: {},
      busy: {},
      error: null,
    });
  });

  it("applies state-change events without polling", () => {
    const store = useConnectionStore.getState();

    store.applySnapshot(snapshot("server-1", "connecting", 1));
    store.applySnapshot(snapshot("server-1", "connected", 2));

    expect(useConnectionStore.getState().connections["server-1"]?.state).toBe("connected");
  });

  it("keeps the newest snapshot when events arrive out of order", () => {
    const store = useConnectionStore.getState();

    store.applySnapshot(snapshot("server-1", "connected", 10));
    store.applySnapshot(snapshot("server-1", "connecting", 5));

    expect(useConnectionStore.getState().connections["server-1"]?.state).toBe("connected");
  });

  it("connects through the command wrapper and clears pending state", async () => {
    vi.mocked(connectServer).mockResolvedValue(snapshot("server-1", "connected", 3));

    await expect(useConnectionStore.getState().connect("server-1")).resolves.toBe(true);

    const state = useConnectionStore.getState();
    expect(connectServer).toHaveBeenCalledWith("server-1");
    expect(state.connections["server-1"]?.state).toBe("connected");
    expect(state.busy["server-1"]).toBe(false);
    expect(state.error).toBeNull();
  });

  it("surfaces typed connection errors and then applies host-key events", async () => {
    vi.mocked(connectServer).mockRejectedValue({
      code: "host_key",
      message: "The server host key is not trusted yet.",
    });

    await expect(useConnectionStore.getState().connect("server-1")).resolves.toBe(false);
    expect(useConnectionStore.getState().error).toBe("The server host key is not trusted yet.");
    expect(useConnectionStore.getState().connections["server-1"]?.state).toBe("error");

    useConnectionStore.getState().applySnapshot({
      ...snapshot("server-1", "host-key-verification", 4),
      hostKey: { keyType: "ssh-ed25519", fingerprint: "SHA256:new" },
    });

    const connection = useConnectionStore.getState().connections["server-1"];
    expect(connection?.state).toBe("host-key-verification");
    expect(connection?.hostKey?.fingerprint).toBe("SHA256:new");
  });

  it("disconnects and refreshes states through command wrappers", async () => {
    vi.mocked(disconnectServer).mockResolvedValue(snapshot("server-1", "disconnected", 5));
    vi.mocked(getConnectionState).mockResolvedValue(snapshot("server-2", "connected", 6));

    await expect(useConnectionStore.getState().disconnect("server-1")).resolves.toBe(true);
    await useConnectionStore.getState().refreshStates(["server-2"]);

    const state = useConnectionStore.getState();
    expect(disconnectServer).toHaveBeenCalledWith("server-1");
    expect(getConnectionState).toHaveBeenCalledWith("server-2");
    expect(state.connections["server-1"]?.state).toBe("disconnected");
    expect(state.connections["server-2"]?.state).toBe("connected");
  });
});
