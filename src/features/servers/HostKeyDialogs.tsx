import { FingerprintIcon, ShieldWarningIcon } from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
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
import type { HostKeyDetails } from "@/types/contracts";

type HostKeyVerificationDialogProps = {
  serverName: string;
  host: string;
  hostKey: HostKeyDetails;
  pending: boolean;
  onTrust: () => void;
  onReject: () => void;
};

function HostKeyVerificationDialog({
  serverName,
  host,
  hostKey,
  pending,
  onTrust,
  onReject,
}: HostKeyVerificationDialogProps) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !pending) {
          onReject();
        }
      }}
    >
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Verify SSH host key</DialogTitle>
          <DialogDescription>
            ServerDesk connected to {serverName} for the first time. Before sending any credentials,
            confirm that this fingerprint belongs to the server.
          </DialogDescription>
        </DialogHeader>

        <dl className="grid grid-cols-[92px_1fr] gap-x-3 gap-y-2 rounded-md border border-border bg-muted/45 px-3.5 py-3">
          <dt className="text-[11px] leading-5 text-muted-foreground">Server</dt>
          <dd className="break-words text-[12px] leading-5 text-foreground">{serverName}</dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Host</dt>
          <dd className="break-words font-mono text-[12px] leading-5 text-foreground">{host}</dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Key type</dt>
          <dd className="break-words font-mono text-[12px] leading-5 text-foreground">
            {hostKey.keyType}
          </dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Fingerprint</dt>
          <dd className="flex min-w-0 items-start gap-1">
            <code className="min-w-0 break-all font-mono text-[12px] leading-5 text-foreground">
              {hostKey.fingerprint}
            </code>
            <CopyButton value={hostKey.fingerprint} label="Copy fingerprint" />
          </dd>
        </dl>

        <Alert variant="warning" className="py-2.5">
          <ShieldWarningIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Only trust a fingerprint you can verify.</AlertTitle>
            <AlertDescription>
              Compare it with the value shown on the server or in your hosting provider&apos;s
              console. Authentication is not attempted until you accept the key.
            </AlertDescription>
          </div>
        </Alert>

        <DialogFooter>
          <Button variant="secondary" onClick={onReject} disabled={pending}>
            Reject key
          </Button>
          <Button onClick={onTrust} disabled={pending}>
            <FingerprintIcon className="size-3.5" aria-hidden="true" />
            {pending ? "Connecting..." : "Trust and connect"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type HostKeyChangedDialogProps = {
  serverName: string;
  host: string;
  hostKey: HostKeyDetails;
  onDismiss: () => void;
  onManageKeys: () => void;
};

function HostKeyChangedDialog({
  serverName,
  host,
  hostKey,
  onDismiss,
  onManageKeys,
}: HostKeyChangedDialogProps) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          onDismiss();
        }
      }}
    >
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>SSH host key changed</DialogTitle>
          <DialogDescription>
            The connection to {serverName} was blocked before authentication. The key presented by
            the server does not match the fingerprint trusted for this profile.
          </DialogDescription>
        </DialogHeader>

        <Alert variant="destructive" className="py-2.5">
          <ShieldWarningIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>This could indicate a man-in-the-middle attack.</AlertTitle>
            <AlertDescription>
              It may also mean the server was rebuilt or its host key was rotated. ServerDesk did
              not change the saved fingerprint.
            </AlertDescription>
          </div>
        </Alert>

        <dl className="grid grid-cols-[92px_1fr] gap-x-3 gap-y-2 rounded-md border border-border bg-muted/45 px-3.5 py-3">
          <dt className="text-[11px] leading-5 text-muted-foreground">Server</dt>
          <dd className="break-words text-[12px] leading-5 text-foreground">{serverName}</dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Host</dt>
          <dd className="break-words font-mono text-[12px] leading-5 text-foreground">{host}</dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Saved</dt>
          <dd className="min-w-0 break-all font-mono text-[12px] leading-5 text-foreground">
            {hostKey.expectedFingerprint ?? "Unknown"}
          </dd>
          <dt className="text-[11px] leading-5 text-muted-foreground">Received</dt>
          <dd className="min-w-0 break-all font-mono text-[12px] leading-5 text-foreground">
            {hostKey.fingerprint}
          </dd>
        </dl>

        <DialogFooter>
          <Button variant="secondary" onClick={onManageKeys}>
            Manage trusted keys
          </Button>
          <Button onClick={onDismiss}>Dismiss warning</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export { HostKeyChangedDialog, HostKeyVerificationDialog };
