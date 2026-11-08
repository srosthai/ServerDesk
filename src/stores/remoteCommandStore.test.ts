import { beforeEach, describe, expect, it, vi } from "vitest";
import { runRemoteCommand } from "@/services/remote";
import { useRemoteCommandStore } from "@/stores/remoteCommandStore";
import type { RemoteCommandResult } from "@/types/contracts";

vi.mock("@/services/remote", () => ({
  remoteCommands: ["hostname", "uptime"],
  runRemoteCommand: vi.fn(),
}));

const hostnameResult: RemoteCommandResult = {
  serverId: "server-1",
  command: "hostname",
  template: "hostname",
  stdout: "production-1\n",
  stderr: "",
  exitCode: 0,
  succeeded: true,
  durationMs: 12,
  connectionState: "connected",
};

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("remote command store", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useRemoteCommandStore.setState({
      results: {},
      pending: {},
      error: null,
    });
  });

  it("runs a typed command and stores stdout, stderr, and the exit code", async () => {
    vi.mocked(runRemoteCommand).mockResolvedValue(hostnameResult);

    await expect(useRemoteCommandStore.getState().run("server-1", "hostname")).resolves.toBe(true);

    expect(runRemoteCommand).toHaveBeenCalledWith("server-1", "hostname");
    const state = useRemoteCommandStore.getState();
    expect(state.results["server-1"]?.hostname).toEqual(hostnameResult);
    expect(state.pending["server-1"]).toBeNull();
    expect(state.error).toBeNull();
  });

  it("keeps structured failure results and reports transport errors", async () => {
    vi.mocked(runRemoteCommand).mockResolvedValueOnce({
      ...hostnameResult,
      command: "uptime",
      template: "uptime",
      stdout: "",
      stderr: "uptime: failed\n",
      exitCode: 7,
      succeeded: false,
    });

    await useRemoteCommandStore.getState().run("server-1", "uptime");

    expect(useRemoteCommandStore.getState().results["server-1"]?.uptime?.exitCode).toBe(7);
    expect(useRemoteCommandStore.getState().error).toBeNull();

    vi.mocked(runRemoteCommand).mockRejectedValueOnce({
      code: "not_found",
      message: "The server is not connected.",
    });
    await useRemoteCommandStore.getState().run("server-1", "hostname");

    const state = useRemoteCommandStore.getState();
    expect(state.error).toBe("The server is not connected.");
    expect(state.results["server-1"]?.uptime?.stderr).toBe("uptime: failed\n");
    expect(state.pending["server-1"]).toBeNull();
  });

  it("clears results when a session is released", async () => {
    vi.mocked(runRemoteCommand).mockResolvedValue(hostnameResult);
    await useRemoteCommandStore.getState().run("server-1", "hostname");

    useRemoteCommandStore.getState().clearServer("server-1");

    expect(useRemoteCommandStore.getState().results["server-1"]).toBeUndefined();
    expect(useRemoteCommandStore.getState().pending["server-1"]).toBeNull();
  });
});
