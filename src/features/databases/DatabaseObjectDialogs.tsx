import { useEffect, useMemo, useState } from "react";
import { WarningCircleIcon } from "@phosphor-icons/react";
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
import { OutputPanel } from "@/components/ui/output-panel";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import {
  objectActionKindLabel,
  sequenceDataTypeLabel,
  triggerEventLabel,
} from "@/features/databases/databaseObjectModel";
import { getCommandErrorMessage } from "@/lib/errors";
import { previewDatabaseObjectAction, runDatabaseObjectAction } from "@/services/databaseObjects";
import type {
  DatabaseEngine,
  DatabaseObjectActionRequest,
  DatabaseObjectActionResult,
  DatabaseObjectDefinition,
  DatabaseObjectDdlPreview,
  DatabaseObjectKind,
  DatabaseRoutineKind,
  DatabaseSequenceDataType,
  DatabaseTriggerEvent,
  DatabaseTriggerTiming,
} from "@/types/contracts";

export type DatabaseObjectIntent =
  | {
      type: "create";
      kind: Extract<DatabaseObjectKind, "view" | "routine" | "trigger" | "sequence">;
      database: string | null;
      schema: string | null;
    }
  | { type: "edit"; definition: DatabaseObjectDefinition }
  | { type: "drop"; definition: DatabaseObjectDefinition }
  | { type: "refresh"; definition: DatabaseObjectDefinition }
  | { type: "alter_sequence"; definition: DatabaseObjectDefinition };

type DatabaseObjectActionHostProps = {
  profileId: string | null;
  profileName: string;
  engine: DatabaseEngine;
  connected: boolean;
  readOnly: boolean;
  intent: DatabaseObjectIntent | null;
  onClose: () => void;
  onCompleted: (result: DatabaseObjectActionResult) => void;
};

type FormState = {
  name: string;
  query: string;
  routineKind: DatabaseRoutineKind;
  arguments: string;
  returnType: string;
  language: string;
  body: string;
  table: string;
  timing: DatabaseTriggerTiming;
  events: DatabaseTriggerEvent[];
  functionSchema: string;
  functionName: string;
  functionArguments: string;
  sequenceDataType: DatabaseSequenceDataType;
  start: string;
  increment: string;
  minimum: string;
  maximum: string;
  cache: string;
  cycle: boolean;
  restart: string;
  cycleChange: "unchanged" | "yes" | "no";
};

const EMPTY_FORM: FormState = {
  name: "",
  query: "SELECT ",
  routineKind: "function",
  arguments: "",
  returnType: "",
  language: "sql",
  body: "BEGIN\n  -- routine body\nEND",
  table: "",
  timing: "after",
  events: ["insert"],
  functionSchema: "",
  functionName: "",
  functionArguments: "",
  sequenceDataType: "big_int",
  start: "1",
  increment: "1",
  minimum: "",
  maximum: "",
  cache: "",
  cycle: false,
  restart: "",
  cycleChange: "unchanged",
};

function numberValue(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  const parsed = Number(trimmed);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function formForIntent(intent: DatabaseObjectIntent | null): FormState {
  if (!intent || intent.type === "create" || intent.type === "drop" || intent.type === "refresh") {
    return { ...EMPTY_FORM };
  }
  const definition = intent.definition;
  if (intent.type === "edit" && definition.kind === "view") {
    return {
      ...EMPTY_FORM,
      name: definition.name,
      query: definition.viewQuery.status === "available" ? definition.viewQuery.value : "SELECT ",
    };
  }
  if (intent.type === "edit" && definition.kind === "routine") {
    return {
      ...EMPTY_FORM,
      name: definition.name,
      routineKind:
        definition.routineKind.status === "available" ? definition.routineKind.value : "function",
      arguments: definition.arguments.status === "available" ? definition.arguments.value : "",
      returnType: definition.returnType.status === "available" ? definition.returnType.value : "",
      language: definition.language.status === "available" ? definition.language.value : "sql",
      body: definition.routineBody.status === "available" ? definition.routineBody.value : "",
    };
  }
  if (intent.type === "edit" && definition.kind === "trigger" && definition.trigger) {
    const trigger = definition.trigger;
    const functionName =
      trigger.functionName.status === "available"
        ? trigger.functionName.value
        : trigger.function.status === "available"
          ? trigger.function.value
          : "";
    return {
      ...EMPTY_FORM,
      name: definition.name,
      table: trigger.table,
      timing: trigger.timing,
      events: trigger.events,
      body:
        trigger.statement.status === "available"
          ? trigger.statement.value
          : "BEGIN\n  -- trigger body\nEND",
      functionName: /^[A-Za-z_][A-Za-z0-9_$]*$/.test(functionName) ? functionName : "",
    };
  }
  if (intent.type === "alter_sequence" && definition.sequence) {
    return {
      ...EMPTY_FORM,
      name: definition.name,
      increment:
        definition.sequence.increment.status === "available"
          ? definition.sequence.increment.value
          : "",
      minimum:
        definition.sequence.minimum.status === "available" ? definition.sequence.minimum.value : "",
      maximum:
        definition.sequence.maximum.status === "available" ? definition.sequence.maximum.value : "",
      cache:
        definition.sequence.cache.status === "available" ? definition.sequence.cache.value : "",
    };
  }
  return { ...EMPTY_FORM };
}

function requestForIntent(
  intent: DatabaseObjectIntent,
  form: FormState,
  engine: DatabaseEngine,
): DatabaseObjectActionRequest {
  const scope = {
    database: intent.type === "create" ? intent.database : (intent.definition.database ?? null),
    schema: intent.type === "create" ? intent.schema : (intent.definition.schema ?? null),
  };
  if (intent.type === "drop") {
    const definition = intent.definition;
    if (definition.kind === "materialized_view") {
      return { kind: "drop_materialized_view", name: definition.name, ...scope };
    }
    if (definition.kind === "routine") {
      return {
        kind: "drop_routine",
        name: definition.name,
        routineKind:
          definition.routineKind.status === "available" ? definition.routineKind.value : "function",
        arguments: definition.arguments.status === "available" ? definition.arguments.value : null,
        ...scope,
      };
    }
    if (definition.kind === "trigger") {
      return {
        kind: "drop_trigger",
        name: definition.name,
        table: definition.trigger?.table ?? form.table,
        ...scope,
      };
    }
    if (definition.kind === "sequence") {
      return { kind: "drop_sequence", name: definition.name, ...scope };
    }
    return { kind: "drop_view", name: definition.name, materialized: false, ...scope };
  }
  if (intent.type === "refresh") {
    return { kind: "refresh_materialized_view", name: intent.definition.name, ...scope };
  }
  if (intent.type === "alter_sequence") {
    return {
      kind: "alter_sequence",
      name: form.name,
      increment: numberValue(form.increment),
      minimum: numberValue(form.minimum),
      maximum: numberValue(form.maximum),
      restart: numberValue(form.restart),
      cache: numberValue(form.cache),
      cycle: form.cycleChange === "unchanged" ? null : form.cycleChange === "yes",
      ...scope,
    };
  }

  const kind = intent.type === "create" ? intent.kind : intent.definition.kind;
  if (kind === "view") {
    return {
      kind: "create_view",
      name: form.name,
      query: form.query,
      orReplace: intent.type === "edit",
      ...scope,
    };
  }
  if (kind === "routine") {
    return {
      kind: "create_routine",
      name: form.name,
      routineKind: form.routineKind,
      arguments: form.arguments || null,
      returnType: form.routineKind === "function" ? form.returnType || null : null,
      language: form.language || null,
      body: form.body,
      orReplace: intent.type === "edit",
      ...scope,
    };
  }
  if (kind === "trigger") {
    return {
      kind: "create_trigger",
      name: form.name,
      table: form.table,
      timing: form.timing,
      events: form.events,
      body:
        engine === "mysql"
          ? { kind: "statement", body: form.body }
          : {
              kind: "function",
              schema: form.functionSchema || null,
              name: form.functionName,
              arguments: form.functionArguments || null,
            },
      orReplace: intent.type === "edit",
      ...scope,
    };
  }
  return {
    kind: "create_sequence",
    name: form.name,
    dataType: form.sequenceDataType,
    start: numberValue(form.start),
    increment: numberValue(form.increment),
    minimum: numberValue(form.minimum),
    maximum: numberValue(form.maximum),
    cache: numberValue(form.cache),
    cycle: form.cycle,
    ...scope,
  };
}

function intentTitle(intent: DatabaseObjectIntent): string {
  if (intent.type === "create") {
    return `Create ${intent.kind.replace(/_/g, " ")}`;
  }
  if (intent.type === "edit") {
    return `Edit ${intent.definition.kind.replace(/_/g, " ")}`;
  }
  if (intent.type === "drop") {
    return `Drop ${intent.definition.kind.replace(/_/g, " ")}`;
  }
  if (intent.type === "refresh") {
    return "Refresh materialized view";
  }
  return "Alter sequence";
}

function DatabaseObjectActionHost({
  profileId,
  profileName,
  engine,
  connected,
  readOnly,
  intent,
  onClose,
  onCompleted,
}: DatabaseObjectActionHostProps) {
  const [form, setForm] = useState<FormState>(() => formForIntent(intent));
  const [preview, setPreview] = useState<DatabaseObjectDdlPreview | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [previewing, setPreviewing] = useState(false);

  useEffect(() => {
    if (!intent || (intent.type !== "drop" && intent.type !== "refresh")) {
      return;
    }
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) {
        return;
      }
      setPreviewing(true);
      void (async () => {
        try {
          if (!profileId) {
            throw new Error("Select a database connection first.");
          }
          const next = await previewDatabaseObjectAction(
            profileId,
            requestForIntent(intent, formForIntent(intent), engine),
          );
          if (!cancelled) {
            setPreview(next);
          }
        } catch (previewError) {
          if (!cancelled) {
            setError(getCommandErrorMessage(previewError));
          }
        } finally {
          if (!cancelled) {
            setPreviewing(false);
          }
        }
      })();
    });
    return () => {
      cancelled = true;
    };
  }, [intent, profileId, engine]);

  const canSubmit = useMemo(() => {
    if (!intent || !profileId || !connected || readOnly) {
      return false;
    }
    if (intent.type === "drop" || intent.type === "refresh") {
      return preview !== null;
    }
    if (!form.name.trim()) {
      return false;
    }
    if (intent.type === "alter_sequence") {
      return true;
    }
    const kind = intent.type === "create" ? intent.kind : intent.definition.kind;
    if (kind === "view") {
      return form.query.trim().length > 0;
    }
    if (kind === "routine") {
      return (
        form.body.trim().length > 0 &&
        (form.routineKind !== "function" || Boolean(form.returnType.trim()))
      );
    }
    if (kind === "trigger") {
      return (
        form.table.trim().length > 0 &&
        form.events.length > 0 &&
        (engine === "mysql" ? form.body.trim().length > 0 : Boolean(form.functionName.trim()))
      );
    }
    return true;
  }, [connected, engine, form, intent, preview, profileId, readOnly]);

  function update<K extends keyof FormState>(key: K, value: FormState[K]): void {
    setForm((current) => ({ ...current, [key]: value }));
  }

  function toggleEvent(event: DatabaseTriggerEvent): void {
    setForm((current) => ({
      ...current,
      events: current.events.includes(event)
        ? current.events.filter((entry) => entry !== event)
        : [...current.events, event],
    }));
  }

  async function loadPreview(): Promise<void> {
    if (!intent || !profileId) {
      return;
    }
    setPreviewing(true);
    setError(null);
    try {
      const request = requestForIntent(intent, form, engine);
      const next = await previewDatabaseObjectAction(profileId, request);
      setPreview(next);
    } catch (previewError) {
      setError(getCommandErrorMessage(previewError));
    } finally {
      setPreviewing(false);
    }
  }

  async function execute(confirmed: boolean): Promise<void> {
    if (!intent || !profileId) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      const request = requestForIntent(intent, form, engine);
      const result = await runDatabaseObjectAction(profileId, request, confirmed);
      onCompleted(result);
      onClose();
    } catch (executionError) {
      setError(getCommandErrorMessage(executionError));
    } finally {
      setPending(false);
      setConfirmOpen(false);
    }
  }

  function handleRun(): void {
    if (!preview) {
      return;
    }
    if (preview.requiresConfirmation) {
      setConfirmOpen(true);
      return;
    }
    void execute(false);
  }

  const kind =
    intent?.type === "create"
      ? intent.kind
      : intent && "definition" in intent
        ? intent.definition.kind
        : null;
  const title = intent ? intentTitle(intent) : "";
  const showForm = intent && intent.type !== "drop" && intent.type !== "refresh";
  const blockedReason = !connected
    ? `Connect ${profileName} before changing objects.`
    : readOnly
      ? "This database profile is read-only. Save a writable profile before changing objects."
      : null;

  return (
    <>
      <Dialog
        open={intent !== null}
        onOpenChange={(open) => {
          if (!open && !pending) {
            onClose();
          }
        }}
      >
        <DialogContent className="max-h-[92vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>
              {intent?.type === "drop"
                ? "Review the generated statement before dropping this object."
                : intent?.type === "refresh"
                  ? "Review the refresh statement before running it."
                  : "Definitions are sent only to the selected database. Review the generated DDL before execution."}
            </DialogDescription>
          </DialogHeader>

          {blockedReason ? (
            <Alert variant="warning">
              <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <div>
                <AlertTitle>Action unavailable</AlertTitle>
                <AlertDescription>{blockedReason}</AlertDescription>
              </div>
            </Alert>
          ) : null}

          {error ? (
            <Alert variant="destructive">
              <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <div>
                <AlertTitle>Object action failed</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </div>
            </Alert>
          ) : null}

          {showForm ? (
            <div className="grid gap-3">
              <div className="grid gap-1.5">
                <Label htmlFor="object-name">Name</Label>
                <Input
                  id="object-name"
                  value={form.name}
                  onChange={(event) => update("name", event.currentTarget.value)}
                  disabled={intent?.type === "edit"}
                  className="font-mono"
                />
              </div>

              {kind === "view" ? (
                <div className="grid gap-1.5">
                  <Label htmlFor="object-query">SELECT query</Label>
                  <Textarea
                    id="object-query"
                    value={form.query}
                    onChange={(event) => update("query", event.currentTarget.value)}
                    className="min-h-40 font-mono text-[12px]"
                  />
                  <p className="text-[11px] text-muted-foreground">
                    Exactly one SELECT, WITH, VALUES, or TABLE statement. The backend rejects
                    multi-statement input.
                  </p>
                </div>
              ) : null}

              {kind === "routine" ? (
                <>
                  <div className="grid grid-cols-2 gap-3">
                    <div className="grid gap-1.5">
                      <Label>Routine kind</Label>
                      <Select
                        value={form.routineKind}
                        onValueChange={(value) =>
                          update("routineKind", value as DatabaseRoutineKind)
                        }
                        disabled={intent?.type === "edit"}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="function">Function</SelectItem>
                          <SelectItem value="procedure">Procedure</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="grid gap-1.5">
                      <Label htmlFor="routine-language">Language</Label>
                      <Input
                        id="routine-language"
                        value={form.language}
                        onChange={(event) => update("language", event.currentTarget.value)}
                        className="font-mono"
                      />
                    </div>
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="routine-arguments">Arguments</Label>
                    <Input
                      id="routine-arguments"
                      value={form.arguments}
                      onChange={(event) => update("arguments", event.currentTarget.value)}
                      placeholder="user_id bigint"
                      className="font-mono"
                    />
                  </div>
                  {form.routineKind === "function" ? (
                    <div className="grid gap-1.5">
                      <Label htmlFor="routine-return">Return type</Label>
                      <Input
                        id="routine-return"
                        value={form.returnType}
                        onChange={(event) => update("returnType", event.currentTarget.value)}
                        placeholder="text"
                        className="font-mono"
                      />
                    </div>
                  ) : null}
                  <div className="grid gap-1.5">
                    <Label htmlFor="routine-body">Body</Label>
                    <Textarea
                      id="routine-body"
                      value={form.body}
                      onChange={(event) => update("body", event.currentTarget.value)}
                      className="min-h-44 font-mono text-[12px]"
                    />
                  </div>
                </>
              ) : null}

              {kind === "trigger" ? (
                <>
                  <div className="grid grid-cols-2 gap-3">
                    <div className="grid gap-1.5">
                      <Label htmlFor="trigger-table">Target table</Label>
                      <Input
                        id="trigger-table"
                        value={form.table}
                        onChange={(event) => update("table", event.currentTarget.value)}
                        className="font-mono"
                      />
                    </div>
                    <div className="grid gap-1.5">
                      <Label>Timing</Label>
                      <Select
                        value={form.timing}
                        onValueChange={(value) => update("timing", value as DatabaseTriggerTiming)}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="before">Before</SelectItem>
                          <SelectItem value="after">After</SelectItem>
                          {engine === "postgresql" ? (
                            <SelectItem value="instead_of">Instead of</SelectItem>
                          ) : null}
                        </SelectContent>
                      </Select>
                    </div>
                  </div>
                  <div className="grid gap-1.5">
                    <Label>Events</Label>
                    <div className="flex flex-wrap gap-2">
                      {(["insert", "update", "delete", "truncate"] as DatabaseTriggerEvent[]).map(
                        (event) => (
                          <label
                            key={event}
                            className="flex items-center gap-1.5 rounded-sm border border-border px-2 py-1 text-[11px]"
                          >
                            <input
                              type="checkbox"
                              checked={form.events.includes(event)}
                              disabled={engine === "mysql" && event === "truncate"}
                              onChange={() => toggleEvent(event)}
                            />
                            {triggerEventLabel(event)}
                          </label>
                        ),
                      )}
                    </div>
                    {engine === "mysql" ? (
                      <p className="text-[11px] text-muted-foreground">
                        MySQL supports one event per trigger.
                      </p>
                    ) : null}
                  </div>
                  {engine === "mysql" ? (
                    <div className="grid gap-1.5">
                      <Label htmlFor="trigger-body">Statement body</Label>
                      <Textarea
                        id="trigger-body"
                        value={form.body}
                        onChange={(event) => update("body", event.currentTarget.value)}
                        className="min-h-36 font-mono text-[12px]"
                      />
                    </div>
                  ) : (
                    <>
                      <div className="grid grid-cols-2 gap-3">
                        <div className="grid gap-1.5">
                          <Label htmlFor="trigger-function-schema">Function schema</Label>
                          <Input
                            id="trigger-function-schema"
                            value={form.functionSchema}
                            onChange={(event) =>
                              update("functionSchema", event.currentTarget.value)
                            }
                            className="font-mono"
                          />
                        </div>
                        <div className="grid gap-1.5">
                          <Label htmlFor="trigger-function">Function name</Label>
                          <Input
                            id="trigger-function"
                            value={form.functionName}
                            onChange={(event) => update("functionName", event.currentTarget.value)}
                            className="font-mono"
                          />
                        </div>
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="trigger-function-args">Function arguments</Label>
                        <Input
                          id="trigger-function-args"
                          value={form.functionArguments}
                          onChange={(event) =>
                            update("functionArguments", event.currentTarget.value)
                          }
                          className="font-mono"
                        />
                      </div>
                    </>
                  )}
                </>
              ) : null}

              {kind === "sequence" && intent?.type === "create" ? (
                <>
                  <div className="grid grid-cols-2 gap-3">
                    <div className="grid gap-1.5">
                      <Label>Data type</Label>
                      <Select
                        value={form.sequenceDataType}
                        onValueChange={(value) =>
                          update("sequenceDataType", value as DatabaseSequenceDataType)
                        }
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {(["small_int", "integer", "big_int"] as DatabaseSequenceDataType[]).map(
                            (dataType) => (
                              <SelectItem key={dataType} value={dataType}>
                                {sequenceDataTypeLabel(dataType)}
                              </SelectItem>
                            ),
                          )}
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="grid gap-1.5">
                      <Label htmlFor="sequence-start">Start</Label>
                      <Input
                        id="sequence-start"
                        type="number"
                        value={form.start}
                        onChange={(event) => update("start", event.currentTarget.value)}
                      />
                    </div>
                    <div className="grid gap-1.5">
                      <Label htmlFor="sequence-increment">Increment</Label>
                      <Input
                        id="sequence-increment"
                        type="number"
                        value={form.increment}
                        onChange={(event) => update("increment", event.currentTarget.value)}
                      />
                    </div>
                    <div className="grid gap-1.5">
                      <Label htmlFor="sequence-minimum">Minimum</Label>
                      <Input
                        id="sequence-minimum"
                        type="number"
                        value={form.minimum}
                        onChange={(event) => update("minimum", event.currentTarget.value)}
                      />
                    </div>
                    <div className="grid gap-1.5">
                      <Label htmlFor="sequence-maximum">Maximum</Label>
                      <Input
                        id="sequence-maximum"
                        type="number"
                        value={form.maximum}
                        onChange={(event) => update("maximum", event.currentTarget.value)}
                      />
                    </div>
                    <div className="grid gap-1.5">
                      <Label htmlFor="sequence-cache">Cache</Label>
                      <Input
                        id="sequence-cache"
                        type="number"
                        value={form.cache}
                        onChange={(event) => update("cache", event.currentTarget.value)}
                      />
                    </div>
                  </div>
                  <label className="flex items-center gap-2 text-[12px]">
                    <input
                      type="checkbox"
                      checked={form.cycle}
                      onChange={(event) => update("cycle", event.currentTarget.checked)}
                    />
                    Cycle after reaching the maximum
                  </label>
                </>
              ) : null}

              {intent?.type === "alter_sequence" ? (
                <div className="grid grid-cols-2 gap-3">
                  <div className="grid gap-1.5">
                    <Label htmlFor="sequence-increment">Increment</Label>
                    <Input
                      id="sequence-increment"
                      type="number"
                      value={form.increment}
                      onChange={(event) => update("increment", event.currentTarget.value)}
                    />
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="sequence-minimum">Minimum</Label>
                    <Input
                      id="sequence-minimum"
                      type="number"
                      value={form.minimum}
                      onChange={(event) => update("minimum", event.currentTarget.value)}
                    />
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="sequence-maximum">Maximum</Label>
                    <Input
                      id="sequence-maximum"
                      type="number"
                      value={form.maximum}
                      onChange={(event) => update("maximum", event.currentTarget.value)}
                    />
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="sequence-restart">Restart with</Label>
                    <Input
                      id="sequence-restart"
                      type="number"
                      value={form.restart}
                      onChange={(event) => update("restart", event.currentTarget.value)}
                    />
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="sequence-cache">Cache</Label>
                    <Input
                      id="sequence-cache"
                      type="number"
                      value={form.cache}
                      onChange={(event) => update("cache", event.currentTarget.value)}
                    />
                  </div>
                  <div className="grid gap-1.5">
                    <Label>Cycle</Label>
                    <Select
                      value={form.cycleChange}
                      onValueChange={(value) =>
                        update("cycleChange", value as FormState["cycleChange"])
                      }
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="unchanged">Unchanged</SelectItem>
                        <SelectItem value="yes">Enable</SelectItem>
                        <SelectItem value="no">Disable</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              ) : null}
            </div>
          ) : null}

          {preview ? (
            <div className="space-y-2">
              <div className="flex items-center justify-between gap-2">
                <p className="text-[11px] font-semibold">
                  {objectActionKindLabel(preview.action)} · {preview.statements.length} statement
                  {preview.statements.length === 1 ? "" : "s"}
                </p>
                {preview.requiresConfirmation ? (
                  <span className="rounded-sm border border-warning/40 bg-warning/10 px-1.5 py-0.5 text-[10px] text-warning">
                    Confirmation required
                  </span>
                ) : null}
              </div>
              <OutputPanel
                title="Generated DDL"
                content={preview.ddl}
                maxHeightClassName="max-h-64"
              />
              {preview.warnings.length > 0 ? (
                <Alert variant="warning" className="py-2.5">
                  <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                  <div>
                    <AlertTitle>Review before running</AlertTitle>
                    <AlertDescription>
                      <ul className="list-disc space-y-1 pl-4">
                        {preview.warnings.map((warning) => (
                          <li key={warning}>{warning}</li>
                        ))}
                      </ul>
                    </AlertDescription>
                  </div>
                </Alert>
              ) : null}
            </div>
          ) : null}

          {previewing ? (
            <p role="status" className="text-[12px] text-muted-foreground">
              Generating preview…
            </p>
          ) : null}

          <DialogFooter>
            <Button variant="secondary" onClick={onClose} disabled={pending}>
              Cancel
            </Button>
            {showForm && !preview ? (
              <Button onClick={() => void loadPreview()} disabled={!canSubmit || previewing}>
                {previewing ? "Generating…" : "Preview DDL"}
              </Button>
            ) : (
              <>
                {showForm ? (
                  <Button variant="secondary" onClick={() => setPreview(null)} disabled={pending}>
                    Back
                  </Button>
                ) : null}
                <Button
                  variant={preview?.destructive ? "destructive" : "default"}
                  onClick={handleRun}
                  disabled={!preview || !canSubmit || pending}
                >
                  {pending
                    ? "Running…"
                    : intent?.type === "refresh"
                      ? "Refresh view"
                      : preview?.destructive
                        ? "Run drop"
                        : "Run DDL"}
                </Button>
              </>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title={`Confirm ${preview?.action ? objectActionKindLabel(preview.action).toLowerCase() : "object change"}`}
        description={`This action changes ${profileName}. The exact DDL shown in the preview will run after confirmation.`}
        confirmLabel={preview?.destructive ? "Drop object" : "Run replacement"}
        destructive={preview?.destructive ?? false}
        pending={pending}
        details={[
          { label: "Connection", value: profileName },
          { label: "Object", value: preview?.object.name ?? "" },
          {
            label: "Action",
            value: preview?.action ? objectActionKindLabel(preview.action) : "",
          },
        ]}
        preview={preview ? { label: "Generated DDL", text: preview.ddl } : null}
        warningTitle="This cannot be undone."
        warningDescription={
          preview?.destructive
            ? "Dropping the object permanently removes its definition and may break dependents."
            : "Replacing an object changes its definition immediately for every caller."
        }
        onConfirm={() => void execute(true)}
      />
    </>
  );
}

export { DatabaseObjectActionHost };
