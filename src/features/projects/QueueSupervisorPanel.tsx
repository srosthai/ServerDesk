import {
  ArrowClockwiseIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  GearSixIcon,
  InfoIcon,
  MinusCircleIcon,
  QuestionIcon,
  StackIcon,
  WarningCircleIcon,
  XCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { LoadingState } from "@/components/ui/states";
import { StatusBadge } from "@/components/ui/status-badge";
import {
  QUEUE_RESTART_DISTINCTION,
  SUPERVISOR_CONFIG_NOTE,
  formatWorkerUptime,
  queueWorkerKindLabels,
  supervisorProgramStateLabels,
  supervisorStatusLabels,
} from "@/features/projects/queueSupervisor";
import type { QueueSupervisorEntry } from "@/stores/queueStore";
import type {
  QueueSupervisorStatus,
  ServerProfile,
  SupervisorProgram,
  SupervisorProgramState,
  SupervisorStatus,
} from "@/types/contracts";

const supervisorStatusBadge: Record<
  SupervisorStatus,
  { label: string; variant: "success" | "outline" | "warning"; Icon: typeof CheckCircleIcon }
> = {
  available: { label: supervisorStatusLabels.available, variant: "success", Icon: CheckCircleIcon },
  missing: { label: supervisorStatusLabels.missing, variant: "outline", Icon: MinusCircleIcon },
  unavailable: {
    label: supervisorStatusLabels.unavailable,
    variant: "warning",
    Icon: WarningCircleIcon,
  },
};

const programStateBadge: Record<
  SupervisorProgramState,
  { variant: "success" | "outline" | "warning" | "danger" | "info"; Icon: typeof CheckCircleIcon }
> = {
  running: { variant: "success", Icon: CheckCircleIcon },
  stopped: { variant: "outline", Icon: MinusCircleIcon },
  starting: { variant: "info", Icon: ArrowClockwiseIcon },
  backoff: { variant: "warning", Icon: WarningCircleIcon },
  fatal: { variant: "danger", Icon: XCircleIcon },
  exited: { variant: "warning", Icon: XCircleIcon },
  unknown: { variant: "warning", Icon: QuestionIcon },
};

function formatTimestamp(milliseconds: number) {
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeValue(milliseconds: number) {
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function SupervisorProgramRow({ program }: { program: SupervisorProgram }) {
  const config = programStateBadge[program.state];

  return (
    <li className="flex flex-wrap items-start justify-between gap-2 rounded-sm border border-border bg-card px-2.5 py-2">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="break-all font-mono text-[11px] font-medium text-foreground">
            {program.name}
          </span>
          {program.queueRelated ? <Badge variant="info">Queue-related name</Badge> : null}
        </div>
        {program.detail ? (
          <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">
            {program.detail}
          </p>
        ) : null}
      </div>
      <Badge variant={config.variant}>
        <config.Icon className="size-3" aria-hidden="true" />
        {supervisorProgramStateLabels[program.state]}
      </Badge>
    </li>
  );
}

function SupervisorCard({ status }: { status: QueueSupervisorStatus }) {
  const detection = status.supervisor;
  const config = supervisorStatusBadge[detection.status];
  const relevant = detection.programs.filter((program) => program.queueRelated).length;

  return (
    <div className="rounded-sm border border-border bg-muted/20 px-3 py-2.5">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex items-center gap-1.5">
          <GearSixIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-[12px] font-semibold text-foreground">Supervisor</h3>
        </div>
        <Badge variant={config.variant}>
          <config.Icon className="size-3" aria-hidden="true" />
          {config.label}
        </Badge>
      </div>

      {detection.status === "available" ? (
        <>
          <p className="mt-1 text-[11px] text-muted-foreground">
            {detection.programs.length === 0
              ? "Supervisor is installed and reported no programs."
              : `${detection.programs.length.toString()} program${
                  detection.programs.length === 1 ? "" : "s"
                } reported; ${relevant.toString()} with queue-related names.`}
          </p>
          {detection.programs.length > 0 ? (
            <ul className="mt-2 space-y-1.5">
              {detection.programs.map((program) => (
                <SupervisorProgramRow key={program.name} program={program} />
              ))}
            </ul>
          ) : null}
        </>
      ) : (
        <div className="mt-1.5 rounded-sm border border-warning/25 bg-warning/8 px-2.5 py-2">
          <p className="text-[11px] font-medium text-foreground">Program names unavailable</p>
          <p className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
            {detection.detail ??
              "Supervisor program data could not be read. ServerDesk does not guess program names."}
          </p>
        </div>
      )}

      {detection.status === "available" && detection.detail ? (
        <p className="mt-1.5 text-[10px] text-warning">{detection.detail}</p>
      ) : null}
    </div>
  );
}

function QueueWorkersCard({ status }: { status: QueueSupervisorStatus }) {
  const detection = status.queueWorkers;
  const count = detection.workers.length;

  return (
    <div className="rounded-sm border border-border bg-muted/20 px-3 py-2.5">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex items-center gap-1.5">
          <StackIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-[12px] font-semibold text-foreground">Queue worker processes</h3>
        </div>
        <Badge
          variant={
            detection.status === "unavailable" ? "warning" : count > 0 ? "success" : "outline"
          }
        >
          {detection.status === "unavailable"
            ? "Unavailable"
            : count > 0
              ? `${count.toString()} detected`
              : "None detected"}
        </Badge>
      </div>

      {detection.status === "available" ? (
        count === 0 ? (
          <p className="mt-1 text-[11px] leading-4 text-muted-foreground">
            No php artisan queue:work, queue:listen, or Horizon processes were found in the
            server-wide process list at sample time. This is a snapshot, not a Supervisor
            configuration claim.
          </p>
        ) : (
          <ul className="mt-2 space-y-1.5">
            {detection.workers.map((worker) => (
              <li
                key={worker.pid}
                className="flex flex-wrap items-start justify-between gap-2 rounded-sm border border-border bg-card px-2.5 py-2"
              >
                <div className="min-w-0">
                  <span className="font-mono text-[11px] font-medium text-foreground">
                    php artisan {queueWorkerKindLabels[worker.kind]}
                  </span>
                  <p className="mt-0.5 font-mono text-[10px] text-muted-foreground">
                    PID {worker.pid.toString()} · up {formatWorkerUptime(worker.elapsedSeconds)}
                    {worker.connection ? ` · connection ${worker.connection}` : ""}
                    {worker.queue ? ` · queue ${worker.queue}` : ""}
                  </p>
                </div>
                <Badge variant="info">{queueWorkerKindLabels[worker.kind]}</Badge>
              </li>
            ))}
          </ul>
        )
      ) : (
        <div className="mt-1.5 rounded-sm border border-warning/25 bg-warning/8 px-2.5 py-2">
          <p className="text-[11px] font-medium text-foreground">Worker status unavailable</p>
          <p className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
            {detection.detail ??
              "The remote process list could not be read. ServerDesk does not guess worker status."}
          </p>
        </div>
      )}

      {detection.status === "available" && detection.truncated ? (
        <p className="mt-1.5 text-[10px] text-warning">
          The worker process list was shortened at the ServerDesk safety limit.
        </p>
      ) : null}
    </div>
  );
}

export function QueueSupervisorPanel({
  server,
  connected,
  entry,
  onRefresh,
}: {
  server: ServerProfile;
  connected: boolean;
  entry: QueueSupervisorEntry | undefined;
  onRefresh: () => void;
}) {
  const status = entry?.status ?? null;
  const pending = entry?.pending ?? false;

  return (
    <section
      aria-label={`Queue and Supervisor status for ${server.name}`}
      className="rounded-md border border-border bg-card"
    >
      <div className="flex flex-wrap items-start justify-between gap-2 border-b border-border px-3.5 py-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-[13px] font-semibold text-foreground">Queue and Supervisor</h2>
            <Badge variant="outline">Read-only detection</Badge>
          </div>
          <p className="mt-1 max-w-[90ch] text-[11px] leading-5 text-muted-foreground">
            Supervisor presence and program names come from fixed, read-only commands. Queue worker
            status is a server-wide snapshot from the remote process list. Missing data stays
            unavailable and is never guessed.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <StatusBadge state={connected ? "connected" : "disconnected"} />
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={onRefresh}
            disabled={!connected || pending}
            aria-label="Refresh queue and Supervisor status"
          >
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {pending ? "Refreshing..." : "Refresh status"}
          </Button>
        </div>
      </div>

      <div className="space-y-2.5 px-3.5 py-3">
        {!connected && !status ? (
          <p className="text-[11px] text-warning">
            Connect this server to inspect Supervisor and queue worker status. No status is
            fabricated while disconnected.
          </p>
        ) : null}

        {entry?.error ? (
          <Alert variant="destructive">
            <WarningCircleIcon
              className="mt-0.5 size-4 shrink-0 text-destructive"
              aria-hidden="true"
            />
            <div className="flex-1">
              <AlertTitle>Could not inspect queue and Supervisor status</AlertTitle>
              <AlertDescription>
                <span className="block">{entry.error}</span>
                {entry.errorDetails ? (
                  <span className="mt-1 block whitespace-pre-wrap font-mono text-[10px]">
                    {entry.errorDetails}
                  </span>
                ) : null}
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        {pending && !status ? <LoadingState rows={2} /> : null}

        {!status && !pending && connected && !entry?.error ? (
          <p className="text-[11px] text-muted-foreground">
            No queue and Supervisor snapshot yet. Refresh status to run the fixed read-only
            detection commands.
          </p>
        ) : null}

        {status ? (
          <>
            <div className="grid gap-2.5 xl:grid-cols-2">
              <SupervisorCard status={status} />
              <QueueWorkersCard status={status} />
            </div>

            <Alert variant="info">
              <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
              <div>
                <AlertTitle>Queue restart is not a Supervisor restart</AlertTitle>
                <AlertDescription>{QUEUE_RESTART_DISTINCTION}</AlertDescription>
                <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
                  {SUPERVISOR_CONFIG_NOTE} To restart Supervisor itself, use the Services page and
                  the matching service unit; that is a separate server-wide action.
                </p>
              </div>
            </Alert>

            <p className="border-t border-border pt-2 text-[10px] text-muted-foreground">
              Sampled{" "}
              <time dateTime={dateTimeValue(status.sampledAt)}>
                {formatTimestamp(status.sampledAt)}
              </time>{" "}
              · {status.durationMs.toString()} ms · {status.serverId}
            </p>
          </>
        ) : null}
      </div>
    </section>
  );
}
