import type { Icon } from "@phosphor-icons/react";
import {
  ArrowCounterClockwiseIcon,
  CheckCircleIcon,
  ClockIcon,
  DesktopIcon,
  HardDrivesIcon,
  InfoIcon,
  MoonIcon,
  SunIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { PageHeader } from "@/components/ui/page-header";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { SettingsSection } from "@/features/settings/SettingsSection";
import { UpdateSettingsSection } from "@/features/settings/UpdateSettingsSection";
import { MIN_MONITORING_INTERVAL_SECONDS } from "@/lib/preferences";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/stores/preferencesStore";
import type { ThemePreference } from "@/types/contracts";

type ThemeOption = {
  value: ThemePreference;
  label: string;
  description: string;
  icon: Icon;
};

const themeOptions: ThemeOption[] = [
  {
    value: "system",
    label: "System",
    description: "Follow the operating system appearance.",
    icon: DesktopIcon,
  },
  {
    value: "light",
    label: "Light",
    description: "Bright surfaces for daylight work.",
    icon: SunIcon,
  },
  {
    value: "dark",
    label: "Dark",
    description: "Low-glare surfaces for long sessions.",
    icon: MoonIcon,
  },
];

const intervalOptions = [5, 10, 15, 30, 60, 120, 300];

function SettingsPage() {
  const preferences = usePreferencesStore((state) => state.preferences);
  const status = usePreferencesStore((state) => state.status);
  const error = usePreferencesStore((state) => state.error);
  const setTheme = usePreferencesStore((state) => state.setTheme);
  const setMonitoringInterval = usePreferencesStore((state) => state.setMonitoringInterval);
  const setLastActiveServerId = usePreferencesStore((state) => state.setLastActiveServerId);
  const reset = usePreferencesStore((state) => state.reset);

  const intervals = Array.from(
    new Set([...intervalOptions, preferences.monitoringIntervalSeconds]),
  ).sort((left, right) => left - right);

  return (
    <div className="max-w-4xl space-y-8">
      <PageHeader
        title="Settings"
        description="Appearance, monitoring cadence, local preferences, and application updates."
        actions={
          <Button variant="secondary" size="sm" onClick={reset}>
            <ArrowCounterClockwiseIcon className="size-3.5" aria-hidden="true" />
            Reset defaults
          </Button>
        }
      />

      {status === "error" ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>Preferences may not be saved</AlertTitle>
            <AlertDescription>
              {error ?? "The local preference store returned an error."} Your selection still
              applies for this session.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {status === "unavailable" ? (
        <Alert variant="info">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Web preview</AlertTitle>
            <AlertDescription>
              Preferences are saved in browser storage. The desktop build stores them in SQLite on
              this device.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      <SettingsSection
        title="Appearance"
        description="Theme changes apply immediately and persist across launches."
      >
        <div className="grid gap-2 sm:grid-cols-3">
          {themeOptions.map((option) => {
            const Icon = option.icon;
            const active = preferences.theme === option.value;

            return (
              <button
                key={option.value}
                type="button"
                aria-pressed={active}
                onClick={() => setTheme(option.value)}
                className={cn(
                  "flex flex-col items-start gap-2 rounded-md border p-3 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/40",
                  active
                    ? "border-primary/60 bg-primary/5"
                    : "border-border bg-card hover:border-input hover:bg-muted/50",
                )}
              >
                <span className="flex w-full items-center gap-2">
                  <Icon
                    className={cn("size-4", active ? "text-primary" : "text-muted-foreground")}
                    aria-hidden="true"
                  />
                  <span className="text-[13px] font-medium">{option.label}</span>
                  {active ? (
                    <CheckCircleIcon className="ml-auto size-3.5 text-primary" aria-hidden="true" />
                  ) : null}
                </span>
                <span className="text-[11px] leading-relaxed text-muted-foreground">
                  {option.description}
                </span>
              </button>
            );
          })}
        </div>
      </SettingsSection>

      <SettingsSection
        title="Monitoring"
        description="Snapshots stay lightweight. Old values are marked with their sample time."
      >
        <div className="max-w-sm space-y-2">
          <Label htmlFor="monitoring-interval">Refresh interval</Label>
          <Select
            value={String(preferences.monitoringIntervalSeconds)}
            onValueChange={(value) => setMonitoringInterval(Number(value))}
          >
            <SelectTrigger id="monitoring-interval">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {intervals.map((seconds) => (
                <SelectItem key={seconds} value={String(seconds)}>
                  {seconds < 60
                    ? `${seconds} seconds`
                    : `${seconds / 60} minute${seconds === 60 ? "" : "s"}`}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            Minimum {MIN_MONITORING_INTERVAL_SECONDS} seconds. Manual refresh is always available.
          </p>
        </div>
      </SettingsSection>

      <SettingsSection
        title="Session"
        description="ServerDesk remembers the last server you worked with."
      >
        <div className="flex max-w-xl items-center justify-between gap-4 rounded-md border border-border bg-card px-3.5 py-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-[12px] font-medium">
              <HardDrivesIcon className="size-4 text-muted-foreground" aria-hidden="true" />
              Last active server
            </div>
            <div className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
              {preferences.lastActiveServerId ?? "None selected"}
            </div>
          </div>
          {preferences.lastActiveServerId ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setLastActiveServerId(null)}
              className="shrink-0"
            >
              Clear
            </Button>
          ) : null}
        </div>
      </SettingsSection>

      <UpdateSettingsSection />

      <Separator />

      <div className="flex items-start gap-2.5 text-[11px] leading-relaxed text-muted-foreground">
        <ClockIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
        <p>
          Preferences are stored locally in SQLite. ServerDesk does not send settings, server
          inventory, or credentials to a cloud service.
        </p>
      </div>
    </div>
  );
}

export { SettingsPage };
