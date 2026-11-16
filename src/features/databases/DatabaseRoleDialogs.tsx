import { useMemo, useState } from "react";
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
import {
  defaultPrivilegeTarget,
  privilegeLabel,
  privilegeOptionsFor,
  privilegeTargetKindLabel,
  privilegeTargetKindOptions,
  privilegeTargetLabel,
  roleLabel,
} from "@/features/databases/databaseRolesModel";
import type { DatabaseTableScope } from "@/features/databases/databaseStructureModel";
import { getCommandErrorMessage } from "@/lib/errors";
import { previewDatabaseRoleAction, runDatabaseRoleAction } from "@/services/databaseRoles";
import type {
  DatabaseEngine,
  DatabasePrivilege,
  DatabasePrivilegeDiff,
  DatabasePrivilegeTarget,
  DatabasePrivilegeTargetKind,
  DatabaseRoleActionRequest,
  DatabaseRoleActionResult,
  DatabaseRoleDdlPreview,
  DatabaseRoleKind,
  DatabaseRoleRef,
  DatabaseRoleSummary,
} from "@/types/contracts";

export type DatabaseRoleIntent =
  | { type: "create" }
  | { type: "alter"; role: DatabaseRoleSummary }
  | { type: "grant_privileges"; role: DatabaseRoleSummary }
  | { type: "revoke_privileges"; role: DatabaseRoleSummary }
  | { type: "grant_role"; member: DatabaseRoleSummary }
  | { type: "revoke_role"; member: DatabaseRoleSummary };

type FormState = {
  name: string;
  host: string;
  kind: DatabaseRoleKind;
  canLogin: boolean;
  superuser: boolean;
  createDatabase: boolean;
  createRole: boolean;
  inherit: boolean;
  privileges: DatabasePrivilege[];
  targetKind: DatabasePrivilegeTargetKind;
  database: string;
  schema: string;
  object: string;
  grantOption: boolean;
  roleKey: string;
};

function metadataBool(value: DatabaseRoleSummary["canLogin"], fallback: boolean): boolean {
  return value.status === "available" ? value.value : fallback;
}

function refFor(role: DatabaseRoleSummary): DatabaseRoleRef {
  return { name: role.name, host: role.host ?? null };
}

function roleKey(role: DatabaseRoleSummary): string {
  return roleLabel(refFor(role));
}

function firstRoleOption(
  intent: DatabaseRoleIntent,
  roles: DatabaseRoleSummary[],
): DatabaseRoleSummary | null {
  if (intent.type !== "grant_role" && intent.type !== "revoke_role") {
    return null;
  }
  return roles.find((role) => roleKey(role) !== roleKey(intent.member)) ?? roles[0] ?? null;
}

function formForIntent(
  intent: DatabaseRoleIntent | null,
  engine: DatabaseEngine,
  scope: DatabaseTableScope,
  roles: DatabaseRoleSummary[],
): FormState {
  const targetKind: DatabasePrivilegeTargetKind = "table";
  const target = defaultPrivilegeTarget(engine, targetKind, scope);
  const base: FormState = {
    name: "",
    host: engine === "mysql" ? "%" : "",
    kind: "user",
    canLogin: engine === "mysql" ? false : true,
    superuser: false,
    createDatabase: false,
    createRole: false,
    inherit: true,
    privileges: privilegeOptionsFor(engine, targetKind).slice(0, 1),
    targetKind,
    database: target.database ?? scope.database ?? "",
    schema: target.schema ?? scope.schema ?? (engine === "postgresql" ? "public" : ""),
    object: target.object ?? scope.table ?? "",
    grantOption: false,
    roleKey: "",
  };
  if (!intent || intent.type === "create") {
    return base;
  }
  if (intent.type === "alter") {
    return {
      ...base,
      name: intent.role.name,
      host: intent.role.host ?? "",
      kind: intent.role.kind,
      canLogin: metadataBool(intent.role.canLogin, intent.role.kind === "user"),
      superuser: metadataBool(intent.role.superuser, false),
      createDatabase: metadataBool(intent.role.createDatabase, false),
      createRole: metadataBool(intent.role.createRole, false),
      inherit: metadataBool(intent.role.inherit, true),
    };
  }
  if (intent.type === "grant_role" || intent.type === "revoke_role") {
    return {
      ...base,
      roleKey: firstRoleOption(intent, roles) ? roleKey(firstRoleOption(intent, roles)!) : "",
    };
  }
  return {
    ...base,
    name: intent.role.name,
    host: intent.role.host ?? "",
  };
}

function targetFromForm(form: FormState, engine: DatabaseEngine): DatabasePrivilegeTarget {
  const clean = (value: string): string | null => value.trim() || null;
  switch (form.targetKind) {
    case "global":
      return { kind: "global" };
    case "database":
      return { kind: "database", database: clean(form.database) };
    case "schema":
      return { kind: "schema", schema: clean(form.schema) };
    case "table":
      return {
        kind: "table",
        database: engine === "mysql" ? clean(form.database) : null,
        schema: engine === "postgresql" ? clean(form.schema) : null,
        object: clean(form.object),
      };
    default:
      return {
        kind: "sequence",
        schema: engine === "postgresql" ? clean(form.schema) : null,
        object: clean(form.object),
      };
  }
}

function requestForIntent(
  intent: DatabaseRoleIntent,
  form: FormState,
  engine: DatabaseEngine,
  roles: DatabaseRoleSummary[],
): DatabaseRoleActionRequest {
  if (intent.type === "create") {
    return {
      kind: "create_role",
      name: form.name,
      host: engine === "mysql" ? form.host : null,
      roleKind: form.kind,
      canLogin: form.kind === "role" ? false : form.canLogin,
      superuser: engine === "postgresql" ? form.superuser : null,
      createDatabase: engine === "postgresql" ? form.createDatabase : null,
      createRole: engine === "postgresql" ? form.createRole : null,
      inherit: engine === "postgresql" ? form.inherit : null,
    };
  }
  if (intent.type === "alter") {
    return {
      kind: "alter_role",
      name: form.name,
      host: engine === "mysql" ? form.host : null,
      canLogin: form.canLogin,
      superuser: engine === "postgresql" ? form.superuser : null,
      createDatabase: engine === "postgresql" ? form.createDatabase : null,
      createRole: engine === "postgresql" ? form.createRole : null,
      inherit: engine === "postgresql" ? form.inherit : null,
    };
  }
  if (intent.type === "grant_privileges") {
    return {
      kind: "grant_privileges",
      grantee: refFor(intent.role),
      privileges: form.privileges,
      target: targetFromForm(form, engine),
      withGrantOption: form.grantOption,
    };
  }
  if (intent.type === "revoke_privileges") {
    return {
      kind: "revoke_privileges",
      grantee: refFor(intent.role),
      privileges: form.privileges,
      target: targetFromForm(form, engine),
      revokeGrantOption: form.grantOption,
    };
  }
  const selected = roles.find((role) => roleKey(role) === form.roleKey);
  const role = selected ? refFor(selected) : { name: form.roleKey, host: null };
  return {
    kind: intent.type === "grant_role" ? "grant_role" : "revoke_role",
    role,
    member: refFor(intent.member),
  };
}

function intentTitle(intent: DatabaseRoleIntent): string {
  switch (intent.type) {
    case "create":
      return "Create user or role";
    case "alter":
      return `Modify ${roleLabel(refFor(intent.role))}`;
    case "grant_privileges":
      return `Grant privileges to ${roleLabel(refFor(intent.role))}`;
    case "revoke_privileges":
      return `Revoke privileges from ${roleLabel(refFor(intent.role))}`;
    case "grant_role":
      return `Grant a role to ${roleLabel(refFor(intent.member))}`;
    default:
      return `Revoke a role from ${roleLabel(refFor(intent.member))}`;
  }
}

function AttributeCheckbox({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="flex items-center gap-1.5 rounded-sm border border-border px-2 py-1 text-[11px]">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      {label}
    </label>
  );
}

function DiffList({
  title,
  grants,
  emptyText,
}: {
  title: string;
  grants: DatabasePrivilegeDiff["added"];
  emptyText: string;
}) {
  return (
    <div className="space-y-1">
      <p className="text-[11px] font-semibold text-foreground">{title}</p>
      {grants.length === 0 ? (
        <p className="text-[11px] text-muted-foreground">{emptyText}</p>
      ) : (
        <ul className="space-y-1">
          {grants.map((grant) => (
            <li
              key={`${grant.privilege}:${privilegeTargetLabel(grant.target)}:${grant.grantable}`}
              className="rounded-sm border border-border bg-muted/25 px-2 py-1 font-mono text-[11px]"
            >
              {privilegeLabel(grant.privilege)} on {privilegeTargetLabel(grant.target)}
              {grant.grantable ? " · grant option" : ""}
              {grant.sources.length > 0 ? ` · ${grant.sources.join(", ")}` : ""}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

type DatabaseRoleActionHostProps = {
  profileId: string | null;
  profileName: string;
  engine: DatabaseEngine;
  connected: boolean;
  readOnly: boolean;
  intent: DatabaseRoleIntent | null;
  roles: DatabaseRoleSummary[];
  scope: DatabaseTableScope;
  onClose: () => void;
  onCompleted: (result: DatabaseRoleActionResult) => void;
};

function DatabaseRoleActionHost({
  profileId,
  profileName,
  engine,
  connected,
  readOnly,
  intent,
  roles,
  scope,
  onClose,
  onCompleted,
}: DatabaseRoleActionHostProps) {
  const [form, setForm] = useState<FormState>(() => formForIntent(intent, engine, scope, roles));
  const [preview, setPreview] = useState<DatabaseRoleDdlPreview | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const blockedReason = !connected
    ? `Connect ${profileName} before changing users, roles, or grants.`
    : readOnly
      ? "This database profile is read-only. Save a writable profile before changing roles or grants."
      : !profileId
        ? "Open a saved database connection first."
        : null;

  const options = privilegeOptionsFor(engine, form.targetKind);
  const membership: Extract<DatabaseRoleIntent, { type: "grant_role" | "revoke_role" }> | null =
    intent?.type === "grant_role" || intent?.type === "revoke_role" ? intent : null;

  const canPreview = useMemo(() => {
    if (!intent || blockedReason) {
      return false;
    }
    if (intent.type === "create" || intent.type === "alter") {
      return form.name.trim().length > 0;
    }
    if (intent.type === "grant_role" || intent.type === "revoke_role") {
      return form.roleKey.trim().length > 0;
    }
    if (form.privileges.length === 0) {
      return false;
    }
    if (form.targetKind === "database") {
      return form.database.trim().length > 0;
    }
    if (form.targetKind === "schema") {
      return form.schema.trim().length > 0;
    }
    if (form.targetKind === "table" || form.targetKind === "sequence") {
      return (
        form.object.trim().length > 0 &&
        (engine === "mysql" ? form.database.trim().length > 0 : form.schema.trim().length > 0)
      );
    }
    return true;
  }, [blockedReason, engine, form, intent]);

  function update<K extends keyof FormState>(key: K, value: FormState[K]): void {
    setForm((current) => ({ ...current, [key]: value }));
    setPreview(null);
  }

  function changeTargetKind(kind: DatabasePrivilegeTargetKind): void {
    const target = defaultPrivilegeTarget(engine, kind, scope);
    setForm((current) => ({
      ...current,
      targetKind: kind,
      database: target.database ?? current.database,
      schema: target.schema ?? current.schema,
      object: target.object ?? current.object,
      privileges: privilegeOptionsFor(engine, kind).slice(0, 1),
    }));
    setPreview(null);
  }

  function togglePrivilege(privilege: DatabasePrivilege): void {
    setForm((current) => ({
      ...current,
      privileges: current.privileges.includes(privilege)
        ? current.privileges.filter((entry) => entry !== privilege)
        : [...current.privileges, privilege],
    }));
    setPreview(null);
  }

  async function loadPreview(): Promise<void> {
    if (!intent || !profileId) {
      return;
    }
    setPreviewing(true);
    setError(null);
    try {
      const next = await previewDatabaseRoleAction(
        profileId,
        requestForIntent(intent, form, engine, roles),
      );
      setPreview(next);
    } catch (previewError) {
      setError(getCommandErrorMessage(previewError));
    } finally {
      setPreviewing(false);
    }
  }

  async function execute(): Promise<void> {
    if (!intent || !profileId) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      const result = await runDatabaseRoleAction(
        profileId,
        requestForIntent(intent, form, engine, roles),
        true,
      );
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
    setConfirmOpen(true);
  }

  const showRoleForm = intent?.type === "create" || intent?.type === "alter";
  const showPrivilegeForm =
    intent?.type === "grant_privileges" || intent?.type === "revoke_privileges";

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
        <DialogContent className="max-h-[92vh] max-w-3xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{intent ? intentTitle(intent) : ""}</DialogTitle>
            <DialogDescription>
              Every role and privilege change requires an exact preview and explicit confirmation.
              ServerDesk never reads or displays passwords or password hashes.
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
                <AlertTitle>Role action failed</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </div>
            </Alert>
          ) : null}

          {showRoleForm ? (
            <div className="grid gap-3">
              <div className="grid grid-cols-2 gap-3">
                <div className="grid gap-1.5">
                  <Label htmlFor="role-name">Name</Label>
                  <Input
                    id="role-name"
                    value={form.name}
                    onChange={(event) => update("name", event.currentTarget.value)}
                    disabled={intent?.type === "alter"}
                    className="font-mono"
                  />
                </div>
                {engine === "mysql" ? (
                  <div className="grid gap-1.5">
                    <Label htmlFor="role-host">Account host</Label>
                    <Input
                      id="role-host"
                      value={form.host}
                      onChange={(event) => update("host", event.currentTarget.value)}
                      disabled={intent?.type === "alter"}
                      className="font-mono"
                    />
                  </div>
                ) : null}
              </div>

              {intent?.type === "create" ? (
                <div className="grid gap-1.5">
                  <Label>Kind</Label>
                  <Select
                    value={form.kind}
                    onValueChange={(value) => {
                      const kind = value as DatabaseRoleKind;
                      setForm((current) => ({
                        ...current,
                        kind,
                        canLogin: kind === "user" ? current.canLogin : false,
                      }));
                      setPreview(null);
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="user">User (can log in)</SelectItem>
                      <SelectItem value="role">Role (cannot log in)</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              ) : null}

              <div className="grid gap-1.5">
                <Label>Attributes</Label>
                <div className="flex flex-wrap gap-2">
                  <AttributeCheckbox
                    label="Can log in"
                    checked={
                      intent?.type === "create" && form.kind === "role" ? false : form.canLogin
                    }
                    disabled={
                      (intent?.type === "create" && form.kind === "role") ||
                      (engine === "mysql" && form.kind === "role")
                    }
                    onChange={(value) => update("canLogin", value)}
                  />
                  {engine === "postgresql" ? (
                    <>
                      <AttributeCheckbox
                        label="Superuser"
                        checked={form.superuser}
                        onChange={(value) => update("superuser", value)}
                      />
                      <AttributeCheckbox
                        label="Create databases"
                        checked={form.createDatabase}
                        onChange={(value) => update("createDatabase", value)}
                      />
                      <AttributeCheckbox
                        label="Create roles"
                        checked={form.createRole}
                        onChange={(value) => update("createRole", value)}
                      />
                      <AttributeCheckbox
                        label="Inherit roles"
                        checked={form.inherit}
                        onChange={(value) => update("inherit", value)}
                      />
                    </>
                  ) : null}
                </div>
                <p className="text-[11px] text-muted-foreground">
                  {engine === "mysql" && intent?.type === "create"
                    ? "MySQL users are created locked. Configure authentication outside ServerDesk, then unlock the account with Modify."
                    : "No password is set. Configure authentication outside ServerDesk before allowing this account to sign in."}
                </p>
              </div>
            </div>
          ) : null}

          {showPrivilegeForm ? (
            <div className="grid gap-3">
              <div className="grid grid-cols-2 gap-3">
                <div className="grid gap-1.5">
                  <Label>Target kind</Label>
                  <Select
                    value={form.targetKind}
                    onValueChange={(value) =>
                      changeTargetKind(value as DatabasePrivilegeTargetKind)
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {privilegeTargetKindOptions(engine).map((kind) => (
                        <SelectItem key={kind} value={kind}>
                          {privilegeTargetKindLabel(kind)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                {form.targetKind === "database" || form.targetKind === "table" ? (
                  <div className="grid gap-1.5">
                    <Label htmlFor="privilege-database">Database</Label>
                    <Input
                      id="privilege-database"
                      value={form.database}
                      onChange={(event) => update("database", event.currentTarget.value)}
                      className="font-mono"
                    />
                  </div>
                ) : null}
                {form.targetKind === "schema" ||
                form.targetKind === "table" ||
                form.targetKind === "sequence" ? (
                  <div className="grid gap-1.5">
                    <Label htmlFor="privilege-schema">Schema</Label>
                    <Input
                      id="privilege-schema"
                      value={form.schema}
                      onChange={(event) => update("schema", event.currentTarget.value)}
                      className="font-mono"
                    />
                  </div>
                ) : null}
                {form.targetKind === "table" || form.targetKind === "sequence" ? (
                  <div className="grid gap-1.5">
                    <Label htmlFor="privilege-object">
                      {form.targetKind === "table" ? "Table" : "Sequence"}
                    </Label>
                    <Input
                      id="privilege-object"
                      value={form.object}
                      onChange={(event) => update("object", event.currentTarget.value)}
                      className="font-mono"
                    />
                  </div>
                ) : null}
              </div>

              <div className="grid gap-1.5">
                <Label>Privileges</Label>
                <div className="flex flex-wrap gap-2">
                  {options.map((privilege) => (
                    <label
                      key={privilege}
                      className="flex items-center gap-1.5 rounded-sm border border-border px-2 py-1 text-[11px]"
                    >
                      <input
                        type="checkbox"
                        checked={form.privileges.includes(privilege)}
                        onChange={() => togglePrivilege(privilege)}
                      />
                      {privilegeLabel(privilege)}
                    </label>
                  ))}
                </div>
              </div>

              <AttributeCheckbox
                label={
                  intent?.type === "grant_privileges"
                    ? "WITH GRANT OPTION"
                    : "Revoke grant option only"
                }
                checked={form.grantOption}
                onChange={(value) => update("grantOption", value)}
              />
            </div>
          ) : null}

          {membership ? (
            <div className="grid gap-1.5">
              <Label>Role to {membership.type === "grant_role" ? "grant" : "revoke"}</Label>
              <Select value={form.roleKey} onValueChange={(value) => update("roleKey", value)}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {roles
                    .filter((role) => roleKey(role) !== roleKey(membership.member))
                    .map((role) => (
                      <SelectItem key={roleKey(role)} value={roleKey(role)}>
                        {roleKey(role)}
                      </SelectItem>
                    ))}
                </SelectContent>
              </Select>
            </div>
          ) : null}

          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void loadPreview()}
              disabled={!canPreview || previewing || pending}
            >
              {previewing ? "Generating preview..." : "Preview exact changes"}
            </Button>
            <Button
              size="sm"
              onClick={handleRun}
              disabled={!preview || pending || Boolean(blockedReason)}
            >
              Confirm and apply
            </Button>
          </div>

          {preview ? (
            <div className="space-y-3">
              <OutputPanel
                title="Exact statements"
                content={preview.ddl}
                maxHeightClassName="max-h-56"
              />
              {preview.warnings.length > 0 ? (
                <Alert variant="warning" className="py-2.5">
                  <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                  <div>
                    <AlertTitle>Review before confirming</AlertTitle>
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

              {preview.roleChanges.length > 0 ? (
                <div className="space-y-1">
                  <p className="text-[11px] font-semibold text-foreground">Role attribute diff</p>
                  <div className="overflow-hidden rounded-md border border-border">
                    <table className="w-full text-left text-[11px]">
                      <thead className="bg-muted/40 text-muted-foreground">
                        <tr>
                          <th className="px-2 py-1 font-medium">Attribute</th>
                          <th className="px-2 py-1 font-medium">Before</th>
                          <th className="px-2 py-1 font-medium">After</th>
                        </tr>
                      </thead>
                      <tbody>
                        {preview.roleChanges.map((change) => (
                          <tr key={change.attribute} className="border-t border-border">
                            <td className="px-2 py-1 font-mono">{change.attribute}</td>
                            <td className="px-2 py-1">{change.before}</td>
                            <td className="px-2 py-1">{change.after}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              ) : null}

              <div className="grid gap-3 md:grid-cols-2">
                <DiffList
                  title="Added effective privileges"
                  grants={preview.privilegeDiff.added}
                  emptyText="No privilege is added."
                />
                <DiffList
                  title="Removed effective privileges"
                  grants={preview.privilegeDiff.removed}
                  emptyText="No privilege is removed."
                />
              </div>
              <div className="grid gap-3 md:grid-cols-2">
                <DiffList
                  title="Before"
                  grants={preview.privilegeDiff.before}
                  emptyText="No effective privileges before this change."
                />
                <DiffList
                  title="After"
                  grants={preview.privilegeDiff.after}
                  emptyText="No effective privileges after this change."
                />
              </div>
            </div>
          ) : null}

          <DialogFooter>
            <Button variant="secondary" onClick={onClose} disabled={pending}>
              Cancel
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title={intent ? `Confirm ${intentTitle(intent).toLowerCase()}` : "Confirm role change"}
        description="This privilege change is sent to the selected database exactly as previewed. Confirm the account, target, and privileges."
        confirmLabel="Apply privilege change"
        onConfirm={() => void execute()}
        pending={pending}
        destructive={preview?.destructive ?? true}
        warningTitle="Privileges control access to data"
        warningDescription="A broad grant can expose every row, and a revoke can lock an application out. Re-read the preview before applying it."
        details={[
          {
            label: "Account",
            value: preview ? roleLabel(preview.role) : "",
          },
          {
            label: "Target",
            value: preview?.target ? privilegeTargetLabel(preview.target) : "Role attributes",
          },
          {
            label: "Changes",
            value: preview
              ? `${preview.privilegeDiff.added.length} added, ${preview.privilegeDiff.removed.length} removed`
              : "",
          },
        ]}
        preview={preview ? { label: "Exact statements", text: preview.ddl } : null}
      />
    </>
  );
}

export { DatabaseRoleActionHost };
