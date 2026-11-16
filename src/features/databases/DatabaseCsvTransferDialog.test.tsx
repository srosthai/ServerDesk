import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseCsvTransferDialog } from "@/features/databases/DatabaseCsvTransferDialog";
import {
  cancelDatabaseCsvTransfer,
  createDatabaseCsvRequestId,
  exportDatabaseCsv,
  importDatabaseCsv,
  previewDatabaseCsvImport,
} from "@/services/databaseCsv";
import { useDatabaseCsvStore } from "@/stores/databaseCsvStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";
import { pickLocalFile } from "@/lib/fileDialog";
import type {
  DatabaseCsvExportResult,
  DatabaseCsvImportPreview,
  DatabaseCsvImportResult,
  DatabaseProfile,
  DatabaseRowColumn,
} from "@/types/contracts";

vi.mock("@/services/databaseCsv", () => ({
  DATABASE_CSV_TRANSFER_EVENT: "database-csv-transfer-event",
  previewDatabaseCsvImport: vi.fn(),
  exportDatabaseCsv: vi.fn(),
  importDatabaseCsv: vi.fn(),
  cancelDatabaseCsvTransfer: vi.fn(),
  createDatabaseCsvRequestId: vi.fn(),
}));

vi.mock("@/lib/fileDialog", () => ({
  pickLocalFile: vi.fn(),
  pickLocalSavePath: vi.fn(),
}));

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

const columns: DatabaseRowColumn[] = [
  {
    name: "name",
    dataType: "text",
    kind: "text",
    nullable: false,
    ordinalPosition: 1,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: true,
  },
];

function preview(): DatabaseCsvImportPreview {
  return {
    headers: ["name"],
    rows: [
      {
        rowNumber: 2,
        fields: [{ value: "Ada", quoted: false, isNull: false }],
      },
    ],
    warnings: [],
    warningsTruncated: false,
    sampledRows: 1,
    hasMore: false,
    delimiter: "comma",
    encoding: "utf8",
    hasHeader: true,
  };
}

function exportResult(): DatabaseCsvExportResult {
  return {
    status: "completed",
    engine: "postgresql",
    table: { database: "app", schema: "public", name: "users" },
    destinationPath: "/tmp/users.csv",
    rowsExported: 3,
    columns: ["name"],
    bytesWritten: 42,
    encoding: "utf8",
    delimiter: "comma",
    includeHeader: true,
    warnings: [],
    mayBeIncomplete: false,
    valuesTruncated: false,
    durationMs: 20,
  };
}

function importResult(): DatabaseCsvImportResult {
  return {
    status: "cancelled",
    engine: "postgresql",
    table: { database: "app", schema: "public", name: "users" },
    rowsRead: 2,
    rowsImported: 0,
    rowsSkipped: 0,
    warnings: [],
    warningsTruncated: false,
    transaction: "rolled_back",
    mayBeIncomplete: false,
    durationMs: 12,
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

function renderDialog(mode: "export" | "import") {
  return render(
    <DatabaseCsvTransferDialog
      open
      mode={mode}
      onOpenChange={() => {}}
      profileId={profile.id}
      profileName={profile.name}
      serverName="Production"
      table={{ database: "app", schema: "public", name: "users" }}
      columns={columns}
      readOnly={false}
    />,
  );
}

describe("DatabaseCsvTransferDialog", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.resetAllMocks();
    useDatabaseCsvStore.setState({
      preview: { status: "idle" },
      transfer: { direction: "export", status: "idle", cancelling: false },
    });
    useDatabaseWorkspaceStore.setState({ tabs: {}, order: [], activeTabId: null });
    useDatabaseWorkspaceStore.getState().openTab(profile);
    vi.mocked(createDatabaseCsvRequestId).mockReturnValue("csv-1");
    vi.mocked(previewDatabaseCsvImport).mockResolvedValue(preview());
  });

  it("previews, maps, and imports a CSV file, then reports a cancelled rollback", async () => {
    const user = userEvent.setup();
    const pending = deferred<DatabaseCsvImportResult>();
    vi.mocked(importDatabaseCsv).mockReturnValueOnce(pending.promise);
    vi.mocked(cancelDatabaseCsvTransfer).mockResolvedValueOnce({
      requestId: "csv-1",
      cancelled: true,
    });

    renderDialog("import");
    await user.click(screen.getByRole("button", { name: "Choose CSV source" }));
    await user.type(screen.getByLabelText("Source file (absolute path)"), "/tmp/users.csv");
    await user.click(screen.getByRole("button", { name: "Continue to mapping" }));
    await user.click(screen.getByRole("button", { name: "Load preview" }));

    expect(previewDatabaseCsvImport).toHaveBeenCalledWith(
      "/tmp/users.csv",
      expect.objectContaining({ delimiter: "comma", encoding: "utf8", hasHeader: true }),
      20,
    );
    expect(await screen.findByText("Ada")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Review import" }));
    expect(screen.getAllByText(/app \/ public \/ users/).length).toBeGreaterThan(0);
    await user.click(screen.getByLabelText("Acknowledge the CSV import target"));
    await user.click(screen.getByRole("button", { name: "Import CSV" }));

    expect(importDatabaseCsv).toHaveBeenCalledWith(
      profile.id,
      expect.objectContaining({
        table: { database: "app", schema: "public", name: "users" },
        sourcePath: "/tmp/users.csv",
        mappings: [{ sourceIndex: 0, targetColumn: "name" }],
      }),
      "csv-1",
    );

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(cancelDatabaseCsvTransfer).toHaveBeenCalledWith("csv-1");

    pending.resolve(importResult());
    expect(await screen.findByText("Import cancelled")).toBeInTheDocument();
    expect(screen.getByText(/No rows were imported/)).toBeInTheDocument();
    const task = useDatabaseWorkspaceStore.getState().tabs[profile.id]?.tasks[0];
    expect(task?.status).toBe("cancelled");
    expect(task?.detail).toContain("rolled back");
  });

  it("selects a local CSV file with the native picker", async () => {
    const user = userEvent.setup();
    vi.mocked(pickLocalFile).mockResolvedValueOnce("/tmp/users.csv");

    renderDialog("import");
    await user.click(screen.getByRole("button", { name: "Choose CSV source" }));

    expect(pickLocalFile).toHaveBeenCalledWith(
      expect.objectContaining({ title: "Choose a CSV source file" }),
    );
    expect(screen.getByLabelText("Source file (absolute path)")).toHaveValue("/tmp/users.csv");
  });

  it("shows a permission-denied preview error with the file still selected", async () => {
    const user = userEvent.setup();
    vi.mocked(previewDatabaseCsvImport).mockRejectedValueOnce({
      code: "permission",
      message: "The desktop process cannot read /tmp/users.csv.",
    });

    renderDialog("import");
    await user.type(screen.getByLabelText("Source file (absolute path)"), "/tmp/users.csv");
    await user.click(screen.getByRole("button", { name: "Continue to mapping" }));
    await user.click(screen.getByRole("button", { name: "Load preview" }));

    expect(await screen.findByText("Permission denied reading the CSV file")).toBeInTheDocument();
    expect(screen.getByText(/cannot read/)).toBeInTheDocument();
    expect(screen.getByText(/Source file · \/tmp\/users\.csv/)).toBeInTheDocument();
  });

  it("confirms an existing CSV destination before exporting", async () => {
    const user = userEvent.setup();
    vi.mocked(exportDatabaseCsv).mockResolvedValueOnce(exportResult());

    renderDialog("export");
    await user.type(screen.getByLabelText("Destination file (absolute path)"), "/tmp/users.csv");
    await user.click(screen.getByLabelText("Replace an existing destination"));
    await user.click(screen.getByRole("button", { name: "Review export" }));
    await user.click(screen.getByRole("button", { name: "Export CSV" }));

    const confirm = await screen.findByRole("dialog", {
      name: "Replace the existing CSV destination?",
    });
    expect(within(confirm).getByText("/tmp/users.csv")).toBeInTheDocument();
    expect(within(confirm).getByText("Production")).toBeInTheDocument();

    await user.click(within(confirm).getByRole("button", { name: "Replace and export" }));
    await waitFor(() =>
      expect(exportDatabaseCsv).toHaveBeenCalledWith(
        profile.id,
        expect.objectContaining({
          destinationPath: "/tmp/users.csv",
          overwrite: true,
        }),
        "csv-1",
      ),
    );
    expect(await screen.findByText("Export completed")).toBeInTheDocument();
  });

  it("does not allow a read-only import to be acknowledged", async () => {
    render(
      <DatabaseCsvTransferDialog
        open
        mode="import"
        onOpenChange={() => {}}
        profileId={profile.id}
        profileName={profile.name}
        serverName="Production"
        table={{ database: "app", schema: "public", name: "users" }}
        columns={columns}
        readOnly
      />,
    );
    expect(screen.getByText("Read-only profile")).toBeInTheDocument();
  });
});
