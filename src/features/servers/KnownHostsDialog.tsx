import { useEffect, useState } from "react";
import { FingerprintIcon, TrashIcon, WarningCircleIcon } from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { CopyButton } from "@/components/ui/copy-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { useHostKeyStore } from "@/stores/hostKeyStore";
import type { ServerProfile, TrustedHostKey } from "@/types/contracts";

type KnownHostsDialogProps = {
  server: ServerProfile;
  onOpenChange: (open: boolean) => void;
};

const EMPTY_TRUSTED_HOSTS: TrustedHostKey[] = [];

function formatTimestamp(timestamp: number | null) {
  if (!timestamp) {
    return "Unknown";
  }

  return new Date(timestamp * 1000).toLocaleString();
}

function KnownHostsDialog({ server, onOpenChange }: KnownHostsDialogProps) {
  const knownHosts = useHostKeyStore((state) => state.knownHosts[server.id] ?? EMPTY_TRUSTED_HOSTS);
  const status = useHostKeyStore((state) => state.status[server.id] ?? "idle");
  const error = useHostKeyStore((state) => state.error);
  const fetchKnownHosts = useHostKeyStore((state) => state.fetchKnownHosts);
  const removeKnownHost = useHostKeyStore((state) => state.removeKnownHost);
  const [removeTarget, setRemoveTarget] = useState<TrustedHostKey | null>(null);
  const [removePending, setRemovePending] = useState(false);

  useEffect(() => {
    void fetchKnownHosts(server.id);
  }, [fetchKnownHosts, server.id]);

  async function handleRemove() {
    if (!removeTarget) {
      return;
    }

    setRemovePending(true);
    const removed = await removeKnownHost(server.id, removeTarget.keyType);
    setRemovePending(false);

    if (removed) {
      setRemoveTarget(null);
    }
  }

  return (
    <>
      <Dialog open onOpenChange={onOpenChange}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>Trusted host keys</DialogTitle>
            <DialogDescription>
              ServerDesk accepts only these fingerprints for {server.name}. Removing one requires
              verifying the server key again on the next connection.
            </DialogDescription>
          </DialogHeader>

          {error ? (
            <Alert variant="destructive" className="py-2.5">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>Host key action failed</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </div>
            </Alert>
          ) : null}

          {status === "idle" || status === "loading" ? (
            <div className="space-y-2">
              <Skeleton className="h-14 w-full" />
              <Skeleton className="h-14 w-full" />
            </div>
          ) : null}

          {status === "ready" && knownHosts.length === 0 ? (
            <div className="rounded-md border border-dashed border-border bg-card/35 px-4 py-6">
              <div className="flex items-center gap-2 text-[13px] font-medium text-foreground">
                <FingerprintIcon className="size-4 text-muted-foreground" aria-hidden="true" />
                No trusted host keys
              </div>
              <p className="mt-1 max-w-[52ch] text-[12px] leading-relaxed text-muted-foreground">
                The next connection to this server will show its key type and fingerprint before
                authentication so you can accept it explicitly.
              </p>
            </div>
          ) : null}

          {knownHosts.length > 0 ? (
            <ul className="divide-y divide-border overflow-hidden rounded-md border border-border">
              {knownHosts.map((host) => (
                <li key={host.id} className="flex flex-col gap-3 bg-card px-3.5 py-3 sm:flex-row">
                  <div className="min-w-0 flex-1 space-y-1.5">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant="outline">{host.keyType}</Badge>
                      <span className="text-[11px] text-muted-foreground">
                        Trusted {formatTimestamp(host.verifiedAt ?? host.firstSeenAt)}
                      </span>
                    </div>
                    <div className="flex min-w-0 items-start gap-1">
                      <code className="min-w-0 break-all font-mono text-[11px] leading-5 text-foreground">
                        {host.fingerprint}
                      </code>
                      <CopyButton value={host.fingerprint} label="Copy fingerprint" />
                    </div>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="self-start text-destructive hover:bg-destructive/10 hover:text-destructive"
                    onClick={() => setRemoveTarget(host)}
                    aria-label={`Remove ${host.keyType} fingerprint`}
                  >
                    <TrashIcon className="size-3.5" aria-hidden="true" />
                    Remove
                  </Button>
                </li>
              ))}
            </ul>
          ) : null}

          <DialogFooter>
            <Button variant="secondary" onClick={() => onOpenChange(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={removeTarget !== null}
        onOpenChange={(open) => {
          if (!open && !removePending) {
            setRemoveTarget(null);
          }
        }}
        title="Remove trusted fingerprint?"
        description="The next connection will require explicit acceptance of the server host key again. This does not change the remote server."
        confirmLabel="Remove fingerprint"
        warningTitle="This only changes local trust."
        warningDescription="The remote server is not modified. The next connection requires host-key verification again."
        details={
          removeTarget
            ? [
                { label: "Server", value: server.name },
                { label: "Key type", value: removeTarget.keyType },
                { label: "Fingerprint", value: removeTarget.fingerprint },
              ]
            : undefined
        }
        onConfirm={() => void handleRemove()}
        pending={removePending}
      />
    </>
  );
}

export { KnownHostsDialog };
