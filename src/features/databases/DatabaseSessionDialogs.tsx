import { useState } from "react";
import { WarningCircleIcon } from "@phosphor-icons/react";
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
import { Label } from "@/components/ui/label";
import {
  sessionActionLabel,
  sessionConfirmationPhrase,
  sessionDatabaseLabel,
  sessionElapsedLabel,
  sessionHostLabel,
  sessionQueryText,
  sessionStateLabel,
} from "@/features/databases/databaseSessionsModel";
import type { DatabaseSessionActionKind, DatabaseSessionSummary } from "@/types/contracts";

export type DatabaseSessionIntent = {
  action: DatabaseSessionActionKind;
  session: DatabaseSessionSummary;
};

type DatabaseSessionActionDialogProps = {
  intent: DatabaseSessionIntent | null;
  pending: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onConfirm: (confirmation: string) => void;
};

function DatabaseSessionActionDialog({
  intent,
  pending,
  error,
  onOpenChange,
  onConfirm,
}: DatabaseSessionActionDialogProps) {
  const [confirmation, setConfirmation] = useState("");

  if (!intent) {
    return null;
  }

  const { action, session } = intent;
  const phrase = sessionConfirmationPhrase(action, session.id);
  const actionLabel = sessionActionLabel(action);
  const matches = confirmation.trim() === phrase;

  return (
    <Dialog open onOpenChange={(open) => !pending && onOpenChange(open)}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>{actionLabel}</DialogTitle>
          <DialogDescription>
            This interrupts another database session. Type the exact phrase to confirm the target
            and action.
          </DialogDescription>
        </DialogHeader>

        <dl className="grid grid-cols-[104px_1fr] gap-x-3 gap-y-2 rounded-md border border-border bg-muted/45 px-3.5 py-3">
          <dt className="text-[11px] leading-5 text-muted-foreground">Session</dt>
          <dd className="font-mono text-[12px] leading-5 text-foreground">
            {session.id}
            {session.isCurrent ? " (current connection)" : ""}
          </dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Account</dt>
          <dd className="font-mono text-[12px] leading-5 text-foreground">{session.user}</dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Client</dt>
          <dd className="font-mono text-[12px] leading-5 text-foreground">
            {sessionHostLabel(session)}
          </dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Database</dt>
          <dd className="font-mono text-[12px] leading-5 text-foreground">
            {sessionDatabaseLabel(session)}
          </dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">State</dt>
          <dd className="font-mono text-[12px] leading-5 text-foreground">
            {sessionStateLabel(session)}
          </dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Elapsed</dt>
          <dd className="font-mono text-[12px] leading-5 text-foreground">
            {sessionElapsedLabel(session)}
          </dd>
        </dl>

        <div className="space-y-1.5">
          <div className="flex items-center gap-2">
            <p className="text-[11px] font-semibold text-foreground">Query</p>
            {session.query.status === "available" && session.query.value.redacted ? (
              <Badge variant="warning">Redacted</Badge>
            ) : null}
            {session.query.status === "available" && session.query.value.truncated ? (
              <Badge variant="outline">Truncated</Badge>
            ) : null}
          </div>
          <pre className="max-h-40 overflow-auto rounded-md border border-border bg-muted/45 p-3 font-mono text-[11px] leading-5 whitespace-pre-wrap text-foreground">
            {sessionQueryText(session)}
          </pre>
        </div>

        <Alert variant="warning" className="py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>
              {action === "terminate_session"
                ? "The session is disconnected immediately."
                : "The running statement is interrupted."}
            </AlertTitle>
            <AlertDescription>
              {action === "terminate_session"
                ? "An open transaction is rolled back and the client must reconnect. ServerDesk reports success only after the session disappears from the server."
                : "The query is cancelled where the engine supports it. ServerDesk reports success only after the session re-read shows that the query is no longer running."}
            </AlertDescription>
          </div>
        </Alert>

        {error ? (
          <Alert variant="destructive" className="py-2.5">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <div>
              <AlertTitle>The action did not complete</AlertTitle>
              <AlertDescription>{error}</AlertDescription>
            </div>
          </Alert>
        ) : null}

        <div className="space-y-1.5">
          <Label htmlFor="session-confirmation" className="text-[11px]">
            Type <span className="font-mono text-foreground">{phrase}</span> to confirm
          </Label>
          <Input
            id="session-confirmation"
            value={confirmation}
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => setConfirmation(event.target.value)}
            disabled={pending}
          />
        </div>

        <DialogFooter>
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={pending}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={() => onConfirm(confirmation.trim())}
            disabled={pending || !matches}
          >
            {pending ? "Working..." : actionLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export { DatabaseSessionActionDialog };
