import { useEffect, useMemo, useRef, useState } from "react";
import {
  CheckCircleIcon,
  CircleNotchIcon,
  FileCsvIcon,
  FolderOpenIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { TransferSteps } from "@/features/databases/DatabaseTransferSteps";
import {
  beginOperationTask,
  updateOperationTask,
} from "@/features/databases/databaseOperationTasks";
import {
  csvPhaseLabel,
  csvProgressPercent,
  csvTaskDetail,
  formatBytes,
  formatCount,
  formatDurationMs,
  isCsvRetrySafe,
  taskStatusForTransfer,
} from "@/features/databases/databaseTransferModel";
import { useDatabaseCsvTransferEvents } from "@/hooks/useDatabaseCsvTransferEvents";
import { pickLocalFile, pickLocalSavePath } from "@/lib/fileDialog";
import {
  useDatabaseCsvStore,
  type DatabaseCsvPreviewState,
  type DatabaseCsvTransferState,
} from "@/stores/databaseCsvStore";
import type {
  DatabaseCsvDelimiter,
  DatabaseCsvEncoding,
  DatabaseCsvImportOptions,
  DatabaseCsvNullMode,
  DatabaseCsvTable,
  DatabaseRowColumn,
} from "@/types/contracts";

export type DatabaseCsvTransferMode = "export" | "import";

const DELIMITERS: Array<{ value: DatabaseCsvDelimiter; label: string }> = [
  { value: "comma", label: "Comma (,)" },
  { value: "semicolon", label: "Semicolon (;)" },
  { value: "tab", label: "Tab" },
  { value: "pipe", label: "Pipe (|)" },
];

const ENCODINGS: Array<{ value: DatabaseCsvEncoding; label: string }> = [
  { value: "utf8", label: "UTF-8" },
  { value: "utf8_bom", label: "UTF-8 with BOM" },
  { value: "utf16_le", label: "UTF-16 LE" },
  { value: "utf16_be", label: "UTF-16 BE" },
  { value: "latin1", label: "Latin-1" },
];

const SKIP = "__skip__";

type Step = "configure" | "mapping" | "review" | "transfer";

function warningLabel(warning: {
  rowNumber: number;
  columnNumber?: number;
  columnName?: string;
}): string {
  const column = warning.columnName
    ? `, ${warning.columnName}`
    : warning.columnNumber
      ? `, column ${warning.columnNumber}`
      : "";
  return `Row ${warning.rowNumber}${column}`;
}

function mappingDefaults(headers: string[], columns: DatabaseRowColumn[]): Record<number, string> {
  const result: Record<number, string> = {};
  headers.forEach((header, index) => {
    const normalized = header.trim().toLowerCase();
    const match =
      columns.find((column) => column.name.toLowerCase() === normalized) ??
      (headers.length === columns.length ? columns[index] : undefined);
    result[index] = match?.name ?? SKIP;
  });
  return result;
}

function resultTitle(status: string, direction: "export" | "import"): string {
  if (status === "completed") {
    return direction === "export" ? "Export completed" : "Import completed";
  }
  if (status === "cancelled") {
    return direction === "export" ? "Export cancelled" : "Import cancelled";
  }
  return direction === "export" ? "Export failed" : "Import failed";
}

function resultVariant(status: string): "success" | "warning" | "destructive" {
  if (status === "completed") {
    return "success";
  }
  return status === "cancelled" ? "warning" : "destructive";
}

export function DatabaseCsvTransferDialog({
  open,
  mode,
  onOpenChange,
  profileId,
  profileName,
  serverName,
  table,
  columns,
  readOnly,
  sql,
  onImported,
}: {
  open: boolean;
  mode: DatabaseCsvTransferMode;
  onOpenChange: (open: boolean) => void;
  profileId: string;
  profileName?: string;
  serverName?: string;
  table: DatabaseCsvTable;
  columns: DatabaseRowColumn[];
  readOnly: boolean;
  /** When present, export runs this read-only SQL instead of the table. */
  sql?: string;
  onImported?: () => void;
}) {
  const rawPreviewState = useDatabaseCsvStore((state) => state.preview);
  const rawTransfer = useDatabaseCsvStore((state) => state.transfer);
  const previewState: DatabaseCsvPreviewState =
    rawPreviewState.profileId && rawPreviewState.profileId !== profileId
      ? { status: "idle" }
      : rawPreviewState;
  const transfer: DatabaseCsvTransferState =
    rawTransfer.profileId && rawTransfer.profileId !== profileId && rawTransfer.status !== "idle"
      ? {
          direction: mode,
          status: "idle",
          cancelling: false,
        }
      : rawTransfer;
  const busyElsewhere =
    rawTransfer.status === "running" &&
    rawTransfer.profileId !== undefined &&
    rawTransfer.profileId !== profileId;
  const previewImport = useDatabaseCsvStore((state) => state.previewImport);
  const startExport = useDatabaseCsvStore((state) => state.startExport);
  const startImport = useDatabaseCsvStore((state) => state.startImport);
  const cancelTransfer = useDatabaseCsvStore((state) => state.cancelTransfer);
  const clearPreview = useDatabaseCsvStore((state) => state.clearPreview);
  const dismissTransfer = useDatabaseCsvStore((state) => state.dismissTransfer);
  useDatabaseCsvTransferEvents();

  const [path, setPath] = useState("");
  const [delimiter, setDelimiter] = useState<DatabaseCsvDelimiter>("comma");
  const [encoding, setEncoding] = useState<DatabaseCsvEncoding>("utf8");
  const [hasHeader, setHasHeader] = useState(true);
  const [overwrite, setOverwrite] = useState(false);
  const [overwriteConfirmed, setOverwriteConfirmed] = useState(false);
  const [nullKind, setNullKind] = useState<DatabaseCsvNullMode["kind"]>("unquotedEmpty");
  const [nullLiteral, setNullLiteral] = useState("NULL");
  const [mapping, setMapping] = useState<Record<number, string>>({});
  const [previewPath, setPreviewPath] = useState("");
  const [step, setStep] = useState<Step>("configure");
  const [submitted, setSubmitted] = useState(false);
  const [importAcknowledged, setImportAcknowledged] = useState(false);
  const [confirmOverwrite, setConfirmOverwrite] = useState(false);
  const taskIdRef = useRef<string | null>(null);

  const writableColumns = useMemo(
    () => columns.filter((column) => !column.generated && column.identity !== "always"),
    [columns],
  );

  useEffect(() => {
    // The dialog is mounted fresh for each open. Clear any previous transfer
    // document in the external store without resetting local form state.
    dismissTransfer();
    clearPreview();
  }, [dismissTransfer, clearPreview]);

  const options: DatabaseCsvImportOptions = {
    delimiter,
    encoding,
    hasHeader,
    nullMode:
      nullKind === "literal"
        ? { kind: "literal", value: nullLiteral }
        : { kind: nullKind === "never" ? "never" : "unquotedEmpty" },
  };

  const preview = previewState.preview;
  const previewMatches =
    preview !== undefined &&
    previewPath === path.trim() &&
    preview.delimiter === delimiter &&
    preview.encoding === encoding &&
    preview.hasHeader === hasHeader;
  const running = transfer.status === "running";
  const exportResult = transfer.exportResult;
  const importResult = transfer.importResult;
  const progress = transfer.progress;
  const percent = csvProgressPercent(progress);
  const elapsedMs =
    progress && transfer.startedAt
      ? Math.max(0, progress.emittedAt - transfer.startedAt)
      : undefined;

  const mappings = useMemo(
    () =>
      Object.entries(mapping)
        .filter(([, target]) => target && target !== SKIP)
        .map(([source, target]) => ({
          sourceIndex: Number(source),
          targetColumn: target,
        }))
        .sort((left, right) => left.sourceIndex - right.sourceIndex),
    [mapping],
  );

  const stepLabels =
    mode === "import" ? ["File", "Mapping", "Review", "Transfer"] : ["File", "Review", "Transfer"];
  const stepIndex =
    step === "configure"
      ? 0
      : step === "mapping"
        ? 1
        : step === "review"
          ? mode === "import"
            ? 2
            : 1
          : stepLabels.length - 1;

  useEffect(() => {
    const taskId = taskIdRef.current;
    if (!taskId) {
      return;
    }
    if (transfer.transferId !== taskId || transfer.profileId !== profileId) {
      return;
    }
    const result = transfer.direction === "export" ? transfer.exportResult : transfer.importResult;
    updateOperationTask({
      profileId,
      id: taskId,
      status: taskStatusForTransfer(transfer.status),
      detail: csvTaskDetail(
        transfer.direction,
        transfer.status,
        transfer.progress,
        transfer.exportResult,
        transfer.importResult,
      ),
      partial: result?.mayBeIncomplete ?? false,
    });
  }, [
    profileId,
    transfer.direction,
    transfer.exportResult,
    transfer.importResult,
    transfer.profileId,
    transfer.progress,
    transfer.status,
    transfer.transferId,
  ]);

  function updateMapping(sourceIndex: number, target: string) {
    setMapping((current) => {
      const next = { ...current };
      if (target !== SKIP) {
        for (const [key, value] of Object.entries(next)) {
          if (Number(key) !== sourceIndex && value === target) {
            next[Number(key)] = SKIP;
          }
        }
      }
      next[sourceIndex] = target;
      return next;
    });
  }

  function resetPreview() {
    clearPreview();
    setPreviewPath("");
    setMapping({});
  }

  async function handleBrowse() {
    const selected =
      mode === "export"
        ? await pickLocalSavePath({
            title: "Choose a CSV destination",
            defaultPath: `${table.name}.csv`,
            filters: [{ name: "CSV", extensions: ["csv", "tsv", "txt"] }],
          })
        : await pickLocalFile({
            title: "Choose a CSV source file",
            filters: [{ name: "CSV", extensions: ["csv", "tsv", "txt"] }],
          });
    if (selected) {
      setPath(selected);
      resetPreview();
    }
  }

  async function handlePreview() {
    if (!path.trim()) {
      return;
    }
    const parsed = await previewImport(path.trim(), options, 20, profileId);
    if (parsed) {
      setPreviewPath(path.trim());
      setMapping(mappingDefaults(parsed.headers, writableColumns));
    }
  }

  async function runExport(forceOverwrite: boolean) {
    setStep("transfer");
    setSubmitted(true);
    const promise = startExport(profileId, {
      source: sql ? { kind: "query", sql } : { kind: "table", table },
      destinationPath: path.trim(),
      delimiter,
      encoding,
      includeHeader: hasHeader,
      overwrite: overwrite || forceOverwrite,
    });
    const transferId = useDatabaseCsvStore.getState().transfer.transferId;
    if (transferId) {
      taskIdRef.current = transferId;
      beginOperationTask({
        profileId,
        id: transferId,
        label: `CSV ${sql ? "query" : "table"} export · ${table.name}`,
        detail: "Preparing the CSV export",
      });
    }
    return promise;
  }

  async function handleExport() {
    if (!path.trim() || busyElsewhere) {
      return;
    }
    if (overwrite && !overwriteConfirmed) {
      setConfirmOverwrite(true);
      return;
    }
    await runExport(overwriteConfirmed);
  }

  async function runImport() {
    if (!path.trim() || mappings.length === 0 || !previewMatches || busyElsewhere) {
      return;
    }
    setStep("transfer");
    setSubmitted(true);
    const promise = startImport(profileId, {
      table,
      sourcePath: path.trim(),
      options,
      mappings,
    });
    const transferId = useDatabaseCsvStore.getState().transfer.transferId;
    if (transferId) {
      taskIdRef.current = transferId;
      beginOperationTask({
        profileId,
        id: transferId,
        label: `CSV import · ${table.name}`,
        detail: "Preparing the CSV import",
      });
    }
    const imported = await promise;
    if (imported?.status === "completed") {
      onImported?.();
    }
    return imported;
  }

  function retry() {
    if (mode === "export") {
      void runExport(overwriteConfirmed);
    } else {
      void runImport();
    }
  }

  const title =
    mode === "export"
      ? sql
        ? "Export query result to CSV"
        : "Export table to CSV"
      : "Import CSV into table";
  const description =
    mode === "export"
      ? sql
        ? "Run one read-only statement and stream every returned value to a local CSV file. Values are written in full."
        : `Stream every row of ${table.name} to a local CSV file. Values are written in full.`
      : `Map CSV columns to ${table.name}, preview rows, then import in one transaction.`;

  const reviewDetails = [
    { label: "Connection", value: profileName ?? profileId },
    { label: "Server", value: serverName ?? "the selected server" },
    {
      label: mode === "export" ? "Source" : "Target table",
      value:
        mode === "export"
          ? sql
            ? `Read-only query result for ${table.name}`
            : table.name
          : `${table.database ?? "active database"}${table.schema ? ` / ${table.schema}` : ""} / ${table.name}`,
    },
    {
      label: mode === "export" ? "Destination" : "Source file",
      value: path.trim() || "not selected",
    },
    {
      label: "Delimiter",
      value: DELIMITERS.find((item) => item.value === delimiter)?.label ?? delimiter,
    },
    {
      label: "Encoding",
      value: ENCODINGS.find((item) => item.value === encoding)?.label ?? encoding,
    },
    { label: "Header row", value: hasHeader ? "First row is a header" : "No header row" },
    ...(mode === "export"
      ? [
          {
            label: "Existing file",
            value: overwrite ? "Replace after confirmation" : "Reject if it exists",
          },
        ]
      : [
          {
            label: "Mapped columns",
            value: `${formatCount(mappings.length)} of ${formatCount(preview?.headers.length ?? 0)}`,
          },
          {
            label: "Preview",
            value: preview ? `${formatCount(preview.sampledRows)} rows sampled` : "not loaded",
          },
        ]),
  ];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100vh-2rem)] max-w-3xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FileCsvIcon className="size-4 text-primary" aria-hidden="true" />
            {title}
          </DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        <TransferSteps
          steps={stepLabels}
          activeIndex={stepIndex}
          label={`${mode === "export" ? "CSV export" : "CSV import"} steps`}
        />

        {step === "configure" ? (
          <div className="space-y-3">
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5 sm:col-span-2">
                <Label htmlFor="csv-path">
                  {mode === "export"
                    ? "Destination file (absolute path)"
                    : "Source file (absolute path)"}
                </Label>
                <div className="flex gap-2">
                  <Input
                    id="csv-path"
                    value={path}
                    onChange={(event) => {
                      setPath(event.target.value);
                      if (preview) {
                        resetPreview();
                      }
                    }}
                    placeholder={
                      mode === "export"
                        ? "/home/user/exports/users.csv"
                        : "/home/user/imports/users.csv"
                    }
                    className="h-8 bg-card font-mono text-[11px]"
                    disabled={running}
                  />
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    className="h-8 shrink-0"
                    onClick={() => void handleBrowse()}
                    disabled={running}
                    aria-label={mode === "export" ? "Choose CSV destination" : "Choose CSV source"}
                  >
                    <FolderOpenIcon className="size-3.5" aria-hidden="true" />
                    Browse
                  </Button>
                </div>
                <p className="text-[10px] text-muted-foreground">
                  {mode === "export"
                    ? "The file is written to a hidden partial path and renamed only after every row is flushed."
                    : "The file is read in place; no copy is uploaded to the server."}
                </p>
              </div>
              <div className="space-y-1.5">
                <Label>Delimiter</Label>
                <Select
                  value={delimiter}
                  onValueChange={(value) => {
                    setDelimiter(value as DatabaseCsvDelimiter);
                    resetPreview();
                  }}
                  disabled={running}
                >
                  <SelectTrigger className="h-8 bg-card text-[11px]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {DELIMITERS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1.5">
                <Label>Encoding</Label>
                <Select
                  value={encoding}
                  onValueChange={(value) => {
                    setEncoding(value as DatabaseCsvEncoding);
                    resetPreview();
                  }}
                  disabled={running}
                >
                  <SelectTrigger className="h-8 bg-card text-[11px]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {ENCODINGS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div className="flex flex-wrap items-center gap-4 rounded-md border border-border bg-muted/15 px-3 py-2">
              <label className="flex items-center gap-2 text-[11px] text-foreground">
                <input
                  type="checkbox"
                  checked={hasHeader}
                  onChange={(event) => {
                    setHasHeader(event.target.checked);
                    resetPreview();
                  }}
                  disabled={running}
                  className="size-3.5 accent-primary"
                />
                First row is a header
              </label>
              {mode === "export" ? (
                <label className="flex items-center gap-2 text-[11px] text-foreground">
                  <input
                    type="checkbox"
                    checked={overwrite}
                    onChange={(event) => {
                      setOverwrite(event.target.checked);
                      setOverwriteConfirmed(false);
                    }}
                    disabled={running}
                    className="size-3.5 accent-primary"
                  />
                  Replace an existing destination
                </label>
              ) : (
                <div className="flex items-center gap-2">
                  <Label className="text-[11px]">Empty fields</Label>
                  <Select
                    value={nullKind}
                    onValueChange={(value) => {
                      setNullKind(value as DatabaseCsvNullMode["kind"]);
                      resetPreview();
                    }}
                    disabled={running}
                  >
                    <SelectTrigger className="h-7 w-[170px] bg-card text-[11px]">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="unquotedEmpty">Unquoted empty = NULL</SelectItem>
                      <SelectItem value="literal">Literal text = NULL</SelectItem>
                      <SelectItem value="never">Never NULL</SelectItem>
                    </SelectContent>
                  </Select>
                  {nullKind === "literal" ? (
                    <Input
                      aria-label="NULL literal"
                      value={nullLiteral}
                      onChange={(event) => {
                        setNullLiteral(event.target.value);
                        resetPreview();
                      }}
                      disabled={running}
                      className="h-7 w-[100px] bg-card font-mono text-[11px]"
                    />
                  ) : null}
                </div>
              )}
            </div>

            {mode === "import" && readOnly ? (
              <Alert variant="warning">
                <WarningCircleIcon
                  className="mt-0.5 size-4 shrink-0 text-warning"
                  aria-hidden="true"
                />
                <div>
                  <AlertTitle>Read-only profile</AlertTitle>
                  <AlertDescription>
                    This connection cannot write rows. Save a writable profile before importing a
                    CSV file.
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}

            {busyElsewhere ? (
              <Alert variant="warning">
                <WarningCircleIcon
                  className="mt-0.5 size-4 shrink-0 text-warning"
                  aria-hidden="true"
                />
                <div>
                  <AlertTitle>Another connection is transferring</AlertTitle>
                  <AlertDescription>
                    Only one CSV transfer runs at a time in this window. Wait for the active
                    transfer to finish or cancel it from its connection tab.
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}
          </div>
        ) : null}

        {step === "mapping" && mode === "import" ? (
          <div className="space-y-3">
            <p className="truncate rounded-md border border-border bg-muted/15 px-3 py-1.5 font-mono text-[10px] text-muted-foreground">
              Source file · {path.trim()}
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => void handlePreview()}
                disabled={running || !path.trim() || previewState.status === "loading"}
              >
                {previewState.status === "loading" ? (
                  <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
                ) : (
                  <FileCsvIcon className="size-3.5" aria-hidden="true" />
                )}
                {preview ? "Reload preview" : "Load preview"}
              </Button>
              {preview ? (
                <span className="text-[10px] text-muted-foreground">
                  {formatCount(preview.rows.length)} preview row
                  {preview.rows.length === 1 ? "" : "s"} of {formatCount(preview.sampledRows)}{" "}
                  sampled
                  {preview.hasMore ? " · more rows follow" : ""}
                </span>
              ) : null}
            </div>

            {previewState.status === "error" ? (
              <Alert
                variant={previewState.error?.code === "permission" ? "warning" : "destructive"}
              >
                <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <div>
                  <AlertTitle>
                    {previewState.error?.code === "permission"
                      ? "Permission denied reading the CSV file"
                      : "Preview failed"}
                  </AlertTitle>
                  <AlertDescription>{previewState.errorMessage}</AlertDescription>
                </div>
              </Alert>
            ) : null}

            {!preview && previewState.status !== "error" ? (
              <div className="rounded-md border border-dashed border-border bg-muted/20 px-3 py-5 text-[11px] leading-5 text-muted-foreground">
                Choose the CSV file in the previous step, then load a bounded preview. Nothing is
                imported until the review step is confirmed.
              </div>
            ) : null}

            {preview ? (
              <div className="space-y-2">
                <div className="max-h-40 overflow-auto rounded-md border border-border">
                  <table className="w-full border-collapse text-left text-[10px]">
                    <thead className="sticky top-0 bg-muted/40">
                      <tr>
                        <th className="border-b border-border px-2 py-1 font-medium">Row</th>
                        {preview.headers.map((header, index) => (
                          <th
                            key={`${header}-${index}`}
                            className="border-b border-border px-2 py-1 font-medium"
                          >
                            {header || `Column ${index + 1}`}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {preview.rows.map((row) => (
                        <tr key={row.rowNumber} className="border-b border-border/60">
                          <td className="px-2 py-1 font-mono text-muted-foreground">
                            {row.rowNumber}
                          </td>
                          {row.fields.map((field, index) => (
                            <td key={index} className="max-w-[240px] truncate px-2 py-1 font-mono">
                              {field.isNull ? (
                                <span className="text-muted-foreground italic">NULL</span>
                              ) : field.value === "" ? (
                                <span className="text-muted-foreground">&quot;&quot;</span>
                              ) : (
                                field.value
                              )}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                {preview.warnings.length > 0 ? (
                  <div className="rounded-md border border-warning/30 bg-warning/5 px-3 py-2 text-[10px] text-muted-foreground">
                    {preview.warnings.slice(0, 4).map((warning) => (
                      <p
                        key={`${warning.rowNumber}-${warning.columnNumber ?? 0}-${warning.message}`}
                      >
                        {warningLabel(warning)}: {warning.message}
                      </p>
                    ))}
                    {preview.warnings.length > 4 || preview.warningsTruncated ? (
                      <p>More warnings are available when the import runs.</p>
                    ) : null}
                  </div>
                ) : null}

                {!previewMatches ? (
                  <Alert variant="warning">
                    <WarningCircleIcon
                      className="mt-0.5 size-4 shrink-0 text-warning"
                      aria-hidden="true"
                    />
                    <div>
                      <AlertTitle>Preview is out of date</AlertTitle>
                      <AlertDescription>
                        The file path or parsing options changed. Reload the preview before
                        reviewing the import.
                      </AlertDescription>
                    </div>
                  </Alert>
                ) : null}

                <div className="space-y-1.5">
                  <Label>Column mapping</Label>
                  <div className="max-h-48 divide-y divide-border overflow-y-auto rounded-md border border-border">
                    {preview.headers.map((header, sourceIndex) => (
                      <div
                        key={`${header}-${sourceIndex}`}
                        className="flex items-center gap-3 px-3 py-1.5"
                      >
                        <span className="min-w-0 flex-1 truncate font-mono text-[11px]">
                          {header || `Column ${sourceIndex + 1}`}
                        </span>
                        <Select
                          value={mapping[sourceIndex] ?? SKIP}
                          onValueChange={(value) => updateMapping(sourceIndex, value)}
                          disabled={running}
                        >
                          <SelectTrigger
                            aria-label={`Map ${header || `column ${sourceIndex + 1}`}`}
                            className="h-7 w-[200px] bg-card text-[11px]"
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value={SKIP}>Skip this column</SelectItem>
                            {writableColumns.map((column) => (
                              <SelectItem key={column.name} value={column.name}>
                                {column.name}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            ) : null}
          </div>
        ) : null}

        {step === "review" ? (
          <div className="space-y-3">
            <dl className="grid grid-cols-[110px_1fr] gap-x-3 gap-y-1.5 rounded-md border border-border bg-muted/20 px-3.5 py-3">
              {reviewDetails.map((detail) => (
                <div key={detail.label} className="contents">
                  <dt className="text-[11px] leading-5 text-muted-foreground">{detail.label}</dt>
                  <dd className="min-w-0 break-words font-mono text-[11px] leading-5 text-foreground">
                    {detail.value}
                  </dd>
                </div>
              ))}
            </dl>

            {mode === "export" && overwrite ? (
              <Alert variant="warning">
                <WarningCircleIcon
                  className="mt-0.5 size-4 shrink-0 text-warning"
                  aria-hidden="true"
                />
                <div>
                  <AlertTitle>Existing destination will be replaced</AlertTitle>
                  <AlertDescription>
                    The current file at {path.trim()} is kept until the replacement export has been
                    verified. A cancelled or failed export leaves it unchanged.
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}

            {mode === "import" ? (
              <div className="space-y-2 rounded-md border border-border bg-card px-3.5 py-3">
                <p className="text-[11px] leading-5 text-muted-foreground">
                  The import writes to{" "}
                  <span className="font-mono text-foreground">
                    {table.database ?? "active database"}
                    {table.schema ? ` / ${table.schema}` : ""} / {table.name}
                  </span>{" "}
                  in one transaction. A structural parse error, timeout, or cancellation rolls back
                  every row.
                </p>
                <label className="flex items-start gap-2 text-[11px] leading-5 text-foreground">
                  <input
                    type="checkbox"
                    checked={importAcknowledged}
                    onChange={(event) => setImportAcknowledged(event.target.checked)}
                    disabled={running || readOnly}
                    className="mt-1 size-3.5 accent-primary"
                    aria-label="Acknowledge the CSV import target"
                  />
                  I understand this import writes rows into {table.name} on{" "}
                  {serverName ?? "the selected server"}.
                </label>
              </div>
            ) : null}
          </div>
        ) : null}

        {step === "transfer" || running || progress || exportResult || importResult ? (
          <div className="space-y-3">
            {running || progress ? (
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
                    {transfer.direction === "export" ? "Exporting" : "Importing"} CSV
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
                  aria-label={`${transfer.direction} CSV progress`}
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
                  {progress
                    ? `${csvPhaseLabel(progress.phase)} · ${formatCount(progress.rowsWritten)} written · ${formatCount(progress.rowsSkipped)} skipped · ${formatCount(progress.rowsProcessed)} processed${elapsedMs !== undefined ? ` · ${formatDurationMs(elapsedMs)}` : ""}`
                    : "Preparing transfer..."}
                </p>
              </div>
            ) : null}

            {transfer.status === "error" ? (
              <Alert variant={transfer.error?.code === "permission" ? "warning" : "destructive"}>
                <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <AlertTitle>
                    {transfer.error?.code === "permission"
                      ? "Permission denied"
                      : transfer.error?.code === "connection"
                        ? "Connection lost"
                        : "CSV transfer failed"}
                  </AlertTitle>
                  <AlertDescription>{transfer.errorMessage}</AlertDescription>
                  {transfer.error?.code === "conflict" && mode === "export" ? (
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      className="mt-2"
                      onClick={() => {
                        setOverwrite(true);
                        setConfirmOverwrite(true);
                      }}
                    >
                      Replace the existing destination
                    </Button>
                  ) : (
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      className="mt-2"
                      onClick={retry}
                    >
                      Retry
                    </Button>
                  )}
                </div>
              </Alert>
            ) : null}

            {exportResult ? (
              <Alert variant={resultVariant(exportResult.status)}>
                <CheckCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <AlertTitle>{resultTitle(exportResult.status, "export")}</AlertTitle>
                  <AlertDescription>
                    {exportResult.status === "completed"
                      ? `${formatCount(exportResult.rowsExported)} rows and ${formatBytes(exportResult.bytesWritten)} written in ${formatDurationMs(exportResult.durationMs)}. Values truncated: ${exportResult.valuesTruncated ? "yes" : "no"}.`
                      : exportResult.status === "cancelled"
                        ? `Cancelled after ${formatCount(exportResult.rowsExported)} rows and ${formatDurationMs(exportResult.durationMs)}. The partial file was removed; the destination was not changed.`
                        : `The export stopped after ${formatCount(exportResult.rowsExported)} rows and ${formatDurationMs(exportResult.durationMs)}. The partial file was removed; the destination was not changed.`}
                  </AlertDescription>
                  {exportResult.mayBeIncomplete ? (
                    <p className="mt-1 text-[10px] font-medium text-warning">
                      The backend marked this export as potentially incomplete.
                    </p>
                  ) : null}
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
                  {isCsvRetrySafe(exportResult) && exportResult.status !== "completed" ? (
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      className="mt-2"
                      onClick={retry}
                    >
                      Retry export
                    </Button>
                  ) : null}
                </div>
              </Alert>
            ) : null}

            {importResult ? (
              <Alert variant={resultVariant(importResult.status)}>
                <CheckCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <AlertTitle>{resultTitle(importResult.status, "import")}</AlertTitle>
                  <AlertDescription>
                    {importResult.status === "completed"
                      ? `${formatCount(importResult.rowsImported)} rows imported, ${formatCount(importResult.rowsSkipped)} skipped, transaction committed in ${formatDurationMs(importResult.durationMs)}.`
                      : `${formatCount(importResult.rowsRead)} rows read; transaction ${importResult.transaction.replace("_", " ")} after ${formatDurationMs(importResult.durationMs)}. No rows were imported.`}
                  </AlertDescription>
                  {importResult.mayBeIncomplete ? (
                    <p className="mt-1 text-[10px] font-medium text-warning">
                      The backend marked this import as potentially incomplete. Verify the table
                      before retrying.
                    </p>
                  ) : null}
                  {importResult.warnings.length > 0 ? (
                    <div className="mt-1.5 space-y-0.5 text-[10px] text-muted-foreground">
                      {importResult.warnings.slice(0, 4).map((warning) => (
                        <p
                          key={`${warning.rowNumber}-${warning.columnNumber ?? 0}-${warning.message}`}
                        >
                          {warningLabel(warning)}: {warning.message}
                        </p>
                      ))}
                      {importResult.warnings.length > 4 || importResult.warningsTruncated ? (
                        <p>More warnings were reported by the backend.</p>
                      ) : null}
                    </div>
                  ) : null}
                  {isCsvRetrySafe(importResult) && importResult.status !== "completed" ? (
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      className="mt-2"
                      onClick={retry}
                    >
                      Retry import
                    </Button>
                  ) : null}
                </div>
              </Alert>
            ) : null}

            {submitted &&
            !running &&
            !exportResult &&
            !importResult &&
            transfer.status !== "error" ? (
              <p className="text-[11px] text-muted-foreground">
                The transfer has not produced a result yet. Keep this dialog open or check the
                Operations tab for its background task.
              </p>
            ) : null}
          </div>
        ) : null}

        <DialogFooter>
          {step === "configure" ? (
            <>
              <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button
                type="button"
                onClick={() => setStep(mode === "import" ? "mapping" : "review")}
                disabled={!path.trim() || busyElsewhere}
              >
                {mode === "import" ? "Continue to mapping" : "Review export"}
              </Button>
            </>
          ) : step === "mapping" ? (
            <>
              <Button type="button" variant="secondary" onClick={() => setStep("configure")}>
                Back
              </Button>
              <Button
                type="button"
                onClick={() => setStep("review")}
                disabled={!preview || !previewMatches || mappings.length === 0 || busyElsewhere}
              >
                Review import
              </Button>
            </>
          ) : step === "review" ? (
            <>
              <Button
                type="button"
                variant="secondary"
                onClick={() => setStep(mode === "import" ? "mapping" : "configure")}
                disabled={running}
              >
                Back
              </Button>
              {mode === "export" ? (
                <Button
                  type="button"
                  onClick={() => void handleExport()}
                  disabled={running || !path.trim() || busyElsewhere}
                >
                  <FileCsvIcon className="size-3.5" aria-hidden="true" />
                  Export CSV
                </Button>
              ) : (
                <Button
                  type="button"
                  onClick={() => void runImport()}
                  disabled={
                    running ||
                    readOnly ||
                    !importAcknowledged ||
                    mappings.length === 0 ||
                    !preview ||
                    !previewMatches ||
                    !path.trim() ||
                    busyElsewhere
                  }
                >
                  <FileCsvIcon className="size-3.5" aria-hidden="true" />
                  Import CSV
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
        title="Replace the existing CSV destination?"
        description="The current file is not removed until the replacement export is verified. A cancelled or failed export leaves the existing file unchanged."
        confirmLabel="Replace and export"
        details={[
          { label: "Connection", value: profileName ?? profileId },
          { label: "Server", value: serverName ?? "the selected server" },
          {
            label: "Source",
            value: sql ? `Read-only query result for ${table.name}` : table.name,
          },
          { label: "Destination", value: path.trim() },
        ]}
        warningTitle="The existing file will be replaced."
        warningDescription="Review the destination before starting the export."
        onConfirm={() => {
          setOverwriteConfirmed(true);
          setConfirmOverwrite(false);
          void runExport(true);
        }}
      />
    </Dialog>
  );
}
