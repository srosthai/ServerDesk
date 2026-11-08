import { beforeEach, describe, expect, it, vi } from "vitest";
import { getSystemdServices, runServiceAction } from "@/services/services";
import { useServiceStore } from "@/stores/serviceStore";
import type { ServiceActionResult, ServiceDiscovery, ServiceStatus } from "@/types/contracts";

vi.mock("@/services/services", () => ({
  getSystemdServices: vi.fn(),
  runServiceAction: vi.fn(),
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

const stoppedDiscovery: ServiceDiscovery = {
  ...discovery,
  services: discovery.services.map((service) =>
    service.name === "nginx.service"
      ? {
          ...service,
          state: "inactive",
          activeState: "inactive",
          subState: "dead",
        }
      : service,
  ),
  sampledAt: 1_762_665_735_000,
  durationMs: 18,
};

function verifiedStatus(overrides: Partial<ServiceStatus> = {}): ServiceStatus {
  return {
    serverId: "server-1",
    unit: "nginx.service",
    state: "inactive",
    activeState: "inactive",
    subState: "dead",
    loadState: "loaded",
    unitFileState: "enabled",
    description: "A high performance web server",
    sampledAt: 1_762_665_730_000,
    durationMs: 8,
    ...overrides,
  };
}

function actionResult(overrides: Partial<ServiceActionResult> = {}): ServiceActionResult {
  return {
    serverId: "server-1",
    unit: "nginx.service",
    action: "stop",
    command: "/usr/bin/sudo -n -- /usr/bin/systemctl stop -- nginx.service",
    succeeded: true,
    stdout: "stopped\n",
    stderr: "",
    exitCode: 0,
    durationMs: 12,
    permissionDenied: false,
    sudoGuidance: null,
    verifiedStatus: verifiedStatus(),
    stateVerified: true,
    verificationError: null,
    error: null,
    connectionState: "connected",
    ...overrides,
  };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("service store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServiceStore.setState({ entries: {}, actions: {} });
    vi.mocked(runServiceAction).mockReset();
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

  it("stores the action result and refreshes the row from discovery", async () => {
    vi.mocked(getSystemdServices)
      .mockReset()
      .mockResolvedValueOnce(discovery)
      .mockResolvedValue(stoppedDiscovery);
    await useServiceStore.getState().refresh("server-1");
    vi.mocked(runServiceAction).mockResolvedValue(actionResult());

    await expect(
      useServiceStore.getState().runAction("server-1", "nginx.service", "stop"),
    ).resolves.toBe(true);

    expect(runServiceAction).toHaveBeenCalledWith("server-1", "nginx.service", "stop");
    expect(getSystemdServices).toHaveBeenCalledTimes(2);
    const actionEntry = useServiceStore.getState().actions["server-1"];
    expect(actionEntry?.pending).toBeNull();
    expect(actionEntry?.pendingUnit).toBeNull();
    expect(actionEntry?.result?.stdout).toBe("stopped\n");
    expect(actionEntry?.result?.stateVerified).toBe(true);

    const nginx = useServiceStore
      .getState()
      .entries["server-1"]?.discovery?.services.find((service) => service.name === "nginx.service");
    expect(nginx?.state).toBe("inactive");
    expect(nginx?.activeState).toBe("inactive");
    expect(useServiceStore.getState().entries["server-1"]?.discovery?.sampledAt).toBe(
      stoppedDiscovery.sampledAt,
    );
  });

  it("keeps the action result visible when the post-action refresh fails", async () => {
    vi.mocked(getSystemdServices)
      .mockReset()
      .mockResolvedValueOnce(discovery)
      .mockRejectedValueOnce({
        code: "timeout",
        message: "The remote command timed out.",
      });
    await useServiceStore.getState().refresh("server-1");
    vi.mocked(runServiceAction).mockResolvedValue(actionResult());

    await expect(
      useServiceStore.getState().runAction("server-1", "nginx.service", "stop"),
    ).resolves.toBe(true);

    const actionEntry = useServiceStore.getState().actions["server-1"];
    expect(actionEntry?.result?.stdout).toBe("stopped\n");
    expect(actionEntry?.error).toBeNull();
    expect(useServiceStore.getState().entries["server-1"]?.error).toBe(
      "The remote command timed out.",
    );
    const nginx = useServiceStore
      .getState()
      .entries["server-1"]?.discovery?.services.find((service) => service.name === "nginx.service");
    expect(nginx?.state).toBe("inactive");
  });

  it("keeps permission denial visible with sudo guidance and the verified state", async () => {
    vi.mocked(getSystemdServices).mockResolvedValue(discovery);
    await useServiceStore.getState().refresh("server-1");
    vi.mocked(runServiceAction).mockResolvedValue(
      actionResult({
        action: "start",
        command: "/usr/bin/sudo -n -- /usr/bin/systemctl start -- nginx.service",
        succeeded: false,
        stdout: "",
        stderr: "sudo: a password is required\n",
        exitCode: 1,
        permissionDenied: true,
        sudoGuidance:
          "<serverdesk-ssh-user> ALL=(root) NOPASSWD: /usr/bin/systemctl start -- nginx.service",
        verifiedStatus: verifiedStatus({
          state: "active",
          activeState: "active",
          subState: "running",
        }),
        stateVerified: false,
        error: {
          code: "permission",
          message: "The SSH account is not allowed to start nginx.service.",
          details: "Never grant unrestricted sudo.",
        },
      }),
    );

    await expect(
      useServiceStore.getState().runAction("server-1", "nginx.service", "start"),
    ).resolves.toBe(false);

    const result = useServiceStore.getState().actions["server-1"]?.result;
    expect(result?.permissionDenied).toBe(true);
    expect(result?.sudoGuidance).toContain("NOPASSWD");
    expect(result?.error?.code).toBe("permission");
    expect(result?.stderr).toBe("sudo: a password is required\n");
    expect(result?.exitCode).toBe(1);
    expect(result?.verifiedStatus?.state).toBe("active");
  });

  it("keeps the last verified discovery when verification fails", async () => {
    vi.mocked(getSystemdServices).mockResolvedValue(discovery);
    await useServiceStore.getState().refresh("server-1");
    vi.mocked(runServiceAction).mockResolvedValue(
      actionResult({
        verifiedStatus: null,
        stateVerified: null,
        verificationError: "Systemd did not report ActiveState for nginx.service.",
      }),
    );

    await useServiceStore.getState().runAction("server-1", "nginx.service", "stop");

    const nginx = useServiceStore
      .getState()
      .entries["server-1"]?.discovery?.services.find((service) => service.name === "nginx.service");
    expect(nginx?.state).toBe("active");
    expect(useServiceStore.getState().entries["server-1"]?.discovery?.sampledAt).toBe(
      discovery.sampledAt,
    );
    expect(useServiceStore.getState().actions["server-1"]?.result?.verificationError).toContain(
      "did not report ActiveState",
    );
  });

  it("stores transport action errors with details and clears pending state", async () => {
    vi.mocked(runServiceAction).mockRejectedValue({
      code: "timeout",
      message: "The remote command timed out after 30 seconds.",
      details: "No result was received.",
    });

    await expect(
      useServiceStore.getState().runAction("server-1", "nginx.service", "restart"),
    ).resolves.toBe(false);

    const actionEntry = useServiceStore.getState().actions["server-1"];
    expect(actionEntry?.pending).toBeNull();
    expect(actionEntry?.pendingUnit).toBeNull();
    expect(actionEntry?.result).toBeNull();
    expect(actionEntry?.error).toBe("The remote command timed out after 30 seconds.");
    expect(actionEntry?.errorDetails).toBe("No result was received.");
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
