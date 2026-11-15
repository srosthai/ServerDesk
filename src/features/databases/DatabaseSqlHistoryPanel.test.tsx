import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import { DatabaseSqlHistoryPanel } from "@/features/databases/DatabaseSqlHistoryPanel";
import {
  clearDatabaseQueryHistory,
  createDatabaseSavedQuery,
  deleteDatabaseSavedQuery,
  getDatabaseQueryHistoryPolicy,
  listDatabaseQueryHistory,
  listDatabaseSavedQueries,
} from "@/services/databaseHistory";
import { useDatabaseHistoryStore } from "@/stores/databaseHistoryStore";
import type {
  DatabaseProfile,
  DatabaseQueryHistoryEntry,
  DatabaseQueryHistoryPolicy,
  DatabaseSavedQuery,
} from "@/types/contracts";

vi.mock("@/services/databaseHistory", () => ({
  listDatabaseQueryHistory: vi.fn(),
  getDatabaseQueryHistoryPolicy: vi.fn(),
  clearDatabaseQueryHistory: vi.fn(),
  listDatabaseSavedQueries: vi.fn(),
  createDatabaseSavedQuery: vi.fn(),
  updateDatabaseSavedQuery: vi.fn(),
  deleteDatabaseSavedQuery: vi.fn(),
}));

const policy: DatabaseQueryHistoryPolicy = {
  maxEntries: 1_000,
  maxSqlChars: 8_192,
  errorSummaryMaxChars: 512,
  snippetNameMaxChars: 120,
  snippetSqlMaxChars: 65_536,
  retention:
    "ServerDesk keeps the newest 1,000 queries and automatically removes older entries. Stored SQL is limited to 8,192 characters and is sanitized before it is saved.",
};

const connections: DatabaseProfile[] = [
  {
    id: "profile-1",
    serverId: "server-1",
    name: "Primary",
    engine: "postgresql",
    host: "127.0.0.1",
    port: 5432,
    socket: null,
    databaseName: "app",
    username: "app",
    hasCredential: true,
    readOnly: false,
    sslMode: null,
    createdAt: 1,
    updatedAt: 1,
    lastUsedAt: null,
  },
  {
    id: "profile-2",
    serverId: "server-1",
    name: "Reporting",
    engine: "mysql",
    host: "127.0.0.1",
    port: 3306,
    socket: null,
    databaseName: "reports",
    username: "report",
    hasCredential: true,
    readOnly: false,
    sslMode: null,
    createdAt: 1,
    updatedAt: 1,
    lastUsedAt: null,
  },
];

const history: DatabaseQueryHistoryEntry[] = [
  {
    id: "history-1",
    serverId: "server-1",
    connectionId: "profile-1",
    engine: "postgresql",
    databaseName: "app",
    sqlText: "SELECT 1",
    result: "success",
    durationMs: 4,
    rowCount: 1,
    errorSummary: null,
    startedAt: 1_700_000_100,
  },
  {
    id: "history-2",
    serverId: "server-1",
    connectionId: "profile-1",
    engine: "postgresql",
    databaseName: "app",
    sqlText: "SELECT * FORM users",
    result: "failure",
    durationMs: 12,
    rowCount: null,
    errorSummary: 'syntax error at or near "form"',
    startedAt: 1_700_000_200,
  },
];

const snippets: DatabaseSavedQuery[] = [
  {
    id: "snippet-1",
    name: "Active users",
    engine: "postgresql",
    sqlText: "SELECT * FROM users WHERE active = true",
    createdAt: 1,
    updatedAt: 1,
  },
  {
    id: "snippet-2",
    name: "MySQL orders",
    engine: "mysql",
    sqlText: "SELECT * FROM orders",
    createdAt: 1,
    updatedAt: 1,
  },
];

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function renderPanel(overrides: Partial<ComponentProps<typeof DatabaseSqlHistoryPanel>> = {}) {
  const onInsertSnippet = vi.fn();
  const rendered = render(
    <DatabaseSqlHistoryPanel
      profileId="profile-1"
      serverId="server-1"
      engine="postgresql"
      connectionName="Primary"
      currentSql="SELECT current_database();"
      connections={connections}
      onInsertSnippet={onInsertSnippet}
      {...overrides}
    />,
  );
  return { ...rendered, onInsertSnippet };
}

describe("DatabaseSqlHistoryPanel", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseHistoryStore.getState().reset();
    vi.mocked(getDatabaseQueryHistoryPolicy).mockResolvedValue(policy);
    vi.mocked(listDatabaseQueryHistory).mockResolvedValue(history);
    vi.mocked(listDatabaseSavedQueries).mockResolvedValue(snippets);
    vi.mocked(clearDatabaseQueryHistory).mockResolvedValue(undefined);
    vi.mocked(createDatabaseSavedQuery).mockResolvedValue(snippets[0]);
    vi.mocked(deleteDatabaseSavedQuery).mockResolvedValue(undefined);
  });

  it("shows sanitized history and saved snippets with engine compatibility", async () => {
    renderPanel();

    expect(await screen.findByText("SELECT 1")).toBeInTheDocument();
    expect(screen.getByText("SELECT * FORM users")).toBeInTheDocument();
    expect(screen.getByText('syntax error at or near "form"')).toBeInTheDocument();
    expect(screen.getByText("Active users")).toBeInTheDocument();
    expect(screen.getByText("MySQL orders")).toBeInTheDocument();
    expect(screen.getByText(/never stored/i)).toBeInTheDocument();

    expect(screen.getByRole("button", { name: "Insert snippet Active users" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Insert snippet MySQL orders" })).toBeDisabled();
    expect(screen.getByText(/Saved for MySQL/)).toBeInTheDocument();
    expect(listDatabaseQueryHistory).toHaveBeenCalledWith(
      expect.objectContaining({
        serverId: "server-1",
        connectionId: "profile-1",
        result: null,
        limit: 1_000,
      }),
    );
  });

  it("filters history by connection, result, and date through the backend filter", async () => {
    const user = userEvent.setup();
    renderPanel();

    await screen.findByText("SELECT 1");

    await user.click(screen.getByRole("combobox", { name: "History connection filter" }));
    await user.click(await screen.findByRole("option", { name: "All connections" }));

    await waitFor(() => {
      expect(listDatabaseQueryHistory).toHaveBeenLastCalledWith(
        expect.objectContaining({ connectionId: null }),
      );
    });

    await user.click(screen.getByRole("combobox", { name: "History result filter" }));
    await user.click(await screen.findByRole("option", { name: "Failure" }));

    await waitFor(() => {
      expect(listDatabaseQueryHistory).toHaveBeenLastCalledWith(
        expect.objectContaining({ connectionId: null, result: "failure" }),
      );
    });

    fireEvent.change(screen.getByLabelText("History from date"), {
      target: { value: "2026-01-01" },
    });

    await waitFor(() => {
      expect(listDatabaseQueryHistory).toHaveBeenLastCalledWith(
        expect.objectContaining({
          connectionId: null,
          result: "failure",
          startedAfter: expect.any(Number),
        }),
      );
    });
  });

  it("clears history only after confirmation", async () => {
    const user = userEvent.setup();
    renderPanel();

    await screen.findByText("SELECT 1");
    await user.click(screen.getByRole("button", { name: "Clear history" }));

    const dialog = await screen.findByRole("dialog", { name: "Clear query history?" });
    expect(within(dialog).getByText(/Saved snippets are not affected/i)).toBeInTheDocument();
    expect(clearDatabaseQueryHistory).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Clear history" }));

    await waitFor(() => {
      expect(clearDatabaseQueryHistory).toHaveBeenCalledWith(true);
    });
    expect(useDatabaseHistoryStore.getState().history).toEqual([]);
  });

  it("deletes a snippet only after confirmation", async () => {
    const user = userEvent.setup();
    renderPanel();

    await screen.findByText("Active users");
    await user.click(screen.getByRole("button", { name: "Delete snippet Active users" }));

    const dialog = await screen.findByRole("dialog", {
      name: 'Delete snippet "Active users"?',
    });
    expect(deleteDatabaseSavedQuery).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Delete snippet" }));

    await waitFor(() => {
      expect(deleteDatabaseSavedQuery).toHaveBeenCalledWith("snippet-1", true);
    });
    expect(screen.queryByText("Active users")).not.toBeInTheDocument();
  });

  it("inserts a compatible snippet into the current query tab", async () => {
    const user = userEvent.setup();
    const { onInsertSnippet } = renderPanel();

    await screen.findByText("Active users");
    await user.click(screen.getByRole("button", { name: "Insert snippet Active users" }));

    expect(onInsertSnippet).toHaveBeenCalledWith(snippets[0]);
    expect(await screen.findByRole("status")).toHaveTextContent(
      'Inserted "Active users" into the current query tab.',
    );
  });

  it("saves the current SQL as a named snippet", async () => {
    const user = userEvent.setup();
    renderPanel();

    await screen.findByText("Active users");
    await user.type(screen.getByLabelText("Snippet name"), "Current database");
    await user.click(screen.getByRole("button", { name: "Use current SQL" }));
    await user.click(screen.getByRole("button", { name: "Save snippet" }));

    await waitFor(() => {
      expect(createDatabaseSavedQuery).toHaveBeenCalledWith({
        name: "Current database",
        engine: "postgresql",
        sql: "SELECT current_database();",
      });
    });
    expect(await screen.findByRole("status")).toHaveTextContent('Saved snippet "Active users".');
  });
});
