import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  CircleNotchIcon,
  CodeIcon,
  DatabaseIcon,
  KeyIcon,
  PlusIcon,
  TableIcon,
  TrashIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
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
import { LoadingState } from "@/components/ui/states";
import { Textarea } from "@/components/ui/textarea";
import {
  columnTypeLabel,
  constraintKindLabel,
  engineLabel,
  indexActionDecisions,
  type DatabaseStructureIntent,
  type DatabaseTableScope,
  type StructureCapabilities,
  type StructureDecision,
} from "@/features/databases/databaseStructureModel";
import type { DatabaseTableStructureState } from "@/hooks/useDatabaseTableStructure";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import {
  previewDatabaseStructureAction,
  runDatabaseStructureAction,
} from "@/services/databaseStructure";
import type {
  CommandError,
  DatabaseColumnDefaultInput,
  DatabaseColumnDefinitionInput,
  DatabaseColumnIdentity,
  DatabaseColumnSummary,
  DatabaseConstraintInput,
  DatabaseConstraintKind,
  DatabaseIndexSummary,
  DatabaseProfile,
  DatabaseReferentialAction,
  DatabaseStructureActionRequest,
  DatabaseStructureActionResult,
  DatabaseStructureDdlPreview,
  DatabaseUniqueKeyInput,
} from "@/types/contracts";

type PreviewState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; preview: DatabaseStructureDdlPreview }
  | { status: "error"; error: CommandError };
function commandErrorFor(error: unknown): CommandError {
  if (isCommandError(error)) {
    return error;
  }
  return { code: "internal", message: getCommandErrorMessage(error) };
}

function errorTitle(error: CommandError, fallback: string): string {
  switch (error.code) {
    case "permission":
      return "Permission denied";
    case "capability":
      return "Not supported";
    case "conflict":
      return "Blocked by dependencies";
    case "validation":
      return "Check the request";
    default:
      return fallback;
  }
}

function ActionErrorAlert({ error, title }: { error: CommandError; title: string }) {
  return (
    <Alert variant="destructive" className="py-2.5" role="alert">
      <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
      <div className="space-y-1">
        <AlertTitle>{title}</AlertTitle>
        <AlertDescription>{error.message}</AlertDescription>
        {error.scope ? (
          <p className="text-[10px] leading-4 text-muted-foreground">
            Scoped to {error.scope.object} {error.scope.name}.
          </p>
        ) : null}
        {error.details ? (
          <p className="text-[10px] leading-4 text-muted-foreground">{error.details}</p>
        ) : null}
      </div>
    </Alert>
  );
}

function ReadOnlyAlert({ decision }: { decision: StructureDecision }) {
  if (decision.enabled || !decision.reason) {
    return null;
  }
  return (
    <Alert variant="warning" className="py-2.5" role="note">
      <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
      <div>
        <AlertTitle>Action unavailable</AlertTitle>
        <AlertDescription>{decision.reason}</AlertDescription>
      </div>
    </Alert>
  );
}

type StructureDialogShellProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  icon?: ReactNode;
  profile: DatabaseProfile;
  request: DatabaseStructureActionRequest | null;
  requestHint: string;
  submitLabel: string;
  decision: StructureDecision;
  onCompleted: (result: DatabaseStructureActionResult) => void;
  children: ReactNode;
};

function StructureDialogShell({
  open,
  onOpenChange,
  title,
  description,
  icon,
  profile,
  request,
  requestHint,
  submitLabel,
  decision,
  onCompleted,
  children,
}: StructureDialogShellProps) {
  const [previewState, setPreviewState] = useState<PreviewState>({ status: "idle" });
  const [previewKey, setPreviewKey] = useState<string | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [actionError, setActionError] = useState<CommandError | null>(null);
  const [pending, setPending] = useState(false);
  const previewToken = useRef(0);
  const requestKey = request ? JSON.stringify(request) : null;
  const runtimeAvailable = isTauriRuntime();

  const currentPreview = previewKey === requestKey;
  const preview = currentPreview && previewState.status === "ready" ? previewState.preview : null;
  const previewError =
    currentPreview && previewState.status === "error" ? previewState.error : null;
  const previewLoading = currentPreview && previewState.status === "loading";
  const canPreview = Boolean(request) && decision.enabled && runtimeAvailable && !pending;
  const canApply =
    Boolean(request) &&
    preview !== null &&
    decision.enabled &&
    runtimeAvailable &&
    !pending &&
    (!preview.requiresConfirmation || confirmed);

  async function handlePreview() {
    if (!request || !canPreview || !requestKey) {
      return;
    }
    const token = previewToken.current + 1;
    previewToken.current = token;
    setPreviewKey(requestKey);
    setPreviewState({ status: "loading" });
    setActionError(null);
    try {
      const result = await previewDatabaseStructureAction(profile.id, request);
      if (previewToken.current !== token) {
        return;
      }
      setConfirmed(false);
      setPreviewState({ status: "ready", preview: result });
    } catch (error) {
      if (previewToken.current !== token) {
        return;
      }
      setPreviewState({ status: "error", error: commandErrorFor(error) });
    }
  }

  async function handleApply() {
    if (!request || !preview || !canApply) {
      return;
    }
    setPending(true);
    setActionError(null);
    try {
      const result = await runDatabaseStructureAction(
        profile.id,
        request,
        preview.requiresConfirmation ? confirmed : false,
      );
      onCompleted(result);
      onOpenChange(false);
    } catch (error) {
      setActionError(commandErrorFor(error));
    } finally {
      setPending(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (!pending ? onOpenChange(next) : undefined)}>
      <DialogContent className="max-h-[88vh] max-w-3xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {icon ?? <TableIcon className="size-4" aria-hidden="true" />}
            {title}
          </DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        {children}

        <ReadOnlyAlert decision={decision} />

        {!runtimeAvailable ? (
          <Alert variant="info" className="py-2.5">
            <DatabaseIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
            <div>
              <AlertTitle>Desktop build required</AlertTitle>
              <AlertDescription>
                DDL previews and structure changes are available in the desktop application.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        <section aria-label="DDL preview" className="space-y-2 border-t border-border pt-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <p className="flex items-center gap-1.5 text-[11px] font-semibold text-foreground">
                <CodeIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
                Generated DDL
              </p>
              <p className="text-[10px] leading-4 text-muted-foreground">
                Preview is required before applying. No statement has run yet.
              </p>
            </div>
            <div className="flex items-center gap-1.5">
              {preview ? <CopyButton value={preview.ddl} label="Copy DDL" /> : null}
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => void handlePreview()}
                disabled={!canPreview}
                title={!runtimeAvailable ? "Available in the desktop application." : requestHint}
              >
                {previewLoading ? (
                  <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
                ) : (
                  <CodeIcon className="size-3.5" aria-hidden="true" />
                )}
                {previewLoading ? "Generating..." : "Preview DDL"}
              </Button>
            </div>
          </div>

          {preview ? (
            <div className="space-y-2">
              <pre
                data-testid="structure-ddl-preview"
                className="max-h-48 overflow-auto rounded-md border border-border bg-muted/45 p-3 font-mono text-[11px] leading-5 whitespace-pre-wrap text-foreground"
              >
                {preview.ddl || "No DDL is required for this action."}
              </pre>
              <div className="flex flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
                <Badge variant="outline">{engineLabel(preview.engine)}</Badge>
                <span>
                  {preview.statements.length} statement
                  {preview.statements.length === 1 ? "" : "s"}
                </span>
                {preview.destructive ? <Badge variant="danger">Destructive</Badge> : null}
              </div>
              {preview.warnings.length > 0 ? (
                <ul className="space-y-1 text-[10px] leading-4 text-muted-foreground">
                  {preview.warnings.map((warning) => (
                    <li key={warning} className="flex gap-1.5">
                      <WarningCircleIcon className="mt-0.5 size-3 shrink-0" aria-hidden="true" />
                      {warning}
                    </li>
                  ))}
                </ul>
              ) : null}
              {preview.requiresConfirmation ? (
                <label className="flex items-start gap-2 rounded-md border border-warning/35 bg-warning/5 px-3 py-2 text-[11px] leading-5 text-foreground">
                  <input
                    type="checkbox"
                    className="mt-0.5 size-4 accent-primary"
                    checked={confirmed}
                    onChange={(event) => setConfirmed(event.currentTarget.checked)}
                  />
                  <span>
                    I understand this change is destructive and want to apply the previewed DDL.
                  </span>
                </label>
              ) : null}
            </div>
          ) : (
            <div className="rounded-md border border-dashed border-border bg-muted/20 px-3 py-4 text-[11px] text-muted-foreground">
              {previewLoading
                ? "Generating the exact statement..."
                : request
                  ? requestHint
                  : "Complete the form to generate the DDL preview."}
            </div>
          )}

          {previewError ? (
            <ActionErrorAlert
              error={previewError}
              title={errorTitle(previewError, "Preview failed")}
            />
          ) : null}
        </section>

        {actionError ? (
          <ActionErrorAlert
            error={actionError}
            title={errorTitle(actionError, "Structure action failed")}
          />
        ) : null}

        <DialogFooter>
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={pending}>
            Cancel
          </Button>
          <Button
            variant={preview?.destructive ? "destructive" : "default"}
            onClick={() => void handleApply()}
            disabled={!canApply}
          >
            {pending ? "Applying..." : submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type NameActionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  fieldLabel: string;
  placeholder: string;
  initialValue: string;
  submitLabel: string;
  profile: DatabaseProfile;
  decision: StructureDecision;
  buildRequest: (value: string) => DatabaseStructureActionRequest | null;
  onCompleted: (result: DatabaseStructureActionResult) => void;
};

function NameActionDialog({
  open,
  onOpenChange,
  title,
  description,
  fieldLabel,
  placeholder,
  initialValue,
  submitLabel,
  profile,
  decision,
  buildRequest,
  onCompleted,
}: NameActionDialogProps) {
  const [value, setValue] = useState(initialValue);
  const trimmed = value.trim();
  const request = trimmed.length > 0 && trimmed !== initialValue ? buildRequest(trimmed) : null;

  return (
    <StructureDialogShell
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
      icon={<TableIcon className="size-4" aria-hidden="true" />}
      profile={profile}
      request={request}
      requestHint="Enter a new name to generate the DDL preview."
      submitLabel={submitLabel}
      decision={decision}
      onCompleted={onCompleted}
    >
      <div>
        <Label htmlFor="structure-name-action">{fieldLabel}</Label>
        <Input
          id="structure-name-action"
          value={value}
          onChange={(event) => setValue(event.currentTarget.value)}
          placeholder={placeholder}
          autoComplete="off"
          className="mt-1 font-mono"
          autoFocus
        />
        <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
          The name must differ from the current name. The backend validates the identifier before
          any statement is generated.
        </p>
      </div>
    </StructureDialogShell>
  );
}

function RenameTableDialog({
  profile,
  scope,
  capabilities,
  onOpenChange,
  onCompleted,
}: {
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  capabilities: StructureCapabilities;
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseStructureActionResult) => void;
}) {
  return (
    <NameActionDialog
      open
      onOpenChange={onOpenChange}
      title={`Rename table ${scope.table}`}
      description="Rename the table without copying rows. The generated DDL is previewed before it runs."
      fieldLabel="New table name"
      placeholder={`${scope.table}_renamed`}
      initialValue={scope.table}
      submitLabel="Rename table"
      profile={profile}
      decision={capabilities.renameTable}
      buildRequest={(newName) => ({
        kind: "rename_table",
        database: scope.database,
        schema: scope.schema,
        name: scope.table,
        newName,
      })}
      onCompleted={onCompleted}
    />
  );
}

function DuplicateTableDialog({
  profile,
  scope,
  capabilities,
  onOpenChange,
  onCompleted,
}: {
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  capabilities: StructureCapabilities;
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseStructureActionResult) => void;
}) {
  return (
    <NameActionDialog
      open
      onOpenChange={onOpenChange}
      title={`Duplicate table ${scope.table}`}
      description="Copy the table definition into a new table. Rows, foreign keys, and data are not copied; the preview states exactly what is included."
      fieldLabel="New table name"
      placeholder={`${scope.table}_copy`}
      initialValue={scope.table}
      submitLabel="Duplicate table"
      profile={profile}
      decision={capabilities.duplicateTable}
      buildRequest={(newName) => ({
        kind: "duplicate_table",
        database: scope.database,
        schema: scope.schema,
        name: scope.table,
        newName,
      })}
      onCompleted={onCompleted}
    />
  );
}

function RenameColumnDialog({
  profile,
  scope,
  column,
  capabilities,
  onOpenChange,
  onCompleted,
}: {
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  column: DatabaseColumnSummary;
  capabilities: StructureCapabilities;
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseStructureActionResult) => void;
}) {
  return (
    <NameActionDialog
      open
      onOpenChange={onOpenChange}
      title={`Rename column ${column.name}`}
      description="Rename one column. Data, defaults, and constraints stay attached to the column."
      fieldLabel="New column name"
      placeholder={`${column.name}_renamed`}
      initialValue={column.name}
      submitLabel="Rename column"
      profile={profile}
      decision={capabilities.renameColumn}
      buildRequest={(newName) => ({
        kind: "rename_column",
        database: scope.database,
        schema: scope.schema,
        table: scope.table,
        name: column.name,
        newName,
      })}
      onCompleted={onCompleted}
    />
  );
}

function RenameIndexDialog({
  profile,
  scope,
  index,
  capabilities,
  onOpenChange,
  onCompleted,
}: {
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  index: DatabaseIndexSummary;
  capabilities: StructureCapabilities;
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseStructureActionResult) => void;
}) {
  return (
    <NameActionDialog
      open
      onOpenChange={onOpenChange}
      title={`Rename index ${index.name}`}
      description="Rename one index. The index definition and its columns are unchanged."
      fieldLabel="New index name"
      placeholder={`${index.name}_renamed`}
      initialValue={index.name}
      submitLabel="Rename index"
      profile={profile}
      decision={indexActionDecisions(index, capabilities).rename}
      buildRequest={(newName) => ({
        kind: "rename_index",
        database: scope.database,
        schema: scope.schema,
        table: scope.table,
        name: index.name,
        newName,
      })}
      onCompleted={onCompleted}
    />
  );
}

type NewColumnRow = {
  key: string;
  name: string;
  dataType: string;
  nullable: boolean;
  primaryKey: boolean;
};

const DEFAULT_COLUMN_ROW: Omit<NewColumnRow, "key"> = {
  name: "",
  dataType: "",
  nullable: true,
  primaryKey: false,
};

function CreateTableDialog({
  profile,
  scope,
  capabilities,
  onOpenChange,
  onCompleted,
}: {
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  capabilities: StructureCapabilities;
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseStructureActionResult) => void;
}) {
  const [name, setName] = useState("");
  const [rows, setRows] = useState<NewColumnRow[]>([{ ...DEFAULT_COLUMN_ROW, key: "column-1" }]);
  const [engine, setEngine] = useState("");
  const [charset, setCharset] = useState("");
  const [collation, setCollation] = useState("");
  const [tablespace, setTablespace] = useState("");
  const [unlogged, setUnlogged] = useState(false);
  const [comment, setComment] = useState("");
  const nextRow = useRef(1);

  function updateRow(key: string, patch: Partial<Omit<NewColumnRow, "key">>) {
    setRows((current) => current.map((row) => (row.key === key ? { ...row, ...patch } : row)));
  }

  function addRow() {
    nextRow.current += 1;
    setRows((current) => [...current, { ...DEFAULT_COLUMN_ROW, key: `column-${nextRow.current}` }]);
  }

  function removeRow(key: string) {
    setRows((current) =>
      current.length <= 1 ? current : current.filter((row) => row.key !== key),
    );
  }

  const request = useMemo<DatabaseStructureActionRequest | null>(() => {
    const tableName = name.trim();
    const columns: DatabaseColumnDefinitionInput[] = rows
      .filter((row) => row.name.trim().length > 0 && row.dataType.trim().length > 0)
      .map((row) => ({
        name: row.name.trim(),
        dataType: row.dataType.trim(),
        nullable: row.primaryKey ? false : row.nullable,
      }));
    if (tableName.length === 0 || columns.length === 0 || columns.length !== rows.length) {
      return null;
    }

    const primaryKey = rows
      .filter((row) => row.primaryKey && row.name.trim().length > 0)
      .map((row) => row.name.trim());
    const uniqueKeys: DatabaseUniqueKeyInput[] = [];
    const options =
      profile.engine === "mysql"
        ? {
            engine: engine.trim() || null,
            charset: charset.trim() || null,
            collation: collation.trim() || null,
            tablespace: null,
            unlogged: false,
            comment: comment.trim() || null,
          }
        : {
            engine: null,
            charset: null,
            collation: null,
            tablespace: tablespace.trim() || null,
            unlogged,
            comment: comment.trim() || null,
          };

    return {
      kind: "create_table",
      database: scope.database,
      schema: scope.schema,
      name: tableName,
      columns,
      primaryKey,
      uniqueKeys,
      options,
    };
  }, [
    charset,
    collation,
    comment,
    engine,
    name,
    profile.engine,
    rows,
    scope.database,
    scope.schema,
    tablespace,
    unlogged,
  ]);

  return (
    <StructureDialogShell
      open
      onOpenChange={onOpenChange}
      title="Create table"
      description={`Create a table in ${scope.database ?? "the active database"}${
        scope.schema ? ` / ${scope.schema}` : ""
      } on ${profile.name}. Columns and engine options are typed before the DDL preview is generated.`}
      icon={<TableIcon className="size-4" aria-hidden="true" />}
      profile={profile}
      request={request}
      requestHint="Enter a table name and at least one complete column to generate the DDL preview."
      submitLabel="Create table"
      decision={capabilities.createTable}
      onCompleted={onCompleted}
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="sm:col-span-2">
          <Label htmlFor="create-table-name">Table name</Label>
          <Input
            id="create-table-name"
            value={name}
            onChange={(event) => setName(event.currentTarget.value)}
            placeholder="users"
            autoComplete="off"
            className="mt-1 font-mono"
            autoFocus
          />
        </div>
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <p className="text-[11px] font-semibold text-foreground">Columns</p>
          <Button type="button" variant="secondary" size="sm" onClick={addRow}>
            <PlusIcon className="size-3.5" aria-hidden="true" />
            Add column
          </Button>
        </div>
        <div className="space-y-2">
          {rows.map((row, index) => (
            <div
              key={row.key}
              className="grid gap-2 rounded-md border border-border bg-muted/15 p-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto_auto_auto]"
            >
              <div>
                <Label htmlFor={`create-table-column-${row.key}-name`}>
                  Column {index + 1} name
                </Label>
                <Input
                  id={`create-table-column-${row.key}-name`}
                  value={row.name}
                  onChange={(event) => updateRow(row.key, { name: event.currentTarget.value })}
                  placeholder="id"
                  autoComplete="off"
                  className="mt-1 font-mono"
                />
              </div>
              <div>
                <Label htmlFor={`create-table-column-${row.key}-type`}>
                  Column {index + 1} type
                </Label>
                <Input
                  id={`create-table-column-${row.key}-type`}
                  value={row.dataType}
                  onChange={(event) => updateRow(row.key, { dataType: event.currentTarget.value })}
                  placeholder={profile.engine === "mysql" ? "bigint unsigned" : "bigint"}
                  autoComplete="off"
                  className="mt-1 font-mono"
                />
              </div>
              <label className="flex items-end gap-1.5 pb-2 text-[10px] text-muted-foreground">
                <input
                  type="checkbox"
                  className="size-4 accent-primary"
                  checked={row.nullable}
                  disabled={row.primaryKey}
                  onChange={(event) =>
                    updateRow(row.key, { nullable: event.currentTarget.checked })
                  }
                />
                Nullable
              </label>
              <label className="flex items-end gap-1.5 pb-2 text-[10px] text-muted-foreground">
                <input
                  type="checkbox"
                  className="size-4 accent-primary"
                  checked={row.primaryKey}
                  onChange={(event) =>
                    updateRow(row.key, {
                      primaryKey: event.currentTarget.checked,
                      nullable: event.currentTarget.checked ? false : row.nullable,
                    })
                  }
                />
                Primary key
              </label>
              <div className="flex items-end pb-1">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-8 px-2 text-[10px] text-destructive hover:bg-destructive/10 hover:text-destructive"
                  disabled={rows.length <= 1}
                  aria-label={`Remove column ${index + 1}`}
                  onClick={() => removeRow(row.key)}
                >
                  Remove
                </Button>
              </div>
            </div>
          ))}
        </div>
        <p className="text-[10px] leading-4 text-muted-foreground">
          Primary-key columns are forced to NOT NULL. Names and types are validated again by the
          Rust planner before any DDL is generated.
        </p>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        {profile.engine === "mysql" ? (
          <>
            <div>
              <Label htmlFor="create-table-engine">Storage engine</Label>
              <Input
                id="create-table-engine"
                value={engine}
                onChange={(event) => setEngine(event.currentTarget.value)}
                placeholder="InnoDB"
                autoComplete="off"
                className="mt-1 font-mono"
              />
            </div>
            <div>
              <Label htmlFor="create-table-charset">Character set</Label>
              <Input
                id="create-table-charset"
                value={charset}
                onChange={(event) => setCharset(event.currentTarget.value)}
                placeholder="utf8mb4"
                autoComplete="off"
                className="mt-1 font-mono"
              />
            </div>
            <div>
              <Label htmlFor="create-table-collation">Collation</Label>
              <Input
                id="create-table-collation"
                value={collation}
                onChange={(event) => setCollation(event.currentTarget.value)}
                placeholder="utf8mb4_unicode_ci"
                autoComplete="off"
                className="mt-1 font-mono"
              />
            </div>
          </>
        ) : (
          <>
            <div>
              <Label htmlFor="create-table-tablespace">Tablespace</Label>
              <Input
                id="create-table-tablespace"
                value={tablespace}
                onChange={(event) => setTablespace(event.currentTarget.value)}
                placeholder="pg_default"
                autoComplete="off"
                className="mt-1 font-mono"
              />
            </div>
            <label className="flex items-center gap-2 self-end pb-2 text-[11px] text-muted-foreground">
              <input
                type="checkbox"
                className="size-4 accent-primary"
                checked={unlogged}
                onChange={(event) => setUnlogged(event.currentTarget.checked)}
              />
              Unlogged table
            </label>
          </>
        )}
        <div className="sm:col-span-2">
          <Label htmlFor="create-table-comment">Table comment</Label>
          <Textarea
            id="create-table-comment"
            value={comment}
            onChange={(event) => setComment(event.currentTarget.value)}
            placeholder="Optional table comment"
            className="mt-1 min-h-16"
          />
        </div>
      </div>
    </StructureDialogShell>
  );
}

type DefaultMode =
  "none" | "unchanged" | "drop" | "null" | "string" | "number" | "boolean" | "current_timestamp";

function defaultRequestValue(
  mode: DefaultMode,
  value: string,
  booleanValue: boolean,
): { defaultInput: DatabaseColumnDefaultInput | null; dropDefault: boolean } {
  switch (mode) {
    case "null":
      return { defaultInput: { kind: "null" }, dropDefault: false };
    case "string":
      return { defaultInput: { kind: "string", value }, dropDefault: false };
    case "number":
      return { defaultInput: { kind: "number", value }, dropDefault: false };
    case "boolean":
      return { defaultInput: { kind: "boolean", value: booleanValue }, dropDefault: false };
    case "current_timestamp":
      return { defaultInput: { kind: "current_timestamp" }, dropDefault: false };
    case "drop":
      return { defaultInput: null, dropDefault: true };
    default:
      return { defaultInput: null, dropDefault: false };
  }
}

function identityOptions(profile: DatabaseProfile, capabilities: StructureCapabilities) {
  const options: Array<{ value: DatabaseColumnIdentity; label: string }> = [
    { value: "none", label: "None" },
  ];
  if (profile.engine === "mysql" && capabilities.autoIncrementColumns.enabled) {
    options.push({ value: "auto_increment", label: "AUTO_INCREMENT" });
  }
  if (profile.engine === "postgresql" && capabilities.identityColumns.enabled) {
    options.push({ value: "always", label: "GENERATED ALWAYS AS IDENTITY" });
    options.push({ value: "by_default", label: "GENERATED BY DEFAULT AS IDENTITY" });
  }
  return options;
}

function ColumnDialog({
  profile,
  scope,
  capabilities,
  column,
  mode,
  onOpenChange,
  onCompleted,
}: {
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  capabilities: StructureCapabilities;
  column?: DatabaseColumnSummary;
  mode: "add" | "edit";
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseStructureActionResult) => void;
}) {
  const editing = mode === "edit" && column !== undefined;
  const [name, setName] = useState(editing ? (column?.name ?? "") : "");
  const [dataType, setDataType] = useState(editing && column ? columnTypeLabel(column) : "");
  const [nullable, setNullable] = useState(editing && column ? column.nullable : true);
  const [defaultMode, setDefaultMode] = useState<DefaultMode>(editing ? "unchanged" : "none");
  const [defaultValue, setDefaultValue] = useState("");
  const [defaultBoolean, setDefaultBoolean] = useState(false);
  const [identity, setIdentity] = useState<DatabaseColumnIdentity>("none");
  const [generatedExpression, setGeneratedExpression] = useState("");
  const [generatedStored, setGeneratedStored] = useState(false);
  const [comment, setComment] = useState(
    editing && column?.comment.status === "available" ? (column.comment.value ?? "") : "",
  );
  const [removeComment, setRemoveComment] = useState(false);

  const originalComment =
    editing && column?.comment.status === "available" ? (column.comment.value ?? "") : "";
  const generatedEnabled = capabilities.generatedColumns.enabled;
  const identityChoices = identityOptions(profile, capabilities);
  const generated = generatedExpression.trim().length > 0;
  const usesIdentity = identity !== "none";
  const defaultDisabled = generated || usesIdentity;

  const request = useMemo<DatabaseStructureActionRequest | null>(() => {
    const columnName = name.trim();
    const type = dataType.trim();
    if (columnName.length === 0 || type.length === 0) {
      return null;
    }
    if (generated && generatedExpression.trim().length === 0) {
      return null;
    }
    const { defaultInput, dropDefault } = defaultRequestValue(
      defaultDisabled ? "none" : defaultMode,
      defaultValue,
      defaultBoolean,
    );

    if (!editing) {
      const definition: DatabaseColumnDefinitionInput = {
        name: columnName,
        dataType: type,
        nullable: usesIdentity ? false : nullable,
      };
      if (defaultInput) {
        definition.default = defaultInput;
      }
      if (comment.trim().length > 0) {
        definition.comment = comment.trim();
      }
      if (generated) {
        definition.generated = { expression: generatedExpression.trim(), stored: generatedStored };
      }
      if (usesIdentity) {
        definition.identity = identity;
      }
      return {
        kind: "add_column",
        database: scope.database,
        schema: scope.schema,
        table: scope.table,
        column: definition,
      };
    }

    if (!column) {
      return null;
    }
    const changes: Extract<DatabaseStructureActionRequest, { kind: "alter_column" }> = {
      kind: "alter_column",
      database: scope.database,
      schema: scope.schema,
      table: scope.table,
      name: column.name,
    };
    const typeChanged = type !== columnTypeLabel(column);
    const nullableChanged = nullable !== column.nullable;
    const commentDropped =
      removeComment || (comment.trim().length === 0 && originalComment.length > 0);
    const commentChanged = !commentDropped && comment !== originalComment;

    // MySQL MODIFY COLUMN replaces the whole definition, so an edit always
    // sends the complete type and nullability. PostgreSQL accepts partial
    // changes, so only send properties the user actually changed.
    if (profile.engine === "mysql" || typeChanged) {
      changes.dataType = type;
    }
    if (profile.engine === "mysql" || nullableChanged) {
      changes.nullable = nullable;
    }
    if (defaultInput) {
      changes.default = defaultInput;
    }
    if (dropDefault) {
      changes.dropDefault = true;
    }
    if (commentDropped) {
      changes.dropComment = true;
    } else if (commentChanged) {
      changes.comment = comment;
    }

    if (
      changes.dataType === undefined &&
      changes.nullable === undefined &&
      changes.default === undefined &&
      !changes.dropDefault &&
      !changes.dropComment &&
      changes.comment === undefined
    ) {
      return null;
    }
    return changes;
  }, [
    column,
    comment,
    dataType,
    defaultBoolean,
    defaultDisabled,
    defaultMode,
    defaultValue,
    editing,
    generated,
    generatedExpression,
    generatedStored,
    identity,
    name,
    nullable,
    originalComment,
    profile.engine,
    removeComment,
    scope.database,
    scope.schema,
    scope.table,
    usesIdentity,
  ]);

  const decision = editing ? capabilities.editColumn : capabilities.addColumn;
  const columnName = column?.name ?? "";
  const title = editing ? `Edit column ${columnName}` : "Add column";

  return (
    <StructureDialogShell
      open
      onOpenChange={onOpenChange}
      title={title}
      description={
        editing
          ? `Change the type, nullability, default, or comment for ${columnName} on ${profile.name}. MySQL requires the complete type because MODIFY COLUMN replaces the definition.`
          : `Add a column to ${scope.table}. Type, nullability, defaults, identity, and generated options are validated before the DDL preview.`
      }
      icon={<TableIcon className="size-4" aria-hidden="true" />}
      profile={profile}
      request={request}
      requestHint="Enter a column name and data type to generate the DDL preview."
      submitLabel={editing ? "Apply column change" : "Add column"}
      decision={decision}
      onCompleted={onCompleted}
    >
      <div className="grid gap-3 sm:grid-cols-2">
        {!editing ? (
          <div>
            <Label htmlFor="structure-column-name">Column name</Label>
            <Input
              id="structure-column-name"
              value={name}
              onChange={(event) => setName(event.currentTarget.value)}
              placeholder="email"
              autoComplete="off"
              className="mt-1 font-mono"
              autoFocus
            />
          </div>
        ) : null}
        <div className={editing ? "sm:col-span-2" : undefined}>
          <Label htmlFor="structure-column-type">Data type</Label>
          <Input
            id="structure-column-type"
            value={dataType}
            onChange={(event) => setDataType(event.currentTarget.value)}
            placeholder={profile.engine === "mysql" ? "varchar(255)" : "character varying(255)"}
            autoComplete="off"
            className="mt-1 font-mono"
          />
        </div>
        <label className="flex items-center gap-2 self-end pb-2 text-[11px] text-muted-foreground">
          <input
            type="checkbox"
            className="size-4 accent-primary"
            checked={usesIdentity ? false : nullable}
            disabled={usesIdentity}
            onChange={(event) => setNullable(event.currentTarget.checked)}
          />
          Nullable
        </label>
      </div>

      {!editing ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <Label htmlFor="structure-column-identity">Identity / auto increment</Label>
            <Select
              value={identity}
              onValueChange={(value) => setIdentity(value as DatabaseColumnIdentity)}
              disabled={generated || (!generatedEnabled && identityChoices.length === 1)}
            >
              <SelectTrigger id="structure-column-identity" className="mt-1">
                <SelectValue placeholder="None" />
              </SelectTrigger>
              <SelectContent>
                {identityChoices.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {identityChoices.length === 1 ? (
              <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
                {profile.engine === "mysql"
                  ? capabilities.autoIncrementColumns.reason
                  : capabilities.identityColumns.reason}
              </p>
            ) : null}
          </div>
          <div>
            <Label htmlFor="structure-column-generated">Generated expression</Label>
            <Input
              id="structure-column-generated"
              value={generatedExpression}
              onChange={(event) => setGeneratedExpression(event.currentTarget.value)}
              placeholder="lower(email)"
              autoComplete="off"
              className="mt-1 font-mono"
              disabled={!generatedEnabled || usesIdentity}
              title={
                !generatedEnabled
                  ? (capabilities.generatedColumns.reason ?? undefined)
                  : usesIdentity
                    ? "A generated column cannot also be an identity or auto-increment column."
                    : undefined
              }
            />
            <label className="mt-1 flex items-center gap-2 text-[10px] text-muted-foreground">
              <input
                type="checkbox"
                className="size-3.5 accent-primary"
                checked={generatedStored}
                disabled={!generatedEnabled || generatedExpression.trim().length === 0}
                onChange={(event) => setGeneratedStored(event.currentTarget.checked)}
              />
              Stored generated column
            </label>
            {!generatedEnabled ? (
              <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
                {capabilities.generatedColumns.reason}
              </p>
            ) : null}
          </div>
        </div>
      ) : null}

      <div className="grid gap-3 sm:grid-cols-2">
        <div>
          <Label htmlFor="structure-column-default-kind">Default</Label>
          <Select
            value={defaultMode}
            onValueChange={(value) => setDefaultMode(value as DefaultMode)}
            disabled={defaultDisabled}
          >
            <SelectTrigger id="structure-column-default-kind" className="mt-1">
              <SelectValue placeholder="None" />
            </SelectTrigger>
            <SelectContent>
              {editing ? <SelectItem value="unchanged">Leave unchanged</SelectItem> : null}
              {!editing ? <SelectItem value="none">No default</SelectItem> : null}
              {editing ? <SelectItem value="drop">Drop default</SelectItem> : null}
              <SelectItem value="null">NULL</SelectItem>
              <SelectItem value="string">String</SelectItem>
              <SelectItem value="number">Number</SelectItem>
              <SelectItem value="boolean">Boolean</SelectItem>
              <SelectItem value="current_timestamp">CURRENT_TIMESTAMP</SelectItem>
            </SelectContent>
          </Select>
          {defaultDisabled ? (
            <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
              A generated or identity column cannot also have a default value.
            </p>
          ) : null}
        </div>
        {!defaultDisabled && (defaultMode === "string" || defaultMode === "number") ? (
          <div>
            <Label htmlFor="structure-column-default-value">Default value</Label>
            <Input
              id="structure-column-default-value"
              value={defaultValue}
              onChange={(event) => setDefaultValue(event.currentTarget.value)}
              placeholder={defaultMode === "number" ? "0" : "active"}
              autoComplete="off"
              className="mt-1 font-mono"
            />
          </div>
        ) : null}
        {!defaultDisabled && defaultMode === "boolean" ? (
          <label className="flex items-center gap-2 self-end pb-2 text-[11px] text-muted-foreground">
            <input
              type="checkbox"
              className="size-4 accent-primary"
              checked={defaultBoolean}
              onChange={(event) => setDefaultBoolean(event.currentTarget.checked)}
            />
            Default is TRUE
          </label>
        ) : null}
      </div>

      <div>
        <Label htmlFor="structure-column-comment">Comment</Label>
        <Textarea
          id="structure-column-comment"
          value={comment}
          onChange={(event) => {
            setComment(event.currentTarget.value);
            setRemoveComment(false);
          }}
          placeholder="Optional column comment"
          className="mt-1 min-h-16"
        />
        {editing && originalComment.length > 0 ? (
          <label className="mt-1 flex items-center gap-2 text-[10px] text-muted-foreground">
            <input
              type="checkbox"
              className="size-3.5 accent-primary"
              checked={removeComment}
              onChange={(event) => setRemoveComment(event.currentTarget.checked)}
            />
            Remove the existing comment
          </label>
        ) : null}
      </div>

      {!editing &&
      (capabilities.generatedColumns.reason ||
        (profile.engine === "mysql"
          ? capabilities.autoIncrementColumns.reason
          : capabilities.identityColumns.reason)) ? (
        <div className="space-y-1 text-[10px] leading-4 text-muted-foreground">
          {capabilities.generatedColumns.reason ? (
            <p>Generated columns: {capabilities.generatedColumns.reason}</p>
          ) : null}
          {profile.engine === "mysql" && capabilities.autoIncrementColumns.reason ? (
            <p>Auto increment: {capabilities.autoIncrementColumns.reason}</p>
          ) : null}
          {profile.engine === "postgresql" && capabilities.identityColumns.reason ? (
            <p>Identity columns: {capabilities.identityColumns.reason}</p>
          ) : null}
        </div>
      ) : null}
    </StructureDialogShell>
  );
}

function CreateIndexDialog({
  profile,
  scope,
  capabilities,
  columns,
  onOpenChange,
  onCompleted,
}: {
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  capabilities: StructureCapabilities;
  columns: DatabaseColumnSummary[];
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseStructureActionResult) => void;
}) {
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [unique, setUnique] = useState(false);
  const [method, setMethod] = useState("");
  const [predicate, setPredicate] = useState("");

  function toggleColumn(column: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(column)) {
        next.delete(column);
      } else {
        next.add(column);
      }
      return next;
    });
  }

  const orderedColumns = columns
    .slice()
    .sort((left, right) => left.ordinalPosition - right.ordinalPosition)
    .map((column) => column.name)
    .filter((column) => selected.has(column));

  const request = useMemo<DatabaseStructureActionRequest | null>(() => {
    const indexName = name.trim();
    if (indexName.length === 0 || orderedColumns.length === 0) {
      return null;
    }
    return {
      kind: "create_index",
      database: scope.database,
      schema: scope.schema,
      table: scope.table,
      name: indexName,
      columns: orderedColumns,
      unique,
      method: method.trim() || null,
      predicate: capabilities.partialIndexes.enabled ? predicate.trim() || null : null,
    };
  }, [
    capabilities.partialIndexes.enabled,
    method,
    name,
    orderedColumns,
    predicate,
    scope.database,
    scope.schema,
    scope.table,
    unique,
  ]);

  return (
    <StructureDialogShell
      open
      onOpenChange={onOpenChange}
      title="Create index"
      description={`Add a secondary or unique index to ${scope.table}. Column order in the checklist follows the table's ordinal order.`}
      icon={<KeyIcon className="size-4" aria-hidden="true" />}
      profile={profile}
      request={request}
      requestHint="Enter an index name and select at least one column to generate the DDL preview."
      submitLabel="Create index"
      decision={capabilities.createIndex}
      onCompleted={onCompleted}
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div>
          <Label htmlFor="create-index-name">Index name</Label>
          <Input
            id="create-index-name"
            value={name}
            onChange={(event) => setName(event.currentTarget.value)}
            placeholder={`${scope.table}_${profile.engine === "mysql" ? "idx" : "index"}`}
            autoComplete="off"
            className="mt-1 font-mono"
            autoFocus
          />
        </div>
        <div>
          <Label htmlFor="create-index-method">Access method</Label>
          <Input
            id="create-index-method"
            value={method}
            onChange={(event) => setMethod(event.currentTarget.value)}
            placeholder={profile.engine === "mysql" ? "BTREE" : "btree"}
            autoComplete="off"
            className="mt-1 font-mono"
          />
        </div>
      </div>

      <div className="space-y-2">
        <p className="text-[11px] font-semibold text-foreground">Columns</p>
        {columns.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            Column metadata is still loading. Close this dialog and try again once columns appear.
          </p>
        ) : (
          <div className="grid gap-1.5 rounded-md border border-border bg-muted/15 p-2 sm:grid-cols-2">
            {columns.map((column) => (
              <label
                key={column.name}
                className="flex items-center gap-2 rounded-sm px-1.5 py-1 text-[11px] text-foreground hover:bg-muted/40"
              >
                <input
                  type="checkbox"
                  className="size-4 accent-primary"
                  checked={selected.has(column.name)}
                  onChange={() => toggleColumn(column.name)}
                  aria-label={`Index column ${column.name}`}
                />
                <span className="font-mono">{column.name}</span>
                <span className="ml-auto truncate text-[10px] text-muted-foreground">
                  {column.dataType}
                </span>
              </label>
            ))}
          </div>
        )}
        <label className="flex items-center gap-2 text-[11px] text-muted-foreground">
          <input
            type="checkbox"
            className="size-4 accent-primary"
            checked={unique}
            onChange={(event) => setUnique(event.currentTarget.checked)}
          />
          Enforce uniqueness
        </label>
      </div>

      <div>
        <Label htmlFor="create-index-predicate">Partial-index predicate</Label>
        <Textarea
          id="create-index-predicate"
          value={predicate}
          onChange={(event) => setPredicate(event.currentTarget.value)}
          placeholder="status = 'active'"
          className="mt-1 min-h-16 font-mono"
          disabled={!capabilities.partialIndexes.enabled}
          title={
            capabilities.partialIndexes.enabled
              ? undefined
              : (capabilities.partialIndexes.reason ?? undefined)
          }
        />
        <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
          {capabilities.partialIndexes.enabled
            ? "Leave blank for a full index. The predicate is shown in the DDL preview."
            : (capabilities.partialIndexes.reason ?? "Partial indexes are unavailable.")}
        </p>
      </div>
    </StructureDialogShell>
  );
}

const CONSTRAINT_KINDS: Array<{ value: DatabaseConstraintKind; label: string }> = [
  { value: "primary_key", label: "Primary key" },
  { value: "unique", label: "Unique" },
  { value: "foreign_key", label: "Foreign key" },
  { value: "check", label: "Check" },
];

const REFERENTIAL_ACTIONS: Array<{ value: DatabaseReferentialAction; label: string }> = [
  { value: "no_action", label: "No action" },
  { value: "restrict", label: "Restrict" },
  { value: "cascade", label: "Cascade" },
  { value: "set_null", label: "Set null" },
  { value: "set_default", label: "Set default" },
];

function ConstraintDialog({
  profile,
  scope,
  capabilities,
  columns,
  onOpenChange,
  onCompleted,
}: {
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  capabilities: StructureCapabilities;
  columns: DatabaseColumnSummary[];
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseStructureActionResult) => void;
}) {
  const [kind, setKind] = useState<DatabaseConstraintKind>("unique");
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [referencedSchema, setReferencedSchema] = useState("");
  const [referencedTable, setReferencedTable] = useState("");
  const [referencedColumns, setReferencedColumns] = useState("");
  const [onDelete, setOnDelete] = useState<DatabaseReferentialAction>("no_action");
  const [onUpdate, setOnUpdate] = useState<DatabaseReferentialAction>("no_action");
  const [expression, setExpression] = useState("");

  const checkEnabled = capabilities.addCheckConstraint.enabled;

  function toggleColumn(column: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(column)) {
        next.delete(column);
      } else {
        next.add(column);
      }
      return next;
    });
  }

  const orderedColumns = columns
    .slice()
    .sort((left, right) => left.ordinalPosition - right.ordinalPosition)
    .map((column) => column.name)
    .filter((column) => selected.has(column));

  const request = useMemo<DatabaseStructureActionRequest | null>(() => {
    const constraintName = name.trim();
    if (constraintName.length === 0) {
      return null;
    }
    let constraint: DatabaseConstraintInput | null = null;
    if (kind === "primary_key" || kind === "unique") {
      if (orderedColumns.length === 0) {
        return null;
      }
      constraint = { kind, name: constraintName, columns: orderedColumns };
    } else if (kind === "foreign_key") {
      const referenced = referencedColumns
        .split(",")
        .map((column) => column.trim())
        .filter((column) => column.length > 0);
      if (
        orderedColumns.length === 0 ||
        referencedTable.trim().length === 0 ||
        referenced.length === 0
      ) {
        return null;
      }
      constraint = {
        kind: "foreign_key",
        name: constraintName,
        columns: orderedColumns,
        referencedSchema: referencedSchema.trim() || null,
        referencedTable: referencedTable.trim(),
        referencedColumns: referenced,
        onDelete,
        onUpdate,
      };
    } else {
      if (!checkEnabled || expression.trim().length === 0) {
        return null;
      }
      constraint = { kind: "check", name: constraintName, expression: expression.trim() };
    }
    if (!constraint) {
      return null;
    }
    return {
      kind: "add_constraint",
      database: scope.database,
      schema: scope.schema,
      table: scope.table,
      constraint,
    };
  }, [
    checkEnabled,
    expression,
    kind,
    name,
    onDelete,
    onUpdate,
    orderedColumns,
    referencedColumns,
    referencedSchema,
    referencedTable,
    scope.database,
    scope.schema,
    scope.table,
  ]);

  const decision =
    kind === "check" && !checkEnabled
      ? capabilities.addCheckConstraint
      : capabilities.addConstraint;

  return (
    <StructureDialogShell
      open
      onOpenChange={onOpenChange}
      title="Add constraint"
      description={`Add a primary-key, unique, foreign-key, or check constraint to ${scope.table}. Referenced tables and columns are verified by the backend.`}
      icon={<KeyIcon className="size-4" aria-hidden="true" />}
      profile={profile}
      request={request}
      requestHint="Choose a constraint kind, name, and the required columns or expression to generate the DDL preview."
      submitLabel="Add constraint"
      decision={decision}
      onCompleted={onCompleted}
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div>
          <Label htmlFor="constraint-kind">Constraint kind</Label>
          <Select value={kind} onValueChange={(value) => setKind(value as DatabaseConstraintKind)}>
            <SelectTrigger id="constraint-kind" className="mt-1">
              <SelectValue placeholder="Unique" />
            </SelectTrigger>
            <SelectContent>
              {CONSTRAINT_KINDS.map((option) => (
                <SelectItem
                  key={option.value}
                  value={option.value}
                  disabled={option.value === "check" && !checkEnabled}
                >
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {!checkEnabled ? (
            <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
              Check constraints are unavailable: {capabilities.addCheckConstraint.reason}
            </p>
          ) : null}
        </div>
        <div>
          <Label htmlFor="constraint-name">Constraint name</Label>
          <Input
            id="constraint-name"
            value={name}
            onChange={(event) => setName(event.currentTarget.value)}
            placeholder={`${scope.table}_${kind}`}
            autoComplete="off"
            className="mt-1 font-mono"
            autoFocus
          />
        </div>
      </div>

      {kind === "check" ? (
        <div>
          <Label htmlFor="constraint-expression">Check expression</Label>
          <Textarea
            id="constraint-expression"
            value={expression}
            onChange={(event) => setExpression(event.currentTarget.value)}
            placeholder="total >= 0"
            className="mt-1 min-h-20 font-mono"
            disabled={!checkEnabled}
          />
        </div>
      ) : (
        <div className="space-y-2">
          <p className="text-[11px] font-semibold text-foreground">
            {kind === "foreign_key" ? "Local columns" : "Constraint columns"}
          </p>
          {columns.length === 0 ? (
            <p className="text-[11px] text-muted-foreground">
              Column metadata is still loading. Close this dialog and try again once columns appear.
            </p>
          ) : (
            <div className="grid gap-1.5 rounded-md border border-border bg-muted/15 p-2 sm:grid-cols-2">
              {columns.map((column) => (
                <label
                  key={column.name}
                  className="flex items-center gap-2 rounded-sm px-1.5 py-1 text-[11px] text-foreground hover:bg-muted/40"
                >
                  <input
                    type="checkbox"
                    className="size-4 accent-primary"
                    checked={selected.has(column.name)}
                    onChange={() => toggleColumn(column.name)}
                    aria-label={`Constraint column ${column.name}`}
                  />
                  <span className="font-mono">{column.name}</span>
                  <span className="ml-auto truncate text-[10px] text-muted-foreground">
                    {column.dataType}
                  </span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {kind === "foreign_key" ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <Label htmlFor="constraint-referenced-schema">Referenced schema</Label>
            <Input
              id="constraint-referenced-schema"
              value={referencedSchema}
              onChange={(event) => setReferencedSchema(event.currentTarget.value)}
              placeholder="public"
              autoComplete="off"
              className="mt-1 font-mono"
            />
          </div>
          <div>
            <Label htmlFor="constraint-referenced-table">Referenced table</Label>
            <Input
              id="constraint-referenced-table"
              value={referencedTable}
              onChange={(event) => setReferencedTable(event.currentTarget.value)}
              placeholder="users"
              autoComplete="off"
              className="mt-1 font-mono"
            />
          </div>
          <div className="sm:col-span-2">
            <Label htmlFor="constraint-referenced-columns">Referenced columns</Label>
            <Input
              id="constraint-referenced-columns"
              value={referencedColumns}
              onChange={(event) => setReferencedColumns(event.currentTarget.value)}
              placeholder="id"
              autoComplete="off"
              className="mt-1 font-mono"
            />
            <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
              Comma-separated, in the same order as the local columns.
            </p>
          </div>
          <div>
            <Label htmlFor="constraint-on-delete">On delete</Label>
            <Select
              value={onDelete}
              onValueChange={(value) => setOnDelete(value as DatabaseReferentialAction)}
            >
              <SelectTrigger id="constraint-on-delete" className="mt-1">
                <SelectValue placeholder="No action" />
              </SelectTrigger>
              <SelectContent>
                {REFERENTIAL_ACTIONS.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="constraint-on-update">On update</Label>
            <Select
              value={onUpdate}
              onValueChange={(value) => setOnUpdate(value as DatabaseReferentialAction)}
            >
              <SelectTrigger id="constraint-on-update" className="mt-1">
                <SelectValue placeholder="No action" />
              </SelectTrigger>
              <SelectContent>
                {REFERENTIAL_ACTIONS.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      ) : null}
    </StructureDialogShell>
  );
}

type StructureDestructiveDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  confirmLabel: string;
  profile: DatabaseProfile;
  request: DatabaseStructureActionRequest | null;
  decision: StructureDecision;
  details: Array<{ label: string; value: string }>;
  warningTitle: string;
  warningDescription: string;
  onCompleted: (result: DatabaseStructureActionResult) => void;
};

function StructureDestructiveDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  profile,
  request,
  decision,
  details,
  warningTitle,
  warningDescription,
  onCompleted,
}: StructureDestructiveDialogProps) {
  const [previewState, setPreviewState] = useState<PreviewState>({ status: "idle" });
  const [pending, setPending] = useState(false);
  const [actionError, setActionError] = useState<CommandError | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const requestRef = useRef(request);
  const requestKey = request ? JSON.stringify(request) : null;
  const runtimeAvailable = isTauriRuntime();

  useEffect(() => {
    requestRef.current = request;
  }, [request]);

  useEffect(() => {
    if (!open || !requestRef.current) {
      setPreviewState({ status: "idle" });
      return;
    }
    let active = true;
    setPreviewState({ status: "loading" });
    setActionError(null);
    void previewDatabaseStructureAction(profile.id, requestRef.current)
      .then((preview) => {
        if (active) {
          setPreviewState({ status: "ready", preview });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          setPreviewState({ status: "error", error: commandErrorFor(error) });
        }
      });
    return () => {
      active = false;
    };
  }, [open, profile.id, reloadToken, requestKey]);

  const preview = previewState.status === "ready" ? previewState.preview : null;
  const previewError = previewState.status === "error" ? previewState.error : null;
  const canConfirm = preview !== null && decision.enabled && runtimeAvailable && !pending;

  async function handleConfirm() {
    if (!requestRef.current || !preview || !canConfirm) {
      return;
    }
    setPending(true);
    setActionError(null);
    try {
      const result = await runDatabaseStructureAction(profile.id, requestRef.current, true);
      onCompleted(result);
      onOpenChange(false);
    } catch (error) {
      setActionError(commandErrorFor(error));
    } finally {
      setPending(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (!pending ? onOpenChange(next) : undefined)}>
      <DialogContent className="max-h-[88vh] max-w-xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-destructive">
            <TrashIcon className="size-4" aria-hidden="true" />
            {title}
          </DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        {details.length > 0 ? (
          <dl className="grid grid-cols-[96px_1fr] gap-x-3 gap-y-2 rounded-md border border-border bg-muted/45 px-3.5 py-3">
            {details.map((detail) => (
              <div key={detail.label} className="contents">
                <dt className="text-[11px] leading-5 text-muted-foreground">{detail.label}</dt>
                <dd className="break-words font-mono text-[12px] leading-5 text-foreground">
                  {detail.value}
                </dd>
              </div>
            ))}
          </dl>
        ) : null}

        <Alert variant="destructive" className="py-2.5">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>{warningTitle}</AlertTitle>
            <AlertDescription>{warningDescription}</AlertDescription>
          </div>
        </Alert>

        <ReadOnlyAlert decision={decision} />

        <section aria-label="DDL preview" className="space-y-2">
          <div className="flex items-center justify-between gap-2">
            <div>
              <p className="flex items-center gap-1.5 text-[11px] font-semibold text-foreground">
                <CodeIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
                Generated DDL
              </p>
              <p className="text-[10px] leading-4 text-muted-foreground">
                The exact statement is generated before this action can run.
              </p>
            </div>
            {preview ? <CopyButton value={preview.ddl} label="Copy DDL" /> : null}
          </div>

          {previewState.status === "loading" ? (
            <LoadingState rows={1} />
          ) : previewError ? (
            <div className="space-y-2">
              <ActionErrorAlert
                error={previewError}
                title={errorTitle(previewError, "Preview failed")}
              />
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setReloadToken((current) => current + 1)}
              >
                Retry preview
              </Button>
            </div>
          ) : preview ? (
            <div className="space-y-2">
              <pre
                data-testid="structure-ddl-preview"
                className="max-h-44 overflow-auto rounded-md border border-border bg-muted/45 p-3 font-mono text-[11px] leading-5 whitespace-pre-wrap text-foreground"
              >
                {preview.ddl || "No DDL is required for this action."}
              </pre>
              {preview.warnings.length > 0 ? (
                <ul className="space-y-1 text-[10px] leading-4 text-muted-foreground">
                  {preview.warnings.map((warning) => (
                    <li key={warning} className="flex gap-1.5">
                      <WarningCircleIcon className="mt-0.5 size-3 shrink-0" aria-hidden="true" />
                      {warning}
                    </li>
                  ))}
                </ul>
              ) : null}
            </div>
          ) : null}
        </section>

        {actionError ? (
          <ActionErrorAlert
            error={actionError}
            title={errorTitle(actionError, "Structure action failed")}
          />
        ) : null}

        {!runtimeAvailable ? (
          <Alert variant="info" className="py-2.5">
            <DatabaseIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
            <div>
              <AlertTitle>Desktop build required</AlertTitle>
              <AlertDescription>
                DDL previews and structure changes are available in the desktop application.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        <DialogFooter>
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={pending}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={() => void handleConfirm()} disabled={!canConfirm}>
            {pending ? "Working..." : confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type DatabaseStructureActionHostProps = {
  intent: DatabaseStructureIntent | null;
  onOpenChange: (open: boolean) => void;
  profile: DatabaseProfile;
  scope: DatabaseTableScope;
  structure: DatabaseTableStructureState;
  capabilities: StructureCapabilities;
  onCompleted: (result: DatabaseStructureActionResult, intent: DatabaseStructureIntent) => void;
};

function tableDetails(
  profile: DatabaseProfile,
  scope: DatabaseTableScope,
  extra: Array<{ label: string; value: string }> = [],
): Array<{ label: string; value: string }> {
  return [
    { label: "Table", value: scope.table || "New table" },
    { label: "Database", value: scope.database ?? "Active database" },
    { label: "Schema", value: scope.schema ?? "Default schema" },
    { label: "Engine", value: engineLabel(profile.engine) },
    { label: "Profile", value: profile.name },
    ...extra,
  ];
}

function DatabaseStructureActionHost({
  intent,
  onOpenChange,
  profile,
  scope,
  structure,
  capabilities,
  onCompleted,
}: DatabaseStructureActionHostProps) {
  if (!intent) {
    return null;
  }

  const columns = structure.columns.status === "ready" ? structure.columns.data : [];
  const completed = (result: DatabaseStructureActionResult) => onCompleted(result, intent);

  switch (intent.type) {
    case "create_table":
      return (
        <CreateTableDialog
          profile={profile}
          scope={scope}
          capabilities={capabilities}
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "rename_table":
      return (
        <RenameTableDialog
          profile={profile}
          scope={scope}
          capabilities={capabilities}
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "duplicate_table":
      return (
        <DuplicateTableDialog
          profile={profile}
          scope={scope}
          capabilities={capabilities}
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "truncate_table":
      return (
        <StructureDestructiveDialog
          open
          onOpenChange={onOpenChange}
          title={`Truncate table ${scope.table}?`}
          description="Truncating removes every row while keeping the table definition, indexes, and constraints."
          confirmLabel="Truncate table"
          profile={profile}
          request={{
            kind: "truncate_table",
            database: scope.database,
            schema: scope.schema,
            name: scope.table,
          }}
          decision={capabilities.truncateTable}
          details={tableDetails(profile, scope, [{ label: "Action", value: "TRUNCATE TABLE" }])}
          warningTitle="Every row in this table will be deleted."
          warningDescription="The table structure remains, but deleted rows cannot be restored unless you have a backup."
          onCompleted={completed}
        />
      );
    case "drop_table":
      return (
        <StructureDestructiveDialog
          open
          onOpenChange={onOpenChange}
          title={`Drop table ${scope.table}?`}
          description="Dropping removes the table, its rows, indexes, and constraints from the database."
          confirmLabel="Drop table"
          profile={profile}
          request={{
            kind: "drop_table",
            database: scope.database,
            schema: scope.schema,
            name: scope.table,
          }}
          decision={capabilities.dropTable}
          details={tableDetails(profile, scope, [
            { label: "Action", value: "DROP TABLE (permanent)" },
          ])}
          warningTitle="This permanently deletes the table."
          warningDescription="The DDL preview names blocking foreign keys or dependent views before the drop is attempted."
          onCompleted={completed}
        />
      );
    case "add_column":
      return (
        <ColumnDialog
          profile={profile}
          scope={scope}
          capabilities={capabilities}
          mode="add"
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "edit_column":
      return (
        <ColumnDialog
          profile={profile}
          scope={scope}
          capabilities={capabilities}
          mode="edit"
          column={intent.column}
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "rename_column":
      return (
        <RenameColumnDialog
          profile={profile}
          scope={scope}
          column={intent.column}
          capabilities={capabilities}
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "drop_column":
      return (
        <StructureDestructiveDialog
          open
          onOpenChange={onOpenChange}
          title={`Drop column ${intent.column.name}?`}
          description="Dropping a column removes its values and every index or constraint that depends on it."
          confirmLabel="Drop column"
          profile={profile}
          request={{
            kind: "drop_column",
            database: scope.database,
            schema: scope.schema,
            table: scope.table,
            name: intent.column.name,
          }}
          decision={capabilities.dropColumn}
          details={tableDetails(profile, scope, [
            { label: "Column", value: intent.column.name },
            {
              label: "Type",
              value: columnTypeLabel(intent.column),
            },
            { label: "Action", value: "DROP COLUMN (permanent)" },
          ])}
          warningTitle="Column data will be permanently deleted."
          warningDescription="The preview reports dependent constraints and foreign keys before the drop is attempted."
          onCompleted={completed}
        />
      );
    case "create_index":
      return (
        <CreateIndexDialog
          profile={profile}
          scope={scope}
          capabilities={capabilities}
          columns={columns}
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "rename_index":
      return (
        <RenameIndexDialog
          profile={profile}
          scope={scope}
          index={intent.index}
          capabilities={capabilities}
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "drop_index":
      return (
        <StructureDestructiveDialog
          open
          onOpenChange={onOpenChange}
          title={`Drop index ${intent.index.name}?`}
          description="Dropping an index removes the optimization and any uniqueness guarantee it provides."
          confirmLabel="Drop index"
          profile={profile}
          request={{
            kind: "drop_index",
            database: scope.database,
            schema: scope.schema,
            table: scope.table,
            name: intent.index.name,
          }}
          decision={indexActionDecisions(intent.index, capabilities).drop}
          details={tableDetails(profile, scope, [
            { label: "Index", value: intent.index.name },
            { label: "Columns", value: intent.index.columns.join(", ") || "—" },
            { label: "Unique", value: intent.index.unique ? "Yes" : "No" },
          ])}
          warningTitle="The index will be removed from the table."
          warningDescription="The preview names any blocking constraint or incoming foreign key before the drop is attempted."
          onCompleted={completed}
        />
      );
    case "add_constraint":
      return (
        <ConstraintDialog
          profile={profile}
          scope={scope}
          capabilities={capabilities}
          columns={columns}
          onOpenChange={onOpenChange}
          onCompleted={completed}
        />
      );
    case "drop_constraint":
      return (
        <StructureDestructiveDialog
          open
          onOpenChange={onOpenChange}
          title={`Drop constraint ${intent.constraint.name}?`}
          description="Dropping a constraint removes a structural guarantee. Rows are not deleted, but dependent objects may be rejected."
          confirmLabel="Drop constraint"
          profile={profile}
          request={{
            kind: "drop_constraint",
            database: scope.database,
            schema: scope.schema,
            table: scope.table,
            name: intent.constraint.name,
            constraintKind: intent.constraint.kind,
          }}
          decision={
            intent.constraint.kind === "check"
              ? capabilities.addCheckConstraint
              : capabilities.addConstraint
          }
          details={tableDetails(profile, scope, [
            { label: "Constraint", value: intent.constraint.name },
            { label: "Kind", value: constraintKindLabel(intent.constraint.kind) },
            {
              label: "Columns",
              value: intent.constraint.columns.join(", ") || "—",
            },
          ])}
          warningTitle="This removes a structural guarantee."
          warningDescription="The preview names dependent objects and the exact ALTER statement before the drop runs."
          onCompleted={completed}
        />
      );
    default:
      return null;
  }
}

export { DatabaseStructureActionHost, StructureDialogShell };
