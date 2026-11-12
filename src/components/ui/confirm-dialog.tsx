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

type ConfirmationPreview = {
  label: string;
  text: string;
};

type ConfirmDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  confirmLabel: string;
  onConfirm: () => void;
  details?: ConfirmationDetail[];
  preview?: ConfirmationPreview | null;
  destructive?: boolean;
  pending?: boolean;
  warningTitle?: string;
  warningDescription?: string;
};

function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  onConfirm,
  details,
  preview,
  destructive = true,
  pending = false,
  warningTitle = "This action changes the remote server.",
  warningDescription = "Review the target before continuing.",
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
                <dd className="break-words whitespace-pre-line font-mono text-[12px] leading-5 text-foreground">
                  {detail.value}
                </dd>
              </React.Fragment>
            ))}
          </dl>
        ) : null}

        {preview ? (
          <div className="space-y-1.5">
            <p className="text-[11px] font-semibold text-foreground">{preview.label}</p>
            <pre className="max-h-40 overflow-auto rounded-md border border-border bg-muted/45 p-3 font-mono text-[11px] leading-5 whitespace-pre-wrap text-foreground">
              {preview.text}
            </pre>
          </div>
        ) : null}

        {destructive ? (
          <Alert variant="warning" className="py-2.5">
            <WarningCircle className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div>
              <AlertTitle>{warningTitle}</AlertTitle>
              <AlertDescription>{warningDescription}</AlertDescription>
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
