import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  PencilSimpleIcon,
  TrashIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { OutputPanel } from "@/components/ui/output-panel";
import { Skeleton } from "@/components/ui/skeleton";
import type { DatabaseObjectIntent } from "@/features/databases/DatabaseObjectDialogs";
import {
  objectDefinitionLines,
  objectDefinitionTitle,
  objectKindLabel,
  type ObjectCapabilities,
} from "@/features/databases/databaseObjectModel";
import { getCommandErrorMessage } from "@/lib/errors";
import { inspectDatabaseObject } from "@/services/databaseObjects";
import type {
  DatabaseObjectDefinition,
  DatabaseObjectRef,
  DatabaseProfile,
} from "@/types/contracts";

type DatabaseObjectPanelProps = {
  profile: DatabaseProfile | null;
  profileName: string;
  connected: boolean;
  object: DatabaseObjectRef;
  capabilities: ObjectCapabilities;
  onIntent: (intent: DatabaseObjectIntent) => void;
  onRefreshExplorer: () => void;
};

function ActionButton({
  decision,
  label,
  icon,
  onClick,
  destructive,
}: {
  decision: { enabled: boolean; reason: string | null };
  label: string;
  icon: ReactNode;
  onClick: () => void;
  destructive?: boolean;
}) {
  return (
    <Button
      variant={destructive ? "destructive" : "secondary"}
      size="sm"
      onClick={onClick}
      disabled={!decision.enabled}
      title={decision.reason ?? label}
    >
      {icon}
      {label}
    </Button>
  );
}

function DatabaseObjectPanel({
  profile,
  profileName,
  connected,
  object,
  capabilities,
  onIntent,
  onRefreshExplorer,
}: DatabaseObjectPanelProps) {
  const [definition, setDefinition] = useState<DatabaseObjectDefinition | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!profile || !connected) {
      setDefinition(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setDefinition(await inspectDatabaseObject(profile.id, object));
    } catch (loadError) {
      setDefinition(null);
      setError(getCommandErrorMessage(loadError));
    } finally {
      setLoading(false);
    }
  }, [connected, object, profile]);

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (!cancelled) {
        void load();
      }
    });
    return () => {
      cancelled = true;
    };
  }, [load]);

  if (!connected) {
    return (
      <div className="space-y-3">
        <Alert variant="warning">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div>
            <AlertTitle>Connection required</AlertTitle>
            <AlertDescription>
              Connect {profileName} to inspect this {objectKindLabel(object.kind).toLowerCase()}.
            </AlertDescription>
          </div>
        </Alert>
      </div>
    );
  }

  if (loading && !definition) {
    return (
      <div className="space-y-3" aria-busy="true" aria-label="Loading object definition">
        <Skeleton className="h-5 w-48" />
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div className="flex-1">
          <AlertTitle>Could not inspect object</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </div>
        <Button variant="secondary" size="sm" onClick={() => void load()}>
          Try again
        </Button>
      </Alert>
    );
  }

  if (!definition) {
    return null;
  }

  const rows = objectDefinitionLines(definition);
  const definitionText =
    definition.definition.status === "available" ? definition.definition.value : "";
  const isMaterialized = definition.kind === "materialized_view";

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-[15px] font-semibold tracking-[-0.01em]">
            {objectDefinitionTitle(definition)}
          </h2>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            {objectKindLabel(definition.kind)} on {profileName}
            {definition.truncated ? " · definition truncated" : ""}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => void load()}
            disabled={loading}
            aria-label="Reload object definition"
            title="Reload object definition"
          >
            {loading ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
          </Button>
          {isMaterialized ? (
            <ActionButton
              decision={capabilities.refreshMaterializedView}
              label="Refresh"
              icon={<ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />}
              onClick={() => onIntent({ type: "refresh", definition })}
            />
          ) : null}
          {definition.kind === "view" ? (
            <ActionButton
              decision={capabilities.replaceView}
              label="Replace"
              icon={<PencilSimpleIcon className="size-3.5" aria-hidden="true" />}
              onClick={() => onIntent({ type: "edit", definition })}
            />
          ) : null}
          {definition.kind === "routine" ? (
            <ActionButton
              decision={capabilities.editRoutine}
              label="Edit"
              icon={<PencilSimpleIcon className="size-3.5" aria-hidden="true" />}
              onClick={() => onIntent({ type: "edit", definition })}
            />
          ) : null}
          {definition.kind === "trigger" ? (
            <ActionButton
              decision={capabilities.editTrigger}
              label="Edit"
              icon={<PencilSimpleIcon className="size-3.5" aria-hidden="true" />}
              onClick={() => onIntent({ type: "edit", definition })}
            />
          ) : null}
          {definition.kind === "sequence" ? (
            <ActionButton
              decision={capabilities.alterSequence}
              label="Alter"
              icon={<PencilSimpleIcon className="size-3.5" aria-hidden="true" />}
              onClick={() => onIntent({ type: "alter_sequence", definition })}
            />
          ) : null}
          <ActionButton
            decision={
              definition.kind === "materialized_view"
                ? capabilities.dropMaterializedView
                : definition.kind === "view"
                  ? capabilities.dropView
                  : definition.kind === "routine"
                    ? capabilities.dropRoutine
                    : definition.kind === "trigger"
                      ? capabilities.dropTrigger
                      : capabilities.dropSequence
            }
            label="Drop"
            destructive
            icon={<TrashIcon className="size-3.5" aria-hidden="true" />}
            onClick={() => onIntent({ type: "drop", definition })}
          />
        </div>
      </div>

      <dl className="grid grid-cols-[110px_minmax(0,1fr)] gap-x-3 gap-y-2 rounded-md border border-border bg-muted/25 px-3.5 py-3">
        {rows.map((row) => (
          <div key={row.label} className="contents">
            <dt className="text-[11px] leading-5 text-muted-foreground">{row.label}</dt>
            <dd
              className={
                row.mono
                  ? "break-words font-mono text-[12px] leading-5 text-foreground"
                  : "break-words text-[12px] leading-5 text-foreground"
              }
            >
              {row.value}
            </dd>
          </div>
        ))}
      </dl>

      {definition.trigger?.statement.status === "available" ? (
        <OutputPanel
          title="Trigger statement"
          content={definition.trigger.statement.value}
          maxHeightClassName="max-h-52"
        />
      ) : null}

      <OutputPanel
        title="Definition"
        content={definitionText}
        emptyText="The database account cannot read this definition."
        maxHeightClassName="max-h-96"
      />

      <p className="text-[10px] leading-4 text-muted-foreground">
        Definitions are shown as text and never executed by the UI. Generated DDL is previewed in a
        separate dialog before execution.
      </p>
      <Button variant="ghost" size="sm" onClick={onRefreshExplorer}>
        Refresh explorer metadata
      </Button>
    </div>
  );
}

export { DatabaseObjectPanel };
