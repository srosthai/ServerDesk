import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArchiveBoxIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  DatabaseIcon,
  FolderOpenIcon,
  HardDriveIcon,
  InfoIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  DATABASE_DUMP_FORMAT_LABELS,
  dumpExportTaskDetail,
  dumpPhaseLabel,
  dumpProgressPercent,
  dumpRestoreTaskDetail,
  dumpScopeLabel,
  formatBytes,
  formatDurationMs,
  isRestoreRetrySafe,
  restoreConfirmationDetails,
  taskStatusForTransfer,
  toolAvailabilityLabel,
} from "@/features/databases/databaseTransferModel";
import { TransferSteps } from "@/features/databases/DatabaseTransferSteps";
import {
  beginOperationTask,
  updateOperationTask,
} from "@/features/databases/databaseOperationTasks";
import { useDatabaseBackupEvents } from "@/hooks/useDatabaseBackupEvents";
import { useDatabaseDumpEvents } from "@/hooks/useDatabaseDumpEvents";
import { pickLocalFile, pickLocalSavePath } from "@/lib/fileDialog";
import {
  useDatabaseBackupStore,
  type DatabaseBackupPreviewState,
  type DatabaseBackupToolsState,
  type DatabaseBackupTransferState,
} from "@/stores/databaseBackupStore";
import {
  useDatabaseDumpStore,
  type DatabaseDumpToolsState,
  type DatabaseDumpTransferState,
} from "@/stores/databaseDumpStore";
import type {
  DatabaseDumpExportRequest,
  DatabaseDumpExportResult,
  DatabaseDumpFormat,
  DatabaseDumpRestorePreviewRequest,
  DatabaseDumpRestoreRequest,
  DatabaseDumpRestoreResult,
  DatabaseDumpScope,
  DatabaseDumpTool,
  DatabaseEngine,
  DatabaseProfile,
  ServerProfile,
} from "@/types/contracts";

export type DatabaseDumpTransferMode = "export" | "restore";
export type DatabaseDumpTransferPurpose = "dump" | "backup";

type ScopeKind = "database" | "schema" | "tables" | "cluster";

function engineLabel(engine: DatabaseEngine): string {
  return engine === "mysql" ? "MySQL" : "PostgreSQL";
}

function expectedExportTool(engine: DatabaseEngine, scope: DatabaseDumpScope): DatabaseDumpTool {
  if (engine === "mysql") {
    return "mysqldump";
  }
  if (scope.kind === "cluster") {
    return "pg_dumpall";
  }
  return "pg_dump";
}

function expectedRestoreTool(engine: DatabaseEngine, format: DatabaseDumpFormat): DatabaseDumpTool {
  if (engine === "mysql") {
    return "mysql";
  }
  return format === "plain_sql" ? "psql" : "pg_restore";
}

function defaultExtension(format: DatabaseDumpFormat): string {
  if (format === "custom") {
    return "dump";
  }
  if (format === "tar") {
    return "tar";
  }
  return "sql";
}

function defaultRemoteOutputPath(
  server: ServerProfile | null,
  database: string | null,
  format: DatabaseDumpFormat,
): string {
  const name = (database ?? "database").replace(/[^a-zA-Z0-9._-]+/g, "_");
  const owner = (server?.username ?? "user").replace(/[^a-zA-Z0-9._-]+/g, "_");
  const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
  return `/home/${owner}/${name}-${stamp}.${defaultExtension(format)}`;
}

function splitTables(value: string): string[] {
  return Array.from(
    new Set(
      value
        .split(/[\n,]+/)
        .map((table) => table.trim())
        .filter(Boolean),
    ),
  );
}

function resultVariant(status: string): "success" | "warning" | "destructive" {
  if (status === "completed") {
    return "success";
  }
  return status === "cancelled" ? "warning" : "destructive";
}

export function DatabaseDumpTransferDialog({
  open,
  mode,
  purpose,
  onOpenChange,
  profile,
  server,
  database,
  schema,
  selectedTable,
  connected,
  onConnect,
  onCompleted,
}: {
  open: boolean;
  mode: DatabaseDumpTransferMode;
  purpose: DatabaseDumpTransferPurpose;
  onOpenChange: (open: boolean) => void;
  profile: DatabaseProfile;
  server: ServerProfile | null;
  database: string | null;
  schema: string | null;
  selectedTable: string | null;
  connected: boolean;
  onConnect: () => void;
  onCompleted?: (result: DatabaseDumpExportResult | DatabaseDumpRestoreResult) => void;
}) {
  const isBackup = purpose === "backup";
  const rawDumpTools = useDatabaseDumpStore((state) => state.tools);
  const rawDumpTransfer = useDatabaseDumpStore((state) => state.transfer);
  const detectDumpTools = useDatabaseDumpStore((state) => state.detectTools);
  const startDumpExport = useDatabaseDumpStore((state) => state.startExport);
  const startDumpRestore = useDatabaseDumpStore((state) => state.startRestore);
  const cancelDumpTransfer = useDatabaseDumpStore((state) => state.cancelTransfer);
  const dismissDumpTransfer = useDatabaseDumpStore((state) => state.dismissTransfer);
  const clearDumpTools = useDatabaseDumpStore((state) => state.clearTools);

  const rawBackupTools = useDatabaseBackupStore((state) => state.tools);
  const backupPreview = useDatabaseBackupStore((state) => state.preview);
  const rawBackupTransfer = useDatabaseBackupStore((state) => state.transfer);
  const detectBackupTools = useDatabaseBackupStore((state) => state.detectTools);
  const previewRestore = useDatabaseBackupStore((state) => state.previewRestore);
  const startBackup = useDatabaseBackupStore((state) => state.startBackup);
  const startBackupRestore = useDatabaseBackupStore((state) => state.startRestore);
  const cancelBackupTransfer = useDatabaseBackupStore((state) => state.cancelTransfer);
  const dismissBackupTransfer = useDatabaseBackupStore((state) => state.dismissTransfer);
  const clearBackupPreview = useDatabaseBackupStore((state) => state.clearPreview);

  useDatabaseDumpEvents();
  useDatabaseBackupEvents();

  const rawTools = isBackup ? rawBackupTools : rawDumpTools;
  const tools: DatabaseDumpToolsState | DatabaseBackupToolsState =
    rawTools.profileId && rawTools.profileId !== profile.id ? { status: "idle" } : rawTools;
  const rawTransfer = isBackup ? rawBackupTransfer : rawDumpTransfer;
  const busyElsewhere =
    rawTransfer.status === "running" &&
    rawTransfer.profileId !== undefined &&
    rawTransfer.profileId !== profile.id;
  const transfer: DatabaseDumpTransferState | DatabaseBackupTransferState = busyElsewhere
    ? isBackup
      ? {
          direction: mode === "export" ? "backup" : "restore",
          status: "idle",
          cancelling: false,
        }
      : {
          direction: mode === "export" ? "export" : "restore",
          status: "idle",
          cancelling: false,
        }
    : rawTransfer;
  const preview: DatabaseBackupPreviewState =
    backupPreview.profileId && backupPreview.profileId !== profile.id
      ? { status: "idle" }
      : backupPreview;
  const detectTools = isBackup ? detectBackupTools : detectDumpTools;
  const cancelTransfer = isBackup ? cancelBackupTransfer : cancelDumpTransfer;
  const running = transfer.status === "running";

  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [scopeKind, setScopeKind] = useState<ScopeKind>("database");
  const [format, setFormat] = useState<DatabaseDumpFormat>("plain_sql");
  const [destinationKind, setDestinationKind] = useState<"remote" | "local">("remote");
  const [path, setPath] = useState(() =>
    mode === "export" ? defaultRemoteOutputPath(server, database, "plain_sql") : "",
  );
  const [overwrite, setOverwrite] = useState(false);
  const [noOwner, setNoOwner] = useState(false);
  const [tablesInput, setTablesInput] = useState(selectedTable ?? "");
  const [schemaName, setSchemaName] = useState(
    schema ?? (profile.engine === "mysql" ? (database ?? "") : "public"),
  );
  const [sourceKind, setSourceKind] = useState<"remote" | "local">("remote");
  const [sourcePath, setSourcePath] = useState("");
  const [targetDatabase, setTargetDatabase] = useState(database ?? profile.databaseName ?? "");
  const [clean, setClean] = useState(false);
  const [confirmRestore, setConfirmRestore] = useState(false);
  const [confirmOverwrite, setConfirmOverwrite] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const taskIdRef = useRef<string | null>(null);

  useEffect(() => {
    clearBackupPreview();
    if (isBackup) {
      dismissBackupTransfer();
    } else {
      dismissDumpTransfer();
      clearDumpTools();
    }
    if (connected) {
      void detectTools(profile.id);
    }
    // The dialog is mounted for one open; reset only on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const tables = useMemo(() => splitTables(tablesInput), [tablesInput]);

  const scope = useMemo<DatabaseDumpScope>(() => {
    const scopedDatabase = database ?? profile.databaseName ?? null;
    switch (scopeKind) {
      case "cluster":
        return { kind: "cluster" };
      case "schema":
        return {
          kind: "schema",
          database: scopedDatabase,
          schema: schemaName.trim() || "public",
        };
      case "tables":
        return {
          kind: "tables",
          database: scopedDatabase,
          schema: profile.engine === "mysql" ? scopedDatabase : schemaName.trim() || null,
          tables,
        };
      default:
        return { kind: "database", database: scopedDatabase };
    }
  }, [database, profile.databaseName, profile.engine, schemaName, scopeKind, tables]);

  const effectiveFormat: DatabaseDumpFormat =
    profile.engine === "mysql" || scopeKind === "cluster" ? "plain_sql" : format;

  const exportTool = expectedExportTool(profile.engine, scope);
  const restoreTool = expectedRestoreTool(profile.engine, effectiveFormat);
  const restoreClean =
    profile.engine === "postgresql" && effectiveFormat !== "plain_sql" ? clean : false;
  const restoreNoOwner =
    profile.engine === "postgresql" && effectiveFormat !== "plain_sql" ? noOwner : false;
  const restorePlanCurrent = preview.preview
    ? preview.preview.targetDatabase === targetDatabase.trim() &&
      preview.preview.sourceKind === sourceKind &&
      preview.preview.sourcePath === sourcePath.trim() &&
      preview.preview.format === effectiveFormat &&
      preview.preview.clean === restoreClean &&
      preview.preview.noOwner === restoreNoOwner
    : false;
  const report = tools.report;
  const expectedToolStatus = report?.tools.find(
    (tool) => tool.tool === (mode === "export" ? exportTool : restoreTool),
  );
  const toolReady = expectedToolStatus
    ? expectedToolStatus.available && expectedToolStatus.compatible
    : mode === "export"
      ? report?.exportReady
      : report?.restoreReady;
  const scopeLabel = dumpScopeLabel(scope, profile.engine, database);
  const percent = dumpProgressPercent(transfer.progress);
  const elapsedMs =
    transfer.progress && transfer.startedAt
      ? Math.max(0, transfer.progress.emittedAt - transfer.startedAt)
      : undefined;

  const exportResult = isBackup ? rawBackupTransfer.backupResult : rawDumpTransfer.exportResult;
  const restoreResult = isBackup ? rawBackupTransfer.restoreResult : rawDumpTransfer.restoreResult;
  const result = mode === "export" ? exportResult : restoreResult;

  const stepLabels =
    mode === "export"
      ? ["Scope & output", "Review", "Transfer"]
      : ["Source & target", "Confirm", "Transfer"];
  const stepIndex = step - 1;

  useEffect(() => {
    const taskId = taskIdRef.current;
    if (!taskId || transfer.transferId !== taskId || transfer.profileId !== profile.id) {
      return;
    }
    const isExport = transfer.direction === "export" || transfer.direction === "backup";
    const taskResult = isExport ? exportResult : restoreResult;
    updateOperationTask({
      profileId: profile.id,
      id: taskId,
      status: taskStatusForTransfer(transfer.status),
      detail: isExport
        ? dumpExportTaskDetail(transfer.status, transfer.progress, exportResult)
        : dumpRestoreTaskDetail(transfer.status, transfer.progress, restoreResult),
      partial: taskResult?.mayBeIncomplete ?? false,
    });
  }, [
    exportResult,
    profile.id,
    restoreResult,
    transfer.direction,
    transfer.profileId,
    transfer.progress,
    transfer.status,
    transfer.transferId,
  ]);

  async function browseDestination() {
    const selected = await pickLocalSavePath({
      title: isBackup ? "Choose a backup destination" : "Choose a dump destination",
      defaultPath: `${database ?? "database"}.${defaultExtension(effectiveFormat)}`,
      filters: [{ name: "Database dump", extensions: ["sql", "dump", "tar", "gz"] }],
    });
    if (selected) {
      setPath(selected);
    }
  }

  async function browseSource() {
    const selected = await pickLocalFile({
      title: "Choose a dump file to restore",
      filters: [{ name: "Database dump", extensions: ["sql", "dump", "tar", "gz"] }],
    });
    if (selected) {
      setSourcePath(selected);
    }
  }

  function exportRequest(): DatabaseDumpExportRequest {
    return {
      scope,
      format: effectiveFormat,
      tool: null,
      destination:
        destinationKind === "remote"
          ? { kind: "remote", path: path.trim(), overwrite }
          : { kind: "local", path: path.trim(), overwrite },
      noOwner: profile.engine === "postgresql" && scopeKind !== "cluster" ? noOwner : false,
    };
  }

  function restorePreviewRequest(): DatabaseDumpRestorePreviewRequest {
    return {
      source:
        sourceKind === "remote"
          ? { kind: "remote", path: sourcePath.trim() }
          : { kind: "local", path: sourcePath.trim() },
      format: effectiveFormat,
      tool: null,
      targetDatabase: targetDatabase.trim(),
      clean: restoreClean,
      noOwner: restoreNoOwner,
    };
  }

  async function startExport(forceOverwrite: boolean) {
    if (busyElsewhere) {
      return null;
    }
    setSubmitted(true);
    setStep(3);
    const request = exportRequest();
    if (forceOverwrite) {
      request.destination = { ...request.destination, overwrite: true };
    }
    const promise = isBackup
      ? startBackup(profile.id, request)
      : startDumpExport(profile.id, request);
    const transferId = isBackup
      ? useDatabaseBackupStore.getState().transfer.transferId
      : useDatabaseDumpStore.getState().transfer.transferId;
    if (transferId) {
      taskIdRef.current = transferId;
      beginOperationTask({
        profileId: profile.id,
        id: transferId,
        label: `${isBackup ? "Backup" : "SQL dump export"} · ${scopeLabel}`,
        detail: "Preparing the dump",
      });
    }
    const completed = await promise;
    if (completed?.status === "completed") {
      onCompleted?.(completed);
    }
    return completed;
  }

  async function handleExport() {
    if (!path.trim() || busyElsewhere) {
      return;
    }
    if (overwrite) {
      setConfirmOverwrite(true);
      return;
    }
    await startExport(false);
  }

  async function handlePreviewRestore() {
    if (!sourcePath.trim() || !targetDatabase.trim() || busyElsewhere) {
      return;
    }
    setStep(2);
    await previewRestore(profile.id, restorePreviewRequest());
  }

  async function runRestore() {
    const plan = preview.preview;
    if (!plan || !restorePlanCurrent || busyElsewhere) {
      return;
    }
    setConfirmRestore(false);
    setSubmitted(true);
    setStep(3);
    const request: DatabaseDumpRestoreRequest = {
      ...restorePreviewRequest(),
      confirmed: true,
      confirmation: { ...plan.confirmation, acknowledged: true },
    };
    const promise = isBackup
      ? startBackupRestore(profile.id, request)
      : startDumpRestore(profile.id, request);
    const transferId = isBackup
      ? useDatabaseBackupStore.getState().transfer.transferId
      : useDatabaseDumpStore.getState().transfer.transferId;
    if (transferId) {
      taskIdRef.current = transferId;
      beginOperationTask({
        profileId: profile.id,
        id: transferId,
        label: `${isBackup ? "Restore" : "SQL dump restore"} · ${plan.targetDatabase}`,
        detail: "Preparing the restore",
      });
    }
    const completed = await promise;
    if (completed?.status === "completed") {
      onCompleted?.(completed);
    }
    return completed;
  }

  function retry() {
    if (mode === "export") {
      void startExport(overwrite);
    } else if (preview.preview) {
      void runRestore();
    } else {
      void handlePreviewRestore();
    }
  }

  const title =
    mode === "export"
      ? isBackup
        ? "Create database backup"
        : "Export SQL dump"
      : isBackup
        ? "Restore database backup"
        : "Restore SQL dump";
  const description =
    mode === "export"
      ? `Write a verified ${engineLabel(profile.engine)} dump to a remote path or a local file. Tool availability and version compatibility are checked before the transfer starts.`
      : `Restore a ${engineLabel(profile.engine)} dump into a target database. The preview names the server, database, and data that may be replaced.`;

  const scopeOptions: Array<{ value: ScopeKind; label: string; hint: string }> =
    profile.engine === "mysql"
      ? [
          { value: "database", label: "Database", hint: "Every table in one database" },
          { value: "tables", label: "Selected tables", hint: "Up to 100 tables" },
        ]
      : [
          { value: "database", label: "Database", hint: "Every object in one database" },
          { value: "schema", label: "Schema", hint: "One PostgreSQL schema" },
          { value: "tables", label: "Selected tables", hint: "Up to 100 tables" },
          { value: "cluster", label: "Entire cluster", hint: "Every database and role" },
        ];

  const reviewDetails =
    mode === "export"
      ? [
          {
            label: "Server",
            value: server ? `${server.name} (${server.host})` : "the selected server",
          },
          { label: "Connection", value: profile.name },
          { label: "Engine", value: engineLabel(profile.engine) },
          { label: "Scope", value: scopeLabel },
          { label: "Format", value: DATABASE_DUMP_FORMAT_LABELS[effectiveFormat] },
          {
            label: "Output",
            value: `${destinationKind === "remote" ? "Remote server" : "Local desktop"} · ${path.trim() || "not selected"}`,
          },
          { label: "Tool", value: exportTool },
          {
            label: "Existing output",
            value: overwrite ? "Replace after confirmation" : "Reject if it exists",
          },
        ]
      : [
          {
            label: "Server",
            value: preview.preview
              ? `${preview.preview.serverName} (${preview.preview.serverId})`
              : server
                ? `${server.name} (${server.host})`
                : "the selected server",
          },
          { label: "Connection", value: profile.name },
          { label: "Target database", value: targetDatabase.trim() || "not selected" },
          {
            label: "Source",
            value: `${sourceKind === "remote" ? "Remote server" : "Local desktop"} · ${sourcePath.trim() || "not selected"}`,
          },
          { label: "Format", value: DATABASE_DUMP_FORMAT_LABELS[effectiveFormat] },
          { label: "Tool", value: preview.preview?.tool ?? restoreTool },
          {
            label: "Replaced data",
            value:
              preview.preview?.replacedData ??
              "Preview the restore to see exactly what may be replaced.",
          },
        ];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100vh-2rem)] max-w-3xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ArchiveBoxIcon className="size-4 text-primary" aria-hidden="true" />
            {title}
          </DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        <TransferSteps steps={stepLabels} activeIndex={stepIndex} label={`${title} steps`} />

        {!connected ? (
          <Alert variant="warning">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div className="flex-1">
              <AlertTitle>Connection is not live</AlertTitle>
              <AlertDescription>
                {profile.name} must be connected before tool detection or a transfer can start.
                Reconnect the profile and try again.
              </AlertDescription>
            </div>
            <Button variant="secondary" size="sm" onClick={onConnect}>
              Connect
            </Button>
          </Alert>
        ) : null}

        {busyElsewhere ? (
          <Alert variant="warning">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div>
              <AlertTitle>Another connection is transferring</AlertTitle>
              <AlertDescription>
                Only one dump or backup transfer runs at a time in this window. Wait for the active
                transfer to finish or cancel it from its connection tab.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border bg-muted/15 px-3 py-2">
          <div className="min-w-0">
            <p className="flex items-center gap-1.5 text-[11px] font-medium text-foreground">
              <HardDriveIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
              Backup tools on {server?.name ?? "the server"}
            </p>
            <p className="mt-0.5 font-mono text-[10px] text-muted-foreground">
              {report
                ? `${engineLabel(report.engine)} server ${report.serverVersion} · ${expectedToolStatus?.binary ?? (mode === "export" ? exportTool : restoreTool)} ${toolAvailabilityLabel(expectedToolStatus ?? { available: false, compatible: false })}`
                : tools.status === "loading"
                  ? "Checking remote tools..."
                  : "Tool readiness has not been checked."}
            </p>
          </div>
          <div className="flex items-center gap-2">
            {report ? (
              <Badge variant={toolReady ? "success" : "warning"}>
                {toolReady ? "Ready" : "Tool unavailable"}
              </Badge>
            ) : null}
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void detectTools(profile.id)}
              disabled={tools.status === "loading" || !connected}
            >
              {tools.status === "loading" ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <ArchiveBoxIcon className="size-3.5" aria-hidden="true" />
              )}
              Check tools
            </Button>
          </div>
        </div>

        {tools.status === "error" ? (
          <Alert variant={tools.error?.code === "permission" ? "warning" : "destructive"}>
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <div className="flex-1">
              <AlertTitle>
                {tools.error?.code === "permission"
                  ? "Permission denied while checking tools"
                  : "Tool detection failed"}
              </AlertTitle>
              <AlertDescription>{tools.errorMessage}</AlertDescription>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                className="mt-2"
                onClick={() => void detectTools(profile.id)}
              >
                Retry detection
              </Button>
            </div>
          </Alert>
        ) : null}

        {report && !toolReady ? (
          <Alert variant="warning">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div>
              <AlertTitle>
                {mode === "export" ? "Export tool unavailable" : "Restore tool unavailable"}
              </AlertTitle>
              <AlertDescription>
                {expectedToolStatus
                  ? `${expectedToolStatus.binary} is ${expectedToolStatus.available ? "not compatible with the server version" : "not installed on the server"}. Install or upgrade the ${engineLabel(profile.engine)} client tools before starting this transfer.`
                  : `${mode === "export" ? exportTool : restoreTool} was not reported by the server. Check the remote client tools before starting.`}
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        {step === 1 && mode === "export" ? (
          <div className="space-y-3">
            <div className="space-y-1.5">
              <Label>Scope</Label>
              <div className="grid gap-2 sm:grid-cols-2">
                {scopeOptions.map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    aria-pressed={scopeKind === option.value}
                    onClick={() => setScopeKind(option.value)}
                    className={
                      scopeKind === option.value
                        ? "rounded-md border border-primary/50 bg-primary/8 px-3 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                        : "rounded-md border border-border bg-card px-3 py-2 text-left outline-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring/40"
                    }
                  >
                    <span className="block text-[11px] font-medium text-foreground">
                      {option.label}
                    </span>
                    <span className="mt-0.5 block text-[10px] text-muted-foreground">
                      {option.hint}
                    </span>
                  </button>
                ))}
              </div>
            </div>

            {scopeKind === "schema" ? (
              <div className="space-y-1.5">
                <Label htmlFor="dump-schema">Schema</Label>
                <Input
                  id="dump-schema"
                  value={schemaName}
                  onChange={(event) => setSchemaName(event.target.value)}
                  className="h-8 bg-card font-mono text-[11px]"
                  placeholder="public"
                />
              </div>
            ) : null}

            {scopeKind === "tables" ? (
              <div className="space-y-1.5">
                <Label htmlFor="dump-tables">Tables</Label>
                <Input
                  id="dump-tables"
                  value={tablesInput}
                  onChange={(event) => setTablesInput(event.target.value)}
                  className="h-8 bg-card font-mono text-[11px]"
                  placeholder="users, orders"
                />
                <p className="text-[10px] text-muted-foreground">
                  Separate table names with commas or new lines. {tables.length.toLocaleString()}{" "}
                  selected; the backend accepts up to 100.
                </p>
              </div>
            ) : null}

            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label>Format</Label>
                <div className="flex flex-wrap gap-1.5">
                  {(profile.engine === "mysql"
                    ? (["plain_sql"] as DatabaseDumpFormat[])
                    : scopeKind === "cluster"
                      ? (["plain_sql"] as DatabaseDumpFormat[])
                      : (["plain_sql", "custom", "tar"] as DatabaseDumpFormat[])
                  ).map((option) => (
                    <button
                      key={option}
                      type="button"
                      aria-pressed={effectiveFormat === option}
                      onClick={() => setFormat(option)}
                      className={
                        effectiveFormat === option
                          ? "rounded-md border border-primary/50 bg-primary/8 px-2.5 py-1.5 text-[11px] font-medium text-foreground"
                          : "rounded-md border border-border bg-card px-2.5 py-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                      }
                    >
                      {DATABASE_DUMP_FORMAT_LABELS[option]}
                    </button>
                  ))}
                </div>
                <p className="text-[10px] text-muted-foreground">
                  {profile.engine === "mysql"
                    ? "MySQL dumps use plain SQL only."
                    : scopeKind === "cluster"
                      ? "Cluster dumps use plain SQL only."
                      : "Custom and tar archives require pg_dump and pg_restore."}
                </p>
              </div>
              <div className="space-y-1.5">
                <Label>Output location</Label>
                <div className="flex gap-1.5">
                  <button
                    type="button"
                    aria-pressed={destinationKind === "remote"}
                    onClick={() => setDestinationKind("remote")}
                    className={
                      destinationKind === "remote"
                        ? "flex-1 rounded-md border border-primary/50 bg-primary/8 px-2.5 py-1.5 text-[11px] font-medium text-foreground"
                        : "flex-1 rounded-md border border-border bg-card px-2.5 py-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                    }
                  >
                    Remote server
                  </button>
                  <button
                    type="button"
                    aria-pressed={destinationKind === "local"}
                    onClick={() => setDestinationKind("local")}
                    className={
                      destinationKind === "local"
                        ? "flex-1 rounded-md border border-primary/50 bg-primary/8 px-2.5 py-1.5 text-[11px] font-medium text-foreground"
                        : "flex-1 rounded-md border border-border bg-card px-2.5 py-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                    }
                  >
                    Local desktop
                  </button>
                </div>
                <p className="text-[10px] text-muted-foreground">
                  {destinationKind === "remote"
                    ? "The dump stays on the server. The path must be absolute."
                    : "The dump is downloaded to this computer. The path must be absolute."}
                </p>
              </div>
            </div>

            <div className="space-y-1.5">
              <Label htmlFor="dump-destination">Output file (absolute path)</Label>
              <div className="flex gap-2">
                <Input
                  id="dump-destination"
                  value={path}
                  onChange={(event) => setPath(event.target.value)}
                  placeholder={
                    destinationKind === "remote"
                      ? `/home/deploy/backups/app.${defaultExtension(effectiveFormat)}`
                      : `/home/user/backups/app.${defaultExtension(effectiveFormat)}`
                  }
                  className="h-8 bg-card font-mono text-[11px]"
                  disabled={running}
                />
                {destinationKind === "local" ? (
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    className="h-8 shrink-0"
                    onClick={() => void browseDestination()}
                    disabled={running}
                    aria-label="Choose local dump destination"
                  >
                    <FolderOpenIcon className="size-3.5" aria-hidden="true" />
                    Browse
                  </Button>
                ) : null}
              </div>
            </div>

            <div className="flex flex-wrap items-center gap-4 rounded-md border border-border bg-muted/15 px-3 py-2">
              <label className="flex items-center gap-2 text-[11px] text-foreground">
                <input
                  type="checkbox"
                  checked={overwrite}
                  onChange={(event) => setOverwrite(event.target.checked)}
                  className="size-3.5 accent-primary"
                />
                Replace an existing output
              </label>
              <label
                className="flex items-center gap-2 text-[11px] text-foreground"
                title={
                  profile.engine === "mysql"
                    ? "MySQL dumps do not support the no-owner option."
                    : scopeKind === "cluster"
                      ? "Cluster dumps do not support the no-owner option."
                      : "Omit ownership commands from the dump"
                }
              >
                <input
                  type="checkbox"
                  checked={noOwner}
                  onChange={(event) => setNoOwner(event.target.checked)}
                  disabled={profile.engine === "mysql" || scopeKind === "cluster"}
                  className="size-3.5 accent-primary"
                />
                Omit object ownership
              </label>
            </div>
          </div>
        ) : null}

        {step === 1 && mode === "restore" ? (
          <div className="space-y-3">
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label>Source location</Label>
                <div className="flex gap-1.5">
                  <button
                    type="button"
                    aria-pressed={sourceKind === "remote"}
                    onClick={() => setSourceKind("remote")}
                    className={
                      sourceKind === "remote"
                        ? "flex-1 rounded-md border border-primary/50 bg-primary/8 px-2.5 py-1.5 text-[11px] font-medium text-foreground"
                        : "flex-1 rounded-md border border-border bg-card px-2.5 py-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                    }
                  >
                    Remote server
                  </button>
                  <button
                    type="button"
                    aria-pressed={sourceKind === "local"}
                    onClick={() => setSourceKind("local")}
                    className={
                      sourceKind === "local"
                        ? "flex-1 rounded-md border border-primary/50 bg-primary/8 px-2.5 py-1.5 text-[11px] font-medium text-foreground"
                        : "flex-1 rounded-md border border-border bg-card px-2.5 py-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                    }
                  >
                    Local desktop
                  </button>
                </div>
              </div>
              <div className="space-y-1.5">
                <Label>Format</Label>
                <div className="flex flex-wrap gap-1.5">
                  {(profile.engine === "mysql"
                    ? (["plain_sql"] as DatabaseDumpFormat[])
                    : (["plain_sql", "custom", "tar"] as DatabaseDumpFormat[])
                  ).map((option) => (
                    <button
                      key={option}
                      type="button"
                      aria-pressed={effectiveFormat === option}
                      onClick={() => setFormat(option)}
                      className={
                        effectiveFormat === option
                          ? "rounded-md border border-primary/50 bg-primary/8 px-2.5 py-1.5 text-[11px] font-medium text-foreground"
                          : "rounded-md border border-border bg-card px-2.5 py-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                      }
                    >
                      {DATABASE_DUMP_FORMAT_LABELS[option]}
                    </button>
                  ))}
                </div>
              </div>
            </div>

            <div className="space-y-1.5">
              <Label htmlFor="dump-source">Source file (absolute path)</Label>
              <div className="flex gap-2">
                <Input
                  id="dump-source"
                  value={sourcePath}
                  onChange={(event) => setSourcePath(event.target.value)}
                  placeholder={
                    sourceKind === "remote"
                      ? `/home/deploy/backups/app.${defaultExtension(effectiveFormat)}`
                      : `/home/user/backups/app.${defaultExtension(effectiveFormat)}`
                  }
                  className="h-8 bg-card font-mono text-[11px]"
                  disabled={running}
                />
                {sourceKind === "local" ? (
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    className="h-8 shrink-0"
                    onClick={() => void browseSource()}
                    disabled={running}
                    aria-label="Choose local dump source"
                  >
                    <FolderOpenIcon className="size-3.5" aria-hidden="true" />
                    Browse
                  </Button>
                ) : null}
              </div>
            </div>

            <div className="space-y-1.5">
              <Label htmlFor="restore-target">Target database</Label>
              <Input
                id="restore-target"
                value={targetDatabase}
                onChange={(event) => setTargetDatabase(event.target.value)}
                placeholder={database ?? profile.databaseName ?? "app"}
                className="h-8 bg-card font-mono text-[11px]"
                disabled={running}
              />
              <p className="text-[10px] text-muted-foreground">
                The restore writes into this exact database. The confirmation names it before any
                command runs.
              </p>
            </div>

            <div className="flex flex-wrap items-center gap-4 rounded-md border border-border bg-muted/15 px-3 py-2">
              <label
                className="flex items-center gap-2 text-[11px] text-foreground"
                title={
                  profile.engine === "mysql" || effectiveFormat === "plain_sql"
                    ? "Plain SQL restores do not support clean or no-owner options."
                    : "Drop existing objects before restoring"
                }
              >
                <input
                  type="checkbox"
                  checked={clean}
                  onChange={(event) => setClean(event.target.checked)}
                  disabled={profile.engine === "mysql" || effectiveFormat === "plain_sql"}
                  className="size-3.5 accent-primary"
                />
                Clean existing objects
              </label>
              <label
                className="flex items-center gap-2 text-[11px] text-foreground"
                title={
                  profile.engine === "mysql" || effectiveFormat === "plain_sql"
                    ? "Plain SQL restores do not support clean or no-owner options."
                    : "Ignore ownership commands in the archive"
                }
              >
                <input
                  type="checkbox"
                  checked={noOwner}
                  onChange={(event) => setNoOwner(event.target.checked)}
                  disabled={profile.engine === "mysql" || effectiveFormat === "plain_sql"}
                  className="size-3.5 accent-primary"
                />
                Ignore ownership
              </label>
            </div>

            {profile.readOnly ? (
              <Alert variant="warning">
                <WarningCircleIcon
                  className="mt-0.5 size-4 shrink-0 text-warning"
                  aria-hidden="true"
                />
                <div>
                  <AlertTitle>Read-only profile</AlertTitle>
                  <AlertDescription>
                    This connection cannot restore data. Save a writable profile before restoring a
                    dump.
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}
          </div>
        ) : null}

        {step === 2 ? (
          <div className="space-y-3">
            {mode === "export" ? (
              <dl className="grid grid-cols-[120px_1fr] gap-x-3 gap-y-1.5 rounded-md border border-border bg-muted/20 px-3.5 py-3">
                {reviewDetails.map((detail) => (
                  <div key={detail.label} className="contents">
                    <dt className="text-[11px] leading-5 text-muted-foreground">{detail.label}</dt>
                    <dd className="min-w-0 break-words font-mono text-[11px] leading-5 text-foreground">
                      {detail.value}
                    </dd>
                  </div>
                ))}
              </dl>
            ) : (
              <p className="truncate rounded-md border border-border bg-muted/15 px-3 py-1.5 font-mono text-[10px] text-muted-foreground">
                {targetDatabase.trim() || "no target database"} ·{" "}
                {sourceKind === "remote" ? "remote" : "local"} ·{" "}
                {sourcePath.trim() || "no source file"}
              </p>
            )}

            {mode === "restore" ? (
              <>
                {preview.status === "loading" ? (
                  <p className="flex items-center gap-2 text-[11px] text-muted-foreground">
                    <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
                    Building the restore confirmation from the server metadata...
                  </p>
                ) : null}
                {preview.status === "error" ? (
                  <Alert variant={preview.error?.code === "permission" ? "warning" : "destructive"}>
                    <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                    <div className="flex-1">
                      <AlertTitle>
                        {preview.error?.code === "permission"
                          ? "Permission denied building the restore preview"
                          : "Restore preview failed"}
                      </AlertTitle>
                      <AlertDescription>{preview.errorMessage}</AlertDescription>
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        className="mt-2"
                        onClick={() => void handlePreviewRestore()}
                      >
                        Retry preview
                      </Button>
                    </div>
                  </Alert>
                ) : null}
                {preview.preview ? (
                  <div className="space-y-2">
                    <div className="rounded-md border border-border bg-card px-3.5 py-3">
                      <p className="flex items-center gap-1.5 text-[11px] font-medium text-foreground">
                        <InfoIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
                        Restore plan
                      </p>
                      <dl className="mt-2 grid grid-cols-[110px_1fr] gap-x-3 gap-y-1.5">
                        {restoreConfirmationDetails(
                          preview.preview,
                          server?.name ?? "the selected server",
                        ).map((detail) => (
                          <div key={detail.label} className="contents">
                            <dt className="text-[10px] leading-5 text-muted-foreground">
                              {detail.label}
                            </dt>
                            <dd className="min-w-0 break-words font-mono text-[10px] leading-5 text-foreground">
                              {detail.value}
                            </dd>
                          </div>
                        ))}
                      </dl>
                    </div>
                    {preview.preview.warnings.length > 0 ? (
                      <Alert variant="warning">
                        <WarningCircleIcon
                          className="mt-0.5 size-4 shrink-0 text-warning"
                          aria-hidden="true"
                        />
                        <div>
                          <AlertTitle>Restore warnings</AlertTitle>
                          <AlertDescription>
                            <ul className="space-y-0.5">
                              {preview.preview.warnings.map((warning, index) => (
                                <li key={`${warning}-${index}`}>{warning}</li>
                              ))}
                            </ul>
                          </AlertDescription>
                        </div>
                      </Alert>
                    ) : null}
                    {!restorePlanCurrent ? (
                      <Alert variant="warning">
                        <WarningCircleIcon
                          className="mt-0.5 size-4 shrink-0 text-warning"
                          aria-hidden="true"
                        />
                        <div>
                          <AlertTitle>Preview is out of date</AlertTitle>
                          <AlertDescription>
                            The source, format, or target database changed after the preview. Go
                            back and preview the restore again before starting it.
                          </AlertDescription>
                        </div>
                      </Alert>
                    ) : null}
                    {!preview.preview.canRestore ? (
                      <Alert variant="destructive">
                        <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                        <div>
                          <AlertTitle>Restore is blocked</AlertTitle>
                          <AlertDescription>
                            The preview did not confirm a compatible tool and target. Resolve the
                            warnings before restoring.
                          </AlertDescription>
                        </div>
                      </Alert>
                    ) : null}
                  </div>
                ) : null}
              </>
            ) : (
              <>
                <Alert variant="warning">
                  <WarningCircleIcon
                    className="mt-0.5 size-4 shrink-0 text-warning"
                    aria-hidden="true"
                  />
                  <div>
                    <AlertTitle>Review the output target</AlertTitle>
                    <AlertDescription>
                      The dump is written to a sibling partial file and renamed only after the tool
                      exits successfully and the size is verified. A cancelled or failed dump leaves
                      an existing destination unchanged.
                    </AlertDescription>
                  </div>
                </Alert>
                {scope.kind === "cluster" ? (
                  <p className="text-[10px] leading-4 text-muted-foreground">
                    A cluster dump includes every database and role in the PostgreSQL cluster. It
                    can be large and may contain sensitive role definitions.
                  </p>
                ) : null}
              </>
            )}
          </div>
        ) : null}

        {step === 3 ? (
          <div className="space-y-3">
            {running || transfer.progress ? (
              <div className="rounded-md border border-border bg-muted/15 px-3 py-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="flex items-center gap-1.5 text-[11px] font-medium">
                    {running ? (
                      <CircleNotchIcon
                        className="size-3.5 animate-spin text-primary"
                        aria-hidden="true"
                      />
                    ) : (
                      <CheckCircleIcon className="size-3.5 text-success" aria-hidden="true" />
                    )}
                    {transfer.direction === "export" || transfer.direction === "backup"
                      ? "Writing dump"
                      : "Restoring dump"}
                  </span>
                  {running ? (
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      className="h-6 px-2 text-[10px]"
                      onClick={() => void cancelTransfer()}
                      disabled={transfer.cancelling}
                    >
                      <XIcon className="size-3" aria-hidden="true" />
                      {transfer.cancelling ? "Cancelling..." : "Cancel"}
                    </Button>
                  ) : null}
                </div>
                <div
                  role="progressbar"
                  aria-label={`${transfer.direction} dump progress`}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={percent ?? undefined}
                  className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
                >
                  <div
                    className={
                      percent === null
                        ? "h-full w-1/3 animate-pulse rounded-full bg-info"
                        : "h-full rounded-full bg-info transition-[width] duration-200"
                    }
                    style={percent === null ? undefined : { width: `${percent}%` }}
                  />
                </div>
                <p className="mt-1.5 font-mono text-[10px] text-muted-foreground">
                  {transfer.progress
                    ? `${dumpPhaseLabel(transfer.progress.phase)} · ${transfer.progress.message} · ${formatBytes(transfer.progress.bytesProcessed)}${transfer.progress.totalBytes ? ` of ${formatBytes(transfer.progress.totalBytes)}` : ""}${elapsedMs !== undefined ? ` · ${formatDurationMs(elapsedMs)}` : ""}`
                    : "Preparing transfer..."}
                </p>
              </div>
            ) : null}

            {transfer.status === "error" ? (
              <Alert variant={transfer.error?.code === "permission" ? "warning" : "destructive"}>
                <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <div className="flex-1">
                  <AlertTitle>
                    {transfer.error?.code === "permission"
                      ? "Permission denied"
                      : transfer.error?.code === "connection"
                        ? "Connection lost"
                        : "Transfer failed"}
                  </AlertTitle>
                  <AlertDescription>{transfer.errorMessage}</AlertDescription>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    className="mt-2"
                    onClick={retry}
                  >
                    Retry
                  </Button>
                </div>
              </Alert>
            ) : null}

            {mode === "export" && exportResult ? (
              <Alert variant={resultVariant(exportResult.status)}>
                <CheckCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <AlertTitle>
                    {exportResult.status === "completed"
                      ? isBackup
                        ? "Backup completed"
                        : "Dump export completed"
                      : exportResult.status === "cancelled"
                        ? isBackup
                          ? "Backup cancelled"
                          : "Dump export cancelled"
                        : isBackup
                          ? "Backup failed"
                          : "Dump export failed"}
                  </AlertTitle>
                  <AlertDescription>
                    {exportResult.status === "completed"
                      ? `${formatBytes(exportResult.bytesWritten)} written to ${exportResult.destinationPath} in ${formatDurationMs(exportResult.durationMs)}. Size verified: ${exportResult.verified ? "yes" : "no"}.`
                      : `${exportResult.failureSummary ?? "The transfer did not complete."} ${formatBytes(exportResult.bytesWritten)} written in ${formatDurationMs(exportResult.durationMs)}.`}
                  </AlertDescription>
                  <div className="mt-1.5 space-y-0.5 text-[10px] text-muted-foreground">
                    <p>
                      Tool {exportResult.tool}
                      {exportResult.toolVersion ? ` ${exportResult.toolVersion}` : ""}
                      {exportResult.exitCode !== undefined
                        ? ` · exit ${exportResult.exitCode}`
                        : ""}
                    </p>
                    <p>
                      Partial output removed: {exportResult.partialRemoved ? "yes" : "no"} ·
                      temporary files cleaned: {exportResult.tempCleaned ? "yes" : "no"}
                    </p>
                    {exportResult.mayBeIncomplete ? (
                      <p className="font-medium text-warning">
                        The backend marked this dump as potentially incomplete. Do not treat the
                        destination as a complete backup.
                      </p>
                    ) : null}
                  </div>
                  {exportResult.warnings.length > 0 ? (
                    <div className="mt-1.5 space-y-0.5 text-[10px] text-muted-foreground">
                      {exportResult.warnings.slice(0, 4).map((warning, index) => (
                        <p key={`${warning}-${index}`}>{warning}</p>
                      ))}
                      {exportResult.warnings.length > 4 ? (
                        <p>More warnings were reported by the backend.</p>
                      ) : null}
                    </div>
                  ) : null}
                  {exportResult.status !== "completed" ? (
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      className="mt-2"
                      onClick={retry}
                    >
                      Retry {isBackup ? "backup" : "export"}
                    </Button>
                  ) : null}
                </div>
              </Alert>
            ) : null}

            {mode === "restore" && restoreResult ? (
              <Alert variant={resultVariant(restoreResult.status)}>
                <CheckCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <AlertTitle>
                    {restoreResult.status === "completed"
                      ? isBackup
                        ? "Restore completed"
                        : "Dump restore completed"
                      : restoreResult.status === "cancelled"
                        ? isBackup
                          ? "Restore cancelled"
                          : "Dump restore cancelled"
                        : isBackup
                          ? "Restore failed"
                          : "Dump restore failed"}
                  </AlertTitle>
                  <AlertDescription>
                    {restoreResult.status === "completed"
                      ? `${formatBytes(restoreResult.bytesRead)} restored into ${restoreResult.targetDatabase} in ${formatDurationMs(restoreResult.durationMs)}.`
                      : `${restoreResult.failureSummary ?? "The restore did not complete."} ${formatBytes(restoreResult.bytesRead)} read in ${formatDurationMs(restoreResult.durationMs)}.`}
                  </AlertDescription>
                  <div className="mt-1.5 space-y-0.5 text-[10px] text-muted-foreground">
                    <p>
                      Target {restoreResult.targetDatabase} · tool {restoreResult.tool}
                      {restoreResult.toolVersion ? ` ${restoreResult.toolVersion}` : ""}
                      {restoreResult.exitCode !== undefined
                        ? ` · exit ${restoreResult.exitCode}`
                        : ""}
                    </p>
                    <p>
                      Partial output removed: {restoreResult.partialRemoved ? "yes" : "no"} ·
                      temporary files cleaned: {restoreResult.tempCleaned ? "yes" : "no"}
                    </p>
                    {restoreResult.mayBeIncomplete ? (
                      <p className="font-medium text-warning">
                        The target database may contain partially restored objects. Verify the
                        schema and data before retrying; a retry can duplicate or overwrite rows.
                      </p>
                    ) : null}
                  </div>
                  {restoreResult.warnings.length > 0 ? (
                    <div className="mt-1.5 space-y-0.5 text-[10px] text-muted-foreground">
                      {restoreResult.warnings.slice(0, 4).map((warning, index) => (
                        <p key={`${warning}-${index}`}>{warning}</p>
                      ))}
                      {restoreResult.warnings.length > 4 ? (
                        <p>More warnings were reported by the backend.</p>
                      ) : null}
                    </div>
                  ) : null}
                  {restoreResult.status !== "completed" ? (
                    isRestoreRetrySafe(restoreResult.status, restoreResult.mayBeIncomplete) ? (
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        className="mt-2"
                        onClick={retry}
                      >
                        Retry restore
                      </Button>
                    ) : (
                      <p className="mt-2 text-[10px] font-medium text-warning">
                        Retry is disabled because the target may be partially restored. Verify it
                        first, then start a new restore from the beginning.
                      </p>
                    )
                  ) : null}
                </div>
              </Alert>
            ) : null}

            {submitted && !running && !result && transfer.status !== "error" ? (
              <p className="text-[11px] text-muted-foreground">
                The transfer has not produced a result yet. Keep this dialog open or check the
                Operations tab for its background task.
              </p>
            ) : null}
          </div>
        ) : null}

        <DialogFooter>
          {step === 1 ? (
            <>
              <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              {mode === "export" ? (
                <Button
                  type="button"
                  onClick={() => setStep(2)}
                  disabled={
                    !path.trim() ||
                    !connected ||
                    busyElsewhere ||
                    toolReady === false ||
                    (scopeKind === "tables" && tables.length === 0)
                  }
                >
                  Review {isBackup ? "backup" : "export"}
                </Button>
              ) : (
                <Button
                  type="button"
                  onClick={() => void handlePreviewRestore()}
                  disabled={
                    !sourcePath.trim() ||
                    !targetDatabase.trim() ||
                    !connected ||
                    busyElsewhere ||
                    profile.readOnly ||
                    toolReady === false
                  }
                >
                  {preview.status === "loading" ? "Building preview..." : "Preview restore"}
                </Button>
              )}
            </>
          ) : step === 2 ? (
            <>
              <Button
                type="button"
                variant="secondary"
                onClick={() => setStep(1)}
                disabled={running}
              >
                Back
              </Button>
              {mode === "export" ? (
                <Button
                  type="button"
                  onClick={() => void handleExport()}
                  disabled={
                    running || !connected || busyElsewhere || !path.trim() || toolReady === false
                  }
                >
                  <ArchiveBoxIcon className="size-3.5" aria-hidden="true" />
                  {isBackup ? "Create backup" : "Export dump"}
                </Button>
              ) : (
                <Button
                  type="button"
                  onClick={() => setConfirmRestore(true)}
                  disabled={
                    running ||
                    busyElsewhere ||
                    profile.readOnly ||
                    !preview.preview ||
                    !preview.preview.canRestore ||
                    !restorePlanCurrent
                  }
                >
                  <DatabaseIcon className="size-3.5" aria-hidden="true" />
                  {isBackup ? "Restore backup" : "Restore dump"}
                </Button>
              )}
            </>
          ) : (
            <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
              {running ? "Run in background" : "Close"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>

      <ConfirmDialog
        open={confirmOverwrite}
        onOpenChange={(open) => {
          if (!open) {
            setConfirmOverwrite(false);
          }
        }}
        title="Replace the existing dump destination?"
        description="The current file is kept until the new dump has been written to a partial path, verified, and renamed."
        confirmLabel="Replace and export"
        details={[
          { label: "Server", value: server ? `${server.name} (${server.host})` : profile.serverId },
          { label: "Connection", value: profile.name },
          { label: "Scope", value: scopeLabel },
          { label: "Destination", value: path.trim() },
        ]}
        warningTitle="The existing output will be replaced."
        warningDescription="Review the scope and destination before starting the dump."
        onConfirm={() => {
          setConfirmOverwrite(false);
          void startExport(true);
        }}
      />

      <ConfirmDialog
        open={confirmRestore}
        onOpenChange={(open) => {
          if (!open) {
            setConfirmRestore(false);
          }
        }}
        title={`Restore into ${preview.preview?.targetDatabase ?? targetDatabase.trim()}?`}
        description="This changes the target database. The confirmation below is generated from the server metadata and is sent to the backend before any tool runs."
        confirmLabel={isBackup ? "Restore backup" : "Restore dump"}
        details={restoreConfirmationDetails(preview.preview, server?.name ?? "the selected server")}
        warningTitle={`This may replace data in ${preview.preview?.targetDatabase ?? targetDatabase.trim()}.`}
        warningDescription={
          preview.preview?.replacedData ??
          "Existing objects and rows that conflict with the dump may be replaced."
        }
        onConfirm={() => void runRestore()}
      />
    </Dialog>
  );
}
