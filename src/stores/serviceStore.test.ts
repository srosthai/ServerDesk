import { beforeEach, describe, expect, it, vi } from "vitest";
import { getSystemdServices } from "@/services/services";
import { useServiceStore } from "@/stores/serviceStore";
import type { ServiceDiscovery } from "@/types/contracts";

vi.mock("@/services/services", () => ({
  getSystemdServices: vi.fn(),
}));

const discovery: ServiceDiscovery = {
  serverId: "server-1",
  status: "available",
  services: [
    {
      name: "nginx.service",
      state: "active",
      activeState: "active",
      subState: "running",
      loadState: "loaded",
      description: "A high performance web server",
    },
    {
      name: "php8.3-fpm.service",
      state: "active",
      activeState: "active",
      subState: "running",
      loadState: "loaded",
      description: "The PHP 8.3 FastCGI Process Manager",
    },
  ],
  detail: null,
  truncated: false,
  sampledAt: 1_762_665_720_000,
  durationMs: 24,
  connectionState: "connected",
};

const unsupported: ServiceDiscovery = {
  ...discovery,
  status: "unsupported",
  services: [],
  detail: "Systemd is not available on this server (exit code 127).",
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("service store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServiceStore.setState({ entries: {} });
  });

  it("stores a typed discovery result and clears pending state", async () => {
    vi.mocked(getSystemdServices).mockResolvedValue(discovery);

    await expect(useServiceStore.getState().refresh("server-1")).resolves.toBe(true);

    expect(getSystemdServices).toHaveBeenCalledWith("server-1");
    const entry = useServiceStore.getState().entries["server-1"];
    expect(entry?.discovery).toEqual(discovery);
    expect(entry?.pending).toBe(false);
    expect(entry?.error).toBeNull();
    expect(entry?.lastAttemptAt).not.toBeNull();
  });

  it("stores a typed unsupported result as a successful discovery", async () => {
    vi.mocked(getSystemdServices).mockResolvedValue(unsupported);

    await expect(useServiceStore.getState().refresh("server-1")).resolves.toBe(true);

    const entry = useServiceStore.getState().entries["server-1"];
    expect(entry?.discovery?.status).toBe("unsupported");
    expect(entry?.discovery?.services).toEqual([]);
    expect(entry?.error).toBeNull();
  });

  it("keeps the last discovery visible after a failed refresh", async () => {
    vi.mocked(getSystemdServices).mockResolvedValueOnce(discovery);
    await useServiceStore.getState().refresh("server-1");

    vi.mocked(getSystemdServices).mockRejectedValueOnce({
      code: "timeout",
      message: "The remote command timed out.",
    });
    await expect(useServiceStore.getState().refresh("server-1")).resolves.toBe(false);

    const entry = useServiceStore.getState().entries["server-1"];
    expect(entry?.discovery).toEqual(discovery);
    expect(entry?.error).toBe("The remote command timed out.");
    expect(entry?.pending).toBe(false);
  });

  it("reports command errors without inventing a discovery result", async () => {
    vi.mocked(getSystemdServices).mockRejectedValueOnce({
      code: "not_found",
      message: "The server is not connected.",
    });

    await expect(useServiceStore.getState().refresh("server-1")).resolves.toBe(false);

    const entry = useServiceStore.getState().entries["server-1"];
    expect(entry?.discovery).toBeNull();
    expect(entry?.error).toBe("The server is not connected.");
    expect(entry?.pending).toBe(false);
  });

  it("ignores a slow response after a newer refresh", async () => {
    let resolveFirst: ((value: ServiceDiscovery) => void) | undefined;
    vi.mocked(getSystemdServices).mockImplementationOnce(
      () =>
        new Promise<ServiceDiscovery>((resolve) => {
          resolveFirst = resolve;
        }),
    );

    const first = useServiceStore.getState().refresh("server-1");
    const newer = { ...discovery, sampledAt: discovery.sampledAt + 1_000 };
    vi.mocked(getSystemdServices).mockResolvedValueOnce(newer);
    await useServiceStore.getState().refresh("server-1");

    resolveFirst?.(discovery);
    await first;

    expect(useServiceStore.getState().entries["server-1"]?.discovery).toEqual(newer);
  });

  it("clears per-server data and errors", async () => {
    vi.mocked(getSystemdServices).mockResolvedValue(discovery);
    await useServiceStore.getState().refresh("server-1");

    useServiceStore.getState().clearError("server-1");
    useServiceStore.getState().clearServer("server-1");

    const entry = useServiceStore.getState().entries["server-1"];
    expect(entry).toBeUndefined();
  });
});
