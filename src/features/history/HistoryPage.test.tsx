import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { HistoryPage } from "@/features/history/HistoryPage";
import {
  clearOperationHistory,
  getOperationHistoryPolicy,
  listOperationHistory,
} from "@/services/history";
import { useServerStore } from "@/stores/serverStore";
import type {
  OperationHistoryEntry,
  OperationHistoryPolicy,
  ServerProfile,
} from "@/types/contracts";

vi.mock("@/services/history", () => ({
  listOperationHistory: vi.fn(),
  getOperationHistoryPolicy: vi.fn(),
  clearOperationHistory: vi.fn(),
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

const policy: OperationHistoryPolicy = {
  maxEntries: 1_000,
  outputSummaryMaxChars: 360,
  retention:
    "ServerDesk keeps the newest 1,000 operations and automatically removes older entries.",
};

const entries: OperationHistoryEntry[] = [
  {
    id: "operation-2",
    serverId: server.id,
    projectId: null,
    category: "service",
    operation: "service-restart",
    safeSummary: "Restarted nginx.service (exit code 0)",
    path: null,
    outputSummary: "restarted nginx.service",
    result: "success",
    exitCode: 0,
    startedAt: 1_762_665_720,
    durationMs: 42,
  },
  {
    id: "operation-1",
    serverId: server.id,
    projectId: "project-1",
    category: "artisan",
    operation: "artisan-migrate-status",
    safeSummary: "Ran php artisan migrate:status in /var/www/storefront (exit code 1)",
    path: "/var/www/storefront",
    outputSummary: "[redacted environment line] | Migration table not found",
    result: "failure",
    exitCode: 1,
    startedAt: 1_762_665_600,
    durationMs: 1_250,
  },
];

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function renderPage() {
  return render(
    <MemoryRouter>
      <HistoryPage />
    </MemoryRouter>,
  );
}

describe("HistoryPage", () => {
  beforeEach(() => {
    enableTauriRuntime();
    useServerStore.setState({
      servers: [server],
      status: "ready",
      error: null,
      fetchServers: vi.fn(),
    });
    vi.mocked(getOperationHistoryPolicy).mockReset();
    vi.mocked(getOperationHistoryPolicy).mockResolvedValue(policy);
    vi.mocked(listOperationHistory).mockReset();
    vi.mocked(listOperationHistory).mockResolvedValue(entries);
    vi.mocked(clearOperationHistory).mockReset();
    vi.mocked(clearOperationHistory).mockResolvedValue(undefined);
  });

  it("lists successful and failed operations with server, category, result, and timing", async () => {
    renderPage();

    expect(await screen.findByText("service-restart")).toBeInTheDocument();
    expect(screen.getByText("artisan-migrate-status")).toBeInTheDocument();
    expect(screen.getAllByText("Production").length).toBe(2);
    expect(screen.getAllByText("Success").length).toBeGreaterThan(0);
    expect(screen.getByText("Failure")).toBeInTheDocument();
    expect(screen.getByText("42 ms")).toBeInTheDocument();
    expect(screen.getByText("1.3 s")).toBeInTheDocument();
    expect(screen.getByText("exit 1")).toBeInTheDocument();
    expect(listOperationHistory).toHaveBeenCalledWith({
      serverId: null,
      category: null,
      result: null,
      startedAfter: null,
      startedBefore: null,
      limit: 1_000,
    });
  });

  it("shows details without raw output or secret-bearing fields", async () => {
    const user = userEvent.setup();
    renderPage();

    await screen.findByText("service-restart");
    await user.click(screen.getByRole("button", { name: "View details for service-restart" }));

    const dialog = await screen.findByRole("dialog", { name: "Operation details" });
    expect(within(dialog).getByText("Restarted nginx.service (exit code 0)")).toBeInTheDocument();
    expect(within(dialog).getByText("restarted nginx.service")).toBeInTheDocument();
    expect(within(dialog).getByText("Production")).toBeInTheDocument();
    expect(within(dialog).getByText("Exit code 0")).toBeInTheDocument();
    expect(within(dialog).queryByText(/stdout/i)).not.toBeInTheDocument();
    expect(within(dialog).queryByText(/terminal keystrokes/i)).toBeInTheDocument();
  });

  it("filters by category, result, and date through the backend filter", async () => {
    const user = userEvent.setup();
    renderPage();

    await screen.findByText("service-restart");

    await user.click(screen.getByRole("combobox", { name: "Category" }));
    await user.click(await screen.findByRole("option", { name: "Service" }));

    await waitFor(() => {
      expect(listOperationHistory).toHaveBeenLastCalledWith(
        expect.objectContaining({ category: "service" }),
      );
    });

    await user.click(screen.getByRole("combobox", { name: "Result" }));
    await user.click(await screen.findByRole("option", { name: "Failure" }));

    await waitFor(() => {
      expect(listOperationHistory).toHaveBeenLastCalledWith(
        expect.objectContaining({ category: "service", result: "failure" }),
      );
    });

    fireEvent.change(screen.getByLabelText("From date"), {
      target: { value: "2026-01-01" },
    });

    await waitFor(() => {
      expect(listOperationHistory).toHaveBeenLastCalledWith(
        expect.objectContaining({
          category: "service",
          result: "failure",
          startedAfter: expect.any(Number),
        }),
      );
    });
  });

  it("clears history only after confirmation", async () => {
    const user = userEvent.setup();
    vi.mocked(listOperationHistory).mockResolvedValueOnce(entries).mockResolvedValue([]);
    renderPage();

    await screen.findByText("service-restart");
    await user.click(screen.getByRole("button", { name: "Clear history" }));

    const dialog = await screen.findByRole("dialog", {
      name: "Clear all local operation history?",
    });
    expect(within(dialog).getByText(/only clears local history/i)).toBeInTheDocument();
    expect(clearOperationHistory).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Clear history" }));

    await waitFor(() => {
      expect(clearOperationHistory).toHaveBeenCalledWith(true);
    });
    expect(await screen.findByText("No operations recorded")).toBeInTheDocument();
  });

  it("shows an empty state when nothing has been recorded", async () => {
    vi.mocked(listOperationHistory).mockResolvedValue([]);
    renderPage();

    expect(await screen.findByText("No operations recorded")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Clear history" })).toBeDisabled();
  });
});
