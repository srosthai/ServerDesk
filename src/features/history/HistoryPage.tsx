import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowClockwiseIcon,
  CaretLeftIcon,
  CaretRightIcon,
  ClockCounterClockwiseIcon,
  InfoIcon,
  ShieldCheckIcon,
  TrashIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ErrorState, LoadingState } from "@/components/ui/states";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { isTauriRuntime } from "@/lib/tauri";
import { getCommandErrorMessage } from "@/lib/errors";
import {
  clearOperationHistory,
  getOperationHistoryPolicy,
  listOperationHistory,
} from "@/services/history";
import { useServerStore } from "@/stores/serverStore";
import type {
  OperationCategory,
  OperationHistoryEntry,
  OperationHistoryPolicy,
  OperationResult,
  ServerProfile,
} from "@/types/contracts";

const PAGE_SIZE = 50;

const categoryLabels: Record<OperationCategory, string> = {
  command: "Command",
  service: "Service",
  artisan: "Artisan",
  sftp: "SFTP",
  queue: "Queue",
};

const resultLabels: Record<OperationResult, string> = {
  success: "Success",
  failure: "Failure",
  cancelled: "Cancelled",
  unknown: "Unknown",
};

const resultBadgeVariants: Record<OperationResult, "success" | "danger" | "warning" | "outline"> = {
  success: "success",
  failure: "danger",
  cancelled: "warning",
  unknown: "outline",
};

const fallbackPolicy: OperationHistoryPolicy = {
  maxEntries: 1_000,
  outputSummaryMaxChars: 360,
  retention:
    "ServerDesk keeps the newest 1,000 operations and automatically removes older entries.",
};

function formatTimestamp(seconds: number) {
  const date = new Date(seconds * 1_000);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

function timestampValue(seconds: number) {
  const date = new Date(seconds * 1_000);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function formatDuration(durationMs: number) {
  if (durationMs < 1_000) {
    return `${durationMs.toString()} ms`;
  }
  return `${(durationMs / 1_000).toFixed(1)} s`;
}

function startOfDaySeconds(value: string): number | null {
  if (!value) {
    return null;
  }
  const date = new Date(`${value}T00:00:00`);
  return Number.isNaN(date.getTime()) ? null : Math.floor(date.getTime() / 1_000);
}

function endOfDaySeconds(value: string): number | null {
  if (!value) {
    return null;
  }
  const date = new Date(`${value}T23:59:59.999`);
  return Number.isNaN(date.getTime()) ? null : Math.floor(date.getTime() / 1_000);
}

function serverLabel(serverId: string | null, servers: ServerProfile[]) {
  if (!serverId) {
    return "Unknown server";
  }
  return servers.find((server) => server.id === serverId)?.name ?? "Deleted server";
}

function CategoryBadge({ category }: { category: OperationCategory }) {
  return <Badge variant="secondary">{categoryLabels[category]}</Badge>;
}

function ResultBadge({ result }: { result: OperationResult }) {
  return <Badge variant={resultBadgeVariants[result]}>{resultLabels[result]}</Badge>;
}

function HistoryDetailsDialog({
  entry,
  servers,
  onClose,
}: {
  entry: OperationHistoryEntry | null;
  servers: ServerProfile[];
  onClose: () => void;
}) {
  const serverName = entry ? serverLabel(entry.serverId, servers) : "";

  return (
    <Dialog open={entry !== null} onOpenChange={(open) => (!open ? onClose() : undefined)}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Operation details</DialogTitle>
          <DialogDescription>
            Sanitized metadata only. Passwords, keys, passphrases, tokens, environment files, raw
            output, and terminal keystrokes are never stored in history.
          </DialogDescription>
        </DialogHeader>

        {entry ? (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              <CategoryBadge category={entry.category} />
              <ResultBadge result={entry.result} />
              <span className="font-mono text-[11px] text-muted-foreground">{entry.operation}</span>
            </div>

            <p className="rounded-md border border-border bg-muted/35 px-3 py-2 text-[13px] leading-relaxed text-foreground">
              {entry.safeSummary}
            </p>

            <dl className="grid grid-cols-1 gap-x-5 gap-y-3 rounded-md border border-border px-3.5 py-3 text-[12px] sm:grid-cols-2">
              <div>
                <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                  Server
                </dt>
                <dd className="mt-0.5 text-foreground">{serverName}</dd>
                {entry.serverId ? (
                  <dd className="mt-0.5 font-mono text-[10px] text-muted-foreground">
                    {entry.serverId}
                  </dd>
                ) : null}
              </div>
              <div>
                <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                  Started
                </dt>
                <dd className="mt-0.5 text-foreground">
                  <time dateTime={timestampValue(entry.startedAt)}>
                    {formatTimestamp(entry.startedAt)}
                  </time>
                </dd>
              </div>
              <div>
                <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                  Duration
                </dt>
                <dd className="mt-0.5 font-mono text-foreground">
                  {formatDuration(entry.durationMs)}
                </dd>
              </div>
              <div>
                <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                  Exit status
                </dt>
                <dd className="mt-0.5 font-mono text-foreground">
                  {entry.exitCode === null ? "No exit status" : `Exit code ${entry.exitCode}`}
                </dd>
              </div>
              <div className="sm:col-span-2">
                <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                  Project or remote path
                </dt>
                <dd className="mt-0.5 break-all font-mono text-foreground">
                  {entry.path ?? "Not applicable"}
                </dd>
              </div>
              <div className="sm:col-span-2">
                <dt className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                  Sanitized output summary
                </dt>
                <dd className="mt-1 break-words whitespace-pre-wrap rounded-sm bg-muted/40 px-2.5 py-2 font-mono text-[11px] leading-relaxed text-foreground">
                  {entry.outputSummary ?? "No output summary was recorded."}
                </dd>
              </div>
            </dl>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function HistoryPage() {
  const servers = useServerStore((state) => state.servers);
  const serverStatus = useServerStore((state) => state.status);
  const serverError = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);

  const [entries, setEntries] = useState<OperationHistoryEntry[]>([]);
  const [loading, setLoading] = useState(() => isTauriRuntime());
  const [loadError, setLoadError] = useState<string | null>(null);
  const [policy, setPolicy] = useState<OperationHistoryPolicy>(fallbackPolicy);
  const [serverFilter, setServerFilter] = useState("all");
  const [categoryFilter, setCategoryFilter] = useState<OperationCategory | "all">("all");
  const [resultFilter, setResultFilter] = useState<OperationResult | "all">("all");
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [page, setPage] = useState(0);
  const [reloadToken, setReloadToken] = useState(0);
  const [selected, setSelected] = useState<OperationHistoryEntry | null>(null);
  const [clearOpen, setClearOpen] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);

  const runtime = isTauriRuntime();
  const filtersActive =
    serverFilter !== "all" ||
    categoryFilter !== "all" ||
    resultFilter !== "all" ||
    dateFrom.length > 0 ||
    dateTo.length > 0;

  useEffect(() => {
    if (serverStatus === "idle") {
      void fetchServers();
    }
  }, [serverStatus, fetchServers]);

  useEffect(() => {
    if (!runtime) {
      return;
    }

    let cancelled = false;

    void listOperationHistory({
      serverId: serverFilter === "all" ? null : serverFilter,
      category: categoryFilter === "all" ? null : categoryFilter,
      result: resultFilter === "all" ? null : resultFilter,
      startedAfter: startOfDaySeconds(dateFrom),
      startedBefore: endOfDaySeconds(dateTo),
      limit: policy.maxEntries,
    })
      .then((history) => {
        if (!cancelled) {
          setEntries(history);
          setLoading(false);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setLoadError(getCommandErrorMessage(error));
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    runtime,
    serverFilter,
    categoryFilter,
    resultFilter,
    dateFrom,
    dateTo,
    reloadToken,
    policy.maxEntries,
  ]);

  useEffect(() => {
    if (!runtime) {
      return;
    }
    let cancelled = false;
    void getOperationHistoryPolicy()
      .then((nextPolicy) => {
        if (!cancelled) {
          setPolicy(nextPolicy);
        }
      })
      .catch(() => {
        // The fallback policy matches the backend default and keeps the page usable.
      });
    return () => {
      cancelled = true;
    };
  }, [runtime]);

  const pageCount = Math.max(1, Math.ceil(entries.length / PAGE_SIZE));
  const currentPage = Math.min(page, pageCount - 1);
  const start = currentPage * PAGE_SIZE;
  const visible = useMemo(() => entries.slice(start, start + PAGE_SIZE), [entries, start]);

  function beginReload() {
    setLoading(true);
    setLoadError(null);
  }

  function changeServerFilter(value: string) {
    beginReload();
    setServerFilter(value);
    setPage(0);
  }

  function changeCategoryFilter(value: OperationCategory | "all") {
    beginReload();
    setCategoryFilter(value);
    setPage(0);
  }

  function changeResultFilter(value: OperationResult | "all") {
    beginReload();
    setResultFilter(value);
    setPage(0);
  }

  function changeDateFrom(value: string) {
    beginReload();
    setDateFrom(value);
    setPage(0);
  }

  function changeDateTo(value: string) {
    beginReload();
    setDateTo(value);
    setPage(0);
  }

  function clearFilters() {
    beginReload();
    setServerFilter("all");
    setCategoryFilter("all");
    setResultFilter("all");
    setDateFrom("");
    setDateTo("");
    setPage(0);
  }

  async function confirmClear() {
    setClearing(true);
    setClearError(null);
    try {
      await clearOperationHistory(true);
      setClearOpen(false);
      setSelected(null);
      setEntries([]);
      setReloadToken((token) => token + 1);
    } catch (error) {
      setClearError(getCommandErrorMessage(error));
    } finally {
      setClearing(false);
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Command History"
        description="A local audit trail of ServerDesk actions. Each entry records the server, category, safe summary, project path, timing, exit status, and a short sanitized output summary."
        actions={
          <>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                beginReload();
                setReloadToken((token) => token + 1);
              }}
              disabled={!runtime || loading}
            >
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
              Refresh
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => {
                setClearError(null);
                setClearOpen(true);
              }}
              disabled={!runtime || (entries.length === 0 && !filtersActive)}
            >
              <TrashIcon className="size-3.5" aria-hidden="true" />
              Clear history
            </Button>
          </>
        }
      />

      {!runtime ? (
        <Alert variant="info">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Web preview</AlertTitle>
            <AlertDescription>
              Operation history is stored in the local desktop database. Open the desktop build to
              review or clear recorded actions.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      <Alert variant="info">
        <ShieldCheckIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
        <div>
          <AlertTitle>Secrets are redacted before storage</AlertTitle>
          <AlertDescription>
            History never records terminal keystrokes, raw command output, environment files,
            passwords, private keys, passphrases, or tokens. {policy.retention} The stored output
            summary is limited to {policy.outputSummaryMaxChars} characters.
          </AlertDescription>
        </div>
      </Alert>

      <section
        aria-label="History filters"
        className="rounded-md border border-border bg-card px-3 py-3"
      >
        <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-5">
          <div className="min-w-0">
            <label
              htmlFor="history-server"
              className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
            >
              Server
            </label>
            <Select value={serverFilter} onValueChange={changeServerFilter}>
              <SelectTrigger id="history-server" className="mt-1">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All servers</SelectItem>
                {servers.map((server) => (
                  <SelectItem key={server.id} value={server.id}>
                    {server.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="min-w-0">
            <label
              htmlFor="history-category"
              className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
            >
              Category
            </label>
            <Select
              value={categoryFilter}
              onValueChange={(value) => changeCategoryFilter(value as OperationCategory | "all")}
            >
              <SelectTrigger id="history-category" className="mt-1">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All categories</SelectItem>
                {(Object.keys(categoryLabels) as OperationCategory[]).map((category) => (
                  <SelectItem key={category} value={category}>
                    {categoryLabels[category]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="min-w-0">
            <label
              htmlFor="history-result"
              className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
            >
              Result
            </label>
            <Select
              value={resultFilter}
              onValueChange={(value) => changeResultFilter(value as OperationResult | "all")}
            >
              <SelectTrigger id="history-result" className="mt-1">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All results</SelectItem>
                {(Object.keys(resultLabels) as OperationResult[]).map((result) => (
                  <SelectItem key={result} value={result}>
                    {resultLabels[result]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="min-w-0">
            <label
              htmlFor="history-from"
              className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
            >
              From date
            </label>
            <Input
              id="history-from"
              type="date"
              value={dateFrom}
              onChange={(event) => changeDateFrom(event.target.value)}
              className="mt-1"
            />
          </div>

          <div className="min-w-0">
            <label
              htmlFor="history-to"
              className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
            >
              To date
            </label>
            <Input
              id="history-to"
              type="date"
              value={dateTo}
              onChange={(event) => changeDateTo(event.target.value)}
              className="mt-1"
            />
          </div>
        </div>

        <div className="mt-2.5 flex flex-wrap items-center justify-between gap-2 border-t border-border pt-2.5">
          <p className="text-[11px] text-muted-foreground" aria-live="polite">
            {loading
              ? "Loading recorded operations..."
              : `${entries.length.toString()} operation${entries.length === 1 ? "" : "s"} recorded`}
          </p>
          {filtersActive ? (
            <Button type="button" variant="ghost" size="sm" onClick={clearFilters}>
              Clear filters
            </Button>
          ) : null}
        </div>
      </section>

      {clearError ? (
        <Alert variant="destructive">
          <WarningCircleIcon
            className="mt-0.5 size-4 shrink-0 text-destructive"
            aria-hidden="true"
          />
          <div className="flex-1">
            <AlertTitle>Could not clear history</AlertTitle>
            <AlertDescription>{clearError}</AlertDescription>
          </div>
          <Button variant="ghost" size="sm" onClick={() => setClearError(null)}>
            Dismiss
          </Button>
        </Alert>
      ) : null}

      {loadError ? (
        <ErrorState
          title="Could not load operation history"
          message={loadError}
          onRetry={() => setReloadToken((token) => token + 1)}
        />
      ) : null}

      {serverError && servers.length === 0 ? (
        <Alert variant="warning">
          <WarningCircleIcon className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
          <div>
            <AlertTitle>Server names could not be loaded</AlertTitle>
            <AlertDescription>
              History entries are still shown, but server names may appear as deleted servers.
            </AlertDescription>
          </div>
        </Alert>
      ) : null}

      {loading && entries.length === 0 ? <LoadingState rows={5} /> : null}

      {!loading && !loadError && entries.length === 0 && !filtersActive ? (
        <EmptyState
          icon={<ClockCounterClockwiseIcon className="size-4" aria-hidden="true" />}
          title="No operations recorded"
          description="Completed service actions, Artisan commands, SFTP operations, and fixed remote commands appear here with their server, result, timing, and sanitized summary."
          action={
            <Button asChild size="sm">
              <Link to="/servers">Open servers</Link>
            </Button>
          }
        />
      ) : null}

      {!loading && !loadError && entries.length === 0 && filtersActive ? (
        <EmptyState
          icon={<ClockCounterClockwiseIcon className="size-4" aria-hidden="true" />}
          title="No operations match these filters"
          description="Try another server, category, result, or date range. The history itself has not been changed."
          action={
            <Button type="button" variant="secondary" size="sm" onClick={clearFilters}>
              Clear filters
            </Button>
          }
        />
      ) : null}

      {entries.length > 0 && !loadError ? (
        <section
          aria-label="Recorded operations"
          className="overflow-hidden rounded-md border border-border bg-card"
        >
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Started</TableHead>
                <TableHead>Server</TableHead>
                <TableHead>Category</TableHead>
                <TableHead>Operation</TableHead>
                <TableHead>Result</TableHead>
                <TableHead className="text-right">Duration</TableHead>
                <TableHead className="text-right">Details</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.map((entry) => (
                <TableRow key={entry.id}>
                  <TableCell className="align-top whitespace-nowrap text-[12px]">
                    <time dateTime={timestampValue(entry.startedAt)}>
                      {formatTimestamp(entry.startedAt)}
                    </time>
                  </TableCell>
                  <TableCell className="align-top">
                    <div className="max-w-[18ch] truncate text-[12px]">
                      {serverLabel(entry.serverId, servers)}
                    </div>
                    {entry.serverId ? (
                      <div className="mt-0.5 max-w-[18ch] truncate font-mono text-[10px] text-muted-foreground">
                        {entry.serverId}
                      </div>
                    ) : null}
                  </TableCell>
                  <TableCell className="align-top">
                    <CategoryBadge category={entry.category} />
                  </TableCell>
                  <TableCell className="max-w-[48ch] align-top">
                    <div className="font-mono text-[11px] text-muted-foreground">
                      {entry.operation}
                    </div>
                    <div
                      className="mt-0.5 line-clamp-2 text-[12px] text-foreground"
                      title={entry.safeSummary}
                    >
                      {entry.safeSummary}
                    </div>
                    {entry.path ? (
                      <div
                        className="mt-0.5 max-w-[52ch] truncate font-mono text-[10px] text-muted-foreground"
                        title={entry.path}
                      >
                        {entry.path}
                      </div>
                    ) : null}
                  </TableCell>
                  <TableCell className="align-top">
                    <ResultBadge result={entry.result} />
                    <div className="mt-0.5 font-mono text-[10px] text-muted-foreground">
                      {entry.exitCode === null ? "no exit status" : `exit ${entry.exitCode}`}
                    </div>
                  </TableCell>
                  <TableCell className="align-top text-right font-mono text-[11px] whitespace-nowrap text-muted-foreground">
                    {formatDuration(entry.durationMs)}
                  </TableCell>
                  <TableCell className="align-top text-right">
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-[11px]"
                      onClick={() => setSelected(entry)}
                      aria-label={`View details for ${entry.operation}`}
                    >
                      View
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>

          <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border px-3 py-2 text-[10px] text-muted-foreground">
            <span>
              Showing {start + 1}-{start + visible.length} of {entries.length} operations · history
              keeps the newest {policy.maxEntries.toLocaleString()} entries
            </span>
            <nav aria-label="History pages" className="flex items-center gap-1">
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
      ) : null}

      <HistoryDetailsDialog entry={selected} servers={servers} onClose={() => setSelected(null)} />

      <ConfirmDialog
        open={clearOpen}
        onOpenChange={setClearOpen}
        title="Clear all local operation history?"
        description="This removes every recorded server action from this device. It cannot be undone. Remote servers and their data are not changed."
        confirmLabel="Clear history"
        onConfirm={() => void confirmClear()}
        pending={clearing}
        warningTitle="This only clears local history."
        warningDescription="No remote command is sent and no server data is removed."
      />
    </div>
  );
}

export { HistoryPage };
