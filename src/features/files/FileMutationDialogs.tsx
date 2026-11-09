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
  onClose,
  onCreate,
}: {
  open: boolean;
  parent: string;
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
          <div className="rounded-md border border-border bg-muted/45 px-3 py-2">
            <p className="text-[10px] tracking-wide text-muted-foreground uppercase">Parent</p>
            <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">{parent}</p>
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
  onClose,
  onRename,
}: {
  entry: RemoteFileEntry;
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
          <div className="rounded-md border border-border bg-muted/45 px-3 py-2">
            <p className="text-[10px] tracking-wide text-muted-foreground uppercase">Path</p>
            <p className="mt-0.5 break-all font-mono text-[11px] text-foreground">{entry.path}</p>
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
