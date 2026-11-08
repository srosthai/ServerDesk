import { useRef, useState } from "react";
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
import { CircleNotchIcon, PlugsIcon, WarningCircleIcon } from "@phosphor-icons/react";
import { HostKeyVerificationDialog } from "@/features/servers/HostKeyDialogs";
import { describeConnectionError } from "@/lib/connectionErrors";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { testServerConnection } from "@/services/connections";
import {
  createCredential,
  createPrivateKeyCredential,
  deleteCredential,
  storeCredential,
  storePrivateKeyCredential,
} from "@/services/credentials";
import { useServerStore } from "@/stores/serverStore";
import type {
  AuthType,
  CommandError,
  HostKeyDetails,
  ServerProfile,
  ServerProfileInput,
  TrustedHostKeyInput,
} from "@/types/contracts";

type ServerProfileDialogProps = {
  server: ServerProfile | null;
  onOpenChange: (open: boolean) => void;
};

type TestState =
  | { status: "idle" }
  | { status: "testing" }
  | { status: "success"; durationMs: number }
  | { status: "error"; error: CommandError }
  | { status: "cancelled" };

function formatDuration(durationMs: number) {
  return durationMs < 1000 ? `${durationMs} ms` : `${(durationMs / 1000).toFixed(1)} s`;
}

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
  const [secret, setSecret] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [testState, setTestState] = useState<TestState>({ status: "idle" });
  const [testPending, setTestPending] = useState(false);
  const [pendingHostKey, setPendingHostKey] = useState<HostKeyDetails | null>(null);
  const [acceptedHostKey, setAcceptedHostKey] = useState<TrustedHostKeyInput | null>(null);
  const testRequest = useRef(0);
  const authChanged = server !== null && authType !== server.authType;
  const trimmedHost = host.trim();
  const trimmedUsername = username.trim();
  const canTest = isTauriRuntime();

  function resetTestState() {
    testRequest.current += 1;
    setTestPending(false);
    setTestState({ status: "idle" });
  }

  function resetForHostChange() {
    setAcceptedHostKey(null);
    setPendingHostKey(null);
    resetTestState();
  }

  function validateTestInput(): string | null {
    if (!trimmedHost) {
      return "Host is required.";
    }
    const numericPort = Number(port);
    if (!Number.isInteger(numericPort) || numericPort < 1 || numericPort > 65535) {
      return "SSH port must be between 1 and 65535.";
    }
    if (!trimmedUsername) {
      return "Username is required.";
    }
    if (authChanged && secret.length === 0) {
      return authType === "private_key"
        ? "Enter a private key before testing key authentication."
        : "Enter the SSH password before testing password authentication.";
    }
    if (secret.length === 0 && !server?.credentialRef) {
      return authType === "private_key"
        ? "Enter a private key before testing the connection."
        : "Enter the SSH password before testing the connection.";
    }
    if (authType === "private_key" && secret.length === 0 && passphrase.length > 0) {
      return "Enter the private key again to test the new passphrase.";
    }

    return null;
  }

  async function runConnectionTest(acceptedKey: TrustedHostKeyInput | null = acceptedHostKey) {
    if (!isTauriRuntime() || testPending) {
      return;
    }

    const validationError = validateTestInput();
    if (validationError) {
      setTestState({
        status: "error",
        error: { code: "validation", message: validationError },
      });
      return;
    }

    const requestId = ++testRequest.current;
    setTestPending(true);
    setTestState({ status: "testing" });

    try {
      const outcome = await testServerConnection({
        serverId: server?.id ?? null,
        host: trimmedHost,
        port: Number(port),
        username: trimmedUsername,
        authType,
        credentialRef: secret.length === 0 ? (server?.credentialRef ?? null) : null,
        secret: secret.length > 0 ? secret : null,
        passphrase: authType === "private_key" && passphrase.length > 0 ? passphrase : null,
        acceptedHostKey: acceptedKey,
      });

      if (testRequest.current !== requestId) {
        return;
      }

      if (outcome.status === "hostKeyVerification") {
        setPendingHostKey(outcome.hostKey);
        setTestState({ status: "idle" });
      } else {
        setPendingHostKey(null);
        if (acceptedKey) {
          setAcceptedHostKey(acceptedKey);
        }
        setTestState({ status: "success", durationMs: outcome.durationMs });
      }
    } catch (testError) {
      if (testRequest.current !== requestId) {
        return;
      }

      setPendingHostKey(null);
      setTestState({
        status: "error",
        error: isCommandError(testError)
          ? testError
          : { code: "connection", message: getCommandErrorMessage(testError) },
      });
    } finally {
      if (testRequest.current === requestId) {
        setTestPending(false);
      }
    }
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setError(null);

    if (isTauriRuntime()) {
      if (authChanged && secret.length === 0) {
        setError(
          authType === "private_key"
            ? "Enter a private key before switching this server to key authentication."
            : "Enter the SSH password before switching this server to password authentication.",
        );
        setPending(false);
        return;
      }
      if (authType === "private_key" && secret.length === 0 && passphrase.length > 0) {
        setError("Enter the private key again to change its passphrase.");
        setPending(false);
        return;
      }
    }

    let credentialRef = server?.credentialRef ?? null;
    let createdReference: string | null = null;

    if (secret.length > 0 && isTauriRuntime()) {
      try {
        if (authType === "private_key") {
          const keyPassphrase = passphrase.length > 0 ? passphrase : null;
          if (credentialRef) {
            await storePrivateKeyCredential(credentialRef, secret, keyPassphrase);
          } else {
            credentialRef = await createPrivateKeyCredential(secret, keyPassphrase);
            createdReference = credentialRef;
          }
        } else if (credentialRef) {
          await storeCredential(credentialRef, secret);
        } else {
          credentialRef = await createCredential(secret);
          createdReference = credentialRef;
        }
      } catch (credentialError) {
        setError(getCommandErrorMessage(credentialError));
        setPending(false);
        return;
      }
    }

    const input: ServerProfileInput = {
      name,
      host,
      port: Number(port),
      username,
      authType,
      credentialRef,
      groupName: groupName.trim() || null,
      tags: tags
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
      notes: notes.trim() || null,
    };

    const saved = server ? await updateServer(server.id, input) : await createServer(input);

    if (!saved && createdReference) {
      try {
        await deleteCredential(createdReference);
      } catch {
        // The profile did not save; cleanup is best effort.
      }
    }

    setPending(false);

    if (saved) {
      onOpenChange(false);
    } else {
      setError(useServerStore.getState().error ?? "Could not save the server profile.");
    }
  }

  const testError = testState.status === "error" ? describeConnectionError(testState.error) : null;
  const testDestination = `${trimmedHost || "server"}:${port || "22"}`;
  const hostKeyTrustIsEphemeral =
    acceptedHostKey !== null &&
    (!server ||
      server.host !== trimmedHost ||
      server.port !== Number(port) ||
      server.username !== trimmedUsername);

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !pending && !testPending) {
          onOpenChange(false);
        }
      }}
    >
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

          {testState.status === "testing" ? (
            <Alert variant="info" className="py-2.5" aria-live="polite">
              <CircleNotchIcon
                className="mt-0.5 size-4 shrink-0 animate-spin text-info"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>Testing connection</AlertTitle>
                <AlertDescription>
                  Resolving {testDestination}, checking the host key, authenticating, and running a
                  remote check.
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {testState.status === "success" ? (
            <Alert variant="success" className="py-2.5" aria-live="polite">
              <PlugsIcon className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
              <div>
                <AlertTitle>Connection test passed</AlertTitle>
                <AlertDescription>
                  Connected to {testDestination} as {trimmedUsername || "the configured user"} in{" "}
                  {formatDuration(testState.durationMs)}. The host key, authentication, and remote
                  check all succeeded.
                  {hostKeyTrustIsEphemeral ? (
                    <span className="mt-1 block">
                      The host key was accepted for this test only. Verify it again when the saved
                      profile connects.
                    </span>
                  ) : null}
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {testState.status === "error" && testError ? (
            <Alert variant="destructive" className="py-2.5">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>{testError.title}</AlertTitle>
                <AlertDescription>
                  <p>{testState.error.message}</p>
                  <p className="mt-1">{testState.error.details ?? testError.action}</p>
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {testState.status === "cancelled" ? (
            <Alert variant="info" className="py-2.5" aria-live="polite">
              <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
              <div>
                <AlertTitle>Connection test cancelled</AlertTitle>
                <AlertDescription>
                  The host key was rejected. No credentials were sent to the server.
                </AlertDescription>
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
                onChange={(event) => {
                  setHost(event.currentTarget.value);
                  resetForHostChange();
                }}
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
                onChange={(event) => {
                  setPort(event.currentTarget.value);
                  resetForHostChange();
                }}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="server-username">Username</Label>
              <Input
                id="server-username"
                value={username}
                onChange={(event) => {
                  setUsername(event.currentTarget.value);
                  resetTestState();
                }}
                placeholder="deploy"
                autoComplete="username"
                spellCheck={false}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="server-auth">Authentication</Label>
              <Select
                value={authType}
                onValueChange={(value) => {
                  if (value !== authType) {
                    setAuthType(value as AuthType);
                    setSecret("");
                    setPassphrase("");
                    setPendingHostKey(null);
                    resetTestState();
                  }
                }}
              >
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
            {isTauriRuntime() ? (
              <div className="space-y-2 sm:col-span-2">
                <Label htmlFor="server-secret">
                  {authType === "password" ? "SSH password" : "Private key"}
                </Label>
                {authType === "password" ? (
                  <Input
                    id="server-secret"
                    type="password"
                    value={secret}
                    onChange={(event) => {
                      setSecret(event.currentTarget.value);
                      resetTestState();
                    }}
                    placeholder={
                      server?.credentialRef
                        ? "Leave blank to keep the existing secret"
                        : "Optional now; stored in the OS credential store"
                    }
                    autoComplete="new-password"
                  />
                ) : (
                  <>
                    <Textarea
                      id="server-secret"
                      value={secret}
                      onChange={(event) => {
                        setSecret(event.currentTarget.value);
                        resetTestState();
                      }}
                      placeholder={
                        server?.credentialRef
                          ? "Leave blank to keep the existing key"
                          : "Paste an OpenSSH or PEM private key, or enter a key file path"
                      }
                      className="min-h-20 font-mono text-[11px]"
                      spellCheck={false}
                    />
                    <div className="space-y-2 pt-1">
                      <Label htmlFor="server-passphrase">Key passphrase (optional)</Label>
                      <Input
                        id="server-passphrase"
                        type="password"
                        value={passphrase}
                        onChange={(event) => {
                          setPassphrase(event.currentTarget.value);
                          resetTestState();
                        }}
                        placeholder={
                          secret.length > 0
                            ? "Leave blank if the key is not encrypted"
                            : server?.credentialRef
                              ? "Leave blank to keep the saved key and passphrase"
                              : "Leave blank if the key is not encrypted"
                        }
                        autoComplete="new-password"
                      />
                    </div>
                  </>
                )}
                <p className="text-[11px] leading-relaxed text-muted-foreground">
                  Secrets are stored in the OS credential store, never in SQLite or logs.
                  {authType === "private_key"
                    ? " OpenSSH, PEM (RSA, ECDSA, Ed25519), and PuTTY PPK keys are supported; encrypted keys use the passphrase above."
                    : null}
                </p>
              </div>
            ) : null}
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
              disabled={pending || testPending}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => void runConnectionTest()}
              disabled={pending || testPending || !canTest}
              title={canTest ? undefined : "Connection tests are available in the desktop app."}
            >
              {testPending ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <PlugsIcon className="size-3.5" aria-hidden="true" />
              )}
              {testPending ? "Testing..." : "Test connection"}
            </Button>
            <Button type="submit" disabled={pending || testPending}>
              {pending ? "Saving..." : "Save server"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>

      {pendingHostKey ? (
        <HostKeyVerificationDialog
          serverName={name.trim() || trimmedHost || "this server"}
          host={`${trimmedHost}:${port}`}
          hostKey={pendingHostKey}
          pending={testPending}
          onTrust={() => {
            const acceptedKey = {
              keyType: pendingHostKey.keyType,
              fingerprint: pendingHostKey.fingerprint,
            };
            setAcceptedHostKey(acceptedKey);
            void runConnectionTest(acceptedKey);
          }}
          onReject={() => {
            setPendingHostKey(null);
            setAcceptedHostKey(null);
            setTestState({ status: "cancelled" });
          }}
        />
      ) : null}
    </Dialog>
  );
}

export { ServerProfileDialog };
