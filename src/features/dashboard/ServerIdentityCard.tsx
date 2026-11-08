import { useEffect, useRef } from "react";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  IdentificationCardIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useSystemIdentityStore } from "@/stores/systemIdentityStore";
import { cn } from "@/lib/utils";

type ServerIdentityCardProps = {
  serverId: string;
  connected: boolean;
};

function formatUptime(totalSeconds: number) {
  const safeSeconds = Math.max(0, Math.floor(totalSeconds));
  const days = Math.floor(safeSeconds / 86_400);
  const hours = Math.floor((safeSeconds % 86_400) / 3_600);
  const minutes = Math.floor((safeSeconds % 3_600) / 60);
  const seconds = safeSeconds % 60;
  const parts: string[] = [];

  if (days > 0) {
    parts.push(`${days} ${days === 1 ? "day" : "days"}`);
  }
  if (hours > 0) {
    parts.push(`${hours} ${hours === 1 ? "hour" : "hours"}`);
  }
  if (minutes > 0) {
    parts.push(`${minutes} ${minutes === 1 ? "minute" : "minutes"}`);
  }
  if (parts.length === 0) {
    parts.push(`${seconds} ${seconds === 1 ? "second" : "seconds"}`);
  }

  return parts.slice(0, 2).join(", ");
}

function formatSampleTime(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function IdentityField({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string | null;
  mono?: boolean;
}) {
  return (
    <div>
      <dt className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {label}
      </dt>
      <dd
        className={cn(
          "mt-1 break-words text-[12px] text-foreground",
          mono && "font-mono text-[11px]",
        )}
      >
        {value ?? <span className="text-muted-foreground">Unavailable</span>}
      </dd>
    </div>
  );
}

function ServerIdentityCard({ serverId, connected }: ServerIdentityCardProps) {
  const identity = useSystemIdentityStore((state) => state.identities[serverId]);
  const pending = useSystemIdentityStore((state) => state.pending[serverId] ?? false);
  const stale = useSystemIdentityStore((state) => state.stale[serverId] ?? false);
  const lastAttemptAt = useSystemIdentityStore((state) => state.lastAttemptAt[serverId] ?? null);
  const error = useSystemIdentityStore((state) => state.error);
  const refresh = useSystemIdentityStore((state) => state.refresh);
  const clearServer = useSystemIdentityStore((state) => state.clearServer);
  const clearError = useSystemIdentityStore((state) => state.clearError);
  const requested = useRef(false);

  useEffect(() => {
    if (!connected) {
      requested.current = false;
      clearServer(serverId);
      return;
    }

    if (!identity && !pending && !requested.current) {
      requested.current = true;
      void refresh(serverId);
    }
  }, [clearServer, connected, identity, pending, refresh, serverId]);

  function handleRefresh() {
    requested.current = true;
    void refresh(serverId);
  }

  return (
    <section aria-label="Server identity">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h3 className="flex items-center gap-1.5 text-[13px] font-semibold text-foreground">
            <IdentificationCardIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            Server identity
          </h3>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            Hostname, distribution, kernel, architecture, and uptime sampled with read-only commands
            over the authenticated session.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {stale ? (
            <Badge variant="warning" aria-label="Identity sample is stale">
              Stale sample
            </Badge>
          ) : null}
          <Button
            variant="secondary"
            size="sm"
            onClick={handleRefresh}
            disabled={!connected || pending}
            aria-label="Refresh identity"
          >
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {pending ? "Sampling..." : "Refresh identity"}
          </Button>
        </div>
      </div>

      {error ? (
        <Alert variant="destructive" className="mt-3 py-2.5">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div className="flex-1">
            <AlertTitle>Could not collect server identity</AlertTitle>
            <AlertDescription>
              {error}
              {identity ? (
                <>
                  {" "}
                  Showing the last successful sample from{" "}
                  <time dateTime={new Date(identity.sampledAt).toISOString()}>
                    {formatSampleTime(identity.sampledAt)}
                  </time>
                  . These values are stale and are not live.
                </>
              ) : null}
            </AlertDescription>
          </div>
          <Button variant="ghost" size="sm" onClick={clearError}>
            Dismiss
          </Button>
        </Alert>
      ) : null}

      {!connected && !identity ? (
        <p className="mt-3 rounded-md border border-dashed border-border px-3 py-2 text-[11px] text-muted-foreground">
          Connect the server to collect its identity. Fields the remote system does not expose stay
          marked as unavailable.
        </p>
      ) : null}

      {pending && !identity ? (
        <div
          className="mt-3 flex items-center gap-2 text-[11px] text-muted-foreground"
          aria-busy="true"
        >
          <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
          Collecting server identity...
        </div>
      ) : null}

      {identity ? (
        <>
          <dl className="mt-4 grid gap-x-6 gap-y-4 border-t border-border/70 pt-4 sm:grid-cols-2 xl:grid-cols-3">
            <IdentityField label="Hostname" value={identity.hostname} mono />
            <IdentityField label="Distribution" value={identity.distribution} />
            <IdentityField label="Version" value={identity.distributionVersion} mono />
            <IdentityField label="Kernel" value={identity.kernel} mono />
            <IdentityField label="Architecture" value={identity.architecture} mono />
            <IdentityField
              label="Uptime"
              value={identity.uptimeSeconds === null ? null : formatUptime(identity.uptimeSeconds)}
            />
            <div>
              <dt className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
                Sampled
              </dt>
              <dd className="mt-1 text-[12px] text-foreground">
                <time dateTime={new Date(identity.sampledAt).toISOString()}>
                  {formatSampleTime(identity.sampledAt)}
                </time>
              </dd>
            </div>
          </dl>
          <p className="mt-3 text-[11px] text-muted-foreground">
            Read-only sample completed in {identity.durationMs} ms. Unavailable fields were not
            guessed from other system data.
          </p>
          {stale && lastAttemptAt !== null ? (
            <p className="mt-1 text-[11px] text-muted-foreground">
              Last refresh attempt{" "}
              <time dateTime={new Date(lastAttemptAt).toISOString()}>
                {formatSampleTime(lastAttemptAt)}
              </time>
            </p>
          ) : null}
        </>
      ) : null}
    </section>
  );
}

export { ServerIdentityCard };
