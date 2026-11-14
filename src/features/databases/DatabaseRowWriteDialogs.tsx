import { useState } from "react";
import {
  ArrowClockwiseIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  InfoIcon,
  PlusIcon,
  ProhibitIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import {
  buildInsertValues,
  buildUpdatePayload,
  editFields,
  identityValues,
  insertFields,
  updateField,
  type RowFieldDraft,
} from "@/features/databases/databaseRowWriteModel";
import type {
  DatabaseRowColumn,
  DatabaseRowIdentityValue,
  DatabaseRowReadResult,
  DatabaseRowRecord,
  DatabaseRowWriteResult,
  DatabaseRowWriteValue,
} from "@/types/contracts";
import { cn } from "@/lib/utils";

function fieldInput(
  field: RowFieldDraft,
  onChange: (patch: Partial<Pick<RowFieldDraft, "mode" | "value">>) => void,
) {
  if (field.mode === "null") {
    return (
      <div className="flex h-8 items-center rounded-md border border-border bg-muted/30 px-2 font-mono text-[11px] text-muted-foreground italic">
        SQL NULL
      </div>
    );
  }
  if (field.mode === "default") {
    return (
      <div className="flex h-8 items-center rounded-md border border-border bg-muted/30 px-2 font-mono text-[11px] text-muted-foreground">
        Column default
      </div>
    );
  }
  if (field.column.kind === "boolean") {
    return (
      <Select
        value={field.value === "false" ? "false" : "true"}
        onValueChange={(value) => onChange({ value })}
      >
        <SelectTrigger
          aria-label={`${field.column.name} value`}
          className="h-8 bg-card text-[12px]"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="true">true</SelectItem>
          <SelectItem value="false">false</SelectItem>
        </SelectContent>
      </Select>
    );
  }
  if (field.column.kind === "json" || field.column.kind === "text") {
    return (
      <Textarea
        aria-label={`${field.column.name} value`}
        value={field.value}
        onChange={(event) => onChange({ value: event.target.value })}
        rows={field.column.kind === "json" ? 3 : 2}
        maxLength={1_048_576}
        disabled={field.truncated}
        className="min-h-8 bg-card font-mono text-[11px]"
        placeholder={field.column.nullable ? "Empty string" : "Required value"}
      />
    );
  }
  return (
    <Input
      aria-label={`${field.column.name} value`}
      value={field.value}
      inputMode={
        field.column.kind === "integer" || field.column.kind === "decimal" ? "numeric" : undefined
      }
      onChange={(event) => onChange({ value: event.target.value })}
      maxLength={1_048_576}
      disabled={field.truncated}
      className="h-8 bg-card font-mono text-[11px]"
      placeholder={field.column.nullable ? "Empty string" : "Required value"}
    />
  );
}

function FieldModeSelect({
  field,
  allowDefault,
  onChange,
}: {
  field: RowFieldDraft;
  allowDefault: boolean;
  onChange: (mode: RowFieldDraft["mode"]) => void;
}) {
  const options: Array<{ value: RowFieldDraft["mode"]; label: string }> = [
    ...(field.column.identity === "always" ? [] : [{ value: "value" as const, label: "Value" }]),
    ...(field.column.nullable ? [{ value: "null" as const, label: "NULL" }] : []),
    ...(allowDefault ? [{ value: "default" as const, label: "DEFAULT" }] : []),
  ];
  if (options.length === 0) {
    options.push({ value: "default", label: "DEFAULT" });
  }
  return (
    <Select value={field.mode} onValueChange={(value) => onChange(value as RowFieldDraft["mode"])}>
      <SelectTrigger
        aria-label={`${field.column.name} input mode`}
        className="h-8 w-[104px] bg-card text-[11px]"
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function FieldList({
  fields,
  allowDefault,
  onChange,
}: {
  fields: RowFieldDraft[];
  allowDefault: boolean;
  onChange: (index: number, patch: Partial<Pick<RowFieldDraft, "mode" | "value">>) => void;
}) {
  if (fields.length === 0) {
    return (
      <p className="rounded-md border border-border bg-muted/20 px-3 py-2 text-[12px] text-muted-foreground">
        This table has no writable columns. Generated and identity-only tables stay read-only.
      </p>
    );
  }

  return (
    <div className="max-h-[46vh] divide-y divide-border overflow-y-auto rounded-md border border-border bg-card">
      {fields.map((field, index) => {
        const identityNote =
          field.column.identity === "always"
            ? "Generated always; an explicit value is rejected."
            : field.column.identity === "auto_increment"
              ? "AUTO_INCREMENT; DEFAULT lets the engine assign the next value."
              : null;
        return (
          <div key={field.column.name} className="space-y-1.5 px-3 py-2.5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="min-w-0">
                <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                  <span className="truncate font-mono text-[11px] font-medium text-foreground">
                    {field.column.name}
                  </span>
                  <Badge variant="outline" className="font-mono text-[9px]">
                    {field.column.dataType}
                  </Badge>
                  {field.column.primaryKey ? <Badge variant="warning">PK</Badge> : null}
                  {field.column.generated ? <Badge variant="secondary">generated</Badge> : null}
                  {!field.column.nullable ? <Badge variant="secondary">required</Badge> : null}
                  {field.truncated ? <Badge variant="warning">truncated</Badge> : null}
                </div>
                {identityNote ? (
                  <p className="mt-0.5 text-[10px] text-muted-foreground">{identityNote}</p>
                ) : null}
              </div>
              <FieldModeSelect
                field={field}
                allowDefault={allowDefault}
                onChange={(mode) => onChange(index, { mode })}
              />
            </div>
            {fieldInput(field, (patch) => onChange(index, patch))}
            {field.truncated ? (
              <p role="note" className="text-[10px] text-warning">
                The server returned only the first part of this value. Open the full value before
                editing it.
              </p>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

export type InsertRowDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  tableName: string;
  columns: DatabaseRowColumn[];
  pending: boolean;
  error: string | null;
  onInsert: (values: DatabaseRowWriteValue[]) => void;
};

export function InsertRowDialog({
  open,
  onOpenChange,
  tableName,
  columns,
  pending,
  error,
  onInsert,
}: InsertRowDialogProps) {
  const [fields, setFields] = useState<RowFieldDraft[]>(() => insertFields(columns));

  function change(index: number, patch: Partial<Pick<RowFieldDraft, "mode" | "value">>) {
    setFields((current) => updateField(current, index, patch));
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (!pending ? onOpenChange(next) : undefined)}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Insert row into {tableName}</DialogTitle>
          <DialogDescription>
            Values are bound as typed parameters. Choose Value, NULL, or DEFAULT per column; an
            empty value is an empty string, not NULL.
          </DialogDescription>
        </DialogHeader>

        {error ? (
          <Alert variant="destructive" className="py-2.5">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}

        <FieldList fields={fields} allowDefault onChange={change} />

        <DialogFooter>
          <Button
            type="button"
            variant="secondary"
            onClick={() => onOpenChange(false)}
            disabled={pending}
          >
            Cancel
          </Button>
          <Button
            type="button"
            onClick={() => onInsert(buildInsertValues(fields))}
            disabled={pending || fields.length === 0}
          >
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <PlusIcon className="size-3.5" aria-hidden="true" />
            )}
            Insert row
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export type EditRowDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  tableName: string;
  result: DatabaseRowReadResult;
  record: DatabaseRowRecord | null;
  pending: boolean;
  error: string | null;
  onUpdate: (
    identity: DatabaseRowIdentityValue[],
    payload: ReturnType<typeof buildUpdatePayload>,
  ) => void;
};

export function EditRowDialog({
  open,
  onOpenChange,
  tableName,
  result,
  record,
  pending,
  error,
  onUpdate,
}: EditRowDialogProps) {
  const [fields, setFields] = useState<RowFieldDraft[]>(() =>
    record ? editFields(result.columns, result, record) : [],
  );
  const [localError, setLocalError] = useState<string | null>(null);

  function change(index: number, patch: Partial<Pick<RowFieldDraft, "mode" | "value">>) {
    setFields((current) => updateField(current, index, patch));
  }

  function submit() {
    if (!record) {
      return;
    }
    const payload = buildUpdatePayload(fields);
    if (typeof payload === "string") {
      setLocalError(payload);
      return;
    }
    setLocalError(null);
    onUpdate(identityValues(record), payload);
  }

  const identity = record?.identity ?? [];

  return (
    <Dialog open={open} onOpenChange={(next) => (!pending ? onOpenChange(next) : undefined)}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Edit row in {tableName}</DialogTitle>
          <DialogDescription>
            The update is addressed by the stable row identity. Every changed field keeps its
            original value as a guard, so a concurrent change rolls back instead of overwriting
            newer data.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-wrap items-center gap-1.5 rounded-md border border-border bg-muted/20 px-3 py-2">
          <span className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            Identity
          </span>
          {identity.map((entry) => (
            <Badge key={entry.column} variant="outline" className="font-mono text-[10px]">
              {entry.column}={entry.value ?? "NULL"}
            </Badge>
          ))}
        </div>

        {error || localError ? (
          <Alert variant="destructive" className="py-2.5">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <AlertDescription>{error ?? localError}</AlertDescription>
          </Alert>
        ) : null}

        <FieldList fields={fields} allowDefault={false} onChange={change} />

        <DialogFooter>
          <Button
            type="button"
            variant="secondary"
            onClick={() => onOpenChange(false)}
            disabled={pending}
          >
            Cancel
          </Button>
          <Button type="button" onClick={submit} disabled={pending || !record}>
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <CheckCircleIcon className="size-3.5" aria-hidden="true" />
            )}
            Save changes
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function RowWriteResultBanner({
  result,
  errorMessage,
  errorDetails,
  onDismiss,
  onRetry,
}: {
  result: DatabaseRowWriteResult | null;
  errorMessage: string | null;
  errorDetails?: string | null;
  onDismiss: () => void;
  onRetry?: () => void;
}) {
  if (!result && !errorMessage) {
    return null;
  }

  if (errorMessage) {
    const rolledBack = errorDetails?.includes("rolled back") ?? false;
    return (
      <Alert variant="destructive" className="py-2.5" aria-label="Row write failed">
        <ProhibitIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div className="flex-1">
          <AlertTitle className="text-[12px]">
            {rolledBack
              ? "Row write failed — transaction rolled back"
              : "Row write failed — no transaction committed"}
          </AlertTitle>
          <AlertDescription className="mb-0">{errorMessage}</AlertDescription>
          {errorDetails ? (
            <p className="mt-1 font-mono text-[10px] break-words text-muted-foreground">
              {errorDetails}
            </p>
          ) : null}
          <p className="mt-1 text-[10px] text-muted-foreground">
            {rolledBack
              ? "No rows were changed. Re-read the table before retrying."
              : "No rows were reported as committed. Re-read the table before retrying."}
          </p>
        </div>
        <div className="flex shrink-0 items-start gap-1">
          {onRetry ? (
            <Button type="button" variant="ghost" size="sm" onClick={onRetry}>
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
              Refresh
            </Button>
          ) : null}
          <Button type="button" variant="ghost" size="sm" onClick={onDismiss}>
            Dismiss
          </Button>
        </div>
      </Alert>
    );
  }

  if (!result) {
    return null;
  }

  return (
    <Alert variant="success" className="py-2.5" aria-label="Row write succeeded">
      <CheckCircleIcon className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
      <div className="flex-1">
        <AlertTitle className="text-[12px]">
          Committed · {result.affectedRows} row{result.affectedRows === 1 ? "" : "s"} affected
        </AlertTitle>
        <AlertDescription className={cn("mb-0", "flex flex-wrap items-center gap-2")}>
          <Badge variant={result.verified ? "success" : "warning"}>
            {result.verified ? "Verified" : "Unverified"}
          </Badge>
          <span>{result.verification}</span>
        </AlertDescription>
      </div>
      <div className="flex shrink-0 items-start gap-1">
        <Button type="button" variant="ghost" size="sm" onClick={onDismiss}>
          Dismiss
        </Button>
      </div>
    </Alert>
  );
}

export function RowWriteInfo() {
  return (
    <p className="flex items-start gap-1.5 text-[10px] leading-4 text-muted-foreground">
      <InfoIcon className="mt-0.5 size-3 shrink-0" aria-hidden="true" />
      Writes run in a transaction, verify the affected row count, re-read the row where safe, and
      commit only after verification. History records the action, table, and outcome, never row
      values.
    </p>
  );
}
