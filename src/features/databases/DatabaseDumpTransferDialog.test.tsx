import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import { DatabaseDumpTransferDialog } from "@/features/databases/DatabaseDumpTransferDialog";
import {
  createDatabaseBackupRequestId,
  getDatabaseBackupTools,
  previewDatabaseRestore,
  restoreDatabaseBackup,
} from "@/services/databaseBackup";
import {
  cancelDatabaseDump,
  createDatabaseDumpRequestId,
  exportDatabaseDump,
  getDatabaseDumpTools,
} from "@/services/databaseDump";
import { useDatabaseBackupStore } from "@/stores/databaseBackupStore";
import { useDatabaseDumpStore } from "@/stores/databaseDumpStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";
import type {
  DatabaseBackupRestorePreview,
  DatabaseBackupRestoreResult,
  DatabaseBackupResult,
  DatabaseDumpToolReport,
  DatabaseProfile,
  ServerProfile,
} from "@/types/contracts";

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

vi.mock("@/lib/fileDialog", () => ({
  pickLocalFile: vi.fn(),
  pickLocalSavePath: vi.fn(),
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
      {
        tool: "pg_dumpall",
        binary: "pg_dumpall",
        available: true,
        version: "16.2.0",
        versionMajor: 16,
        vendor: "postgresql",
        compatible: true,
      },
      {
        tool: "pg_restore",
        binary: "pg_restore",
        available: true,
        version: "16.2.0",
        versionMajor: 16,
        vendor: "postgresql",
        compatible: true,
      },
      {
        tool: "psql",
        binary: "psql",
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

function backupResult(overrides: Partial<DatabaseBackupResult> = {}): DatabaseBackupResult {
  return {
    status: "completed",
    engine: "postgresql",
    scope: { kind: "database", database: "app" },
    format: "plain_sql",
    destinationKind: "remote",
    destinationPath: "/home/deploy/backups/app.sql",
    bytesWritten: 8192,
    verified: true,
    tool: "pg_dump",
    toolVersion: "16.2.0",
    exitCode: 0,
    warnings: [],
    partialRemoved: false,
    tempCleaned: true,
    mayBeIncomplete: false,
    durationMs: 4200,
    ...overrides,
  };
}

function restorePreview(): DatabaseBackupRestorePreview {
  return {
    serverId: server.id,
    serverName: server.name,
    profileId: profile.id,
    profileName: profile.name,
    engine: "postgresql",
    targetDatabase: "app",
    sourceKind: "remote",
    sourcePath: "/home/deploy/backups/app.sql",
    format: "plain_sql",
    tool: "psql",
    toolAvailable: true,
    toolCompatible: true,
    toolVersion: "16.2.0",
    replacesExistingData: true,
    replacedData:
      "existing objects and rows that conflict with the dump in database app may be replaced",
    clean: false,
    noOwner: false,
    canRestore: true,
    warnings: ["The restore does not drop objects that are absent from the dump."],
    confirmation: {
      serverId: server.id,
      serverName: server.name,
      targetDatabase: "app",
      replacedData:
        "existing objects and rows that conflict with the dump in database app may be replaced",
      acknowledged: false,
    },
  };
}

function restoreResult(
  overrides: Partial<DatabaseBackupRestoreResult> = {},
): DatabaseBackupRestoreResult {
  return {
    status: "failed",
    engine: "postgresql",
    format: "plain_sql",
    sourceKind: "remote",
    sourcePath: "/home/deploy/backups/app.sql",
    targetDatabase: "app",
    bytesRead: 4096,
    tool: "psql",
    toolVersion: "16.2.0",
    exitCode: 1,
    warnings: [],
    failureSummary: "psql exited with status 1.",
    partialRemoved: false,
    tempCleaned: true,
    mayBeIncomplete: true,
    durationMs: 900,
    ...overrides,
  };
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
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

function resetStores() {
  useDatabaseDumpStore.setState({
    tools: { status: "idle" },
    transfer: { direction: "export", status: "idle", cancelling: false },
  });
  useDatabaseBackupStore.setState({
    tools: { status: "idle" },
    preview: { status: "idle" },
    transfer: { direction: "backup", status: "idle", cancelling: false },
  });
  useDatabaseWorkspaceStore.setState({ tabs: {}, order: [], activeTabId: null });
  useDatabaseWorkspaceStore.getState().openTab(profile);
}

function renderDialog(overrides: Partial<ComponentProps<typeof DatabaseDumpTransferDialog>> = {}) {
  return render(
    <DatabaseDumpTransferDialog
      open
      mode="export"
      purpose="dump"
      onOpenChange={() => {}}
      profile={profile}
      server={server}
      database="app"
      schema="public"
      selectedTable={null}
      connected
      onConnect={() => {}}
      {...overrides}
    />,
  );
}

describe("DatabaseDumpTransferDialog", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.resetAllMocks();
    resetStores();
    vi.mocked(getDatabaseDumpTools).mockResolvedValue(toolReport());
    vi.mocked(getDatabaseBackupTools).mockResolvedValue(toolReport());
    vi.mocked(createDatabaseDumpRequestId).mockReturnValue("dump-1");
    vi.mocked(createDatabaseBackupRequestId).mockReturnValue("backup-1");
  });

  it("runs a scoped dump export with progress, byte counts, duration, and cleanup", async () => {
    const user = userEvent.setup();
    const pending = deferred<DatabaseBackupResult>();
    vi.mocked(exportDatabaseDump).mockReturnValueOnce(pending.promise);
    const onCompleted = vi.fn();

    renderDialog({ onCompleted });

    await waitFor(() => expect(getDatabaseDumpTools).toHaveBeenCalledWith(profile.id));
    expect(await screen.findByText("Ready")).toBeInTheDocument();

    const output = screen.getByLabelText("Output file (absolute path)");
    await user.clear(output);
    await user.type(output, "/home/deploy/backups/app.sql");
    await user.click(screen.getByRole("button", { name: "Review export" }));

    expect(screen.getByText("Database app")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Export dump" }));

    expect(exportDatabaseDump).toHaveBeenCalledWith(
      profile.id,
      expect.objectContaining({
        scope: { kind: "database", database: "app" },
        format: "plain_sql",
        destination: { kind: "remote", path: "/home/deploy/backups/app.sql", overwrite: false },
      }),
      "dump-1",
    );

    useDatabaseDumpStore.getState().applyTransferProgress({
      transferId: "dump-1",
      direction: "export",
      status: "running",
      phase: "dumping",
      bytesProcessed: 2048,
      totalBytes: 4096,
      percent: 50,
      message: "Writing the dump on the server.",
      mayBeIncomplete: false,
      emittedAt: Date.now(),
    });

    expect(await screen.findByText(/2\.0 KB of 4\.0 KB/)).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "export dump progress" })).toHaveAttribute(
      "aria-valuenow",
      "50",
    );

    pending.resolve(backupResult());
    expect(await screen.findByText("Dump export completed")).toBeInTheDocument();
    expect(screen.getByText(/8\.0 KB written to/)).toBeInTheDocument();
    expect(screen.getByText(/4.2 s/)).toBeInTheDocument();
    expect(screen.getByText(/temporary files cleaned: yes/)).toBeInTheDocument();
    expect(onCompleted).toHaveBeenCalledWith(backupResult());
    const task = useDatabaseWorkspaceStore.getState().tabs[profile.id]?.tasks[0];
    expect(task?.status).toBe("succeeded");
    expect(task?.detail).toContain("8.0 KB");
  });

  it("cancels an in-flight dump and reports the destination as unchanged", async () => {
    const user = userEvent.setup();
    const pending = deferred<DatabaseBackupResult>();
    vi.mocked(exportDatabaseDump).mockReturnValueOnce(pending.promise);
    vi.mocked(cancelDatabaseDump).mockResolvedValueOnce({ requestId: "dump-1", cancelled: true });

    renderDialog();
    await waitFor(() => expect(getDatabaseDumpTools).toHaveBeenCalled());
    const output = screen.getByLabelText("Output file (absolute path)");
    await user.clear(output);
    await user.type(output, "/home/deploy/backups/app.sql");
    await user.click(screen.getByRole("button", { name: "Review export" }));
    await user.click(screen.getByRole("button", { name: "Export dump" }));

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(cancelDatabaseDump).toHaveBeenCalledWith("dump-1");

    pending.resolve(
      backupResult({
        status: "cancelled",
        verified: false,
        exitCode: 143,
        bytesWritten: 1024,
        mayBeIncomplete: false,
        partialRemoved: true,
        failureSummary: "Cancellation was requested.",
        durationMs: 700,
      }),
    );

    expect(await screen.findByText("Dump export cancelled")).toBeInTheDocument();
    expect(screen.getByText(/Cancellation was requested/)).toBeInTheDocument();
    expect(screen.getByText(/Partial output removed: yes/)).toBeInTheDocument();
    const task = useDatabaseWorkspaceStore.getState().tabs[profile.id]?.tasks[0];
    expect(task?.status).toBe("cancelled");
  });

  it("retries a failed dump export after the backend rejects it", async () => {
    const user = userEvent.setup();
    vi.mocked(exportDatabaseDump)
      .mockRejectedValueOnce({
        code: "capability",
        message: "pg_dump is not installed on the remote server.",
      })
      .mockResolvedValueOnce(backupResult());

    renderDialog();
    await waitFor(() => expect(getDatabaseDumpTools).toHaveBeenCalled());
    const output = screen.getByLabelText("Output file (absolute path)");
    await user.clear(output);
    await user.type(output, "/home/deploy/backups/app.sql");
    await user.click(screen.getByRole("button", { name: "Review export" }));
    await user.click(screen.getByRole("button", { name: "Export dump" }));

    expect(await screen.findByText("Transfer failed")).toBeInTheDocument();
    expect(screen.getByText(/not installed/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));

    await waitFor(() => expect(exportDatabaseDump).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("Dump export completed")).toBeInTheDocument();
  });

  it("shows a permission-denied state for tool detection and retries it", async () => {
    const user = userEvent.setup();
    vi.mocked(getDatabaseDumpTools)
      .mockRejectedValueOnce({
        code: "permission",
        message: "The SSH account cannot inspect the database client tools.",
      })
      .mockResolvedValueOnce(toolReport());

    renderDialog();
    expect(await screen.findByText("Permission denied while checking tools")).toBeInTheDocument();
    expect(screen.getByText(/cannot inspect/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Retry detection" }));
    await waitFor(() => expect(getDatabaseDumpTools).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("Ready")).toBeInTheDocument();
  });

  it("confirms overwrite with the destination named before replacing it", async () => {
    const user = userEvent.setup();
    vi.mocked(exportDatabaseDump).mockResolvedValueOnce(backupResult());

    renderDialog();
    await waitFor(() => expect(getDatabaseDumpTools).toHaveBeenCalled());
    const output = screen.getByLabelText("Output file (absolute path)");
    await user.clear(output);
    await user.type(output, "/home/deploy/backups/app.sql");
    await user.click(screen.getByLabelText("Replace an existing output"));
    await user.click(screen.getByRole("button", { name: "Review export" }));
    await user.click(screen.getByRole("button", { name: "Export dump" }));

    const confirm = await screen.findByRole("dialog", {
      name: "Replace the existing dump destination?",
    });
    expect(within(confirm).getByText("/home/deploy/backups/app.sql")).toBeInTheDocument();
    expect(within(confirm).getByText(/Production \(198.51.100.24\)/)).toBeInTheDocument();

    await user.click(within(confirm).getByRole("button", { name: "Replace and export" }));
    await waitFor(() =>
      expect(exportDatabaseDump).toHaveBeenCalledWith(
        profile.id,
        expect.objectContaining({
          destination: {
            kind: "remote",
            path: "/home/deploy/backups/app.sql",
            overwrite: true,
          },
        }),
        "dump-1",
      ),
    );
  });

  it("requires a restore confirmation that names the server, database, and replaced data", async () => {
    const user = userEvent.setup();
    vi.mocked(previewDatabaseRestore).mockResolvedValueOnce(restorePreview());
    vi.mocked(restoreDatabaseBackup).mockResolvedValueOnce(
      restoreResult({ status: "completed", exitCode: 0, mayBeIncomplete: false }),
    );

    renderDialog({ mode: "restore", purpose: "backup" });
    await waitFor(() => expect(getDatabaseBackupTools).toHaveBeenCalledWith(profile.id));
    expect(await screen.findByText("Ready")).toBeInTheDocument();

    await user.type(
      screen.getByLabelText("Source file (absolute path)"),
      "/home/deploy/backups/app.sql",
    );
    await user.click(screen.getByRole("button", { name: "Preview restore" }));

    expect(previewDatabaseRestore).toHaveBeenCalledWith(
      profile.id,
      expect.objectContaining({
        targetDatabase: "app",
        source: { kind: "remote", path: "/home/deploy/backups/app.sql" },
      }),
    );
    expect(await screen.findByText("Restore plan")).toBeInTheDocument();
    expect(screen.getAllByText(/may be replaced/).length).toBeGreaterThan(0);

    await user.click(screen.getByRole("button", { name: "Restore backup" }));
    const confirm = await screen.findByRole("dialog", { name: "Restore into app?" });
    expect(within(confirm).getByText(/Production \(server-1\)/)).toBeInTheDocument();
    expect(within(confirm).getByText("app")).toBeInTheDocument();
    expect(within(confirm).getAllByText(/may be replaced/).length).toBeGreaterThan(0);

    await user.click(within(confirm).getByRole("button", { name: "Restore backup" }));
    await waitFor(() =>
      expect(restoreDatabaseBackup).toHaveBeenCalledWith(
        profile.id,
        expect.objectContaining({
          confirmed: true,
          targetDatabase: "app",
          confirmation: expect.objectContaining({
            serverId: server.id,
            serverName: "Production",
            targetDatabase: "app",
            acknowledged: true,
          }),
        }),
        "backup-1",
      ),
    );
    expect(await screen.findByText("Restore completed")).toBeInTheDocument();
  });

  it("shows a partial restore as incomplete and disables retry", async () => {
    const user = userEvent.setup();
    vi.mocked(previewDatabaseRestore).mockResolvedValueOnce(restorePreview());
    vi.mocked(restoreDatabaseBackup).mockResolvedValueOnce(restoreResult());

    renderDialog({ mode: "restore", purpose: "backup" });
    await waitFor(() => expect(getDatabaseBackupTools).toHaveBeenCalled());
    await user.type(
      screen.getByLabelText("Source file (absolute path)"),
      "/home/deploy/backups/app.sql",
    );
    await user.click(screen.getByRole("button", { name: "Preview restore" }));
    await screen.findByText("Restore plan");
    await user.click(screen.getByRole("button", { name: "Restore backup" }));
    const confirm = await screen.findByRole("dialog", { name: "Restore into app?" });
    await user.click(within(confirm).getByRole("button", { name: "Restore backup" }));

    await waitFor(() => expect(restoreDatabaseBackup).toHaveBeenCalled());
    await waitFor(() =>
      expect(useDatabaseBackupStore.getState().transfer.restoreResult?.status).toBe("failed"),
    );
    expect(await screen.findByText("Restore failed")).toBeInTheDocument();
    expect(screen.getByText(/may contain partially restored objects/)).toBeInTheDocument();
    expect(screen.getByText(/Retry is disabled/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Retry restore" })).not.toBeInTheDocument();
    expect(screen.getByText(/temporary files cleaned: yes/)).toBeInTheDocument();
    const task = useDatabaseWorkspaceStore.getState().tabs[profile.id]?.tasks[0];
    expect(task?.status).toBe("failed");
    expect(task?.partial).toBe(true);
  });
});
