import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import type { RemoteFileEntry } from "@/types/contracts";

function validateName(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return "Enter a name.";
  }
  if (trimmed === "." || trimmed === ".." || trimmed.includes("/") || trimmed.includes("\0")) {
    return "Names cannot contain slashes, parent segments, or control characters.";
  }
  if (trimmed.length > 255) {
    return "Names must be 255 characters or fewer.";
  }
  if ([...trimmed].some((character) => character.charCodeAt(0) < 0x20 || character === "\x7f")) {
    return "Names cannot contain control characters.";
  }
  return null;
}

export function CreateFolderDialog({
  open,
  parent,
  serverLabel,
  onClose,
  onCreate,
}: {
  open: boolean;
  parent: string;
  serverLabel: string;
  onClose: () => void;
  onCreate: (name: string) => Promise<boolean>;
}) {
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function submit() {
    const validation = validateName(name);
    if (validation) {
      setError(validation);
      return;
    }
    setPending(true);
    try {
      await onCreate(name.trim());
      onClose();
    } finally {
      setPending(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(value) => (value ? undefined : onClose())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create a remote folder</DialogTitle>
          <DialogDescription>
            The folder is created inside the current directory over the authenticated SSH
            connection.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-2 rounded-md border border-border bg-muted/45 px-3 py-2">
            <div>
              <p className="text-[10px] tracking-wide text-muted-foreground uppercase">Server</p>
              <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">
                {serverLabel}
              </p>
            </div>
            <div>
              <p className="text-[10px] tracking-wide text-muted-foreground uppercase">Parent</p>
              <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">{parent}</p>
            </div>
          </div>
          <div>
            <label htmlFor="new-folder-name" className="text-[11px] font-medium text-foreground">
              Folder name
            </label>
            <Input
              id="new-folder-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void submit();
                }
              }}
              spellCheck={false}
              autoComplete="off"
              className="mt-1"
              aria-invalid={error ? true : undefined}
            />
            {error ? (
              <p className="mt-1 text-[11px] text-destructive" role="alert">
                {error}
              </p>
            ) : null}
          </div>
        </div>
        <DialogFooter>
          <Button variant="secondary" onClick={onClose} disabled={pending}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={pending}>
            {pending ? "Creating..." : "Create folder"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function RenameEntryDialog({
  entry,
  serverLabel,
  onClose,
  onRename,
}: {
  entry: RemoteFileEntry;
  serverLabel: string;
  onClose: () => void;
  onRename: (newName: string) => Promise<boolean>;
}) {
  const [name, setName] = useState(entry.name);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function submit() {
    const validation = validateName(name);
    if (validation) {
      setError(validation);
      return;
    }
    if (name.trim() === entry.name) {
      setError("Enter a different name.");
      return;
    }
    setPending(true);
    try {
      await onRename(name.trim());
      onClose();
    } finally {
      setPending(false);
    }
  }

  return (
    <Dialog open onOpenChange={(value) => (value ? undefined : onClose())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Rename remote entry</DialogTitle>
          <DialogDescription>
            The entry is renamed in place. ServerDesk re-reads the old and new paths to verify the
            result.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-2 rounded-md border border-border bg-muted/45 px-3 py-2">
            <div>
              <p className="text-[10px] tracking-wide text-muted-foreground uppercase">Server</p>
              <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">
                {serverLabel}
              </p>
            </div>
            <div>
              <p className="text-[10px] tracking-wide text-muted-foreground uppercase">Path</p>
              <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">{entry.path}</p>
            </div>
          </div>
          <div>
            <label htmlFor="rename-entry-name" className="text-[11px] font-medium text-foreground">
              New name
            </label>
            <Input
              id="rename-entry-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void submit();
                }
              }}
              spellCheck={false}
              autoComplete="off"
              className="mt-1"
              aria-invalid={error ? true : undefined}
            />
            {error ? (
              <p className="mt-1 text-[11px] text-destructive" role="alert">
                {error}
              </p>
            ) : null}
          </div>
        </div>
        <DialogFooter>
          <Button variant="secondary" onClick={onClose} disabled={pending}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={pending}>
            {pending ? "Renaming..." : "Rename"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function DownloadSelectionDialog({
  open,
  entries,
  serverLabel,
  remoteDirectory,
  onClose,
  onStart,
}: {
  open: boolean;
  entries: RemoteFileEntry[];
  serverLabel: string;
  remoteDirectory: string;
  onClose: () => void;
  onStart: (localDirectory: string) => void;
}) {
  const [localDirectory, setLocalDirectory] = useState("");
  const [error, setError] = useState<string | null>(null);
  const downloadable = entries.filter(
    (entry) => entry.entryType === "file" || entry.entryType === "directory",
  );
  const names = downloadable.map((entry) => entry.name);

  function submit() {
    const trimmed = localDirectory.trim();
    if (!trimmed.startsWith("/")) {
      setError("Enter an absolute local directory such as /home/user/Downloads.");
      return;
    }
    if ([...trimmed].some((character) => character.charCodeAt(0) < 0x20 || character === "\x7f")) {
      setError("Local paths cannot contain control characters.");
      return;
    }
    setError(null);
    onStart(trimmed);
    onClose();
  }

  return (
    <Dialog open={open} onOpenChange={(value) => (value ? undefined : onClose())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Download selected entries</DialogTitle>
          <DialogDescription>
            Each selected entry is downloaded from {remoteDirectory} into the local folder you
            choose. The filename stays the same, directories are transferred recursively, and
            symbolic links are skipped.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-2 rounded-md border border-border bg-muted/45 px-3 py-2">
            <div>
              <p className="text-[10px] tracking-wide text-muted-foreground uppercase">Server</p>
              <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">
                {serverLabel}
              </p>
            </div>
            <div>
              <p className="text-[10px] tracking-wide text-muted-foreground uppercase">
                Selected entries
              </p>
              <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">
                {names.slice(0, 6).join(", ")}
                {names.length > 6 ? ` and ${names.length - 6} more` : ""}
              </p>
            </div>
          </div>
          <div>
            <label
              htmlFor="download-selection-directory"
              className="text-[11px] font-medium text-foreground"
            >
              Local destination folder
            </label>
            <Input
              id="download-selection-directory"
              value={localDirectory}
              onChange={(event) => {
                setLocalDirectory(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  submit();
                }
              }}
              placeholder="/home/user/Downloads"
              spellCheck={false}
              autoComplete="off"
              className="mt-1 font-mono text-[12px]"
              aria-invalid={error ? true : undefined}
            />
            <p className="mt-1 text-[10px] text-muted-foreground">
              Each file is saved as {"<folder>"}/{"<filename>"}. Existing destinations require
              confirmation in the transfer queue.
            </p>
            {error ? (
              <p className="mt-1 text-[11px] text-destructive" role="alert">
                {error}
              </p>
            ) : null}
          </div>
        </div>
        <DialogFooter>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={downloadable.length === 0}>
            Download {downloadable.length} {downloadable.length === 1 ? "entry" : "entries"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
