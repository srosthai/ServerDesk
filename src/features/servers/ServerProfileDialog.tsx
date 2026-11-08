import { useState } from "react";
import type { FormEvent } from "react";
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { WarningCircleIcon } from "@phosphor-icons/react";
import { useServerStore } from "@/stores/serverStore";
import type { AuthType, ServerProfile, ServerProfileInput } from "@/types/contracts";

type ServerProfileDialogProps = {
  server: ServerProfile | null;
  onOpenChange: (open: boolean) => void;
};

function ServerProfileDialog({ server, onOpenChange }: ServerProfileDialogProps) {
  const createServer = useServerStore((state) => state.createServer);
  const updateServer = useServerStore((state) => state.updateServer);
  const [name, setName] = useState(server?.name ?? "");
  const [host, setHost] = useState(server?.host ?? "");
  const [port, setPort] = useState(String(server?.port ?? 22));
  const [username, setUsername] = useState(server?.username ?? "");
  const [authType, setAuthType] = useState<AuthType>(server?.authType ?? "password");
  const [groupName, setGroupName] = useState(server?.groupName ?? "");
  const [tags, setTags] = useState(server?.tags.join(", ") ?? "");
  const [notes, setNotes] = useState(server?.notes ?? "");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setError(null);

    const input: ServerProfileInput = {
      name,
      host,
      port: Number(port),
      username,
      authType,
      credentialRef: server?.credentialRef ?? null,
      groupName: groupName.trim() || null,
      tags: tags
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
      notes: notes.trim() || null,
    };

    const saved = server ? await updateServer(server.id, input) : await createServer(input);

    setPending(false);

    if (saved) {
      onOpenChange(false);
    } else {
      setError(useServerStore.getState().error ?? "Could not save the server profile.");
    }
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <form onSubmit={handleSubmit} className="space-y-5">
          <DialogHeader>
            <DialogTitle>{server ? "Edit server" : "Add server"}</DialogTitle>
            <DialogDescription>
              Connection metadata is stored locally. Secrets are kept in the OS credential store,
              never in SQLite.
            </DialogDescription>
          </DialogHeader>

          {error ? (
            <Alert variant="destructive" className="py-2.5">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>Could not save server</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </div>
            </Alert>
          ) : null}

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="server-name">Name</Label>
              <Input
                id="server-name"
                value={name}
                onChange={(event) => setName(event.currentTarget.value)}
                placeholder="Production"
                autoFocus
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="server-group">Group</Label>
              <Input
                id="server-group"
                value={groupName}
                onChange={(event) => setGroupName(event.currentTarget.value)}
                placeholder="Production"
              />
            </div>
            <div className="space-y-2 sm:col-span-2">
              <Label htmlFor="server-host">Host</Label>
              <Input
                id="server-host"
                value={host}
                onChange={(event) => setHost(event.currentTarget.value)}
                placeholder="198.51.100.24 or server.example.com"
                spellCheck={false}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="server-port">SSH port</Label>
              <Input
                id="server-port"
                type="number"
                min={1}
                max={65535}
                value={port}
                onChange={(event) => setPort(event.currentTarget.value)}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="server-username">Username</Label>
              <Input
                id="server-username"
                value={username}
                onChange={(event) => setUsername(event.currentTarget.value)}
                placeholder="deploy"
                autoComplete="username"
                spellCheck={false}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="server-auth">Authentication</Label>
              <Select value={authType} onValueChange={(value) => setAuthType(value as AuthType)}>
                <SelectTrigger id="server-auth">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="password">Password</SelectItem>
                  <SelectItem value="private_key">Private key</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="server-tags">Tags</Label>
              <Input
                id="server-tags"
                value={tags}
                onChange={(event) => setTags(event.currentTarget.value)}
                placeholder="web, laravel"
              />
            </div>
            <div className="space-y-2 sm:col-span-2">
              <Label htmlFor="server-notes">Notes</Label>
              <Textarea
                id="server-notes"
                value={notes}
                onChange={(event) => setNotes(event.currentTarget.value)}
                placeholder="Optional non-sensitive notes"
              />
            </div>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="secondary"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={pending}>
              {pending ? "Saving..." : "Save server"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export { ServerProfileDialog };
