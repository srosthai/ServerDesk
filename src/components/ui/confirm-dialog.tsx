import * as React from "react";
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
import { WarningCircle } from "@phosphor-icons/react";

type ConfirmationDetail = {
  label: string;
  value: string;
};

type ConfirmDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  confirmLabel: string;
  onConfirm: () => void;
  details?: ConfirmationDetail[];
  destructive?: boolean;
  pending?: boolean;
};

function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  onConfirm,
  details,
  destructive = true,
  pending = false,
}: ConfirmDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        {details?.length ? (
          <dl className="grid grid-cols-[88px_1fr] gap-x-3 gap-y-2 rounded-md border border-border bg-muted/45 px-3.5 py-3">
            {details.map((detail) => (
              <React.Fragment key={detail.label}>
                <dt className="text-[11px] leading-5 text-muted-foreground">{detail.label}</dt>
                <dd className="break-words font-mono text-[12px] leading-5 text-foreground">
                  {detail.value}
                </dd>
              </React.Fragment>
            ))}
          </dl>
        ) : null}

        {destructive ? (
          <Alert variant="warning" className="py-2.5">
            <WarningCircle className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div>
              <AlertTitle>This action changes the remote server.</AlertTitle>
              <AlertDescription>Review the target before continuing.</AlertDescription>
            </div>
          </Alert>
        ) : null}

        <DialogFooter>
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={pending}>
            Cancel
          </Button>
          <Button
            variant={destructive ? "destructive" : "default"}
            onClick={onConfirm}
            disabled={pending}
          >
            {pending ? "Working..." : confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export { ConfirmDialog };
