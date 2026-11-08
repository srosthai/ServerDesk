import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  CaretDownIcon,
  CaretLeftIcon,
  CaretRightIcon,
  CaretUpDownIcon,
  CaretUpIcon,
  CheckCircleIcon,
  CircleNotchIcon,
  GaugeIcon,
  InfoIcon,
  MagnifyingGlassIcon,
  MinusCircleIcon,
  PlayIcon,
  QuestionIcon,
  StopIcon,
  WarningCircleIcon,
  XCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { OutputPanel } from "@/components/ui/output-panel";
import { PageHeader } from "@/components/ui/page-header";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ErrorState, LoadingState } from "@/components/ui/states";
import { StatusBadge } from "@/components/ui/status-badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  filterAndSortServices,
  serviceActionTranscript,
  type ServiceSort,
  type ServiceSortKey,
  type ServiceStateFilter,
} from "@/features/services/serviceView";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import {
  useServiceStore,
  type ServiceActionEntry,
  type ServiceDiscoveryEntry,
} from "@/stores/serviceStore";
import type {
  ServerProfile,
  ServiceActionName,
  ServiceActionResult,
  ServiceDiscovery,
  ServiceState,
} from "@/types/contracts";

const SERVICE_PAGE_SIZE = 25;

type ServiceStateBadgeConfig = {
  label: string;
  variant: "success" | "warning" | "danger" | "outline";
  Icon: typeof CheckCircleIcon;
};

const serviceStateBadges: Record<ServiceState, ServiceStateBadgeConfig> = {
  active: { label: "Active", variant: "success", Icon: CheckCircleIcon },
  inactive: { label: "Inactive", variant: "outline", Icon: MinusCircleIcon },
  failed: { label: "Failed", variant: "danger", Icon: XCircleIcon },
  unknown: { label: "Unknown", variant: "warning", Icon: QuestionIcon },
};

const serviceStates: ServiceState[] = ["active", "inactive", "failed", "unknown"];

function ServiceStateBadge({ state }: { state: ServiceState }) {
  const config = serviceStateBadges[state];

  return (
    <Badge variant={config.variant}>
      <config.Icon className="size-3" aria-hidden="true" />
      {config.label}
    </Badge>
  );
}

function formatSampleTime(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function dateTimeValue(sampledAt: number) {
  const date = new Date(sampledAt);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function discoveryBadge(
  connected: boolean,
  entry: ServiceDiscoveryEntry | undefined,
): { label: string; variant: "success" | "warning" | "info" | "outline" } {
  const discovery = entry?.discovery;

  if (!discovery) {
    return entry?.pending
      ? { label: "Discovering", variant: "info" }
      : { label: connected ? "Not discovered" : "Disconnected", variant: "outline" };
  }
  if (entry?.pending) {
    return { label: "Refreshing", variant: "info" };
  }
  if (!connected) {
    return { label: "Read-only", variant: "outline" };
  }
  if (discovery.status === "unsupported") {
    return { label: "Unsupported", variant: "warning" };
  }
  if (discovery.status === "unavailable") {
    return { label: "Unavailable", variant: "warning" };
  }

  return { label: "Discovered", variant: "success" };
}

function stateCounts(discovery: ServiceDiscovery | null) {
  const counts: Record<ServiceState, number> = {
    active: 0,
    inactive: 0,
    failed: 0,
    unknown: 0,
  };
  for (const service of discovery?.services ?? []) {
    counts[service.state] += 1;
  }
  return counts;
}

function ServicesToolbar({
  servers,
  selected,
  onSelect,
  connected,
  entry,
  onRefresh,
}: {
  servers: ServerProfile[];
  selected: ServerProfile;
  onSelect: (serverId: string) => void;
  connected: boolean;
  entry: ServiceDiscoveryEntry | undefined;
  onRefresh: () => void;
}) {
  const pending = entry?.pending ?? false;
  const badge = discoveryBadge(connected, entry);

  return (
    <section
      aria-label="Service discovery controls"
      className="rounded-md border border-border bg-card px-3 py-3"
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 flex-1">
          <label
            htmlFor="services-server"
            className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            Server
          </label>
          <Select value={selected.id} onValueChange={onSelect}>
            <SelectTrigger id="services-server" className="mt-1 max-w-md">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {servers.map((server) => (
                <SelectItem key={server.id} value={server.id}>
                  {server.name} · {server.username}@{server.host}:{server.port}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-[10px] text-muted-foreground">
            <span>
              {selected.username}@{selected.host}:{selected.port}
            </span>
            <span aria-hidden="true">·</span>
            <span>
              {connected
                ? "Systemd units are read live; every action re-queries the unit"
                : "Connect this server to discover and control its actual units"}
            </span>
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <StatusBadge state={connected ? "connected" : "disconnected"} />
          <Badge variant={badge.variant}>{badge.label}</Badge>
          <Button
            variant="secondary"
            size="sm"
            onClick={onRefresh}
            disabled={!connected || pending}
            aria-label="Refresh discovered services"
          >
            {pending ? (
              <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
            )}
            {pending ? "Refreshing..." : "Refresh now"}
          </Button>
        </div>
      </div>
    </section>
  );
}

type ServiceActionConfig = {
  label: string;
  Icon: typeof InfoIcon;
};

const serviceActionConfig: Record<ServiceActionName, ServiceActionConfig> = {
  status: { label: "Inspect", Icon: InfoIcon },
  start: { label: "Start", Icon: PlayIcon },
  stop: { label: "Stop", Icon: StopIcon },
  restart: { label: "Restart", Icon: ArrowClockwiseIcon },
};

const serviceActionOrder: ServiceActionName[] = ["status", "start", "stop", "restart"];

function SortableTableHead({
  label,
  sortKey,
  sort,
  onSort,
  className,
}: {
  label: string;
  sortKey: ServiceSortKey;
  sort: ServiceSort;
  onSort: (key: ServiceSortKey) => void;
  className?: string;
}) {
  const active = sort.key === sortKey;
  const ariaSort = active ? (sort.direction === "asc" ? "ascending" : "descending") : "none";

  return (
    <TableHead aria-sort={ariaSort} className={className}>
      <button
        type="button"
        onClick={() => onSort(sortKey)}
        className="inline-flex items-center gap-1 rounded-sm text-[11px] font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/30"
        aria-label={`Sort by ${label.toLowerCase()}`}
      >
        {label}
        {active ? (
          sort.direction === "asc" ? (
            <CaretUpIcon className="size-3" aria-hidden="true" />
          ) : (
            <CaretDownIcon className="size-3" aria-hidden="true" />
          )
        ) : (
          <CaretUpDownIcon className="size-3 opacity-50" aria-hidden="true" />
        )}
      </button>
    </TableHead>
  );
}

function ServiceList({
  discovery,
  actionEntry,
  discoveryPending,
  connected,
  onAction,
}: {
  discovery: ServiceDiscovery;
  actionEntry: ServiceActionEntry | undefined;
  discoveryPending: boolean;
  connected: boolean;
  onAction: (unit: string, action: ServiceActionName) => void;
}) {
  const [query, setQuery] = useState("");
  const [stateFilter, setStateFilter] = useState<ServiceStateFilter>("all");
  const [sort, setSort] = useState<ServiceSort>({ key: "name", direction: "asc" });
  const [page, setPage] = useState(0);

  const counts = stateCounts(discovery);
  const total = discovery.services.length;
  const filtered = useMemo(
    () => filterAndSortServices(discovery.services, query, stateFilter, sort),
    [discovery.services, query, stateFilter, sort],
  );
  const pageCount = Math.max(1, Math.ceil(filtered.length / SERVICE_PAGE_SIZE));
  const currentPage = Math.min(page, pageCount - 1);
  const start = currentPage * SERVICE_PAGE_SIZE;
  const visible = filtered.slice(start, start + SERVICE_PAGE_SIZE);
  const filtersActive = query.trim().length > 0 || stateFilter !== "all";
  const actionPending = actionEntry?.pending ?? null;
  const busy = Boolean(actionPending) || discoveryPending;

  if (total === 0) {
    return (
      <EmptyState
        icon={<GaugeIcon className="size-4" aria-hidden="true" />}
        title="No systemd services discovered"
        description="The server reported systemd but did not list any service units. ServerDesk never adds common service names that the server did not report."
      />
    );
  }

  function changeQuery(nextQuery: string) {
    setQuery(nextQuery);
    setPage(0);
  }

  function changeStateFilter(nextFilter: ServiceStateFilter) {
    setStateFilter(nextFilter);
    setPage(0);
  }

  function toggleSort(key: ServiceSortKey) {
    setSort((current) =>
      current.key === key
        ? { key, direction: current.direction === "asc" ? "desc" : "asc" }
        : { key, direction: "asc" },
    );
    setPage(0);
  }

  function clearFilters() {
    setQuery("");
    setStateFilter("all");
    setPage(0);
  }

  return (
    <section
      aria-label="Discovered systemd services"
      className="overflow-hidden rounded-md border border-border bg-card"
    >
      <div className="border-b border-border px-3 py-2.5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="text-[13px] font-semibold text-foreground">Discovered systemd units</h2>
            <p className="mt-0.5 text-[11px] text-muted-foreground">
              {total} unit{total === 1 ? "" : "s"} reported by systemctl
              {discovery.truncated ? " · output was truncated at the safety limit" : ""}
            </p>
          </div>
          <div
            role="group"
            aria-label="Filter by service state"
            className="flex flex-wrap items-center gap-1.5"
          >
            <Button
              type="button"
              variant={stateFilter === "all" ? "default" : "secondary"}
              size="sm"
              className="h-7 gap-1 px-2 text-[11px]"
              aria-pressed={stateFilter === "all"}
              onClick={() => changeStateFilter("all")}
            >
              All {total}
            </Button>
            {serviceStates.map((state) => {
              const config = serviceStateBadges[state];
              return (
                <Button
                  key={state}
                  type="button"
                  variant={stateFilter === state ? "default" : "secondary"}
                  size="sm"
                  className="h-7 gap-1 px-2 text-[11px]"
                  aria-pressed={stateFilter === state}
                  aria-label={`Show ${config.label.toLowerCase()} units (${counts[state].toString()})`}
                  onClick={() => changeStateFilter(state)}
                >
                  <config.Icon className="size-3" aria-hidden="true" />
                  {config.label} {counts[state]}
                </Button>
              );
            })}
          </div>
        </div>

        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          <div className="relative w-full max-w-sm min-w-[210px]">
            <label htmlFor="service-filter" className="sr-only">
              Filter units
            </label>
            <MagnifyingGlassIcon
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <Input
              id="service-filter"
              type="search"
              value={query}
              onChange={(event) => changeQuery(event.target.value)}
              placeholder="Filter by unit, state, or description"
              className="h-8 pl-8 text-[12px]"
            />
          </div>
          {filtersActive ? (
            <Button type="button" variant="ghost" size="sm" onClick={clearFilters}>
              Clear filters
            </Button>
          ) : null}
          <p className="text-[11px] text-muted-foreground" aria-live="polite">
            {filtered.length} of {total} units
          </p>
        </div>
      </div>

      {filtered.length === 0 ? (
        <div className="px-3 py-8">
          <p className="text-[13px] font-medium text-foreground">No units match this filter</p>
          <p className="mt-1 max-w-[56ch] text-[12px] text-muted-foreground">
            Search covers unit names, descriptions, load state, and raw systemd active and sub
            states. Try a different term or clear the filters.
          </p>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="mt-3"
            onClick={clearFilters}
          >
            Clear filters
          </Button>
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <SortableTableHead label="Unit" sortKey="name" sort={sort} onSort={toggleSort} />
              <SortableTableHead label="State" sortKey="state" sort={sort} onSort={toggleSort} />
              <SortableTableHead
                label="Description"
                sortKey="description"
                sort={sort}
                onSort={toggleSort}
              />
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {visible.map((service) => (
              <TableRow key={service.name} data-service-state={service.state}>
                <TableCell className="align-top">
                  <div className="font-mono text-[12px] font-medium">{service.name}</div>
                  <div className="mt-0.5 font-mono text-[10px] text-muted-foreground">
                    {service.loadState ? `load ${service.loadState}` : "load unknown"}
                  </div>
                </TableCell>
                <TableCell className="align-top">
                  <ServiceStateBadge state={service.state} />
                  <div className="mt-0.5 font-mono text-[10px] text-muted-foreground">
                    {service.activeState}
                    {service.subState ? ` / ${service.subState}` : ""}
                  </div>
                </TableCell>
                <TableCell className="max-w-[52ch] align-top text-[12px] text-muted-foreground">
                  <span className="line-clamp-2" title={service.description ?? undefined}>
                    {service.description ?? "No description reported"}
                  </span>
                </TableCell>
                <TableCell className="align-top">
                  <div
                    role="group"
                    aria-label={`Actions for ${service.name}`}
                    className="flex flex-wrap justify-end gap-1"
                  >
                    {serviceActionOrder.map((action) => {
                      const config = serviceActionConfig[action];
                      const actionIsPending =
                        actionPending === action && actionEntry?.pendingUnit === service.name;
                      return (
                        <Button
                          key={action}
                          type="button"
                          variant={action === "status" ? "ghost" : "secondary"}
                          size="sm"
                          className="h-7 gap-1 px-2 text-[11px]"
                          aria-label={`${config.label} ${service.name}`}
                          title={`${config.label} ${service.name}`}
                          disabled={busy || !connected}
                          onClick={() => onAction(service.name, action)}
                        >
                          {actionIsPending ? (
                            <CircleNotchIcon className="size-3 animate-spin" aria-hidden="true" />
                          ) : (
                            <config.Icon className="size-3" aria-hidden="true" />
                          )}
                          {config.label}
                        </Button>
                      );
                    })}
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border px-3 py-2 text-[10px] text-muted-foreground">
        <span>
          Showing{" "}
          {filtered.length === 0
            ? "0"
            : `${(start + 1).toString()}-${(start + visible.length).toString()}`}{" "}
          of {filtered.length}
          {filtered.length !== total ? ` filtered (${total.toString()} discovered)` : ""} units ·
          sampled{" "}
          <time dateTime={dateTimeValue(discovery.sampledAt)}>
            {formatSampleTime(discovery.sampledAt)}
          </time>{" "}
          in {discovery.durationMs} ms.
        </span>
        <nav aria-label="Service table pages" className="flex items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-2"
            aria-label="Previous page"
            disabled={currentPage === 0}
            onClick={() => setPage(currentPage - 1)}
          >
            <CaretLeftIcon className="size-3" aria-hidden="true" />
            Previous
          </Button>
          <span className="px-1 font-mono">
            Page {(currentPage + 1).toString()} of {pageCount.toString()}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-2"
            aria-label="Next page"
            disabled={currentPage >= pageCount - 1}
            onClick={() => setPage(currentPage + 1)}
          >
            Next
            <CaretRightIcon className="size-3" aria-hidden="true" />
          </Button>
        </nav>
      </div>
    </section>
  );
}

function ServiceActionResultPanel({
  result,
  onDismiss,
}: {
  result: ServiceActionResult;
  onDismiss: () => void;
}) {
  const verified = result.verifiedStatus;
  const verifiedBadge = verified ? serviceStateBadges[verified.state] : null;
  const actionLabel = serviceActionConfig[result.action].label;
  const verifiedSuccess = result.succeeded && result.stateVerified === true;
  const verificationMismatch = result.succeeded && result.stateVerified === false;
  const outcome = !result.succeeded
    ? { label: "Command failed", variant: "danger" as const }
    : verificationMismatch
      ? { label: "State mismatch", variant: "warning" as const }
      : result.action === "status"
        ? { label: "Inspected", variant: "info" as const }
        : result.stateVerified === true
          ? { label: "Verified", variant: "success" as const }
          : { label: "Unverified", variant: "warning" as const };

  return (
    <section
      aria-label="Service action result"
      className="rounded-md border border-border bg-card px-3 py-3"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-[13px] font-semibold text-foreground">
              {actionLabel} {result.unit}
            </h2>
            <Badge variant={outcome.variant}>{outcome.label}</Badge>
          </div>
          <p className="mt-0.5 break-all font-mono text-[10px] text-muted-foreground">
            {result.command}
          </p>
        </div>
        <Button variant="ghost" size="sm" className="shrink-0" onClick={onDismiss}>
          Dismiss
        </Button>
      </div>

      <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-2 rounded-md border border-border bg-muted/35 px-3 py-2.5 text-[11px] sm:grid-cols-4">
        <div>
          <dt className="text-muted-foreground">Exit status</dt>
          <dd className="font-mono text-foreground">
            {result.exitCode === null
              ? "No exit status"
              : `Exit code ${result.exitCode.toString()}`}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Duration</dt>
          <dd className="font-mono text-foreground">{result.durationMs} ms</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Verified state</dt>
          <dd>
            {verified && verifiedBadge ? (
              <ServiceStateBadge state={verified.state} />
            ) : (
              <Badge variant="warning">Unverified</Badge>
            )}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Verification</dt>
          <dd className="text-foreground">
            {result.stateVerified === true
              ? "Matches the requested state"
              : result.stateVerified === false
                ? "Does not match the requested state"
                : verified
                  ? "State re-queried"
                  : "Not verified"}
          </dd>
        </div>
      </dl>

      {result.error ? (
        <Alert variant="destructive" className="mt-3">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>
              {result.permissionDenied ? "Sudo permission denied" : "Service action failed"}
            </AlertTitle>
            <AlertDescription>{result.error.message}</AlertDescription>
          </div>
        </Alert>
      ) : null}

      {result.permissionDenied && result.sudoGuidance ? (
        <OutputPanel
          title="Required scoped sudo rule"
          content={result.sudoGuidance}
          className="mt-3"
          maxHeightClassName="max-h-52"
        />
      ) : null}

      {verifiedSuccess ? (
        <Alert variant="success" className="mt-3">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
          <div>
            <AlertTitle>Verified state after the action</AlertTitle>
            <AlertDescription>
              systemd reports {result.unit} as {verified?.state}. This is the state the unit list
              keeps showing after its refresh.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {verificationMismatch ? (
        <Alert variant="warning" className="mt-3">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Verified state differs from the request</AlertTitle>
            <AlertDescription>
              The command exited successfully, but systemd reports {result.unit} as{" "}
              {verified?.state ?? "unknown"}. No optimistic success is shown.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {result.succeeded && !verified ? (
        <Alert variant="warning" className="mt-3">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Action could not be verified</AlertTitle>
            <AlertDescription>
              The command exited successfully, but the follow-up status query did not return a
              verified state. Treat this as unconfirmed.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {result.verificationError ? (
        <Alert variant="warning" className="mt-3">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Verification query failed</AlertTitle>
            <AlertDescription>{result.verificationError}</AlertDescription>
          </div>
        </Alert>
      ) : null}

      <OutputPanel
        title="Command output"
        content={serviceActionTranscript(result)}
        meta={
          result.exitCode === null
            ? "no exit status"
            : `exit ${result.exitCode.toString()} · ${result.durationMs.toString()} ms`
        }
        className="mt-3"
        maxHeightClassName="max-h-64"
      />
    </section>
  );
}

function ServicesPage() {
  const servers = useServerStore((state) => state.servers);
  const status = useServerStore((state) => state.status);
  const error = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const connections = useConnectionStore((state) => state.connections);
  const lastActiveServerId = usePreferencesStore((state) => state.preferences.lastActiveServerId);
  const refresh = useServiceStore((state) => state.refresh);
  const clearError = useServiceStore((state) => state.clearError);
  const runAction = useServiceStore((state) => state.runAction);
  const clearActionResult = useServiceStore((state) => state.clearActionResult);

  const [selectedId, setSelectedId] = useState<string | null>(lastActiveServerId);
  const [pendingAction, setPendingAction] = useState<{
    unit: string;
    action: Extract<ServiceActionName, "stop" | "restart">;
  } | null>(null);

  const selected =
    servers.find((server) => server.id === selectedId) ??
    servers.find((server) => server.id === lastActiveServerId) ??
    servers[0] ??
    null;
  const selectedServerId = selected?.id ?? null;
  const connection = selected ? connections[selected.id] : undefined;
  const connected = connection?.state === "connected";
  const entry = useServiceStore((state) => (selected ? state.entries[selected.id] : undefined));
  const actionEntry = useServiceStore((state) =>
    selected ? state.actions[selected.id] : undefined,
  );
  const discovery = entry?.discovery ?? null;
  const resultRef = useRef<HTMLDivElement>(null);

  const requestAction = (unit: string, action: ServiceActionName) => {
    if (!selectedServerId || !connected) {
      return;
    }
    if (action === "stop" || action === "restart") {
      setPendingAction({ unit, action });
      return;
    }

    void runAction(selectedServerId, unit, action);
  };

  const selectServer = (serverId: string) => {
    setPendingAction(null);
    setSelectedId(serverId);
  };

  useEffect(() => {
    if (status === "idle") {
      void fetchServers();
    }
  }, [status, fetchServers]);

  useEffect(() => {
    if (status === "ready" && servers.length > 0) {
      void refreshConnectionStates(servers.map((server) => server.id));
    }
  }, [status, servers, refreshConnectionStates]);

  useEffect(() => {
    if (!connected || !selectedServerId) {
      return;
    }
    if (entry?.pending || entry?.discovery || entry?.error) {
      return;
    }

    void refresh(selectedServerId);
  }, [connected, selectedServerId, entry?.pending, entry?.discovery, entry?.error, refresh]);

  useEffect(() => {
    if (actionEntry?.result) {
      resultRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }
  }, [actionEntry?.result]);

  return (
    <div className="space-y-6">
      <PageHeader
        title="Services"
        description="Discover the systemd service units that actually exist on the target machine, filter and sort large unit lists, and run verified actions with copyable command output."
        actions={
          <Button asChild variant="secondary" size="sm">
            <Link to="/servers">Open servers</Link>
          </Button>
        }
      />

      {!isTauriRuntime() ? (
        <Alert variant="info">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Web preview</AlertTitle>
            <AlertDescription>
              Service discovery needs a live SSH session. Open the desktop build and connect a
              server to inspect its real systemd units. No service names or states are fabricated in
              browser preview.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {status === "loading" && servers.length === 0 ? <LoadingState rows={4} /> : null}

      {status === "error" && servers.length === 0 ? (
        <ErrorState message={error ?? "Could not load server profiles."} onRetry={fetchServers} />
      ) : null}

      {status !== "loading" && servers.length === 0 ? (
        <EmptyState
          icon={<GaugeIcon className="size-4" aria-hidden="true" />}
          title="No server selected"
          description="Add a server profile and connect it to discover its actual systemd services."
          action={
            <Button asChild size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      ) : null}

      {selected ? (
        <>
          <ServicesToolbar
            servers={servers}
            selected={selected}
            onSelect={selectServer}
            connected={connected}
            entry={entry}
            onRefresh={() => void refresh(selected.id)}
          />

          {entry?.error ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Could not discover services</AlertTitle>
                <AlertDescription>
                  {entry.error}
                  {discovery
                    ? " The last successful discovery stays visible with its original timestamp. No service states were invented."
                    : null}
                </AlertDescription>
              </div>
              <Button variant="ghost" size="sm" onClick={() => clearError(selected.id)}>
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {discovery?.status === "unsupported" ? (
            <Alert variant="warning">
              <InfoIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
              <div>
                <AlertTitle>Systemd is not available</AlertTitle>
                <AlertDescription>
                  {discovery.detail ??
                    "This server does not run systemd, so service discovery cannot list units."}
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {discovery?.status === "unavailable" ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div>
                <AlertTitle>Systemd discovery failed</AlertTitle>
                <AlertDescription>
                  {discovery.detail ??
                    "The remote systemd command failed. Try again while the server is connected."}
                </AlertDescription>
              </div>
            </Alert>
          ) : null}

          {entry?.pending && !discovery ? <LoadingState rows={5} /> : null}

          {discovery?.status === "available" ? (
            <ServiceList
              key={discovery.serverId}
              discovery={discovery}
              actionEntry={actionEntry}
              discoveryPending={entry?.pending ?? false}
              connected={connected}
              onAction={requestAction}
            />
          ) : null}

          {actionEntry?.error ? (
            <Alert variant="destructive">
              <WarningCircleIcon
                className="mt-0.5 size-4 shrink-0 text-destructive"
                aria-hidden="true"
              />
              <div className="flex-1">
                <AlertTitle>Service action failed</AlertTitle>
                <AlertDescription>
                  <span className="block">{actionEntry.error}</span>
                  {actionEntry.errorDetails ? (
                    <span className="mt-1.5 block whitespace-pre-wrap font-mono text-[11px]">
                      {actionEntry.errorDetails}
                    </span>
                  ) : null}
                </AlertDescription>
              </div>
              <Button variant="ghost" size="sm" onClick={() => clearActionResult(selected.id)}>
                Dismiss
              </Button>
            </Alert>
          ) : null}

          {actionEntry?.result ? (
            <div ref={resultRef}>
              <ServiceActionResultPanel
                result={actionEntry.result}
                onDismiss={() => clearActionResult(selected.id)}
              />
            </div>
          ) : null}

          {!entry?.pending && !discovery && !entry?.error ? (
            <EmptyState
              icon={<GaugeIcon className="size-4" aria-hidden="true" />}
              title={connected ? "No discovery yet" : "Service discovery is paused"}
              description={
                connected
                  ? "Refresh to query systemd on the selected server."
                  : "Connect this server to query its systemd units. Service discovery never assumes that common units exist."
              }
              action={
                connected ? (
                  <Button size="sm" onClick={() => void refresh(selected.id)}>
                    Discover services
                  </Button>
                ) : undefined
              }
            />
          ) : null}

          <ConfirmDialog
            open={pendingAction !== null}
            onOpenChange={(open) => {
              if (!open) {
                setPendingAction(null);
              }
            }}
            title={
              pendingAction
                ? `${serviceActionConfig[pendingAction.action].label} ${pendingAction.unit} on ${selected.name}?`
                : "Confirm service action"
            }
            description={
              pendingAction?.action === "stop"
                ? "Stopping this unit can interrupt connections and dependent services. ServerDesk sends the fixed command, then immediately queries the unit again."
                : "Restarting this unit can interrupt connections while it starts again. ServerDesk sends the fixed command, then immediately queries the unit again."
            }
            confirmLabel={
              pendingAction
                ? `${serviceActionConfig[pendingAction.action].label} unit`
                : "Run action"
            }
            details={
              pendingAction
                ? [
                    {
                      label: "Server",
                      value: `${selected.name} (${selected.username}@${selected.host}:${selected.port})`,
                    },
                    { label: "Unit", value: pendingAction.unit },
                    {
                      label: "Action",
                      value: `systemctl ${pendingAction.action} -- ${pendingAction.unit}`,
                    },
                  ]
                : undefined
            }
            pending={Boolean(actionEntry?.pending)}
            onConfirm={() => {
              if (!pendingAction || !selectedServerId) {
                return;
              }
              const { unit, action } = pendingAction;
              void runAction(selectedServerId, unit, action);
              setPendingAction(null);
            }}
          />
        </>
      ) : null}

      {status === "error" && servers.length > 0 ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div>
            <AlertTitle>Could not refresh server profiles</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </div>
        </Alert>
      ) : null}
    </div>
  );
}

export { ServicesPage };
