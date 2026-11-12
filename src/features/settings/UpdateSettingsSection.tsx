import {
  ArrowClockwiseIcon,
  ArrowSquareOutIcon,
  CheckCircleIcon,
  DownloadSimpleIcon,
  InfoIcon,
  SpinnerGapIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { SettingsSection } from "@/features/settings/SettingsSection";
import { isTauriRuntime } from "@/lib/tauri";
import {
  formatBytes,
  formatLastChecked,
  formatVersionLabel,
  getUpdateErrorCopy,
  type UpdateProgress,
} from "@/lib/updates";
import { useUpdateStore } from "@/stores/updateStore";

function UpdateSettingsSection() {
  const status = useUpdateStore((state) => state.status);
  const currentVersion = useUpdateStore((state) => state.currentVersion);
  const bundleType = useUpdateStore((state) => state.bundleType);
  const selfUpdateSupported = useUpdateStore((state) => state.selfUpdateSupported);
  const available = useUpdateStore((state) => state.available);
  const progress = useUpdateStore((state) => state.progress);
  const error = useUpdateStore((state) => state.error);
  const lastCheckedAt = useUpdateStore((state) => state.lastCheckedAt);
  const checkForUpdate = useUpdateStore((state) => state.checkForUpdate);
  const installUpdate = useUpdateStore((state) => state.installUpdate);
  const openReleasePage = useUpdateStore((state) => state.openReleasePage);

  const busy =
    status === "checking" ||
    status === "downloading" ||
    status === "installing" ||
    status === "restarting";
  const disabled = status === "disabled";
  const manualPackageUpdate = bundleType === "deb" || bundleType === "rpm";
  const showAvailablePanel =
    available !== null &&
    (status === "available" || status === "downloading" || status === "installing");
  const errorCopy = error ? getUpdateErrorCopy(error) : null;

  return (
    <SettingsSection
      title="Updates"
      description="ServerDesk checks GitHub Releases for signed updates. Update checks never run in the browser preview or a development build."
    >
      <div className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-border bg-card px-3.5 py-3">
          <div className="min-w-0">
            <div className="text-[12px] font-medium">
              Installed version{" "}
              <span className="font-mono text-[11px] text-muted-foreground">
                {formatVersionLabel(currentVersion)}
              </span>
            </div>
            <div className="mt-1 text-[11px] text-muted-foreground">
              {formatLastChecked(lastCheckedAt)}
            </div>
          </div>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void checkForUpdate({ trigger: "manual" })}
            disabled={busy || disabled}
            aria-label="Check for updates"
          >
            {status === "checking" ? (
              <SpinnerGapIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {status === "checking" ? "Checking…" : "Check for updates"}
          </Button>
        </div>

        {disabled ? (
          <Alert variant="info">
            <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
            <div>
              <AlertTitle>Update checks are disabled here</AlertTitle>
              <AlertDescription>
                {!isTauriRuntime()
                  ? "Update checks run in the installed desktop app, not the browser preview."
                  : "Update checks never run in development builds. Use a packaged release to test updates."}
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        {status === "checking" ? (
          <p role="status" className="flex items-center gap-2 text-[12px] text-muted-foreground">
            <SpinnerGapIcon className="size-3.5 animate-spin" aria-hidden="true" />
            Checking GitHub Releases for a signed update…
          </p>
        ) : null}

        {status === "up-to-date" ? (
          <Alert variant="success">
            <CheckCircleIcon className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
            <div>
              <AlertTitle>ServerDesk is up to date</AlertTitle>
              <AlertDescription>
                No newer signed release is available for this installation.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        {showAvailablePanel && available ? (
          <div className="space-y-3 rounded-md border border-border bg-card p-3.5">
            <div className="flex items-start gap-2.5">
              <DownloadSimpleIcon
                className="mt-0.5 size-4 shrink-0 text-primary"
                aria-hidden="true"
              />
              <div className="min-w-0">
                <h3 className="text-[13px] font-medium">
                  ServerDesk {formatVersionLabel(available.version)} is available
                </h3>
                <p className="mt-0.5 text-[11px] text-muted-foreground">
                  Installed {formatVersionLabel(available.currentVersion)}
                  {available.date ? ` · Released ${available.date.slice(0, 10)}` : ""}
                </p>
              </div>
            </div>

            <div>
              <div className="text-[11px] font-medium text-muted-foreground">Release notes</div>
              {available.notes ? (
                <div className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-sm border border-border bg-background px-3 py-2 text-[12px] leading-relaxed">
                  {available.notes}
                </div>
              ) : (
                <p className="mt-1 text-[12px] text-muted-foreground">
                  No release notes were provided for this version.
                </p>
              )}
            </div>

            {manualPackageUpdate ? (
              <Alert variant="warning">
                <WarningCircleIcon
                  className="mt-0.5 size-4 shrink-0 text-warning"
                  aria-hidden="true"
                />
                <div>
                  <AlertTitle>Manual update required</AlertTitle>
                  <AlertDescription>
                    This {bundleType?.toUpperCase()} installation is managed by the system package
                    manager, so ServerDesk cannot replace it in place. Download the new package from
                    GitHub Releases and install it with your package manager.
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}

            {status === "downloading" || status === "installing" ? (
              <UpdateProgressView status={status} progress={progress} />
            ) : null}

            {status === "available" ? (
              <div className="flex flex-wrap gap-2">
                {selfUpdateSupported ? (
                  <Button size="sm" onClick={() => void installUpdate()}>
                    <DownloadSimpleIcon className="size-3.5" aria-hidden="true" />
                    Download and install
                  </Button>
                ) : (
                  <Button size="sm" onClick={() => void openReleasePage()}>
                    <ArrowSquareOutIcon className="size-3.5" aria-hidden="true" />
                    Open download page
                  </Button>
                )}
              </div>
            ) : null}

            {!selfUpdateSupported && !manualPackageUpdate ? (
              <p className="text-[11px] text-muted-foreground">
                This installation type cannot install updates in place. Use the release page to
                download the correct package.
              </p>
            ) : null}
          </div>
        ) : null}

        {status === "restarting" ? (
          <Alert variant="success">
            <SpinnerGapIcon
              className="mt-0.5 size-4 shrink-0 animate-spin text-success"
              aria-hidden="true"
            />
            <div>
              <AlertTitle>Update installed</AlertTitle>
              <AlertDescription>
                Restarting ServerDesk to finish the update. The app may close and reopen.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        {status === "installed" ? (
          <Alert variant="warning">
            <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div>
              <AlertTitle>Restart required</AlertTitle>
              <AlertDescription>
                The update was installed, but ServerDesk could not restart automatically. Quit and
                open ServerDesk again to finish the update.
                {error ? <span className="mt-1 block">{error.message}</span> : null}
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        {status === "error" && error && errorCopy ? (
          <Alert variant="destructive">
            <WarningCircleIcon
              className="mt-0.5 size-4 shrink-0 text-destructive"
              aria-hidden="true"
            />
            <div>
              <AlertTitle>{errorCopy.title}</AlertTitle>
              <AlertDescription>
                {errorCopy.description}
                <span className="mt-1 block break-words font-mono text-[11px]">
                  {error.message}
                </span>
                <span className="mt-2 block">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => void checkForUpdate({ trigger: "manual" })}
                    disabled={disabled}
                  >
                    <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
                    Check again
                  </Button>
                </span>
              </AlertDescription>
            </div>
          </Alert>
        ) : null}
      </div>
    </SettingsSection>
  );
}

function UpdateProgressView({
  status,
  progress,
}: {
  status: "downloading" | "installing";
  progress: UpdateProgress | null;
}) {
  const percent = progress?.percent ?? null;
  const label =
    status === "installing"
      ? "Installing update…"
      : percent !== null
        ? `Downloading update… ${percent}%`
        : "Downloading update…";

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-3 text-[11px] text-muted-foreground">
        <span role="status">{label}</span>
        <span>
          {progress ? formatBytes(progress.downloadedBytes) : "0 B"}
          {progress?.totalBytes ? ` of ${formatBytes(progress.totalBytes)}` : ""}
        </span>
      </div>
      <div
        role="progressbar"
        aria-label="Update download progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        className="h-1.5 overflow-hidden rounded-full bg-muted"
      >
        <div
          className="h-full rounded-full bg-primary transition-[width] duration-200"
          style={{ width: `${percent ?? 0}%` }}
        />
      </div>
    </div>
  );
}

export { UpdateSettingsSection };
