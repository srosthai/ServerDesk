import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseSqlWorkspace } from "@/features/databases/DatabaseSqlWorkspace";
import {
  analyzeDatabaseSql,
  cancelDatabaseSql,
  createDatabaseQueryRequestId,
  executeDatabaseSql,
} from "@/services/databaseQueries";
import {
  clearDatabaseQueryHistory,
  createDatabaseSavedQuery,
  deleteDatabaseSavedQuery,
  getDatabaseQueryHistoryPolicy,
  listDatabaseQueryHistory,
  listDatabaseSavedQueries,
  updateDatabaseSavedQuery,
} from "@/services/databaseHistory";
import { useDatabaseHistoryStore } from "@/stores/databaseHistoryStore";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import { useDatabaseQueryStore } from "@/stores/databaseQueryStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";
import type {
  DatabaseCapabilities,
  DatabaseConnectionSnapshot,
  DatabaseProfile,
  DatabaseQueryExecutionResult,
  DatabaseSqlAnalysis,
} from "@/types/contracts";

vi.mock("@/services/databaseQueries", () => ({
  analyzeDatabaseSql: vi.fn(),
  cancelDatabaseSql: vi.fn(),
  createDatabaseQueryRequestId: vi.fn(),
  executeDatabaseSql: vi.fn(),
}));

vi.mock("@/services/databaseHistory", () => ({
  listDatabaseQueryHistory: vi.fn(),
  getDatabaseQueryHistoryPolicy: vi.fn(),
  clearDatabaseQueryHistory: vi.fn(),
  listDatabaseSavedQueries: vi.fn(),
  createDatabaseSavedQuery: vi.fn(),
  updateDatabaseSavedQuery: vi.fn(),
  deleteDatabaseSavedQuery: vi.fn(),
}));

const capabilities: DatabaseCapabilities = {
  engine: "postgresql",
  version: "16.2.0",
  flags: {
    schemas: true,
    sequences: true,
    materializedViews: true,
    routines: true,
    triggers: true,
    roles: true,
    sessions: true,
    transactions: true,
    savepoints: true,
    cancelQuery: true,
    explain: true,
    checkConstraints: true,
    generatedColumns: true,
    identityColumns: true,
    autoIncrementColumns: false,
    columnTypeChange: true,
    columnRename: true,
    tableRename: true,
    tableDuplicate: true,
    columnReorder: false,
    databaseRename: true,
    indexRename: true,
    partialIndexes: true,
    queryTimeout: true,
  },
  actions: ["run_query", "cancel_query"],
};

const profile: DatabaseProfile = {
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
  sslMode: "required",
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: 1_700_000_000,
};

const snapshot: DatabaseConnectionSnapshot = {
  profileId: profile.id,
  serverId: profile.serverId,
  name: profile.name,
  engine: profile.engine,
  state: "connected",
  readOnly: false,
  databaseName: "app",
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  capabilities,
  tunnel: {
    profileId: profile.id,
    serverId: profile.serverId,
    state: "active",
    localHost: "127.0.0.1",
    localPort: 52341,
    remoteHost: "127.0.0.1",
    remotePort: 5432,
    openedAt: 1,
  },
  changedAt: 10,
};

const analysis: DatabaseSqlAnalysis = {
  engine: "postgresql",
  statementCount: 1,
  hasWrites: false,
  destructive: false,
  requiresConfirmation: false,
  warnings: [],
};

function queryResult(
  columns: Array<{ name: string; dataType: string }>,
  rows: string[][],
  options: { durationMs?: number; affectedRows?: number | null } = {},
): DatabaseQueryExecutionResult {
  const durationMs = options.durationMs ?? 4;
  return {
    engine: "postgresql",
    readOnly: false,
    results: [
      {
        index: 0,
        columns,
        rows: rows.map((row) => row.map((value) => ({ value, truncated: false }))),
        returnedRows: rows.length,
        affectedRows: options.affectedRows ?? null,
        lastInsertId: null,
        truncated: false,
        columnsTruncated: false,
      },
    ],
    warnings: [],
    destructive: false,
    requiresConfirmation: false,
    truncated: false,
    maxRows: 500,
    durationMs,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function openWorkspace(currentSnapshot: DatabaseConnectionSnapshot = snapshot) {
  useDatabaseWorkspaceStore.getState().openTab(profile);
  const tab = useDatabaseWorkspaceStore.getState().tabs[profile.id];
  const onConnect = vi.fn();
  const onConnectionMayHaveClosed = vi.fn();
  const rendered = render(
    <DatabaseSqlWorkspace
      tab={tab}
      snapshot={currentSnapshot}
      onConnect={onConnect}
      onConnectionMayHaveClosed={onConnectionMayHaveClosed}
    />,
  );
  return { ...rendered, onConnect, onConnectionMayHaveClosed, tab };
}

function sqlEditor() {
  return screen.getByLabelText("SQL editor for Primary") as HTMLTextAreaElement;
}

async function runSql(user: ReturnType<typeof userEvent.setup>, sql: string) {
  const editor = sqlEditor();
  await user.clear(editor);
  await user.type(editor, sql);
  await user.click(screen.getByRole("button", { name: "Run" }));
}

describe("DatabaseSqlWorkspace", () => {
  let idCounter = 0;

  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    idCounter = 0;
    useDatabaseWorkspaceStore.setState({ tabs: {}, order: [], activeTabId: null });
    useDatabaseQueryStore.setState({ entries: {} });
    useDatabaseProfileStore.setState({ profiles: { [profile.serverId]: [profile] } });
    useDatabaseHistoryStore.getState().reset();
    vi.mocked(getDatabaseQueryHistoryPolicy).mockResolvedValue({
      maxEntries: 1_000,
      maxSqlChars: 8_192,
      errorSummaryMaxChars: 512,
      snippetNameMaxChars: 120,
      snippetSqlMaxChars: 65_536,
      retention: "Keeps the newest 1,000 queries.",
    });
    vi.mocked(listDatabaseQueryHistory).mockResolvedValue([]);
    vi.mocked(listDatabaseSavedQueries).mockResolvedValue([]);
    vi.mocked(clearDatabaseQueryHistory).mockResolvedValue(undefined);
    vi.mocked(createDatabaseSavedQuery).mockResolvedValue({
      id: "snippet-1",
      name: "Snippet",
      engine: "postgresql",
      sqlText: "SELECT 1",
      createdAt: 1,
      updatedAt: 1,
    });
    vi.mocked(updateDatabaseSavedQuery).mockResolvedValue({
      id: "snippet-1",
      name: "Snippet",
      engine: "postgresql",
      sqlText: "SELECT 1",
      createdAt: 1,
      updatedAt: 1,
    });
    vi.mocked(deleteDatabaseSavedQuery).mockResolvedValue(undefined);
    vi.mocked(createDatabaseQueryRequestId).mockImplementation(() => {
      idCounter += 1;
      return `request-${idCounter}`;
    });
    vi.mocked(analyzeDatabaseSql).mockResolvedValue(analysis);
    vi.mocked(executeDatabaseSql).mockResolvedValue(
      queryResult([{ name: "value", dataType: "text" }], [["ann"]]),
    );
    vi.mocked(cancelDatabaseSql).mockResolvedValue({
      requestId: "request-1",
      cancelled: true,
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn().mockReturnValue("blob:query-result"),
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: vi.fn(),
    });
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  });

  it("shows connection, engine, database, schema, read-only, and query-tab context", () => {
    openWorkspace();

    const context = screen.getByLabelText("SQL context for Primary");
    expect(within(context).getByText("Primary")).toBeInTheDocument();
    expect(within(context).getByText("PostgreSQL")).toBeInTheDocument();
    expect(within(context).getByText("app")).toBeInTheDocument();
    expect(within(context).getByText("default")).toBeInTheDocument();
    expect(within(context).getByText("DB connected")).toBeInTheDocument();
    expect(within(context).getByRole("switch", { name: "Read-only mode" })).not.toBeChecked();
    expect(screen.getByRole("tab", { name: "Query 1" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByLabelText("SQL editor for Primary")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Run all" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "New SQL query tab" })).toBeInTheDocument();
    expect(screen.getByText(/No results yet/)).toBeInTheDocument();
  });

  it("executes selected SQL and preserves editor and result state per query tab", async () => {
    const user = userEvent.setup();
    openWorkspace();

    const editor = sqlEditor();
    await user.type(editor, "SELECT 1;{enter}SELECT 2;");
    editor.setSelectionRange(10, 18);
    fireEvent.select(editor);
    await user.click(screen.getByRole("button", { name: "Run" }));

    await waitFor(() => {
      expect(executeDatabaseSql).toHaveBeenCalledWith(
        profile.id,
        "SELECT 2",
        { maxRows: 500, timeoutMs: 10_000, confirmed: false },
        "request-2",
      );
    });
    const firstResult = await screen.findByRole("region", { name: "SQL results for Primary" });
    expect(firstResult).toHaveTextContent("value");
    expect(firstResult).toHaveTextContent("ann");

    await user.click(screen.getByRole("button", { name: "New SQL query tab" }));
    expect(screen.getByRole("tab", { name: "Query 2" })).toHaveAttribute("aria-selected", "true");
    expect(sqlEditor()).toHaveValue("");

    vi.mocked(executeDatabaseSql).mockResolvedValueOnce(
      queryResult([{ name: "other", dataType: "text" }], [["second"]]),
    );
    await runSql(user, "SELECT 3;");
    expect(await screen.findByText("other")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Query 1" }));
    expect(sqlEditor()).toHaveValue("SELECT 1;\nSELECT 2;");
    expect(await screen.findByText("ann")).toBeInTheDocument();

    const tab = useDatabaseWorkspaceStore.getState().tabs[profile.id];
    expect(tab.sqlTabs).toHaveLength(2);
    expect(tab.sqlTabs[0]).toMatchObject({
      title: "Query 1",
      sql: "SELECT 1;\nSELECT 2;",
      selectionStart: 10,
      selectionEnd: 18,
    });
    expect(tab.sqlTabs[1]).toMatchObject({ title: "Query 2", sql: "SELECT 3;" });
    expect(useDatabaseQueryStore.getState().entries[tab.sqlTabs[0].id]?.result).toBeDefined();
    expect(useDatabaseQueryStore.getState().entries[tab.sqlTabs[1].id]?.result).toBeDefined();
  });

  it("runs the statement at the cursor and Run all submits every statement", async () => {
    const user = userEvent.setup();
    openWorkspace();
    const editor = sqlEditor();
    await user.type(editor, "SELECT 1;{enter}SELECT 2;");
    editor.setSelectionRange(3, 3);
    fireEvent.select(editor);
    await user.click(screen.getByRole("button", { name: "Run" }));

    await waitFor(() => {
      expect(executeDatabaseSql).toHaveBeenLastCalledWith(
        profile.id,
        "SELECT 1",
        expect.any(Object),
        "request-2",
      );
    });

    await user.click(screen.getByRole("button", { name: "Run all" }));
    await waitFor(() => {
      expect(executeDatabaseSql).toHaveBeenLastCalledWith(
        profile.id,
        "SELECT 1;\nSELECT 2;",
        expect.any(Object),
        "request-4",
      );
    });
  });

  it("executes the current statement from the editor keyboard shortcut", async () => {
    const user = userEvent.setup();
    openWorkspace();
    const editor = sqlEditor();
    await user.type(editor, "SELECT 1;{enter}SELECT 2;");
    editor.setSelectionRange(12, 12);
    fireEvent.select(editor);

    await user.keyboard("{Control>}{Enter}{/Control}");

    await waitFor(() => {
      expect(executeDatabaseSql).toHaveBeenCalledWith(
        profile.id,
        "SELECT 2",
        { maxRows: 500, timeoutMs: 10_000, confirmed: false },
        "request-2",
      );
    });
  });

  it("cancels a running query and reports the cancellation", async () => {
    const user = userEvent.setup();
    const pending = deferred<DatabaseQueryExecutionResult>();
    vi.mocked(executeDatabaseSql).mockReturnValueOnce(pending.promise);
    const { onConnectionMayHaveClosed } = openWorkspace();

    await runSql(user, "SELECT pg_sleep(30);");
    expect(await screen.findByRole("status", { name: "Query running" })).toBeInTheDocument();
    vi.mocked(cancelDatabaseSql).mockResolvedValueOnce({
      requestId: "request-2",
      cancelled: true,
    });

    await user.click(screen.getByRole("button", { name: "Cancel query" }));

    expect(cancelDatabaseSql).toHaveBeenCalledWith("request-2");
    expect(
      await screen.findByRole("status", { name: "Query cancelled for Primary" }),
    ).toBeInTheDocument();
    expect(onConnectionMayHaveClosed).toHaveBeenCalled();

    pending.reject({ code: "cancelled", message: "The SQL execution was cancelled." });
    await waitFor(() => {
      expect(
        useDatabaseQueryStore.getState().entries[
          useDatabaseWorkspaceStore.getState().tabs[profile.id].sqlTabs[0].id
        ]?.status,
      ).toBe("cancelled");
    });
  });

  it("reports execution duration and affected rows for write statements", async () => {
    const user = userEvent.setup();
    vi.mocked(executeDatabaseSql).mockResolvedValueOnce(
      queryResult([], [], { affectedRows: 3, durationMs: 12 }),
    );
    openWorkspace();

    await runSql(user, "UPDATE users SET plan = 'team' WHERE id = 1;");

    const result = await screen.findByRole("region", { name: "SQL results for Primary" });
    expect(result).toHaveTextContent("Query succeeded");
    expect(result).toHaveTextContent("12 ms");
    expect(result).toHaveTextContent("3 rows affected");
    expect(result).toHaveTextContent("3 affected");
  });

  it("shows timeout feedback and asks for a reconnect", async () => {
    const user = userEvent.setup();
    vi.mocked(executeDatabaseSql).mockRejectedValueOnce({
      code: "timeout",
      message: "The SQL execution exceeded its 10 second timeout.",
      durationMs: 10_000,
    });
    const { onConnectionMayHaveClosed } = openWorkspace();

    await runSql(user, "SELECT pg_sleep(30);");

    const alert = await screen.findByRole("alert", { name: "Query timed out for Primary" });
    expect(alert).toHaveTextContent("The SQL execution exceeded its 10 second timeout.");
    expect(alert).toHaveTextContent("10.00 s");
    expect(alert).toHaveTextContent("Reconnect the profile");
    expect(onConnectionMayHaveClosed).toHaveBeenCalled();
  });

  it("renders structured SQL errors with code, position, and duration", async () => {
    const user = userEvent.setup();
    vi.mocked(executeDatabaseSql).mockRejectedValueOnce({
      code: "query",
      message: "The SQL statement was rejected.",
      details: "Check the statement around the reported position.",
      databaseCode: "42601",
      position: 3,
      durationMs: 12,
    });
    openWorkspace();

    await runSql(user, "SELEC 1;");

    const alert = await screen.findByRole("alert", { name: "SQL error for Primary" });
    expect(alert).toHaveTextContent("The SQL statement was rejected.");
    expect(alert).toHaveTextContent("code query");
    expect(alert).toHaveTextContent("SQLSTATE 42601");
    expect(alert).toHaveTextContent("line 1, column 3");
    expect(alert).toHaveTextContent("12 ms");
    expect(alert).toHaveTextContent("Check the statement around the reported position.");
    expect(within(alert).getByRole("button", { name: "Go to error" })).toBeInTheDocument();
    expect(within(alert).getByRole("button", { name: "Run again" })).toBeInTheDocument();
  });

  it("blocks writes in read-only mode before execution", async () => {
    const user = userEvent.setup();
    vi.mocked(analyzeDatabaseSql).mockResolvedValueOnce({
      ...analysis,
      hasWrites: true,
      statementCount: 1,
    });
    openWorkspace();

    await user.click(screen.getByRole("switch", { name: "Read-only mode" }));
    expect(screen.getByRole("switch", { name: "Read-only mode" })).toBeChecked();
    expect(screen.getByText(/Guard on · writes blocked before execution/)).toBeInTheDocument();

    await runSql(user, "DELETE FROM users;");

    const alert = await screen.findByRole("alert", {
      name: "Read-only mode blocked SQL for Primary",
    });
    expect(alert).toHaveTextContent("Blocked by read-only mode");
    expect(alert).toHaveTextContent("No statement was sent to the database");
    expect(alert).toHaveTextContent("0 executed");
    expect(executeDatabaseSql).not.toHaveBeenCalled();
  });

  it("locks the read-only switch when the connection profile is read-only", () => {
    openWorkspace({ ...snapshot, readOnly: true });

    const toggle = screen.getByRole("switch", { name: "Read-only mode" });
    expect(toggle).toBeChecked();
    expect(toggle).toBeDisabled();
    expect(screen.getByText(/Profile-enforced · adapter rejects writes/)).toBeInTheDocument();
    expect(screen.getByText(/The adapter rejects writes before execution/)).toBeInTheDocument();
  });

  it("asks for confirmation before destructive SQL and passes confirmed on execute", async () => {
    const user = userEvent.setup();
    vi.mocked(analyzeDatabaseSql).mockResolvedValueOnce({
      ...analysis,
      hasWrites: true,
      destructive: true,
      requiresConfirmation: true,
      warnings: [
        {
          code: "drop",
          statementIndex: 1,
          message: "Statement 1 contains DROP and can permanently remove a database object.",
        },
      ],
    });
    openWorkspace();

    await runSql(user, "DROP TABLE users;");

    expect(await screen.findByText("Run destructive SQL?")).toBeInTheDocument();
    expect(screen.getByText("DROP TABLE users;")).toBeInTheDocument();
    expect(screen.getByText(/Statement 1 contains DROP/)).toBeInTheDocument();
    expect(executeDatabaseSql).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Run destructive SQL" }));

    await waitFor(() => {
      expect(executeDatabaseSql).toHaveBeenCalledWith(
        profile.id,
        "DROP TABLE users",
        { maxRows: 500, timeoutMs: 10_000, confirmed: true },
        "request-2",
      );
    });
  });

  it("inserts a saved snippet into the active query tab", async () => {
    const user = userEvent.setup();
    vi.mocked(listDatabaseSavedQueries).mockResolvedValueOnce([
      {
        id: "snippet-1",
        name: "Active users",
        engine: "postgresql",
        sqlText: "SELECT * FROM users WHERE active = true",
        createdAt: 1,
        updatedAt: 1,
      },
    ]);
    openWorkspace();

    await screen.findByText("Active users");
    expect(sqlEditor().value).toBe("");

    await user.click(screen.getByRole("button", { name: "Insert snippet Active users" }));

    await waitFor(() => {
      expect(sqlEditor().value).toBe("SELECT * FROM users WHERE active = true");
    });
    expect(screen.getByRole("status")).toHaveTextContent(
      'Inserted "Active users" into the current query tab.',
    );
  });

  it("exports CSV and JSON and copies the selected result set", async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined);
    vi.mocked(executeDatabaseSql).mockResolvedValueOnce(
      queryResult(
        [
          { name: "id", dataType: "int4" },
          { name: "name", dataType: "text" },
        ],
        [["1", 'Ann, "Ace"']],
      ),
    );
    openWorkspace();

    await runSql(user, "SELECT 1;");
    await screen.findByRole("region", { name: "SQL results for Primary" });

    await user.click(screen.getByRole("button", { name: "CSV" }));
    expect(URL.createObjectURL).toHaveBeenCalled();
    expect(HTMLAnchorElement.prototype.click).toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent("Exported primary-query-1-result-1.csv.");

    await user.click(screen.getByRole("button", { name: "JSON" }));
    expect((URL.createObjectURL as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(2);
    expect(screen.getByRole("status")).toHaveTextContent("Exported primary-query-1-result-1.json.");

    await user.click(screen.getByRole("button", { name: "Copy result" }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith('id\tname\n1\tAnn, "Ace"');
    });
  });
});
