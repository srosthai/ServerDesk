import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LogsPage } from "@/features/logs/LogsPage";
import { DEFAULT_PREFERENCES } from "@/lib/preferences";
import { getConnectionState } from "@/services/connections";
import { listLogSources, readLogTail } from "@/services/logs";
import { listProjects } from "@/services/projects";
import { useConnectionStore } from "@/stores/connectionStore";
import { useLogStore } from "@/stores/logStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useProjectStore } from "@/stores/projectStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  ConnectionSnapshot,
  LogTail,
  ServerProfile,
  SystemLogSource,
} from "@/types/contracts";

vi.mock("@/services/connections", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/services/connections")>();
  return {
    ...actual,
    getConnectionState: vi.fn(),
  };
});

vi.mock("@/services/logs", () => ({
  listLogSources: vi.fn(),
  readLogTail: vi.fn(),
}));

vi.mock("@/services/projects", () => ({
  listProjects: vi.fn(),
  browseRemoteDirectories: vi.fn(),
  scanRemoteProjects: vi.fn(),
  runProjectArtisanAction: vi.fn(),
  deleteProject: vi.fn(),
}));

const server: ServerProfile = {
  id: "server-1",
  name: "Production",
  host: "198.51.100.24",
  port: 22,
  username: "deploy",
  authType: "password",
  credentialRef: null,
  groupName: null,
  tags: [],
  notes: null,
  createdAt: 1,
  updatedAt: 1,
};

const connected: ConnectionSnapshot = {
  serverId: server.id,
  state: "connected",
  changedAt: 1,
};

const syslogSource: SystemLogSource = {
  path: "/var/log/syslog",
  label: "System log (syslog)",
  description: "General system messages.",
};

const laravelLogPath = "/var/www/storefront/storage/logs/laravel.log";

function logTail(overrides: Partial<LogTail> = {}): LogTail {
  return {
    serverId: server.id,
    requestedPath: laravelLogPath,
    resolvedPath: laravelLogPath,
    sourceKind: "laravel",
    status: "available",
    detail: null,
    content: "line one\nERROR queue failed\nline three\n",
    returnedLines: 3,
    returnedBytes: 41,
    fileSizeBytes: 128_000,
    lineLimit: 200,
    byteLimit: 64 * 1024,
    truncated: true,
    truncatedBytes: true,
    truncatedLines: false,
    partialFirstLine: true,
    secretsRedacted: false,
    rotated: false,
    rotationDetail: null,
    retrievedAt: 1_762_665_720_000,
    durationMs: 18,
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

function renderPage(path = laravelLogPath) {
  const query = new URLSearchParams({ server: server.id, path });
  return render(
    <MemoryRouter initialEntries={[`/logs?${query.toString()}`]}>
      <LogsPage />
    </MemoryRouter>,
  );
}

describe("LogsPage", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServerStore.setState({ servers: [server], status: "ready", error: null });
    useConnectionStore.setState({
      connections: { [server.id]: connected },
      busy: {},
      error: null,
    });
    usePreferencesStore.setState({
      preferences: { ...DEFAULT_PREFERENCES, lastActiveServerId: server.id },
    });
    useProjectStore.setState({ entries: {}, actions: {} });
    useLogStore.setState({
      sources: [],
      sourcesPending: false,
      sourcesError: null,
      sourcesLoaded: false,
      entries: {},
    });

    vi.mocked(getConnectionState).mockResolvedValue(connected);
    vi.mocked(listLogSources).mockReset();
    vi.mocked(listLogSources).mockResolvedValue([syslogSource]);
    vi.mocked(readLogTail).mockReset();
    vi.mocked(listProjects).mockReset();
    vi.mocked(listProjects).mockResolvedValue([]);
  });

  it("opens the linked Laravel log path and shows path, retrieval time, and truncation", async () => {
    vi.mocked(readLogTail).mockResolvedValue(logTail());
    renderPage();

    await waitFor(() => expect(readLogTail).toHaveBeenCalledWith(server.id, laravelLogPath, 200));

    expect(await screen.findByText(laravelLogPath)).toBeInTheDocument();
    expect(screen.getAllByText(/Retrieved/).length).toBeGreaterThan(0);
    expect(screen.getByText("Tail was truncated at the safety limit")).toBeInTheDocument();
    expect(screen.getByText(/Only the last 64 KiB/)).toBeInTheDocument();
    expect(
      screen.getByText(
        (_, element) =>
          element?.tagName === "PRE" && (element.textContent ?? "").includes("ERROR queue failed"),
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Laravel log")).toBeInTheDocument();
  });

  it("filters the retrieved tail locally and refreshes with a new bounded read", async () => {
    vi.mocked(readLogTail).mockResolvedValue(logTail());
    const user = userEvent.setup();
    renderPage();

    await screen.findByText(
      (_, element) =>
        element?.tagName === "PRE" && (element.textContent ?? "").includes("ERROR queue failed"),
    );
    expect(readLogTail).toHaveBeenCalledTimes(1);

    await user.type(screen.getByLabelText("Filter log text"), "error");

    expect(
      screen.getByText(
        (_, element) => element?.tagName === "PRE" && element.textContent === "ERROR queue failed",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(
        (_, element) =>
          element?.tagName === "PRE" && (element.textContent ?? "").includes("line one"),
      ),
    ).not.toBeInTheDocument();
    expect(screen.getByText("1 of 3 lines")).toBeInTheDocument();
    expect(readLogTail).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Refresh log tail" }));
    await waitFor(() => expect(readLogTail).toHaveBeenCalledTimes(2));
  });

  it("shows a clear missing-file state without an empty log panel", async () => {
    vi.mocked(readLogTail).mockResolvedValue(
      logTail({
        status: "missing",
        detail: "The log file does not exist on the server.",
        content: "",
        returnedLines: 0,
        returnedBytes: 0,
        fileSizeBytes: null,
        truncated: false,
        truncatedBytes: false,
        partialFirstLine: false,
      }),
    );
    renderPage();

    expect(await screen.findByText("Log file not found")).toBeInTheDocument();
    expect(screen.getByText("The log file does not exist on the server.")).toBeInTheDocument();
    expect(screen.getByText(laravelLogPath)).toBeInTheDocument();
    expect(screen.queryByText("Bounded log tail")).not.toBeInTheDocument();
  });

  it("reports unreadable logs with permission guidance", async () => {
    vi.mocked(readLogTail).mockResolvedValue(
      logTail({
        status: "unreadable",
        detail: "The SSH account cannot read this log file. ServerDesk does not elevate with sudo.",
        content: "",
        returnedLines: 0,
        returnedBytes: 0,
        truncated: false,
        truncatedBytes: false,
        partialFirstLine: false,
      }),
    );
    renderPage();

    expect(await screen.findByText("Log file is not readable")).toBeInTheDocument();
    expect(screen.getByText(/does not elevate with sudo/)).toBeInTheDocument();
  });

  it("shows the backend rejection for a path outside approved locations", async () => {
    const traversal = "/var/www/app/storage/logs/../../../../etc/passwd";
    vi.mocked(readLogTail).mockRejectedValue({
      code: "validation",
      message: "This path is not an approved log location.",
      details: "Use a common system log or a Laravel storage/logs .log file.",
    });
    renderPage(traversal);

    expect(await screen.findByText("Could not read this log")).toBeInTheDocument();
    expect(screen.getByText("This path is not an approved log location.")).toBeInTheDocument();
    expect(
      screen.getByText("Use a common system log or a Laravel storage/logs .log file."),
    ).toBeInTheDocument();
  });

  it("retains a previous tail with its retrieval time when a refresh fails", async () => {
    vi.mocked(readLogTail).mockResolvedValueOnce(logTail()).mockRejectedValueOnce({
      code: "remote",
      message: "The log tail command failed.",
    });
    const user = userEvent.setup();
    renderPage();

    await screen.findByText(
      (_, element) =>
        element?.tagName === "PRE" && (element.textContent ?? "").includes("ERROR queue failed"),
    );
    await user.click(screen.getByRole("button", { name: "Refresh log tail" }));

    expect(await screen.findByText("The log tail command failed.")).toBeInTheDocument();
    expect(
      screen.getByText(
        (_, element) =>
          element?.tagName === "PRE" && (element.textContent ?? "").includes("ERROR queue failed"),
      ),
    ).toBeInTheDocument();
    expect(screen.getAllByText(/Retrieved/).length).toBeGreaterThan(0);
  });
});
