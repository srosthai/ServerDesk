import { useState } from "react";
import {
  ArrowDownIcon,
  ArrowUpIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  ProhibitIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
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
import { formatFileSize } from "@/lib/remotePaths";
import type { FileTransferState } from "@/stores/fileStore";
import type {
  FileTransferStatus,
  RemoteFileEntry,
  RemoteFileListing,
  ServerProfile,
} from "@/types/contracts";

export type TransferPrompt =
  { direction: "upload" } | { direction: "download"; entry: RemoteFileEntry };

const statusLabels: Record<FileTransferStatus, string> = {
  running: "Running",
  completed: "Completed",
  cancelled: "Cancelled",
  failed: "Failed",
};

const statusVariants: Record<FileTransferStatus, "info" | "success" | "warning" | "danger"> = {
  running: "info",
  completed: "success",
  cancelled: "warning",
  failed: "danger",
};

function transferName(transfer: FileTransferState): string {
  const path =
    transfer.direction === "upload" ? transfer.request.remotePath : transfer.request.remotePath;
  const segments = path.split("/").filter(Boolean);
  return segments[segments.length - 1] ?? path;
}

function TransferProgress({ transfer }: { transfer: FileTransferState }) {
  const total = transfer.totalBytes;
  const percent =
    total && total > 0
      ? Math.min(100, Math.round((transfer.bytesTransferred / total) * 100))
      : null;

  return (
    <div className="mt-2">
      <div
        role="progressbar"
        aria-label={`${transfer.direction} progress for ${transferName(transfer)}`}
        aria-valuemin={0}
        aria-valuemax={total ?? undefined}
        aria-valuenow={total ? Math.min(transfer.bytesTransferred, total) : undefined}
        className="h-1.5 overflow-hidden rounded-full bg-muted"
      >
        <div
          className="h-full rounded-full bg-info transition-[width] duration-200"
          style={{ width: percent === null ? "35%" : `${percent}%` }}
        />
      </div>
      <div className="mt-1.5 flex flex-wrap items-center justify-between gap-2 font-mono text-[10px] text-muted-foreground">
        <span>
          {formatFileSize(transfer.bytesTransferred)}
          {total === null ? "" : ` of ${formatFileSize(total)}`}
        </span>
        <span>
          {transfer.filesTransferred}
          {transfer.filesTotal === null ? "" : ` / ${transfer.filesTotal}`} file
          {transfer.filesTransferred === 1 ? "" : "s"}
          {percent === null ? "" : ` · ${percent}%`}
        </span>
      </div>
    </div>
  );
}

export function FileTransferPanel({
  transfers,
  onCancel,
  onRetry,
  onRequestOverwrite,
  onDismiss,
}: {
  transfers: FileTransferState[];
  onCancel: (transferId: string) => void;
  onRetry: (transferId: string) => void;
  onRequestOverwrite: (transfer: FileTransferState) => void;
  onDismiss: (transferId: string) => void;
}) {
  if (transfers.length === 0) {
    return null;
  }

  return (
    <section
      aria-label="File transfers"
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="border-b border-border px-3.5 py-3">
        <h2 className="text-[13px] font-semibold text-foreground">Transfers</h2>
        <p className="mt-0.5 text-[11px] text-muted-foreground">
          Progress is reported per transfer. Cancelled or interrupted transfers keep their partial
          file and are never reported as complete.
        </p>
      </div>
      <div className="divide-y divide-border">
        {transfers.map((transfer) => (
          <div key={transfer.transferId} className="px-3.5 py-3">
            <div className="flex flex-wrap items-start justify-between gap-2">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  {transfer.direction === "upload" ? (
                    <ArrowUpIcon className="size-3.5 text-info" aria-hidden="true" />
                  ) : (
                    <ArrowDownIcon className="size-3.5 text-info" aria-hidden="true" />
                  )}
                  <span className="text-[12px] font-medium text-foreground">
                    {transfer.direction === "upload" ? "Upload" : "Download"}{" "}
                    {transferName(transfer)}
                  </span>
                  <Badge variant={statusVariants[transfer.status]}>
                    {transfer.status === "running" && transfer.cancelling
                      ? "Cancelling"
                      : statusLabels[transfer.status]}
                  </Badge>
                  <Badge variant="outline">{transfer.kind}</Badge>
                </div>
                <p className="mt-1 break-all font-mono text-[10px] text-muted-foreground">
                  {transfer.direction === "upload"
                    ? transfer.request.localPath
                    : transfer.request.remotePath}{" "}
                  →{" "}
                  {transfer.direction === "upload"
                    ? transfer.request.remotePath
                    : transfer.request.localPath}
                </p>
                {transfer.currentPath ? (
                  <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">
                    Current: {transfer.currentPath}
                  </p>
                ) : null}
              </div>
              <div className="flex flex-wrap items-center gap-1.5">
                {transfer.status === "running" ? (
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => onCancel(transfer.transferId)}
                    disabled={transfer.cancelling}
                  >
                    {transfer.cancelling ? (
                      <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
                    ) : (
                      <ProhibitIcon className="size-3.5" aria-hidden="true" />
                    )}
                    {transfer.cancelling ? "Cancelling..." : "Cancel"}
                  </Button>
                ) : null}
                {(transfer.status === "failed" || transfer.status === "cancelled") &&
                transfer.errorCode !== "conflict" ? (
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => onRetry(transfer.transferId)}
                  >
                    Retry
                  </Button>
                ) : null}
                {transfer.errorCode === "conflict" ? (
                  <Button size="sm" onClick={() => onRequestOverwrite(transfer)}>
                    Overwrite and retry
                  </Button>
                ) : null}
                {transfer.status !== "running" ? (
                  <Button variant="ghost" size="sm" onClick={() => onDismiss(transfer.transferId)}>
                    Dismiss
                  </Button>
                ) : null}
              </div>
            </div>

            <TransferProgress transfer={transfer} />

            {transfer.mayBeIncomplete ? (
              <Alert variant="warning" className="mt-2 py-2">
                <WarningCircleIcon
                  className="mt-0.5 size-4 shrink-0 text-warning"
                  aria-hidden="true"
                />
                <div>
                  <AlertTitle>This transfer may be incomplete</AlertTitle>
                  <AlertDescription>
                    The destination was not replaced. Partial data was kept at{" "}
                    {transfer.partialPaths.length > 0
                      ? transfer.partialPaths.join(", ")
                      : "the reported transfer path"}
                    .
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}

            {transfer.error ? (
              <Alert variant="destructive" className="mt-2 py-2">
                <WarningCircleIcon
                  className="mt-0.5 size-4 shrink-0 text-destructive"
                  aria-hidden="true"
                />
                <div>
                  <AlertTitle>Transfer failed</AlertTitle>
                  <AlertDescription>
                    <span className="block">{transfer.error}</span>
                    {transfer.errorDetails ? (
                      <span className="mt-1 block whitespace-pre-wrap font-mono text-[10px]">
                        {transfer.errorDetails}
                      </span>
                    ) : null}
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}

            {transfer.status === "completed" ? (
              <p className="mt-2 flex items-center gap-1.5 text-[11px] text-success">
                <CheckCircleIcon className="size-3.5" aria-hidden="true" />
                All acknowledged bytes were written before the destination was installed. No
                checksum comparison is performed.
              </p>
            ) : null}

            {transfer.warnings.length > 0 ? (
              <ul className="mt-2 list-disc space-y-1 pl-5 text-[10px] text-muted-foreground">
                {transfer.warnings.slice(0, 3).map((warning) => (
                  <li key={warning}>{warning}</li>
                ))}
              </ul>
            ) : null}
          </div>
        ))}
      </div>
    </section>
  );
}

function baseName(value: string): string {
  const trimmed = value.trim().replace(/[\\/]+$/, "");
  const segments = trimmed.split(/[\\/]/);
  return segments[segments.length - 1] ?? "";
}

function joinRemote(directory: string, name: string): string {
  return directory === "/" ? `/${name}` : `${directory}/${name}`;
}

export function TransferDialog({
  prompt,
  server,
  listing,
  onClose,
  onStart,
}: {
  prompt: TransferPrompt;
  server: ServerProfile | null;
  listing: RemoteFileListing | null;
  onClose: () => void;
  onStart: (input: {
    direction: "upload" | "download";
    remotePath: string;
    localPath: string;
  }) => Promise<void>;
}) {
  const [localPath, setLocalPath] = useState("");
  const [pending, setPending] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);

  const direction = prompt.direction;
  const entry = prompt.direction === "download" ? prompt.entry : null;
  const sourceName = entry?.name ?? baseName(localPath);
  const remotePath = entry
    ? entry.path
    : listing
      ? joinRemote(listing.path, sourceName)
      : sourceName;

  async function handleSubmit() {
    if (!listing || !server) {
      return;
    }
    if (localPath.trim().length === 0) {
      setValidationError(
        direction === "upload"
          ? "Enter the absolute local path of the file or directory to upload."
          : "Enter the absolute local destination path.",
      );
      return;
    }
    if (direction === "upload" && !sourceName) {
      setValidationError("The local path must name a file or directory.");
      return;
    }
    setValidationError(null);
    setPending(true);
    try {
      await onStart({ direction, remotePath, localPath: localPath.trim() });
      onClose();
    } finally {
      setPending(false);
    }
  }

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {direction === "upload" ? "Upload to the remote server" : "Download from the server"}
          </DialogTitle>
          <DialogDescription>
            {direction === "upload"
              ? "The local file or directory is uploaded over the authenticated SSH connection. Directories are transferred recursively and symbolic links are skipped."
              : "The remote file or directory is downloaded over the authenticated SSH connection. Directories are transferred recursively and symbolic links are skipped."}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          {direction === "upload" ? (
            <>
              <div>
                <label
                  htmlFor="file-transfer-local"
                  className="text-[11px] font-medium text-foreground"
                >
                  Local source path
                </label>
                <Input
                  id="file-transfer-local"
                  value={localPath}
                  onChange={(event) => setLocalPath(event.target.value)}
                  placeholder="/home/user/backup.tar.gz"
                  spellCheck={false}
                  autoComplete="off"
                  className="mt-1 font-mono text-[12px]"
                />
              </div>
              <div className="rounded-md border border-border bg-muted/45 px-3 py-2">
                <p className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Remote destination
                </p>
                <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">
                  {sourceName ? remotePath : "Enter a local path to compute the destination"}
                </p>
              </div>
            </>
          ) : (
            <>
              <div className="rounded-md border border-border bg-muted/45 px-3 py-2">
                <p className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Remote source
                </p>
                <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">
                  {entry?.path ?? "No entry selected"}
                </p>
              </div>
              <div>
                <label
                  htmlFor="file-transfer-local"
                  className="text-[11px] font-medium text-foreground"
                >
                  Local destination path
                </label>
                <Input
                  id="file-transfer-local"
                  value={localPath}
                  onChange={(event) => setLocalPath(event.target.value)}
                  placeholder="/home/user/Downloads/backup.tar.gz"
                  spellCheck={false}
                  autoComplete="off"
                  className="mt-1 font-mono text-[12px]"
                />
              </div>
            </>
          )}
          <p className="text-[11px] text-muted-foreground">
            {server
              ? `Using ${server.username}@${server.host}:${server.port}. No separate file-transfer port is opened.`
              : "Select a connected server first."}
          </p>
          {validationError ? (
            <p className="text-[11px] text-destructive" role="alert">
              {validationError}
            </p>
          ) : null}
        </div>

        <DialogFooter>
          <Button variant="secondary" onClick={onClose} disabled={pending}>
            Cancel
          </Button>
          <Button onClick={() => void handleSubmit()} disabled={pending || !server}>
            {pending ? "Starting..." : direction === "upload" ? "Start upload" : "Start download"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
