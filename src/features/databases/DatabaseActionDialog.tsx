import { useMemo, useState } from "react";
import {
  CircleNotchIcon,
  CodeIcon,
  DatabaseIcon,
  InfoIcon,
  WarningCircleIcon,
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
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { createDatabase, previewDatabaseAction, renameDatabase } from "@/services/databaseActions";
import type {
  CommandError,
  DatabaseActionRequest,
  DatabaseActionResult,
  DatabaseDdlPreview,
  DatabaseProfile,
} from "@/types/contracts";

type DatabaseActionDialogProps = {
  open: boolean;
  mode: "create" | "rename";
  profile: DatabaseProfile;
  database?: string | null;
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: DatabaseActionResult) => void;
};

type PreviewState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; preview: DatabaseDdlPreview }
  | { status: "error"; error: CommandError };

function requestFromValues(
  mode: "create" | "rename",
  values: {
    name: string;
    encoding: string;
    collation: string;
    owner: string;
    template: string;
  },
  profile: DatabaseProfile,
  database: string | null,
): DatabaseActionRequest {
  if (mode === "rename") {
    return {
      kind: "rename",
      name: database ?? "",
      newName: values.name.trim(),
    };
  }

  return {
    kind: "create",
    name: values.name.trim(),
    encoding: values.encoding.trim() || null,
    collation: values.collation.trim() || null,
    owner: profile.engine === "postgresql" ? values.owner.trim() || null : null,
    template: profile.engine === "postgresql" ? values.template.trim() || null : null,
  };
}

const emptyValues = {
  name: "",
  encoding: "",
  collation: "",
  owner: "",
  template: "",
};

function DatabaseActionDialog({
  open,
  mode,
  profile,
  database = null,
  onOpenChange,
  onCompleted,
}: DatabaseActionDialogProps) {
  const [values, setValues] = useState(emptyValues);
  const [previewState, setPreviewState] = useState<PreviewState>({ status: "idle" });
  const [formError, setFormError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  const request = useMemo(
    () => requestFromValues(mode, values, profile, database),
    [database, mode, profile, values],
  );
  const nameMissing = values.name.trim().length === 0;
  const canPreview = !nameMissing && previewState.status !== "loading" && !pending;
  const canApply =
    previewState.status === "ready" &&
    !nameMissing &&
    !pending &&
    !profile.readOnly &&
    isTauriRuntime();

  function updateValue(field: keyof typeof values, value: string) {
    setValues((current) => ({ ...current, [field]: value }));
    setPreviewState({ status: "idle" });
    setFormError(null);
  }

  async function handlePreview() {
    if (!canPreview) {
      return;
    }
    setPreviewState({ status: "loading" });
    setFormError(null);

    try {
      const preview = await previewDatabaseAction(profile.id, request);
      setPreviewState({ status: "ready", preview });
    } catch (error) {
      const commandError: CommandError = isCommandError(error)
        ? error
        : { code: "internal", message: getCommandErrorMessage(error) };
      setPreviewState({ status: "error", error: commandError });
    }
  }

  async function handleApply() {
    if (!canApply) {
      return;
    }
    setPending(true);
    setFormError(null);

    try {
      const result =
        mode === "create"
          ? await createDatabase(profile.id, {
              name: request.kind === "create" ? request.name : values.name.trim(),
              encoding: request.kind === "create" ? request.encoding : null,
              collation: request.kind === "create" ? request.collation : null,
              owner: request.kind === "create" ? request.owner : null,
              template: request.kind === "create" ? request.template : null,
            })
          : await renameDatabase(profile.id, database ?? "", values.name.trim());
      onCompleted(result);
      onOpenChange(false);
    } catch (error) {
      setFormError(getCommandErrorMessage(error));
    } finally {
      setPending(false);
    }
  }

  const preview = previewState.status === "ready" ? previewState.preview : null;
  const previewError = previewState.status === "error" ? previewState.error : null;
  const title = mode === "create" ? "Create database" : `Rename ${database ?? "database"}`;

  return (
    <Dialog open={open} onOpenChange={(next) => (!pending ? onOpenChange(next) : undefined)}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <DatabaseIcon className="size-4" aria-hidden="true" />
            {title}
          </DialogTitle>
          <DialogDescription>
            {mode === "create"
              ? `Create a new database on ${profile.name}. Options are generated for ${profile.engine === "mysql" ? "MySQL" : "PostgreSQL"} and previewed before the command runs.`
              : `Rename a database on ${profile.name}. PostgreSQL performs the rename without copying data; the connection must not use this catalog.`}
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3 sm:grid-cols-2">
          <div className="sm:col-span-2">
            <Label htmlFor="database-action-name">
              {mode === "create" ? "Database name" : "New database name"}
            </Label>
            <Input
              id="database-action-name"
              value={values.name}
              onChange={(event) => updateValue("name", event.currentTarget.value)}
              placeholder={mode === "create" ? "app_next" : "app_renamed"}
              className="mt-1 font-mono"
              autoComplete="off"
            />
          </div>

          {mode === "create" ? (
            <>
              <div>
                <Label htmlFor="database-action-encoding">Encoding</Label>
                <Input
                  id="database-action-encoding"
                  value={values.encoding}
                  onChange={(event) => updateValue("encoding", event.currentTarget.value)}
                  placeholder={profile.engine === "mysql" ? "utf8mb4" : "UTF8"}
                  className="mt-1 font-mono"
                  autoComplete="off"
                />
              </div>
              <div>
                <Label htmlFor="database-action-collation">Collation</Label>
                <Input
                  id="database-action-collation"
                  value={values.collation}
                  onChange={(event) => updateValue("collation", event.currentTarget.value)}
                  placeholder={profile.engine === "mysql" ? "utf8mb4_unicode_ci" : "C.UTF-8"}
                  className="mt-1 font-mono"
                  autoComplete="off"
                />
              </div>
              {profile.engine === "postgresql" ? (
                <>
                  <div>
                    <Label htmlFor="database-action-owner">Owner</Label>
                    <Input
                      id="database-action-owner"
                      value={values.owner}
                      onChange={(event) => updateValue("owner", event.currentTarget.value)}
                      placeholder={profile.username}
                      className="mt-1 font-mono"
                      autoComplete="off"
                    />
                  </div>
                  <div>
                    <Label htmlFor="database-action-template">Template</Label>
                    <Input
                      id="database-action-template"
                      value={values.template}
                      onChange={(event) => updateValue("template", event.currentTarget.value)}
                      placeholder="template0"
                      className="mt-1 font-mono"
                      autoComplete="off"
                    />
                  </div>
                </>
              ) : null}
            </>
          ) : null}
        </div>

        {profile.engine === "mysql" && mode === "create" ? (
          <p className="flex items-start gap-1.5 text-[11px] leading-5 text-muted-foreground">
            <InfoIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            MySQL has no database owner or template option. The configured account owns the new
            schema, and server defaults apply when encoding and collation are blank.
          </p>
        ) : null}

        {profile.readOnly ? (
          <Alert variant="warning" className="py-2.5">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div>
              <AlertTitle>Read-only profile</AlertTitle>
              <AlertDescription>
                This connection profile is read-only. Save a writable profile before applying this
                structural change.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        <div className="space-y-2">
          <div className="flex items-center justify-between gap-2">
            <div>
              <p className="text-[11px] font-semibold text-foreground">Generated DDL</p>
              <p className="text-[10px] leading-4 text-muted-foreground">
                Preview is required before Apply. No statement has run yet.
              </p>
            </div>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void handlePreview()}
              disabled={!canPreview}
            >
              {previewState.status === "loading" ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <CodeIcon className="size-3.5" aria-hidden="true" />
              )}
              {previewState.status === "loading" ? "Generating..." : "Preview DDL"}
            </Button>
          </div>

          {preview ? (
            <div className="space-y-2">
              <pre className="max-h-40 overflow-auto rounded-md border border-border bg-muted/45 p-3 font-mono text-[11px] leading-5 whitespace-pre-wrap text-foreground">
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
          ) : (
            <div className="rounded-md border border-dashed border-border bg-muted/20 px-3 py-4 text-[11px] text-muted-foreground">
              {previewError
                ? previewError.message
                : nameMissing
                  ? "Enter a database name to generate the DDL preview."
                  : "Choose Preview DDL to inspect the exact statement."}
            </div>
          )}

          {previewError ? (
            <Alert variant="destructive" className="py-2.5">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>Preview failed</AlertTitle>
                <AlertDescription>{previewError.message}</AlertDescription>
              </div>
            </Alert>
          ) : null}
        </div>

        {formError ? (
          <Alert variant="destructive" className="py-2.5">
            <WarningCircleIcon
              className="mt-0.5 size-4 shrink-0 text-destructive"
              aria-hidden="true"
            />
            <div>
              <AlertTitle>Database action failed</AlertTitle>
              <AlertDescription>{formError}</AlertDescription>
            </div>
          </Alert>
        ) : null}

        <DialogFooter>
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={pending}>
            Cancel
          </Button>
          <Button onClick={() => void handleApply()} disabled={!canApply}>
            {pending ? "Applying..." : mode === "create" ? "Create database" : "Rename database"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export { DatabaseActionDialog };
