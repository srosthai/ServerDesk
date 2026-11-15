import { useEffect, useMemo, useState } from "react";
import {
  CheckCircleIcon,
  CircleNotchIcon,
  FileCsvIcon,
  WarningCircleIcon,
  XIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
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
import { useDatabaseCsvTransferEvents } from "@/hooks/useDatabaseCsvTransferEvents";
import { useDatabaseCsvStore } from "@/stores/databaseCsvStore";
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

function formatCount(value: number): string {
  return value.toLocaleString();
}

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

export function DatabaseCsvTransferDialog({
  open,
  mode,
  onOpenChange,
  profileId,
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
  table: DatabaseCsvTable;
  columns: DatabaseRowColumn[];
  readOnly: boolean;
  /** When present, export runs this read-only SQL instead of the table. */
  sql?: string;
  onImported?: () => void;
}) {
  const previewState = useDatabaseCsvStore((state) => state.preview);
  const transfer = useDatabaseCsvStore((state) => state.transfer);
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
  const [nullKind, setNullKind] = useState<DatabaseCsvNullMode["kind"]>("unquotedEmpty");
  const [nullLiteral, setNullLiteral] = useState("NULL");
  const [mapping, setMapping] = useState<Record<number, string>>({});
  const [submitted, setSubmitted] = useState(false);

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
  const running = transfer.status === "running";
  const exportResult = transfer.exportResult;
  const importResult = transfer.importResult;
  const progress = transfer.progress;

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

  async function handlePreview() {
    if (!path.trim()) {
      return;
    }
    const parsed = await previewImport(path.trim(), options, 20);
    if (parsed) {
      setMapping(mappingDefaults(parsed.headers, writableColumns));
    }
  }

  async function handleExport() {
    if (!path.trim()) {
      return;
    }
    setSubmitted(true);
    await startExport(profileId, {
      source: sql ? { kind: "query", sql } : { kind: "table", table },
      destinationPath: path.trim(),
      delimiter,
      encoding,
      includeHeader: hasHeader,
      overwrite,
    });
  }

  async function handleImport() {
    if (!path.trim() || mappings.length === 0) {
      return;
    }
    setSubmitted(true);
    const imported = await startImport(profileId, {
      table,
      sourcePath: path.trim(),
      options,
      mappings,
    });
    if (imported?.status === "completed") {
      onImported?.();
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

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FileCsvIcon className="size-4 text-primary" aria-hidden="true" />
            {title}
          </DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5 sm:col-span-2">
              <Label htmlFor="csv-path">
                {mode === "export"
                  ? "Destination file (absolute path)"
                  : "Source file (absolute path)"}
              </Label>
              <Input
                id="csv-path"
                value={path}
                onChange={(event) => setPath(event.target.value)}
                placeholder={
                  mode === "export"
                    ? "/home/user/exports/users.csv"
                    : "/home/user/imports/users.csv"
                }
                className="h-8 bg-card font-mono text-[11px]"
                disabled={running}
              />
            </div>
            <div className="space-y-1.5">
              <Label>Delimiter</Label>
              <Select
                value={delimiter}
                onValueChange={(value) => setDelimiter(value as DatabaseCsvDelimiter)}
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
                onValueChange={(value) => setEncoding(value as DatabaseCsvEncoding)}
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
                onChange={(event) => setHasHeader(event.target.checked)}
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
                  onChange={(event) => setOverwrite(event.target.checked)}
                  disabled={running}
                  className="size-3.5 accent-primary"
                />
                Replace an existing destination
              </label>
            ) : null}
            {mode === "import" ? (
              <div className="flex items-center gap-2">
                <Label className="text-[11px]">Empty fields</Label>
                <Select
                  value={nullKind}
                  onValueChange={(value) => setNullKind(value as DatabaseCsvNullMode["kind"])}
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
                    onChange={(event) => setNullLiteral(event.target.value)}
                    disabled={running}
                    className="h-7 w-[100px] bg-card font-mono text-[11px]"
                  />
                ) : null}
              </div>
            ) : null}
          </div>

          {mode === "import" ? (
            <>
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
                  Preview rows
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
                <Alert variant="destructive">
                  <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                  <div>
                    <AlertTitle>Preview failed</AlertTitle>
                    <AlertDescription>{previewState.errorMessage}</AlertDescription>
                  </div>
                </Alert>
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
                              <td
                                key={index}
                                className="max-w-[240px] truncate px-2 py-1 font-mono"
                              >
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
            </>
          ) : null}

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
                aria-valuemax={progress?.totalBytes ?? undefined}
                aria-valuenow={progress?.bytesProcessed ?? undefined}
                className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
              >
                <div
                  className="h-full rounded-full bg-info transition-[width] duration-200"
                  style={{ width: running ? "35%" : "100%" }}
                />
              </div>
              <p className="mt-1.5 font-mono text-[10px] text-muted-foreground">
                {progress
                  ? `${formatCount(progress.rowsWritten)} written · ${formatCount(progress.rowsSkipped)} skipped · ${formatCount(progress.rowsProcessed)} processed`
                  : "Preparing transfer..."}
              </p>
            </div>
          ) : null}

          {transfer.status === "error" ? (
            <Alert variant="destructive">
              <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <div>
                <AlertTitle>CSV transfer failed</AlertTitle>
                <AlertDescription>{transfer.errorMessage}</AlertDescription>
              </div>
            </Alert>
          ) : null}

          {exportResult ? (
            <Alert variant={exportResult.status === "completed" ? "success" : "warning"}>
              <CheckCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <div>
                <AlertTitle>
                  {exportResult.status === "completed" ? "Export completed" : "Export cancelled"}
                </AlertTitle>
                <AlertDescription>
                  {exportResult.status === "completed"
                    ? `${formatCount(exportResult.rowsExported)} rows and ${formatCount(exportResult.bytesWritten)} bytes written. Values truncated: ${exportResult.valuesTruncated ? "yes" : "no"}.`
                    : "The partial file was removed; the destination was not changed."}
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {importResult ? (
            <Alert variant={importResult.status === "completed" ? "success" : "warning"}>
              <CheckCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <div>
                <AlertTitle>
                  {importResult.status === "completed" ? "Import completed" : "Import cancelled"}
                </AlertTitle>
                <AlertDescription>
                  {importResult.status === "completed"
                    ? `${formatCount(importResult.rowsImported)} rows imported, ${formatCount(importResult.rowsSkipped)} skipped, transaction committed.`
                    : "The transaction was rolled back; no rows were imported."}
                </AlertDescription>
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
              </div>
            </Alert>
          ) : null}
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="secondary"
            onClick={() => onOpenChange(false)}
            disabled={running}
          >
            Close
          </Button>
          {mode === "export" ? (
            <Button
              type="button"
              onClick={() => void handleExport()}
              disabled={running || !path.trim()}
            >
              {running ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <FileCsvIcon className="size-3.5" aria-hidden="true" />
              )}
              {submitted && running ? "Exporting..." : "Export CSV"}
            </Button>
          ) : (
            <Button
              type="button"
              onClick={() => void handleImport()}
              disabled={running || readOnly || mappings.length === 0 || !preview}
            >
              {running ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <FileCsvIcon className="size-3.5" aria-hidden="true" />
              )}
              Import CSV
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
