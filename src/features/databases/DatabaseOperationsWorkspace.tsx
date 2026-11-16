import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  ArchiveBoxIcon,
  ArrowClockwiseIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  DatabaseIcon,
  FileCsvIcon,
  HardDriveIcon,
  InfoIcon,
  PlugsIcon,
  RowsIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Label } from "@/components/ui/label";
import { ErrorState } from "@/components/ui/states";
import { DatabaseCsvTransferDialog } from "@/features/databases/DatabaseCsvTransferDialog";
import {
  DatabaseDumpTransferDialog,
  type DatabaseDumpTransferMode,
  type DatabaseDumpTransferPurpose,
} from "@/features/databases/DatabaseDumpTransferDialog";
import {
  csvPhaseLabel,
  csvProgressPercent,
  dumpPhaseLabel,
  dumpProgressPercent,
  formatBytes,
  formatCount,
  formatDurationMs,
  transferStatusLabel,
} from "@/features/databases/databaseTransferModel";
import { useDatabaseBackupEvents } from "@/hooks/useDatabaseBackupEvents";
import { useDatabaseCsvTransferEvents } from "@/hooks/useDatabaseCsvTransferEvents";
import { useDatabaseDumpEvents } from "@/hooks/useDatabaseDumpEvents";
import { isCommandError } from "@/lib/errors";
import { listColumns, listTables } from "@/services/databaseDiscovery";
import { useDatabaseBackupStore } from "@/stores/databaseBackupStore";
import { useDatabaseCsvStore } from "@/stores/databaseCsvStore";
import { useDatabaseDumpStore } from "@/stores/databaseDumpStore";
import {
  useDatabaseWorkspaceStore,
  type DatabaseWorkspaceTask,
  type DatabaseWorkspaceTaskStatus,
  type DatabaseWorkspaceTab,
} from "@/stores/databaseWorkspaceStore";
import type {
  CommandError,
  DatabaseColumnSummary,
  DatabaseConnectionSnapshot,
  DatabaseProfile,
  DatabaseRowColumn,
  DatabaseTableSummary,
  ServerProfile,
} from "@/types/contracts";

type TableState =
  | { status: "idle" | "loading" }
  | { status: "ready"; tables: DatabaseTableSummary[] }
  | { status: "error"; error: CommandError };

type ColumnState =
  | { status: "idle" | "loading" }
  | { status: "ready"; columns: DatabaseRowColumn[] }
  | { status: "error"; error: CommandError };

const EMPTY_TASKS: DatabaseWorkspaceTask[] = [];

function columnFromSummary(column: DatabaseColumnSummary): DatabaseRowColumn {
  return {
    name: column.name,
    dataType: column.dataType,
    kind: "other",
    nullable: column.nullable,
    ordinalPosition: column.ordinalPosition,
    primaryKey: column.primaryKey,
    identity: column.identity.status === "available" ? column.identity.value : "none",
    generated: column.generated.status === "available" ? column.generated.value : false,
    sortable: false,
    filterable: false,
    searchable: false,
  };
}

function commandError(error: unknown): CommandError {
  return isCommandError(error) ? error : { code: "internal", message: String(error) };
}

function taskVariant(
  status: DatabaseWorkspaceTaskStatus,
): "success" | "warning" | "danger" | "outline" {
  switch (status) {
    case "running":
      return "warning";
    case "succeeded":
      return "success";
    case "cancelled":
      return "outline";
    default:
      return "danger";
  }
}

function ActiveTransferCard({
  label,
  status,
  progress,
  detail,
  cancelling,
  partial,
  onCancel,
}: {
  label: string;
  status: string;
  progress: { percent: number | null; detail: string };
  detail: string;
  cancelling: boolean;
  partial?: boolean;
  onCancel: () => void;
}) {
  return (
    <div className="rounded-md border border-border bg-card px-3 py-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-1.5 text-[11px] font-medium text-foreground">
          {status === "running" ? (
            <CircleNotchIcon className="size-3.5 animate-spin text-primary" aria-hidden="true" />
          ) : (
            <CheckCircleIcon className="size-3.5 text-success" aria-hidden="true" />
          )}
          <span className="truncate">{label}</span>
        </span>
        <div className="flex items-center gap-1.5">
          {partial ? <Badge variant="warning">Incomplete</Badge> : null}
          <Badge
            variant={status === "running" ? "info" : status === "completed" ? "success" : "warning"}
          >
            {transferStatusLabel(
              status as "running" | "completed" | "cancelled" | "failed" | "error",
            )}
          </Badge>
          {status === "running" ? (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-6 px-2 text-[10px]"
              onClick={onCancel}
              disabled={cancelling}
            >
              <XIcon className="size-3" aria-hidden="true" />
              {cancelling ? "Cancelling..." : "Cancel"}
            </Button>
          ) : null}
        </div>
      </div>
      <div
        role="progressbar"
        aria-label={`${label} progress`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={progress.percent ?? undefined}
        className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
      >
        <div
          className={
            progress.percent === null
              ? "h-full w-1/3 animate-pulse rounded-full bg-info"
              : "h-full rounded-full bg-info transition-[width] duration-200"
          }
          style={progress.percent === null ? undefined : { width: `${progress.percent}%` }}
        />
      </div>
      <p className="mt-1.5 font-mono text-[10px] text-muted-foreground">
        {progress.detail}
        {detail ? ` · ${detail}` : ""}
      </p>
    </div>
  );
}

export function DatabaseOperationsWorkspace({
  tab,
  profile,
  server,
  snapshot,
  connected,
  onConnect,
}: {
  tab: DatabaseWorkspaceTab;
  profile: DatabaseProfile;
  server: ServerProfile | null;
  snapshot: DatabaseConnectionSnapshot | undefined;
  connected: boolean;
  onConnect: () => void;
}) {
  useDatabaseDumpEvents();
  useDatabaseBackupEvents();
  useDatabaseCsvTransferEvents();

  const dumpTools = useDatabaseDumpStore((state) => state.tools);
  const dumpTransfer = useDatabaseDumpStore((state) => state.transfer);
  const detectDumpTools = useDatabaseDumpStore((state) => state.detectTools);
  const cancelDumpTransfer = useDatabaseDumpStore((state) => state.cancelTransfer);
  const backupTransfer = useDatabaseBackupStore((state) => state.transfer);
  const cancelBackupTransfer = useDatabaseBackupStore((state) => state.cancelTransfer);
  const csvTransfer = useDatabaseCsvStore((state) => state.transfer);
  const cancelCsvTransfer = useDatabaseCsvStore((state) => state.cancelTransfer);
  const tasks = useDatabaseWorkspaceStore(
    (state) => state.tabs[tab.profileId]?.tasks ?? EMPTY_TASKS,
  );
  const removeTask = useDatabaseWorkspaceStore((state) => state.removeTask);

  const [csvMode, setCsvMode] = useState<"export" | "import" | null>(null);
  const [dumpDialog, setDumpDialog] = useState<{
    mode: DatabaseDumpTransferMode;
    purpose: DatabaseDumpTransferPurpose;
  } | null>(null);
  const [tableState, setTableState] = useState<TableState>({ status: "idle" });
  const [columnState, setColumnState] = useState<ColumnState>({ status: "idle" });
  const [tableSelection, setTableSelection] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  const database = tab.selection?.database ?? snapshot?.databaseName ?? tab.databaseName ?? null;
  const schema = tab.selection?.schema ?? (tab.engine === "mysql" ? database : null);
  const readOnly = snapshot?.readOnly ?? profile.readOnly;

  const loadTables = useCallback(async () => {
    if (!connected || !database) {
      return;
    }
    await Promise.resolve();
    setTableState({ status: "loading" });
    try {
      const page = await listTables(profile.id, { database, schema }, { limit: 200, offset: 0 });
      setTableState({ status: "ready", tables: page.items });
    } catch (error) {
      setTableState({ status: "error", error: commandError(error) });
    }
  }, [connected, database, profile.id, schema]);

  useEffect(() => {
    const handle = window.setTimeout(() => {
      void loadTables();
    }, 0);
    return () => window.clearTimeout(handle);
  }, [loadTables, reloadToken]);

  const tableStateForRender = useMemo<TableState>(
    () => (!connected || !database ? { status: "idle" } : tableState),
    [connected, database, tableState],
  );
  const defaultTableName =
    tab.selection?.kind === "table"
      ? (tab.selection.objectName ?? tab.selection.label)
      : tableStateForRender.status === "ready"
        ? (tableStateForRender.tables[0]?.name ?? "")
        : "";
  const selectedTableName =
    tableSelection &&
    (tableStateForRender.status !== "ready" ||
      tableStateForRender.tables.some((table) => table.name === tableSelection))
      ? tableSelection
      : defaultTableName;

  const selectedTable = useMemo(
    () =>
      tableStateForRender.status === "ready"
        ? tableStateForRender.tables.find((table) => table.name === selectedTableName)
        : undefined,
    [selectedTableName, tableStateForRender],
  );

  useEffect(() => {
    if (!connected || !database || !selectedTableName) {
      return;
    }
    let cancelled = false;
    void (async () => {
      await Promise.resolve();
      if (cancelled) {
        return;
      }
      setColumnState({ status: "loading" });
      try {
        const page = await listColumns(
          profile.id,
          { database, schema, table: selectedTableName },
          { limit: 500, offset: 0 },
        );
        if (!cancelled) {
          setColumnState({ status: "ready", columns: page.items.map(columnFromSummary) });
        }
      } catch (error) {
        if (!cancelled) {
          setColumnState({ status: "error", error: commandError(error) });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [connected, database, profile.id, schema, selectedTableName]);

  const columnStateForRender: ColumnState =
    !connected || !database || !selectedTableName ? { status: "idle" } : columnState;

  useEffect(() => {
    if (connected) {
      void detectDumpTools(profile.id);
    }
  }, [connected, detectDumpTools, profile.id]);

  const activeTransfers = useMemo(() => {
    const entries: Array<{
      key: string;
      label: string;
      status: string;
      progress: { percent: number | null; detail: string };
      detail: string;
      cancelling: boolean;
      partial?: boolean;
      onCancel: () => void;
    }> = [];
    if (csvTransfer.profileId === profile.id && csvTransfer.status === "running") {
      const progress = csvTransfer.progress;
      const result = csvTransfer.exportResult ?? csvTransfer.importResult;
      entries.push({
        key: `csv-${csvTransfer.transferId ?? "active"}`,
        label: `CSV ${csvTransfer.direction}${selectedTableName ? ` · ${selectedTableName}` : ""}`,
        status: csvTransfer.status,
        progress: {
          percent: csvProgressPercent(progress),
          detail: progress
            ? `${csvPhaseLabel(progress.phase)} · ${formatCount(progress.rowsProcessed)} rows · ${formatBytes(progress.bytesProcessed)}`
            : "Preparing the transfer",
        },
        detail: result
          ? `${formatDurationMs(result.durationMs)}${result.mayBeIncomplete ? " · marked incomplete" : ""}`
          : "",
        cancelling: csvTransfer.cancelling,
        partial: result?.mayBeIncomplete,
        onCancel: () => void cancelCsvTransfer(),
      });
    }
    if (dumpTransfer.profileId === profile.id && dumpTransfer.status === "running") {
      const result = dumpTransfer.exportResult ?? dumpTransfer.restoreResult;
      entries.push({
        key: `dump-${dumpTransfer.transferId ?? "active"}`,
        label: `SQL dump ${dumpTransfer.direction === "export" ? "export" : "restore"}`,
        status: dumpTransfer.status,
        progress: {
          percent: dumpProgressPercent(dumpTransfer.progress),
          detail: dumpTransfer.progress
            ? `${dumpPhaseLabel(dumpTransfer.progress.phase)} · ${formatBytes(dumpTransfer.progress.bytesProcessed)}`
            : "Preparing the transfer",
        },
        detail: result
          ? `${formatDurationMs(result.durationMs)}${result.mayBeIncomplete ? " · marked incomplete" : ""}`
          : "",
        cancelling: dumpTransfer.cancelling,
        partial: result?.mayBeIncomplete,
        onCancel: () => void cancelDumpTransfer(),
      });
    }
    if (backupTransfer.profileId === profile.id && backupTransfer.status === "running") {
      const result = backupTransfer.backupResult ?? backupTransfer.restoreResult;
      entries.push({
        key: `backup-${backupTransfer.transferId ?? "active"}`,
        label: `Backup ${backupTransfer.direction === "restore" ? "restore" : "creation"}`,
        status: backupTransfer.status,
        progress: {
          percent: dumpProgressPercent(backupTransfer.progress),
          detail: backupTransfer.progress
            ? `${dumpPhaseLabel(backupTransfer.progress.phase)} · ${formatBytes(backupTransfer.progress.bytesProcessed)}`
            : "Preparing the backup",
        },
        detail: result
          ? `${formatDurationMs(result.durationMs)}${result.mayBeIncomplete ? " · marked incomplete" : ""}`
          : "",
        cancelling: backupTransfer.cancelling,
        partial: result?.mayBeIncomplete,
        onCancel: () => void cancelBackupTransfer(),
      });
    }
    return entries;
  }, [
    backupTransfer,
    cancelBackupTransfer,
    cancelCsvTransfer,
    cancelDumpTransfer,
    csvTransfer,
    dumpTransfer,
    profile.id,
    selectedTableName,
  ]);

  const tableLoadError = tableStateForRender.status === "error" ? tableStateForRender.error : null;
  const columnsReady =
    columnStateForRender.status === "ready" && columnStateForRender.columns.length > 0;
  const scopedTools = dumpTools.profileId === profile.id ? dumpTools : undefined;
  const toolsReport = scopedTools?.report;
  const toolsError = scopedTools?.status === "error" ? scopedTools : undefined;
  const toolStatus = toolsReport?.tools.find((tool) =>
    profile.engine === "mysql" ? tool.tool === "mysqldump" : tool.tool === "pg_dump",
  );
  const toolsReady = toolsReport
    ? toolStatus
      ? toolStatus.available && toolStatus.compatible
      : toolsReport.exportReady
    : false;

  function clearFinishedTasks() {
    tasks
      .filter((task) => task.status !== "running")
      .forEach((task) => removeTask(tab.profileId, task.id));
  }

  if (!connected) {
    return (
      <div className="space-y-3">
        <EmptyState
          icon={<PlugsIcon className="size-4" aria-hidden="true" />}
          title="Connection is not live"
          description={`${profile.name} must be connected before imports, exports, backups, or restores can start. Existing tasks stay attached to this connection tab.`}
          action={
            <Button size="sm" onClick={onConnect}>
              Connect {profile.name}
            </Button>
          }
        />
        <RecentTasks
          tasks={tasks}
          onDismiss={(taskId) => removeTask(tab.profileId, taskId)}
          onClearFinished={clearFinishedTasks}
        />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-start justify-between gap-2 rounded-md border border-border bg-card px-3 py-2">
        <div className="min-w-0">
          <p className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
            <DatabaseIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            {profile.name}
          </p>
          <p className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground">
            {server?.name ?? profile.serverId} · {tab.engine === "mysql" ? "MySQL" : "PostgreSQL"} ·{" "}
            {database ?? "no database selected"}
            {schema ? ` / ${schema}` : ""}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {readOnly ? <Badge variant="warning">Read-only</Badge> : null}
          <Badge variant={toolsReady ? "success" : "warning"}>
            {toolsReady ? "Dump tools ready" : "Dump tools unavailable"}
          </Badge>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => void detectDumpTools(profile.id)}
            disabled={scopedTools?.status === "loading"}
          >
            {scopedTools?.status === "loading" ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            Check tools
          </Button>
        </div>
      </div>

      {toolsError ? (
        <Alert variant={toolsError.error?.code === "permission" ? "warning" : "destructive"}>
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div className="flex-1">
            <AlertTitle>
              {toolsError.error?.code === "permission"
                ? "Permission denied while checking backup tools"
                : "Backup tool detection failed"}
            </AlertTitle>
            <AlertDescription>{toolsError.errorMessage}</AlertDescription>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="mt-2"
              onClick={() => void detectDumpTools(profile.id)}
            >
              Retry detection
            </Button>
          </div>
        </Alert>
      ) : null}

      {activeTransfers.length > 0 ? (
        <section aria-label="Active transfers" className="space-y-2">
          <h3 className="flex items-center gap-1.5 text-[10px] font-semibold tracking-wide text-muted-foreground uppercase">
            <HardDriveIcon className="size-3.5" aria-hidden="true" />
            Active transfers
          </h3>
          {activeTransfers.map((entry) => (
            <ActiveTransferCard
              key={entry.key}
              label={entry.label}
              status={entry.status}
              progress={entry.progress}
              detail={entry.detail}
              cancelling={entry.cancelling}
              partial={entry.partial}
              onCancel={entry.onCancel}
            />
          ))}
        </section>
      ) : null}

      <section aria-label="Transfer actions" className="space-y-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 className="text-[10px] font-semibold tracking-wide text-muted-foreground uppercase">
            Start a transfer
          </h3>
          <div className="flex items-center gap-2">
            <Label htmlFor="operations-table" className="text-[10px] text-muted-foreground">
              CSV table
            </Label>
            <select
              id="operations-table"
              aria-label="CSV table"
              value={selectedTableName}
              onChange={(event) => setTableSelection(event.target.value)}
              disabled={
                tableStateForRender.status !== "ready" || tableStateForRender.tables.length === 0
              }
              className="h-7 max-w-[15rem] rounded-md border border-input bg-card px-2 font-mono text-[10px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-60"
            >
              {tableStateForRender.status !== "ready" || tableStateForRender.tables.length === 0 ? (
                <option value="">
                  {tableStateForRender.status === "loading"
                    ? "Loading tables..."
                    : "No table selected"}
                </option>
              ) : (
                tableStateForRender.tables.map((table) => (
                  <option key={table.name} value={table.name}>
                    {table.name}
                  </option>
                ))
              )}
            </select>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              onClick={() => setReloadToken((value) => value + 1)}
              aria-label="Reload tables"
              title="Reload tables"
              disabled={tableStateForRender.status === "loading"}
            >
              {tableStateForRender.status === "loading" ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
              )}
            </Button>
          </div>
        </div>

        {!database ? (
          <Alert variant="warning">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div>
              <AlertTitle>Select a database first</AlertTitle>
              <AlertDescription>
                Choose a database or schema in the explorer before starting a table transfer. SQL
                dumps can still use the connection's active database.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        {tableLoadError ? (
          <ErrorState
            title={
              tableLoadError.code === "permission"
                ? "Permission denied listing tables"
                : "Tables could not be loaded"
            }
            message={tableLoadError.message}
            onRetry={() => setReloadToken((value) => value + 1)}
          />
        ) : null}

        {columnStateForRender.status === "error" ? (
          <Alert
            variant={columnStateForRender.error.code === "permission" ? "warning" : "destructive"}
          >
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <div>
              <AlertTitle>
                {columnStateForRender.error.code === "permission"
                  ? "Permission denied reading table columns"
                  : "Columns could not be loaded"}
              </AlertTitle>
              <AlertDescription>
                {columnStateForRender.error.message} CSV import is disabled until the column list
                loads.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
          <TransferActionCard
            icon={<FileCsvIcon className="size-4" aria-hidden="true" />}
            title="Export CSV"
            description={
              selectedTable
                ? `Stream ${selectedTable.name} to a local CSV file with full values.`
                : "Select a table to export its rows."
            }
            actionLabel="Export CSV"
            disabled={!selectedTable || tableStateForRender.status !== "ready"}
            onAction={() => setCsvMode("export")}
          />
          <TransferActionCard
            icon={<FileCsvIcon className="size-4" aria-hidden="true" />}
            title="Import CSV"
            description={
              readOnly
                ? "Read-only profiles cannot import rows."
                : selectedTable
                  ? `Preview, map, and import a CSV file into ${selectedTable.name}.`
                  : "Select a table to import rows."
            }
            actionLabel="Import CSV"
            disabled={
              !selectedTable || !columnsReady || readOnly || tableStateForRender.status !== "ready"
            }
            onAction={() => setCsvMode("import")}
            reason={
              readOnly
                ? "Read-only profile"
                : !selectedTable
                  ? "Select a table"
                  : !columnsReady
                    ? "Column metadata is not ready"
                    : undefined
            }
          />
          <TransferActionCard
            icon={<ArchiveBoxIcon className="size-4" aria-hidden="true" />}
            title="Export SQL dump"
            description="Run mysqldump, pg_dump, or pg_dumpall on the server and verify the output file."
            actionLabel="Export dump"
            disabled={!database}
            onAction={() => setDumpDialog({ mode: "export", purpose: "dump" })}
            reason={!database ? "Select a database" : undefined}
          />
          <TransferActionCard
            icon={<ArchiveBoxIcon className="size-4" aria-hidden="true" />}
            title="Restore SQL dump"
            description="Preview the target, then restore a remote or local dump with an explicit confirmation."
            actionLabel="Restore dump"
            disabled={!database || readOnly}
            onAction={() => setDumpDialog({ mode: "restore", purpose: "dump" })}
            reason={readOnly ? "Read-only profile" : !database ? "Select a database" : undefined}
          />
          <TransferActionCard
            icon={<HardDriveIcon className="size-4" aria-hidden="true" />}
            title="Create backup"
            description="Write a verified backup to a remote path or download it to this desktop."
            actionLabel="Create backup"
            disabled={!database}
            onAction={() => setDumpDialog({ mode: "export", purpose: "backup" })}
            reason={!database ? "Select a database" : undefined}
          />
          <TransferActionCard
            icon={<HardDriveIcon className="size-4" aria-hidden="true" />}
            title="Restore backup"
            description="Restore a backup after reviewing the server, target database, and replaced data."
            actionLabel="Restore backup"
            disabled={!database || readOnly}
            onAction={() => setDumpDialog({ mode: "restore", purpose: "backup" })}
            reason={readOnly ? "Read-only profile" : !database ? "Select a database" : undefined}
          />
        </div>
      </section>

      {activeTransfers.length === 0 ? (
        <div className="flex items-start gap-2 rounded-md border border-border bg-muted/15 px-3 py-2">
          <InfoIcon className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <p className="text-[10px] leading-4 text-muted-foreground">
            Transfers run in the background and stay attached to {profile.name}. Progress, cancel,
            warnings, cleanup results, and partial state remain available here after a dialog
            closes.
          </p>
        </div>
      ) : null}

      <RecentTasks
        tasks={tasks}
        onDismiss={(taskId) => removeTask(tab.profileId, taskId)}
        onClearFinished={clearFinishedTasks}
      />

      {csvMode && selectedTable ? (
        <DatabaseCsvTransferDialog
          key={csvMode}
          open
          mode={csvMode}
          onOpenChange={(open) => {
            if (!open) {
              setCsvMode(null);
            }
          }}
          profileId={profile.id}
          profileName={profile.name}
          serverName={server?.name ?? profile.serverId}
          table={{ database, schema, name: selectedTable.name }}
          columns={columnStateForRender.status === "ready" ? columnStateForRender.columns : []}
          readOnly={readOnly}
          onImported={() => setReloadToken((value) => value + 1)}
        />
      ) : null}

      {dumpDialog ? (
        <DatabaseDumpTransferDialog
          key={`${dumpDialog.mode}-${dumpDialog.purpose}`}
          open
          mode={dumpDialog.mode}
          purpose={dumpDialog.purpose}
          onOpenChange={(open) => {
            if (!open) {
              setDumpDialog(null);
            }
          }}
          profile={profile}
          server={server}
          database={database}
          schema={schema}
          selectedTable={selectedTableName || null}
          connected={connected}
          onConnect={onConnect}
          onCompleted={() => setReloadToken((value) => value + 1)}
        />
      ) : null}
    </div>
  );
}

function TransferActionCard({
  icon,
  title,
  description,
  actionLabel,
  disabled,
  onAction,
  reason,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  actionLabel: string;
  disabled?: boolean;
  onAction: () => void;
  reason?: string;
}) {
  return (
    <div className="flex min-w-0 flex-col justify-between gap-2 rounded-md border border-border bg-card px-3 py-2.5">
      <div className="min-w-0">
        <p className="flex items-center gap-1.5 text-[11px] font-medium text-foreground">
          <span className="text-muted-foreground">{icon}</span>
          {title}
        </p>
        <p className="mt-1 text-[10px] leading-4 text-muted-foreground">{description}</p>
        {disabled && reason ? (
          <p className="mt-1 text-[9px] font-medium text-warning">{reason}</p>
        ) : null}
      </div>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        className="w-full justify-center"
        onClick={onAction}
        disabled={disabled}
      >
        {actionLabel}
      </Button>
    </div>
  );
}

function RecentTasks({
  tasks,
  onDismiss,
  onClearFinished,
}: {
  tasks: DatabaseWorkspaceTask[];
  onDismiss: (taskId: string) => void;
  onClearFinished: () => void;
}) {
  const finished = tasks.filter((task) => task.status !== "running").length;
  return (
    <section
      aria-label="Recent operations"
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-3 py-2">
        <div className="flex items-center gap-2">
          <RowsIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-[12px] font-semibold text-foreground">Recent operations</h3>
          <Badge variant="outline">{tasks.length}</Badge>
        </div>
        {finished > 0 ? (
          <Button type="button" variant="ghost" size="sm" onClick={onClearFinished}>
            Clear finished
          </Button>
        ) : null}
      </div>
      {tasks.length === 0 ? (
        <p className="px-3 py-3 text-[11px] leading-5 text-muted-foreground">
          No imports, exports, backups, or restores have run for this connection. Finished
          operations stay here until this tab closes or you clear them.
        </p>
      ) : (
        <ul className="divide-y divide-border">
          {[...tasks].reverse().map((task) => (
            <li
              key={task.id}
              className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
            >
              <span className="min-w-0">
                <span className="flex flex-wrap items-center gap-1.5">
                  <span className="truncate text-[11px] font-medium text-foreground">
                    {task.label}
                  </span>
                  <Badge variant={taskVariant(task.status)}>{task.status}</Badge>
                  {task.partial ? <Badge variant="warning">Incomplete</Badge> : null}
                </span>
                {task.detail ? (
                  <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground">
                    {task.detail}
                  </span>
                ) : null}
              </span>
              <span className="flex items-center gap-2">
                {task.status !== "running" ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => onDismiss(task.id)}
                    aria-label={`Dismiss ${task.label}`}
                    title="Dismiss this operation record"
                  >
                    <XIcon className="size-3" aria-hidden="true" />
                  </Button>
                ) : null}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
