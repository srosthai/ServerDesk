import { useRef, useState } from "react";
import type { FormEvent } from "react";
import {
  CheckCircleIcon,
  CircleNotchIcon,
  DatabaseIcon,
  InfoIcon,
  PlugsIcon,
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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  DATABASE_ENGINE_LABELS,
  DATABASE_SSL_MODE_LABELS,
  databaseProfileFormFromProfile,
  databaseProfileInputFromForm,
  defaultPortForEngine,
  emptyDatabaseProfileForm,
  firstDatabaseProfileFormError,
  formatDatabaseEndpoint,
  formatLatency,
  validateDatabaseProfileForm,
} from "@/features/databases/databaseProfileForm";
import type {
  DatabaseProfileFormErrors,
  DatabaseProfileFormField,
  DatabaseProfileFormValues,
} from "@/features/databases/databaseProfileForm";
import { describeConnectionError } from "@/lib/connectionErrors";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";
import { isTauriRuntime } from "@/lib/tauri";
import { testDatabaseProfileDraft } from "@/services/databaseProfiles";
import { useDatabaseProfileStore } from "@/stores/databaseProfileStore";
import type {
  CommandError,
  DatabaseEngine,
  DatabaseProfile,
  DatabaseSslMode,
  DatabaseTestResult,
  ServerProfile,
} from "@/types/contracts";

type TestState =
  | { status: "idle" }
  | { status: "testing" }
  | { status: "success"; result: DatabaseTestResult }
  | { status: "error"; error: CommandError };

type DatabaseProfileDialogProps = {
  servers: ServerProfile[];
  activeServerId: string | null;
  profile: DatabaseProfile | null;
  onOpenChange: (open: boolean) => void;
  onSaved?: (profile: DatabaseProfile) => void;
};

const fieldIds: Record<Exclude<DatabaseProfileFormField, "readOnly">, string> = {
  name: "database-profile-name",
  serverId: "database-profile-server",
  engine: "database-profile-engine",
  host: "database-profile-host",
  port: "database-profile-port",
  socket: "database-profile-socket",
  databaseName: "database-profile-database",
  username: "database-profile-username",
  password: "database-profile-password",
  sslMode: "database-profile-ssl-mode",
};

const engineOptions: DatabaseEngine[] = ["mysql", "postgresql"];

const sslModeOptions: (DatabaseSslMode | "default")[] = [
  "default",
  "disabled",
  "preferred",
  "required",
  "verify-ca",
  "verify-identity",
];

function DatabaseProfileDialog({
  servers,
  activeServerId,
  profile,
  onOpenChange,
  onSaved,
}: DatabaseProfileDialogProps) {
  const createProfile = useDatabaseProfileStore((state) => state.createProfile);
  const updateProfile = useDatabaseProfileStore((state) => state.updateProfile);
  const [values, setValues] = useState<DatabaseProfileFormValues>(() =>
    profile
      ? databaseProfileFormFromProfile(profile, activeServerId)
      : emptyDatabaseProfileForm(activeServerId ?? servers[0]?.id ?? ""),
  );
  const [errors, setErrors] = useState<DatabaseProfileFormErrors>({});
  const [formError, setFormError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [testState, setTestState] = useState<TestState>({ status: "idle" });
  const [testPending, setTestPending] = useState(false);
  const testRequest = useRef(0);

  const savedPasswordExists = profile?.hasCredential ?? false;
  const destination = formatDatabaseEndpoint(values.host, values.port, values.socket);
  const testError = testState.status === "error" ? describeConnectionError(testState.error) : null;

  function resetTestState() {
    testRequest.current += 1;
    setTestPending(false);
    setTestState({ status: "idle" });
  }

  function updateField<K extends keyof DatabaseProfileFormValues>(
    field: K,
    value: DatabaseProfileFormValues[K],
  ) {
    setValues((current) => ({ ...current, [field]: value }));
    setErrors((current) => ({ ...current, [field]: undefined }));
    setFormError(null);
    resetTestState();
  }

  function handleEngineChange(engine: DatabaseEngine) {
    setValues((current) => {
      const previousDefault = defaultPortForEngine(current.engine);
      const port =
        current.port.trim() === "" || current.port.trim() === previousDefault
          ? defaultPortForEngine(engine)
          : current.port;

      return { ...current, engine, port };
    });
    setErrors((current) => ({ ...current, engine: undefined, port: undefined }));
    setFormError(null);
    resetTestState();
  }

  function applyValidationErrors(nextErrors: DatabaseProfileFormErrors): boolean {
    setErrors(nextErrors);
    const first = firstDatabaseProfileFormError(nextErrors);
    if (first) {
      document.getElementById(fieldIds[first])?.focus();
      return false;
    }
    return true;
  }

  async function handleTest() {
    if (testPending) {
      return;
    }

    const nextErrors = validateDatabaseProfileForm(values);
    if (!applyValidationErrors(nextErrors)) {
      setTestState({ status: "idle" });
      return;
    }

    if (!values.password && !savedPasswordExists) {
      setTestState({
        status: "error",
        error: {
          code: "validation",
          message: "Enter the database password before testing this profile.",
        },
      });
      document.getElementById(fieldIds.password)?.focus();
      return;
    }

    if (!isTauriRuntime()) {
      setTestState({
        status: "error",
        error: {
          code: "connection",
          message: "Connection tests are available in the desktop application.",
        },
      });
      return;
    }

    const requestId = ++testRequest.current;
    setFormError(null);
    setTestPending(true);
    setTestState({ status: "testing" });

    try {
      const result = await testDatabaseProfileDraft(
        databaseProfileInputFromForm(values),
        profile?.id ?? null,
      );

      if (testRequest.current !== requestId) {
        return;
      }

      setTestState({ status: "success", result });
    } catch (error) {
      if (testRequest.current !== requestId) {
        return;
      }

      setTestState({
        status: "error",
        error: isCommandError(error)
          ? error
          : { code: "connection", message: getCommandErrorMessage(error) },
      });
    } finally {
      if (testRequest.current === requestId) {
        setTestPending(false);
      }
    }
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) {
      return;
    }

    const nextErrors = validateDatabaseProfileForm(values);
    if (!applyValidationErrors(nextErrors)) {
      return;
    }

    setPending(true);
    setFormError(null);

    const input = databaseProfileInputFromForm(values);
    const saved = profile ? await updateProfile(profile.id, input) : await createProfile(input);
    setPending(false);

    if (saved) {
      onSaved?.(saved);
      onOpenChange(false);
      return;
    }

    setFormError(
      useDatabaseProfileStore.getState().error ?? "Could not save the database profile.",
    );
  }

  const closeBlocked = pending || testPending;

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !closeBlocked) {
          onOpenChange(false);
        }
      }}
    >
      <DialogContent className="max-h-[calc(100dvh-2rem)] max-w-2xl overflow-y-auto">
        <form onSubmit={handleSubmit} className="space-y-5">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <DatabaseIcon className="size-4 text-muted-foreground" aria-hidden="true" />
              {profile ? "Edit database connection" : "Add database connection"}
            </DialogTitle>
            <DialogDescription>
              The profile stores non-secret metadata locally. The password is kept in the OS
              credential store and is never displayed again after it is saved.
            </DialogDescription>
          </DialogHeader>

          {formError ? (
            <Alert variant="destructive" className="py-2.5">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>Could not save the database profile</AlertTitle>
                <AlertDescription>{formError}</AlertDescription>
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
                <AlertTitle>Testing database connection</AlertTitle>
                <AlertDescription>
                  Opening an SSH tunnel to {destination} and authenticating with{" "}
                  {DATABASE_ENGINE_LABELS[values.engine]}. This can take a few seconds.
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {testState.status === "success" ? (
            <Alert variant="success" className="py-2.5" aria-live="polite">
              <CheckCircleIcon className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
              <div>
                <AlertTitle>Connection test passed</AlertTitle>
                <AlertDescription>
                  <p>
                    {testState.result.version.raw || testState.result.version.version} responded in{" "}
                    {formatLatency(testState.result.latencyMs)}.
                  </p>
                  <p className="mt-1">
                    {testState.result.databaseName
                      ? `Active database: ${testState.result.databaseName}. `
                      : "No default database selected. "}
                    {testState.result.readOnly
                      ? "The adapter is in read-only mode."
                      : "The adapter allows writes."}
                  </p>
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

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2 sm:col-span-2">
              <Label htmlFor={fieldIds.name}>Profile name</Label>
              <Input
                id={fieldIds.name}
                value={values.name}
                onChange={(event) => updateField("name", event.currentTarget.value)}
                placeholder="Primary application database"
                autoFocus
                aria-invalid={Boolean(errors.name)}
                aria-describedby={errors.name ? "database-profile-name-error" : undefined}
              />
              {errors.name ? (
                <p id="database-profile-name-error" className="text-[11px] text-destructive">
                  {errors.name}
                </p>
              ) : null}
            </div>

            <div className="space-y-2">
              <Label htmlFor={fieldIds.serverId}>Server</Label>
              <Select
                value={values.serverId}
                onValueChange={(value) => updateField("serverId", value)}
              >
                <SelectTrigger id={fieldIds.serverId} aria-invalid={Boolean(errors.serverId)}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {servers.map((server) => (
                    <SelectItem key={server.id} value={server.id}>
                      {server.name} · {server.host}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {errors.serverId ? (
                <p className="text-[11px] text-destructive">{errors.serverId}</p>
              ) : null}
            </div>

            <div className="space-y-2">
              <Label htmlFor={fieldIds.engine}>Engine</Label>
              <Select
                value={values.engine}
                onValueChange={(value) => handleEngineChange(value as DatabaseEngine)}
              >
                <SelectTrigger id={fieldIds.engine}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {engineOptions.map((engine) => (
                    <SelectItem key={engine} value={engine}>
                      {DATABASE_ENGINE_LABELS[engine]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label htmlFor={fieldIds.host}>Host</Label>
              <Input
                id={fieldIds.host}
                value={values.host}
                onChange={(event) => updateField("host", event.currentTarget.value)}
                placeholder="127.0.0.1"
                spellCheck={false}
                aria-invalid={Boolean(errors.host)}
                aria-describedby={errors.host ? "database-profile-host-error" : undefined}
              />
              {errors.host ? (
                <p id="database-profile-host-error" className="text-[11px] text-destructive">
                  {errors.host}
                </p>
              ) : null}
            </div>

            <div className="space-y-2">
              <Label htmlFor={fieldIds.port}>Port</Label>
              <Input
                id={fieldIds.port}
                type="number"
                min={1}
                max={65535}
                value={values.port}
                onChange={(event) => updateField("port", event.currentTarget.value)}
                aria-invalid={Boolean(errors.port)}
                aria-describedby={errors.port ? "database-profile-port-error" : undefined}
              />
              {errors.port ? (
                <p id="database-profile-port-error" className="text-[11px] text-destructive">
                  {errors.port}
                </p>
              ) : null}
            </div>

            <div className="space-y-2 sm:col-span-2">
              <Label htmlFor={fieldIds.socket}>Socket (optional)</Label>
              <Input
                id={fieldIds.socket}
                value={values.socket}
                onChange={(event) => updateField("socket", event.currentTarget.value)}
                placeholder="/var/run/postgresql/.s.PGSQL.5432"
                spellCheck={false}
                aria-invalid={Boolean(errors.socket)}
                aria-describedby={
                  errors.socket ? "database-profile-socket-error" : "database-profile-socket-help"
                }
              />
              {errors.socket ? (
                <p id="database-profile-socket-error" className="text-[11px] text-destructive">
                  {errors.socket}
                </p>
              ) : (
                <p
                  id="database-profile-socket-help"
                  className="text-[11px] leading-relaxed text-muted-foreground"
                >
                  Recorded as profile context. The tunnel still forwards loopback TCP to host and
                  port.
                </p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor={fieldIds.databaseName}>Database (optional)</Label>
              <Input
                id={fieldIds.databaseName}
                value={values.databaseName}
                onChange={(event) => updateField("databaseName", event.currentTarget.value)}
                placeholder="app"
                spellCheck={false}
                aria-invalid={Boolean(errors.databaseName)}
                aria-describedby={
                  errors.databaseName ? "database-profile-database-error" : undefined
                }
              />
              {errors.databaseName ? (
                <p id="database-profile-database-error" className="text-[11px] text-destructive">
                  {errors.databaseName}
                </p>
              ) : null}
            </div>

            <div className="space-y-2">
              <Label htmlFor={fieldIds.username}>Username</Label>
              <Input
                id={fieldIds.username}
                value={values.username}
                onChange={(event) => updateField("username", event.currentTarget.value)}
                placeholder="app"
                autoComplete="username"
                spellCheck={false}
                aria-invalid={Boolean(errors.username)}
                aria-describedby={errors.username ? "database-profile-username-error" : undefined}
              />
              {errors.username ? (
                <p id="database-profile-username-error" className="text-[11px] text-destructive">
                  {errors.username}
                </p>
              ) : null}
            </div>

            <div className="space-y-2 sm:col-span-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <Label htmlFor={fieldIds.password}>Database password</Label>
                <Badge variant={savedPasswordExists ? "success" : "outline"}>
                  {savedPasswordExists ? "Password saved" : "No password saved"}
                </Badge>
              </div>
              <Input
                id={fieldIds.password}
                type="password"
                value={values.password}
                onChange={(event) => updateField("password", event.currentTarget.value)}
                placeholder={
                  savedPasswordExists
                    ? "Leave blank to keep the saved password"
                    : "Stored in the OS credential store"
                }
                autoComplete="new-password"
                aria-invalid={Boolean(errors.password)}
                aria-describedby={
                  errors.password
                    ? "database-profile-password-error"
                    : "database-profile-password-help"
                }
              />
              {errors.password ? (
                <p id="database-profile-password-error" className="text-[11px] text-destructive">
                  {errors.password}
                </p>
              ) : (
                <p
                  id="database-profile-password-help"
                  className="text-[11px] leading-relaxed text-muted-foreground"
                >
                  {savedPasswordExists
                    ? "The saved password is never loaded into this form. Enter a new password only to replace it."
                    : "The password is written to the OS credential store and is never shown again."}
                </p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor={fieldIds.sslMode}>SSL mode</Label>
              <Select
                value={values.sslMode}
                onValueChange={(value) =>
                  updateField("sslMode", value as DatabaseSslMode | "default")
                }
              >
                <SelectTrigger id={fieldIds.sslMode}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {sslModeOptions.map((mode) => (
                    <SelectItem key={mode} value={mode}>
                      {DATABASE_SSL_MODE_LABELS[mode]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                TLS is negotiated between the database driver and the server inside the SSH tunnel.
              </p>
            </div>

            <div className="flex items-start gap-2.5 rounded-md border border-border bg-muted/25 px-3 py-2.5">
              <input
                id="database-profile-read-only"
                type="checkbox"
                checked={values.readOnly}
                onChange={(event) => updateField("readOnly", event.currentTarget.checked)}
                className="mt-0.5 size-3.5 shrink-0 accent-primary focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:ring-offset-2 focus-visible:ring-offset-background"
              />
              <div className="min-w-0">
                <Label htmlFor="database-profile-read-only">Read-only connection</Label>
                <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                  The adapter rejects writes for this profile. Use it for exploration and reporting.
                </p>
              </div>
            </div>
          </div>

          <Alert variant="info" className="py-2.5">
            <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
            <div>
              <AlertTitle>Connections use an SSH local-forward tunnel</AlertTitle>
              <AlertDescription>
                The database listener stays bound to 127.0.0.1 inside the SSH session. Testing opens
                a transient tunnel and closes it before returning.
              </AlertDescription>
            </div>
          </Alert>

          <DialogFooter className="sticky bottom-0 z-10 border-t border-border bg-card pt-3">
            <Button
              type="button"
              variant="secondary"
              onClick={() => onOpenChange(false)}
              disabled={closeBlocked}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => void handleTest()}
              disabled={pending || testPending}
              title={
                isTauriRuntime()
                  ? "Test these values without saving the profile"
                  : "Connection tests are available in the desktop app."
              }
            >
              {testPending ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <PlugsIcon className="size-3.5" aria-hidden="true" />
              )}
              {testPending ? "Testing..." : "Test connection"}
            </Button>
            <Button type="submit" disabled={pending || testPending}>
              {pending ? "Saving..." : profile ? "Save changes" : "Save connection"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export { DatabaseProfileDialog };
