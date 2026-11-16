import { useEffect, useMemo, useState } from "react";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  PencilSimpleIcon,
  PlusIcon,
  ShieldCheckIcon,
  UserPlusIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import {
  DatabaseRoleActionHost,
  type DatabaseRoleIntent,
} from "@/features/databases/DatabaseRoleDialogs";
import {
  metadataBoolLabel,
  privilegeLabel,
  privilegeTargetLabel,
  roleAttributeRows,
  roleKindLabel,
  roleLabel,
  type RoleCapabilities,
} from "@/features/databases/databaseRolesModel";
import type { DatabaseTableScope } from "@/features/databases/databaseStructureModel";
import { cn } from "@/lib/utils";
import { useDatabaseRoleStore } from "@/stores/databaseRoleStore";
import type {
  DatabaseEngine,
  DatabaseRoleActionResult,
  DatabaseRolePrivilegeSnapshot,
  DatabaseRoleRef,
  DatabaseRoleSummary,
} from "@/types/contracts";

type DatabaseAccessPanelProps = {
  profileId: string | null;
  profileName: string;
  engine: DatabaseEngine;
  connected: boolean;
  readOnly: boolean;
  capabilities: RoleCapabilities;
  scope: DatabaseTableScope;
};

function refFor(role: DatabaseRoleSummary): DatabaseRoleRef {
  return { name: role.name, host: role.host ?? null };
}

function roleKey(role: DatabaseRoleSummary): string {
  return roleLabel(refFor(role));
}

function intentKey(intent: DatabaseRoleIntent | null): string {
  if (!intent) {
    return "none";
  }
  if ("role" in intent) {
    return `${intent.type}:${roleLabel(refFor(intent.role))}`;
  }
  if ("member" in intent) {
    return `${intent.type}:${roleLabel(refFor(intent.member))}`;
  }
  return intent.type;
}

function PrivilegeTable({
  title,
  emptyText,
  grants,
  showSources,
}: {
  title: string;
  emptyText: string;
  grants: Array<{
    privilege: string;
    target: DatabaseRolePrivilegeSnapshot["direct"][number]["target"];
    grantable: boolean;
    sources?: string[];
  }>;
  showSources?: boolean;
}) {
  return (
    <div className="space-y-1.5">
      <h4 className="text-[11px] font-semibold text-foreground">{title}</h4>
      {grants.length === 0 ? (
        <p className="text-[11px] text-muted-foreground">{emptyText}</p>
      ) : (
        <div className="overflow-hidden rounded-md border border-border">
          <table className="w-full text-left text-[11px]">
            <thead className="bg-muted/40 text-muted-foreground">
              <tr>
                <th className="px-2 py-1 font-medium">Privilege</th>
                <th className="px-2 py-1 font-medium">Target</th>
                <th className="px-2 py-1 font-medium">Grant option</th>
                {showSources ? <th className="px-2 py-1 font-medium">Source</th> : null}
              </tr>
            </thead>
            <tbody>
              {grants.map((grant) => (
                <tr
                  key={`${grant.privilege}:${privilegeTargetLabel(grant.target)}:${
                    grant.grantable
                  }:${grant.sources?.join(",") ?? ""}`}
                  className="border-t border-border"
                >
                  <td className="px-2 py-1 font-mono">{privilegeLabel(grant.privilege)}</td>
                  <td className="px-2 py-1 font-mono">{privilegeTargetLabel(grant.target)}</td>
                  <td className="px-2 py-1">{grant.grantable ? "Yes" : "No"}</td>
                  {showSources ? (
                    <td className="px-2 py-1">{grant.sources?.join(", ") || "direct"}</td>
                  ) : null}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function DatabaseAccessPanel({
  profileId,
  profileName,
  engine,
  connected,
  readOnly,
  capabilities,
  scope,
}: DatabaseAccessPanelProps) {
  const entry = useDatabaseRoleStore((state) => (profileId ? state.entries[profileId] : undefined));
  const fetchRoles = useDatabaseRoleStore((state) => state.fetchRoles);
  const selectRole = useDatabaseRoleStore((state) => state.selectRole);
  const fetchSnapshot = useDatabaseRoleStore((state) => state.fetchSnapshot);
  const refresh = useDatabaseRoleStore((state) => state.refresh);
  const [intent, setIntent] = useState<DatabaseRoleIntent | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [verificationError, setVerificationError] = useState<string | null>(null);

  const roles = useMemo(() => entry?.roles ?? [], [entry?.roles]);
  const selectedKey = entry?.selectedKey ?? null;
  const rolesLoading = entry?.status === "loading";
  const rolesError = entry?.error ?? null;
  const snapshot = entry?.snapshot ?? null;
  const snapshotLoading = entry?.snapshotStatus === "loading";
  const snapshotError = verificationError ?? entry?.snapshotError ?? null;

  useEffect(() => {
    if (profileId && connected && capabilities.browse.enabled) {
      void fetchRoles(profileId);
    }
  }, [capabilities.browse.enabled, connected, fetchRoles, profileId]);

  const selectedRole = useMemo(
    () => roles.find((role) => roleKey(role) === selectedKey) ?? null,
    [roles, selectedKey],
  );

  useEffect(() => {
    if (profileId && connected && capabilities.browse.enabled && selectedRole) {
      void fetchSnapshot(profileId, refFor(selectedRole));
    }
  }, [capabilities.browse.enabled, connected, fetchSnapshot, profileId, selectedRole]);

  function openIntent(next: DatabaseRoleIntent): void {
    setNotice(null);
    setVerificationError(null);
    setIntent(next);
  }

  async function handleCompleted(result: DatabaseRoleActionResult): Promise<void> {
    if (result.verified) {
      setNotice(result.verification);
      setVerificationError(null);
    } else {
      setNotice(null);
      setVerificationError(
        result.verificationError ??
          result.verification ??
          "The privilege action finished, but verification did not confirm the result.",
      );
    }
    if (profileId) {
      await refresh(profileId);
    }
  }

  if (!connected) {
    return (
      <Alert variant="warning">
        <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div>
          <AlertTitle>Connection required</AlertTitle>
          <AlertDescription>
            Connect {profileName} before listing users, roles, or grants.
          </AlertDescription>
        </div>
      </Alert>
    );
  }

  if (!capabilities.browse.enabled) {
    return (
      <Alert variant="warning">
        <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div>
          <AlertTitle>Users and roles are unavailable</AlertTitle>
          <AlertDescription>
            {capabilities.browse.reason ??
              "This engine or server version does not expose role metadata."}
          </AlertDescription>
        </div>
      </Alert>
    );
  }

  if (rolesLoading && roles.length === 0) {
    return (
      <div className="space-y-3" aria-busy="true" aria-label="Loading database roles">
        <Skeleton className="h-5 w-56" />
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  if (rolesError) {
    return (
      <Alert variant="destructive">
        <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div className="flex-1">
          <AlertTitle>Could not list users and roles</AlertTitle>
          <AlertDescription>{rolesError}</AlertDescription>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => profileId && void fetchRoles(profileId)}
        >
          Try again
        </Button>
      </Alert>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="flex items-center gap-2 text-[15px] font-semibold tracking-[-0.01em]">
            <ShieldCheckIcon className="size-4" aria-hidden="true" />
            Users, roles &amp; grants
            <Badge variant="outline">Advanced</Badge>
          </h2>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            Effective privileges expand granted roles. Passwords, authentication plugins, and
            password hashes are never selected or displayed.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => profileId && void fetchRoles(profileId)}
            disabled={rolesLoading}
            aria-label="Refresh users and roles"
            title="Refresh users and roles"
          >
            {rolesLoading ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
          </Button>
          <Button
            size="sm"
            onClick={() => openIntent({ type: "create" })}
            disabled={!capabilities.manage.enabled}
            title={capabilities.manage.reason ?? "Create a user or role"}
          >
            <UserPlusIcon className="size-3.5" aria-hidden="true" />
            New user or role
          </Button>
        </div>
      </div>

      {!capabilities.manage.enabled ? (
        <Alert variant="warning" className="py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div>
            <AlertTitle>Read-only access</AlertTitle>
            <AlertDescription>
              {capabilities.manage.reason ??
                "This connection cannot change roles or grants. Listing and inspection remain available."}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {notice ? (
        <Alert variant="success" className="py-2.5">
          <ShieldCheckIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div className="flex-1">
            <AlertTitle>Privilege change verified</AlertTitle>
            <AlertDescription>{notice}</AlertDescription>
          </div>
          <Button variant="ghost" size="sm" onClick={() => setNotice(null)}>
            Dismiss
          </Button>
        </Alert>
      ) : null}

      {roles.length === 0 ? (
        <EmptyState
          icon={<ShieldCheckIcon className="size-4" aria-hidden="true" />}
          title="No users or roles"
          description="This connection did not return any accounts. The database account may lack visibility into role metadata."
        />
      ) : (
        <div className="grid gap-4 lg:grid-cols-[18rem_minmax(0,1fr)]">
          <div className="space-y-1.5">
            <p className="text-[11px] font-semibold text-muted-foreground">
              {roles.length} account{roles.length === 1 ? "" : "s"}
            </p>
            <div className="max-h-[28rem] space-y-1 overflow-y-auto pr-1">
              {roles.map((role) => {
                const key = roleKey(role);
                return (
                  <button
                    key={key}
                    type="button"
                    onClick={() => {
                      if (profileId) {
                        selectRole(profileId, key);
                      }
                      setNotice(null);
                      setVerificationError(null);
                    }}
                    className={cn(
                      "w-full rounded-md border px-2.5 py-2 text-left transition-colors",
                      key === selectedKey
                        ? "border-primary/40 bg-primary/5"
                        : "border-border bg-card hover:bg-muted/40",
                    )}
                    aria-pressed={key === selectedKey}
                  >
                    <span className="flex items-center justify-between gap-2">
                      <span className="truncate font-mono text-[12px] font-medium">{key}</span>
                      <Badge variant={role.kind === "role" ? "secondary" : "outline"}>
                        {roleKindLabel(role.kind)}
                      </Badge>
                    </span>
                    <span className="mt-0.5 block text-[10px] text-muted-foreground">
                      login {metadataBoolLabel(role.canLogin)} · superuser{" "}
                      {metadataBoolLabel(role.superuser)}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className="min-w-0 space-y-3">
            {selectedRole ? (
              <>
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <h3 className="font-mono text-[14px] font-semibold">
                      {roleLabel(refFor(selectedRole))}
                    </h3>
                    <p className="mt-0.5 text-[11px] text-muted-foreground">
                      {roleKindLabel(selectedRole.kind)} on {profileName}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => openIntent({ type: "alter", role: selectedRole })}
                      disabled={!capabilities.manage.enabled}
                      title={capabilities.manage.reason ?? "Modify role attributes"}
                    >
                      <PencilSimpleIcon className="size-3.5" aria-hidden="true" />
                      Modify
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => openIntent({ type: "grant_privileges", role: selectedRole })}
                      disabled={!capabilities.manage.enabled}
                      title={capabilities.manage.reason ?? "Grant privileges"}
                    >
                      <PlusIcon className="size-3.5" aria-hidden="true" />
                      Grant
                    </Button>
                    <Button
                      variant="destructive"
                      size="sm"
                      onClick={() => openIntent({ type: "revoke_privileges", role: selectedRole })}
                      disabled={!capabilities.manage.enabled}
                      title={capabilities.manage.reason ?? "Revoke privileges"}
                    >
                      Revoke
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => openIntent({ type: "grant_role", member: selectedRole })}
                      disabled={!capabilities.manage.enabled || roles.length < 2}
                      title={capabilities.manage.reason ?? "Grant a role to this account"}
                    >
                      Grant role
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => openIntent({ type: "revoke_role", member: selectedRole })}
                      disabled={!capabilities.manage.enabled || roles.length < 2}
                      title={capabilities.manage.reason ?? "Revoke a role from this account"}
                    >
                      Revoke role
                    </Button>
                  </div>
                </div>

                <dl className="grid grid-cols-[130px_minmax(0,1fr)] gap-x-3 gap-y-1.5 rounded-md border border-border bg-muted/25 px-3.5 py-3">
                  {roleAttributeRows(selectedRole).map((row) => (
                    <div key={row.label} className="contents">
                      <dt className="text-[11px] text-muted-foreground">{row.label}</dt>
                      <dd className="text-[12px]">{row.value}</dd>
                    </div>
                  ))}
                </dl>

                {snapshotLoading ? (
                  <div className="space-y-2" aria-busy="true" aria-label="Loading role privileges">
                    <Skeleton className="h-5 w-40" />
                    <Skeleton className="h-24 w-full" />
                  </div>
                ) : snapshotError ? (
                  <Alert variant="destructive" className="py-2.5">
                    <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                    <div className="flex-1">
                      <AlertTitle>Could not inspect privileges</AlertTitle>
                      <AlertDescription>{snapshotError}</AlertDescription>
                    </div>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() =>
                        profileId &&
                        selectedRole &&
                        void fetchSnapshot(profileId, refFor(selectedRole))
                      }
                    >
                      Retry
                    </Button>
                  </Alert>
                ) : snapshot ? (
                  <div className="space-y-4">
                    <div className="space-y-1.5">
                      <h4 className="text-[11px] font-semibold text-foreground">Member of</h4>
                      {snapshot.memberOf.length === 0 ? (
                        <p className="text-[11px] text-muted-foreground">
                          No granted roles. Effective privileges come from direct grants only.
                        </p>
                      ) : (
                        <div className="flex flex-wrap gap-1.5">
                          {snapshot.memberOf.map((role) => (
                            <Badge key={roleLabel(role)} variant="secondary" className="font-mono">
                              {roleLabel(role)}
                            </Badge>
                          ))}
                        </div>
                      )}
                    </div>

                    <PrivilegeTable
                      title="Direct grants"
                      emptyText="This account has no direct object privileges."
                      grants={snapshot.direct}
                    />
                    <PrivilegeTable
                      title="Effective privileges"
                      emptyText="No effective object privileges were reported."
                      grants={snapshot.effective}
                      showSources
                    />
                  </div>
                ) : null}

                <p className="text-[10px] leading-4 text-muted-foreground">
                  Effective privileges show the union of direct grants and granted roles. MySQL
                  roles apply only after activation; a PostgreSQL NOINHERIT member may need SET ROLE
                  before inherited privileges apply.
                </p>
              </>
            ) : (
              <EmptyState
                icon={<ShieldCheckIcon className="size-4" aria-hidden="true" />}
                title="Select an account"
                description="Choose a user or role to inspect direct grants, memberships, and effective privileges."
              />
            )}
          </div>
        </div>
      )}

      {intent ? (
        <DatabaseRoleActionHost
          key={intentKey(intent)}
          profileId={profileId}
          profileName={profileName}
          engine={engine}
          connected={connected}
          readOnly={readOnly}
          intent={intent}
          roles={roles}
          scope={scope}
          onClose={() => setIntent(null)}
          onCompleted={(result) => void handleCompleted(result)}
        />
      ) : null}
    </div>
  );
}

export { DatabaseAccessPanel };
