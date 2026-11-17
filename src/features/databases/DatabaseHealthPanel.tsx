import { useEffect, useMemo, useRef } from "react";
import {
  ArrowClockwiseIcon,
  CircleNotchIcon,
  ClockIcon,
  DatabaseIcon,
  InfoIcon,
  PlugsIcon,
  TableIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { LoadingState } from "@/components/ui/states";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  HEALTH_BASIS_LABELS,
  buildHealthIndicators,
  formatHealthRowEstimate,
  formatHealthSampleAge,
  formatHealthSampleTime,
  formatHealthStatisticsTime,
  formatHealthTableName,
  formatHealthTableSize,
  formatHealthVersion,
  healthScopeMismatch,
  healthTables,
  healthToneLabel,
  isHealthSampleStale,
} from "@/features/databases/databaseHealthModel";
import { DATABASE_ENGINE_LABELS, formatLatency } from "@/features/databases/databaseProfileForm";
import { formatBytes } from "@/features/databases/databaseStructureModel";
import { useDatabaseHealthStore } from "@/stores/databaseHealthStore";
import type { DatabaseEngine, DatabaseMetadataValue } from "@/types/contracts";

type DatabaseHealthPanelProps = {
  profileId: string | null;
  profileName: string;
  engine: DatabaseEngine;
  connected: boolean;
  selectedDatabase: string | null;
};

function toneVariant(tone: "healthy" | "attention" | "unavailable") {
  if (tone === "healthy") {
    return "success" as const;
  }
  if (tone === "attention") {
    return "warning" as const;
  }
  return "outline" as const;
}

function metadataText<T>(value: DatabaseMetadataValue<T>, format: (value: T) => string): string {
  return value.status === "available" ? format(value.value) : "Unavailable";
}

export function DatabaseHealthPanel({
  profileId,
  profileName,
  engine,
  connected,
  selectedDatabase,
}: DatabaseHealthPanelProps) {
  const entry = useDatabaseHealthStore((state) =>
    profileId ? state.entries[profileId] : undefined,
  );
  const fetchHealth = useDatabaseHealthStore((state) => state.fetchHealth);
  const refresh = useDatabaseHealthStore((state) => state.refresh);
  const previousConnected = useRef(connected);

  useEffect(() => {
    if (!profileId || !connected) {
      previousConnected.current = connected;
      return;
    }
    const current = useDatabaseHealthStore.getState().entries[profileId];
    if (!current || current.status === "idle") {
      void fetchHealth(profileId);
    } else if (!previousConnected.current) {
      // A reconnect invalidates the previous sample even if the store still
      // holds it. One explicit sample runs; there is no interval.
      void refresh(profileId);
    }
    previousConnected.current = connected;
  }, [connected, fetchHealth, profileId, refresh]);

  const snapshot = entry?.snapshot ?? null;
  const loading = !snapshot && (entry?.status === "loading" || entry?.status === "idle");
  const refreshing = entry?.refreshing ?? false;
  const error = entry?.error ?? null;
  const indicators = useMemo(() => (snapshot ? buildHealthIndicators(snapshot) : []), [snapshot]);
  const tables = snapshot ? healthTables(snapshot) : null;
  const stale = snapshot ? isHealthSampleStale(snapshot.sampledAt) : false;
  const sampleAge = snapshot ? formatHealthSampleAge(snapshot.sampledAt) : null;
  const scopeMismatch = snapshot ? healthScopeMismatch(snapshot, selectedDatabase) : false;

  if (!connected) {
    return (
      <Alert variant="warning">
        <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div>
          <AlertTitle>Connection required</AlertTitle>
          <AlertDescription>
            Connect {profileName} before sampling database sizes, version, and connection latency.
          </AlertDescription>
        </div>
      </Alert>
    );
  }

  if (!profileId) {
    return (
      <EmptyState
        icon={<DatabaseIcon className="size-4" aria-hidden="true" />}
        title="No connection selected"
        description="Open a saved connection to sample catalog sizes, server version, and latency."
      />
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="flex items-center gap-1.5 text-[13px] font-semibold tracking-[-0.01em] text-foreground">
            <DatabaseIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            Database health
          </h2>
          <p className="mt-0.5 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
            <span className="font-mono text-foreground">
              {snapshot
                ? (snapshot.database ?? "No active database")
                : (selectedDatabase ?? "No active database")}
            </span>
            <Badge variant="outline">{DATABASE_ENGINE_LABELS[engine]}</Badge>
            <span>Manual samples only · no polling</span>
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {snapshot ? (
            <span className="flex items-center gap-1 text-[10px] text-muted-foreground">
              <ClockIcon className="size-3" aria-hidden="true" />
              Last sample{" "}
              <time dateTime={new Date(snapshot.sampledAt).toISOString()}>
                {formatHealthSampleTime(snapshot.sampledAt)}
              </time>
              {sampleAge ? ` · ${sampleAge}` : null}
            </span>
          ) : null}
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="h-7 px-2 text-[11px]"
            onClick={() => void refresh(profileId)}
            disabled={refreshing || loading}
            aria-label="Refresh health sample"
          >
            {refreshing ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {refreshing ? "Refreshing..." : "Refresh health"}
          </Button>
        </div>
      </div>

      {stale && snapshot ? (
        <Alert variant="warning" className="py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div>
            <AlertTitle>This health sample is stale</AlertTitle>
            <AlertDescription>
              It was taken {sampleAge}. Refresh health before acting on size or latency values.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {error ? (
        <Alert variant="warning" className="py-2.5">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div className="flex-1">
            <AlertTitle>The health sample could not be refreshed</AlertTitle>
            <AlertDescription>
              {snapshot
                ? `${error} The previous sample remains visible and is marked stale when it ages out.`
                : error}
            </AlertDescription>
          </div>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => void refresh(profileId)}
          >
            Try again
          </Button>
        </Alert>
      ) : null}

      {scopeMismatch && snapshot ? (
        <Alert variant="info" className="py-2.5">
          <InfoIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div>
            <AlertTitle>Health follows the active database</AlertTitle>
            <AlertDescription>
              This sample is for {snapshot.database ?? "the active database"}. Set{" "}
              {selectedDatabase ?? "the selected database"} active to sample its sizes. The explorer
              selection did not change the live connection.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {loading ? (
        <LoadingState rows={4} />
      ) : !snapshot ? (
        <EmptyState
          icon={<PlugsIcon className="size-4" aria-hidden="true" />}
          title="No health sample yet"
          description={`Take one catalog-only sample for ${profileName}. No table rows are read.`}
          action={
            <Button size="sm" onClick={() => void refresh(profileId)}>
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
              Sample health
            </Button>
          }
        />
      ) : (
        <>
          <section
            aria-label="Health indicators"
            className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3"
          >
            {indicators.map((indicator) => (
              <div
                key={indicator.key}
                className="min-w-0 rounded-md border border-border bg-card px-3 py-2.5"
              >
                <div className="flex flex-wrap items-center justify-between gap-1.5">
                  <p className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                    {indicator.label}
                  </p>
                  <Badge variant="outline" className="text-[9px]">
                    {HEALTH_BASIS_LABELS[indicator.basis]}
                  </Badge>
                </div>
                <p className="mt-1 truncate text-[13px] font-semibold text-foreground">
                  {indicator.value}
                </p>
                <div className="mt-1 flex items-start gap-1.5">
                  <Badge variant={toneVariant(indicator.tone)} className="shrink-0 text-[9px]">
                    {healthToneLabel(indicator.tone)}
                  </Badge>
                  <p className="min-w-0 text-[10px] leading-4 text-muted-foreground">
                    {indicator.detail}
                  </p>
                </div>
              </div>
            ))}
          </section>

          <section
            aria-label="Health sample summary"
            className="overflow-hidden rounded-md border border-border bg-card"
          >
            <div className="flex items-center gap-1.5 border-b border-border bg-muted/15 px-3 py-2">
              <ClockIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
              <h3 className="text-[12px] font-semibold text-foreground">Sample summary</h3>
              <Badge variant="outline" className="ml-auto text-[9px]">
                {HEALTH_BASIS_LABELS.measured}
              </Badge>
            </div>
            <dl className="grid gap-x-6 gap-y-2.5 px-3 py-3 sm:grid-cols-2 xl:grid-cols-4">
              <div className="min-w-0">
                <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Engine
                </dt>
                <dd className="truncate text-[12px] text-foreground">
                  {DATABASE_ENGINE_LABELS[snapshot.engine]}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Server version
                </dt>
                <dd
                  className="truncate font-mono text-[11px] text-foreground"
                  title={snapshot.version.raw}
                >
                  {formatHealthVersion(snapshot.version)}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Active database
                </dt>
                <dd className="truncate font-mono text-[11px] text-foreground">
                  {snapshot.database ?? "Unavailable"}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Connection latency
                </dt>
                <dd className="truncate text-[12px] text-foreground">
                  {metadataText(snapshot.latencyMs, formatLatency)}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Database size
                </dt>
                <dd className="flex items-center gap-1.5 truncate text-[12px] text-foreground">
                  {metadataText(snapshot.databaseSizeBytes, formatBytes)}
                  <Badge variant="outline" className="text-[9px]">
                    {HEALTH_BASIS_LABELS.estimated}
                  </Badge>
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Last sample
                </dt>
                <dd className="truncate text-[12px] text-foreground">
                  {formatHealthSampleAge(snapshot.sampledAt)}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Tables sampled
                </dt>
                <dd className="truncate text-[12px] text-foreground">
                  {snapshot.tableSizes.status === "available"
                    ? `${snapshot.tableSizes.value.items.length}${
                        snapshot.tableSizes.value.truncated ? " (bounded)" : ""
                      }`
                    : "Unavailable"}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                  Sample state
                </dt>
                <dd className="truncate text-[12px] text-foreground">
                  {stale ? "Stale · refresh" : "Current"}
                </dd>
              </div>
            </dl>
          </section>

          <section
            aria-label="Table sizes"
            className="overflow-hidden rounded-md border border-border bg-card"
          >
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/15 px-3 py-2">
              <h3 className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
                <TableIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
                Table sizes
              </h3>
              <div className="flex items-center gap-1.5">
                <span className="text-[10px] text-muted-foreground">
                  Largest tables by allocated bytes
                </span>
                <Badge variant="outline" className="text-[9px]">
                  {HEALTH_BASIS_LABELS.estimated}
                </Badge>
              </div>
            </div>
            <div className="p-3">
              {tables === null ? (
                <p className="flex items-start gap-1.5 text-[10px] leading-4 text-muted-foreground">
                  <WarningCircleIcon
                    className="mt-0.5 size-3 shrink-0 text-warning"
                    aria-hidden="true"
                  />
                  Table sizes are unavailable for this account or engine. They are not shown as
                  zero.
                </p>
              ) : tables.length === 0 ? (
                <p className="text-[10px] leading-4 text-muted-foreground">
                  No tables were reported in the active database.
                </p>
              ) : (
                <>
                  <Table className="[&_td]:px-2 [&_th]:px-2">
                    <TableHeader>
                      <TableRow>
                        <TableHead>Table</TableHead>
                        <TableHead>Size (estimated)</TableHead>
                        <TableHead>Rows (estimated)</TableHead>
                        <TableHead>Statistics</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {tables.map((table) => (
                        <TableRow
                          key={`${table.database ?? ""}:${table.schema ?? ""}:${table.name}`}
                        >
                          <TableCell className="font-mono text-[11px]">
                            {formatHealthTableName(table)}
                          </TableCell>
                          <TableCell className="text-[11px]">
                            {formatHealthTableSize(table)}
                          </TableCell>
                          <TableCell className="text-[11px]">
                            {formatHealthRowEstimate(table)}
                          </TableCell>
                          <TableCell className="text-[11px] text-muted-foreground">
                            {formatHealthStatisticsTime(table.metadataRefreshedAt)}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                  {snapshot.tableSizes.status === "available" &&
                  snapshot.tableSizes.value.truncated ? (
                    <p className="mt-2 text-[10px] text-warning">
                      Only the largest {snapshot.tableSizes.value.limit} tables are shown. The list
                      is bounded to keep routine samples cheap.
                    </p>
                  ) : null}
                </>
              )}
            </div>
          </section>

          <Alert variant="info" className="py-2.5">
            <InfoIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <div>
              <AlertTitle>Catalog estimates, not live measurements</AlertTitle>
              <AlertDescription>
                Sizes and row counts come from engine catalog statistics; no table rows are scanned
                and no statistics maintenance runs. Estimated values are labeled. MySQL may not
                expose statistics refresh times to application accounts, so that field is shown as
                unavailable rather than guessed.
              </AlertDescription>
            </div>
          </Alert>
        </>
      )}
    </div>
  );
}
