import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseOperationsWorkspace } from "@/features/databases/DatabaseOperationsWorkspace";
import { listColumns, listTables } from "@/services/databaseDiscovery";
import { cancelDatabaseCsvTransfer } from "@/services/databaseCsv";
import { getDatabaseDumpTools } from "@/services/databaseDump";
import { useDatabaseBackupStore } from "@/stores/databaseBackupStore";
import { useDatabaseCsvStore } from "@/stores/databaseCsvStore";
import { useDatabaseDumpStore } from "@/stores/databaseDumpStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";
import type {
  DatabaseColumnSummary,
  DatabaseConnectionSnapshot,
  DatabaseDumpToolReport,
  DatabaseProfile,
  DatabaseTableSummary,
  ServerProfile,
} from "@/types/contracts";

vi.mock("@/services/databaseDiscovery", () => ({
  listTables: vi.fn(),
  listColumns: vi.fn(),
}));

vi.mock("@/services/databaseCsv", () => ({
  DATABASE_CSV_TRANSFER_EVENT: "database-csv-transfer-event",
  previewDatabaseCsvImport: vi.fn(),
  exportDatabaseCsv: vi.fn(),
  importDatabaseCsv: vi.fn(),
  cancelDatabaseCsvTransfer: vi.fn(),
  createDatabaseCsvRequestId: vi.fn(),
}));

vi.mock("@/services/databaseDump", () => ({
  DATABASE_DUMP_EVENT: "database-dump-event",
  getDatabaseDumpTools: vi.fn(),
  exportDatabaseDump: vi.fn(),
  importDatabaseDump: vi.fn(),
  cancelDatabaseDump: vi.fn(),
  createDatabaseDumpRequestId: vi.fn(),
}));

vi.mock("@/services/databaseBackup", () => ({
  DATABASE_BACKUP_EVENT: "database-dump-event",
  getDatabaseBackupTools: vi.fn(),
  previewDatabaseRestore: vi.fn(),
  createDatabaseBackup: vi.fn(),
  restoreDatabaseBackup: vi.fn(),
  cancelDatabaseBackup: vi.fn(),
  createDatabaseBackupRequestId: vi.fn(),
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

const profile: DatabaseProfile = {
  id: "profile-1",
  serverId: server.id,
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
  serverId: server.id,
  name: profile.name,
  engine: profile.engine,
  state: "connected",
  databaseName: "app",
  readOnly: false,
  changedAt: 1,
};

function tableSummary(name: string): DatabaseTableSummary {
  return {
    database: "app",
    schema: "public",
    name,
    kind: "table",
    estimatedRows: { status: "available", value: 10 },
    metadataRefreshedAt: { status: "available", value: "2026-01-01T00:00:00Z" },
    owner: { status: "unavailable" },
    encoding: { status: "unavailable" },
    collation: { status: "unavailable" },
    sizeBytes: { status: "unavailable" },
    engine: { status: "unavailable" },
  };
}

function columnSummary(name: string): DatabaseColumnSummary {
  return {
    name,
    dataType: "text",
    length: { status: "unavailable" },
    precision: { status: "unavailable" },
    scale: { status: "unavailable" },
    nullable: false,
    defaultValue: { status: "unavailable" },
    ordinalPosition: 1,
    primaryKey: false,
    identity: { status: "available", value: "none" },
    generated: { status: "available", value: false },
    generationExpression: { status: "unavailable" },
    comment: { status: "unavailable" },
    collation: { status: "unavailable" },
  };
}

function toolReport(): DatabaseDumpToolReport {
  return {
    engine: "postgresql",
    serverVersion: "16.2.0",
    tools: [
      {
        tool: "pg_dump",
        binary: "pg_dump",
        available: true,
        version: "16.2.0",
        versionMajor: 16,
        vendor: "postgresql",
        compatible: true,
      },
    ],
    exportReady: true,
    restoreReady: true,
    checkedAt: 1,
  };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function prepareWorkspace() {
  useDatabaseWorkspaceStore.setState({ tabs: {}, order: [], activeTabId: null });
  useDatabaseWorkspaceStore.getState().openTab(profile);
  useDatabaseWorkspaceStore.getState().setSelection(profile.id, {
    id: "table-row",
    profileId: profile.id,
    kind: "table",
    label: "users",
    engine: "postgresql",
    database: "app",
    schema: "public",
    objectName: "users",
  });
  const tab = useDatabaseWorkspaceStore.getState().tabs[profile.id];
  if (!tab) {
    throw new Error("workspace tab was not created");
  }
  return tab;
}

function renderWorkspace({
  connected = true,
  tab = prepareWorkspace(),
  readOnly = false,
}: {
  connected?: boolean;
  tab?: ReturnType<typeof prepareWorkspace>;
  readOnly?: boolean;
} = {}) {
  return render(
    <DatabaseOperationsWorkspace
      tab={tab}
      profile={readOnly ? { ...profile, readOnly: true } : profile}
      server={server}
      snapshot={readOnly ? { ...snapshot, readOnly: true } : snapshot}
      connected={connected}
      onConnect={() => {}}
    />,
  );
}

describe("DatabaseOperationsWorkspace", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.resetAllMocks();
    useDatabaseDumpStore.setState({
      tools: { status: "idle" },
      transfer: { direction: "export", status: "idle", cancelling: false },
    });
    useDatabaseBackupStore.setState({
      tools: { status: "idle" },
      preview: { status: "idle" },
      transfer: { direction: "backup", status: "idle", cancelling: false },
    });
    useDatabaseCsvStore.setState({
      preview: { status: "idle" },
      transfer: { direction: "export", status: "idle", cancelling: false },
    });
    vi.mocked(listTables).mockResolvedValue({
      items: [tableSummary("users"), tableSummary("orders")],
      limit: 200,
      offset: 0,
      hasMore: false,
      total: 2,
    });
    vi.mocked(listColumns).mockResolvedValue({
      items: [columnSummary("name")],
      limit: 500,
      offset: 0,
      hasMore: false,
      total: 1,
    });
    vi.mocked(getDatabaseDumpTools).mockResolvedValue(toolReport());
    vi.mocked(cancelDatabaseCsvTransfer).mockResolvedValue({
      requestId: "csv-other",
      cancelled: true,
    });
  });

  it("loads tables and columns and exposes CSV, dump, backup, and restore actions", async () => {
    renderWorkspace();

    expect(await screen.findByText("Dump tools ready")).toBeInTheDocument();
    expect(await screen.findByLabelText("CSV table")).toHaveValue("users");
    await waitFor(() =>
      expect(listColumns).toHaveBeenCalledWith(
        profile.id,
        { database: "app", schema: "public", table: "users" },
        { limit: 500, offset: 0 },
      ),
    );

    expect(screen.getByRole("button", { name: "Export CSV" })).toBeEnabled();
    await waitFor(() => expect(screen.getByRole("button", { name: "Import CSV" })).toBeEnabled());
    expect(screen.getByRole("button", { name: "Export dump" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Restore dump" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Create backup" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Restore backup" })).toBeEnabled();
    expect(screen.getByText(/No imports, exports, backups/)).toBeInTheDocument();
  });

  it("shows a disconnected state with connect and recent task history", async () => {
    const tab = prepareWorkspace();
    useDatabaseWorkspaceStore.getState().addTask(profile.id, {
      id: "task-1",
      label: "Backup · database app",
      status: "succeeded",
      detail: "8.0 KB · verified",
      startedAt: 1,
    });

    renderWorkspace({ connected: false, tab });
    expect(screen.getByText("Connection is not live")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Connect Primary/ })).toBeInTheDocument();
    expect(screen.getByText("Backup · database app")).toBeInTheDocument();
  });

  it("shows a permission-denied state when table listing is refused", async () => {
    vi.mocked(listTables).mockRejectedValueOnce({
      code: "permission",
      message: "The database account cannot read the public schema.",
    });

    renderWorkspace();
    expect(await screen.findByText("Permission denied listing tables")).toBeInTheDocument();
    expect(screen.getByText(/cannot read the public schema/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import CSV" })).toBeDisabled();
  });

  it("keeps an active transfer scoped to its own connection tab", async () => {
    useDatabaseCsvStore.setState({
      transfer: {
        transferId: "csv-other",
        profileId: "profile-2",
        direction: "import",
        status: "running",
        cancelling: false,
        startedAt: Date.now(),
      },
    });

    const first = renderWorkspace();
    expect(screen.queryByText("Active transfers")).not.toBeInTheDocument();
    first.unmount();

    useDatabaseCsvStore.setState({
      transfer: {
        transferId: "csv-current",
        profileId: profile.id,
        direction: "import",
        status: "running",
        cancelling: false,
        startedAt: Date.now(),
        progress: {
          transferId: "csv-current",
          direction: "import",
          status: "running",
          phase: "transferring",
          rowsProcessed: 12,
          rowsWritten: 10,
          rowsSkipped: 2,
          totalRows: null,
          bytesProcessed: 2048,
          totalBytes: null,
          warnings: [],
          mayBeIncomplete: false,
          emittedAt: Date.now(),
        },
      },
    });
    vi.mocked(cancelDatabaseCsvTransfer).mockResolvedValueOnce({
      requestId: "csv-current",
      cancelled: true,
    });

    renderWorkspace();
    expect(screen.getByText("Active transfers")).toBeInTheDocument();
    expect(screen.getByText(/CSV import · users/)).toBeInTheDocument();
    expect(screen.getByText(/12 rows/)).toBeInTheDocument();

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(cancelDatabaseCsvTransfer).toHaveBeenCalledWith("csv-current");
  });

  it("marks a partially completed operation as incomplete and clears finished records", async () => {
    const tab = prepareWorkspace();
    useDatabaseWorkspaceStore.getState().addTask(profile.id, {
      id: "task-partial",
      label: "Restore · app",
      status: "failed",
      detail: "psql exited with status 1 · target may be incomplete",
      partial: true,
      startedAt: 1,
    });

    renderWorkspace({ tab });
    expect(screen.getByText("Incomplete")).toBeInTheDocument();
    expect(screen.getByText(/target may be incomplete/)).toBeInTheDocument();

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Clear finished" }));
    expect(screen.queryByText("Restore · app")).not.toBeInTheDocument();
  });

  it("opens the backup wizard from the operations action", async () => {
    const user = userEvent.setup();
    renderWorkspace();
    await user.click(screen.getByRole("button", { name: "Create backup" }));
    expect(
      await screen.findByRole("dialog", { name: "Create database backup" }),
    ).toBeInTheDocument();
  });
});
