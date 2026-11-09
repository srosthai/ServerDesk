import { useEffect, useState } from "react";
import {
  ArrowClockwiseIcon,
  CaretDoubleLeftIcon,
  CaretDoubleRightIcon,
  CaretLeftIcon,
  CaretRightIcon,
  FingerprintIcon,
  FunnelIcon,
  HardDrivesIcon,
  InfoIcon,
  LinkBreakIcon,
  MagnifyingGlassIcon,
  PencilSimpleIcon,
  PlugsIcon,
  PlusIcon,
  TrashIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
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
import { StatusBadge } from "@/components/ui/status-badge";
import { HostKeyChangedDialog, HostKeyVerificationDialog } from "@/features/servers/HostKeyDialogs";
import { KnownHostsDialog } from "@/features/servers/KnownHostsDialog";
import { ServerProfileDialog } from "@/features/servers/ServerProfileDialog";
import { describeConnectionError } from "@/lib/connectionErrors";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { useServerStore } from "@/stores/serverStore";
import type {
  AuthType,
  ConnectionSnapshot,
  HostKeyDetails,
  ServerProfile,
} from "@/types/contracts";

const authLabels: Record<AuthType, string> = {
  password: "Password",
  private_key: "Private key",
};

type HostKeyTarget = {
  server: ServerProfile;
  hostKey: HostKeyDetails;
};

function findHostKeyTarget(
  servers: ServerProfile[],
  connections: Record<string, ConnectionSnapshot>,
  predicate: (connection: ConnectionSnapshot) => boolean,
): HostKeyTarget | null {
  for (const server of servers) {
    const connection = connections[server.id];
    if (connection?.hostKey && predicate(connection)) {
      return { server, hostKey: connection.hostKey };
    }
  }

  return null;
}

function ServersPage() {
  const servers = useServerStore((state) => state.servers);
  const status = useServerStore((state) => state.status);
  const error = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const deleteServer = useServerStore((state) => state.deleteServer);
  const clearError = useServerStore((state) => state.clearError);

  const connections = useConnectionStore((state) => state.connections);
  const connectionBusy = useConnectionStore((state) => state.busy);
  const connectionError = useConnectionStore((state) => state.error);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);
  const connect = useConnectionStore((state) => state.connect);
  const disconnect = useConnectionStore((state) => state.disconnect);
  const trustHostKey = useConnectionStore((state) => state.trustHostKey);
  const rejectHostKey = useConnectionStore((state) => state.rejectHostKey);
  const clearConnectionError = useConnectionStore((state) => state.clearError);

  const [query, setQuery] = useState("");
  const [groupFilter, setGroupFilter] = useState("all");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(10);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<ServerProfile | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ServerProfile | null>(null);
  const [deletePending, setDeletePending] = useState(false);
  const [knownHostsTarget, setKnownHostsTarget] = useState<ServerProfile | null>(null);
  const [dismissedChangedKeys, setDismissedChangedKeys] = useState<Record<string, string>>({});

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

  const groups = Array.from(
    new Set(
      servers.map((server) => server.groupName).filter((group): group is string => Boolean(group)),
    ),
  ).sort((left, right) => left.localeCompare(right));

  const normalizedQuery = query.trim().toLowerCase();
  const filtered = servers.filter((server) => {
    if (groupFilter !== "all" && server.groupName !== groupFilter) {
      return false;
    }
    if (!normalizedQuery) {
      return true;
    }
    return [server.name, server.host, server.username, server.groupName ?? "", ...server.tags]
      .join(" ")
      .toLowerCase()
      .includes(normalizedQuery);
  });

  const totalPages = Math.max(1, Math.ceil(filtered.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const pageItems = filtered.slice((currentPage - 1) * pageSize, currentPage * pageSize);

  function openAddDialog() {
    setEditing(null);
    setDialogOpen(true);
  }

  function openEditDialog(server: ServerProfile) {
    setEditing(server);
    setDialogOpen(true);
  }

  function clearFilters() {
    setQuery("");
    setGroupFilter("all");
  }

  const verificationTarget = findHostKeyTarget(
    servers,
    connections,
    (connection) => connection.state === "host-key-verification",
  );
  const changedKeyTarget = findHostKeyTarget(
    servers,
    connections,
    (connection) =>
      connection.state === "error" &&
      connection.error?.code === "host_key" &&
      Boolean(connection.hostKey?.expectedFingerprint) &&
      dismissedChangedKeys[connection.serverId] !== connection.hostKey?.fingerprint,
  );

  async function handleDelete() {
    if (!deleteTarget) {
      return;
    }

    setDeletePending(true);
    const deleted = await deleteServer(deleteTarget.id);
    setDeletePending(false);

    if (deleted) {
      setDeleteTarget(null);
    }
  }

  return (
    <div className="space-y-6">
      <div className="overflow-hidden rounded-lg border border-border bg-card">
        <div className="flex flex-col gap-4 border-b border-border px-5 py-5 lg:flex-row lg:items-center lg:justify-between">
          <div className="flex items-start gap-3">
            <div className="grid size-10 shrink-0 place-items-center rounded-md bg-muted text-muted-foreground">
              <HardDrivesIcon className="size-4" aria-hidden="true" />
            </div>
            <div>
              <h1 className="text-[18px] font-semibold leading-tight tracking-[-0.02em]">
                Servers
              </h1>
              <p className="mt-0.5 text-[12px] text-muted-foreground">
                Saved VPS profiles, connection details, and grouping.
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void fetchServers()}
              disabled={status === "loading"}
            >
              <ArrowClockwiseIcon className="size-3.5" aria-hidden="true" />
              Refresh
            </Button>
            <Button size="sm" onClick={openAddDialog}>
              <PlusIcon className="size-3.5" aria-hidden="true" />
              Add server
            </Button>
          </div>
        </div>

        {!isTauriRuntime() ? (
          <Alert variant="info" className="mx-4 mt-3">
            <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
            <div>
              <AlertTitle>Web preview</AlertTitle>
              <AlertDescription>
                Profiles are saved in browser storage. The desktop build stores them in SQLite and
                keeps secrets in the OS credential store.
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        {error && servers.length > 0 ? (
          <Alert variant="destructive" className="mx-4 mt-3">
            <WarningCircleIcon
              className="mt-0.5 size-4 shrink-0 text-destructive"
              aria-hidden="true"
            />
            <div className="flex-1">
              <AlertTitle>Server profile action failed</AlertTitle>
              <AlertDescription>{error}</AlertDescription>
            </div>
            <Button variant="ghost" size="sm" onClick={clearError}>
              Dismiss
            </Button>
          </Alert>
        ) : null}

        {connectionError ? (
          <Alert variant="destructive" className="mx-4 mt-3">
            <WarningCircleIcon
              className="mt-0.5 size-4 shrink-0 text-destructive"
              aria-hidden="true"
            />
            <div className="flex-1">
              <AlertTitle>Connection action failed</AlertTitle>
              <AlertDescription>{connectionError}</AlertDescription>
            </div>
            <Button variant="ghost" size="sm" onClick={clearConnectionError}>
              Dismiss
            </Button>
          </Alert>
        ) : null}

        {servers.length > 0 ? (
          <div className="flex flex-col gap-2 border-b border-border px-4 py-3 sm:flex-row">
            <div className="relative flex-1">
              <MagnifyingGlassIcon
                className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
                aria-hidden="true"
              />
              <Input
                aria-label="Search servers"
                value={query}
                onChange={(event) => {
                  setQuery(event.currentTarget.value);
                  setPage(1);
                }}
                placeholder="Search by name, host, user, group, or tag"
                className="pl-8"
              />
            </div>
            <Select
              value={groupFilter}
              onValueChange={(value) => {
                setGroupFilter(value);
                setPage(1);
              }}
            >
              <SelectTrigger aria-label="Filter by group" className="sm:w-48">
                <FunnelIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All groups</SelectItem>
                {groups.map((group) => (
                  <SelectItem key={group} value={group}>
                    {group}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : null}

        {status === "loading" && servers.length === 0 ? (
          <div className="p-4">
            <LoadingState rows={4} />
          </div>
        ) : null}

        {status === "error" && servers.length === 0 ? (
          <div className="p-4">
            <ErrorState
              message={error ?? "Could not load server profiles."}
              onRetry={fetchServers}
            />
          </div>
        ) : null}

        {status !== "loading" && servers.length === 0 ? (
          <EmptyState
            className="m-4"
            icon={<HardDrivesIcon className="size-4" aria-hidden="true" />}
            title="No server profiles yet"
            description="Add your first server to keep connection details organized and ready for SSH."
            action={
              <Button size="sm" onClick={openAddDialog}>
                <PlusIcon className="size-3.5" aria-hidden="true" />
                Add server
              </Button>
            }
          />
        ) : null}

        {servers.length > 0 && filtered.length === 0 ? (
          <EmptyState
            className="m-4"
            title="No servers match"
            description="Clear the search or choose a different group."
            action={
              <Button variant="secondary" size="sm" onClick={clearFilters}>
                Clear filters
              </Button>
            }
          />
        ) : null}

        {filtered.length > 0 ? (
          <>
            <Table>
              <TableHeader className="bg-muted/30">
                <TableRow>
                  <TableHead className="text-[10px] uppercase tracking-[0.08em]">Server</TableHead>
                  <TableHead className="text-[10px] uppercase tracking-[0.08em]">User</TableHead>
                  <TableHead className="text-[10px] uppercase tracking-[0.08em]">Auth</TableHead>
                  <TableHead className="text-[10px] uppercase tracking-[0.08em]">Group</TableHead>
                  <TableHead className="hidden text-[10px] uppercase tracking-[0.08em] xl:table-cell">
                    Tags
                  </TableHead>
                  <TableHead className="text-[10px] uppercase tracking-[0.08em]">
                    Connection
                  </TableHead>
                  <TableHead className="w-24 text-right text-[10px] uppercase tracking-[0.08em]">
                    Actions
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {pageItems.map((server) => {
                  const connection = connections[server.id];
                  const connectionState = connection?.state ?? "disconnected";
                  const connectionError = connection?.error
                    ? describeConnectionError(connection.error)
                    : null;
                  const busy = connectionBusy[server.id] ?? false;
                  const canConnect =
                    connectionState === "disconnected" || connectionState === "error";

                  return (
                    <TableRow key={server.id}>
                      <TableCell>
                        <div className="font-medium text-foreground">{server.name}</div>
                        <div className="mt-0.5 font-mono text-[11px] text-muted-foreground">
                          {server.host}:{server.port}
                        </div>
                      </TableCell>
                      <TableCell className="font-mono text-[12px]">{server.username}</TableCell>
                      <TableCell className="text-[12px] text-muted-foreground">
                        {authLabels[server.authType]}
                      </TableCell>
                      <TableCell>
                        {server.groupName ? (
                          <Badge variant="secondary">{server.groupName}</Badge>
                        ) : (
                          <span className="text-[12px] text-muted-foreground">None</span>
                        )}
                      </TableCell>
                      <TableCell className="hidden xl:table-cell">
                        <div className="flex flex-wrap gap-1">
                          {server.tags.slice(0, 2).map((tag) => (
                            <Badge key={tag} variant="outline">
                              {tag}
                            </Badge>
                          ))}
                          {server.tags.length > 2 ? (
                            <Badge variant="outline">+{server.tags.length - 2}</Badge>
                          ) : null}
                          {server.tags.length === 0 ? (
                            <span className="text-[12px] text-muted-foreground">None</span>
                          ) : null}
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="max-w-56 space-y-1">
                          <StatusBadge state={connectionState} />
                          {connection?.error && connectionError ? (
                            <>
                              <div
                                className="truncate text-[11px] font-medium text-destructive"
                                title={connectionError.title}
                              >
                                {connectionError.title}
                              </div>
                              <div
                                className="truncate text-[11px] text-destructive/90"
                                title={connection.error.message}
                              >
                                {connection.error.message}
                              </div>
                              {connection.error.details ? (
                                <div
                                  className="truncate text-[10px] text-muted-foreground"
                                  title={connection.error.details}
                                >
                                  {connection.error.details}
                                </div>
                              ) : null}
                              <div className="truncate text-[10px] text-muted-foreground">
                                {connectionError.action}
                              </div>
                            </>
                          ) : null}
                          {connection?.hostKey ? (
                            <div
                              className="truncate font-mono text-[10px] text-muted-foreground"
                              title={`${connection.hostKey.keyType} ${connection.hostKey.fingerprint}`}
                            >
                              {connection.hostKey.fingerprint}
                            </div>
                          ) : null}
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex justify-end gap-1">
                          {isTauriRuntime() ? (
                            canConnect ? (
                              <Button
                                variant="secondary"
                                size="icon-sm"
                                onClick={() => void connect(server.id)}
                                disabled={busy}
                                aria-label={`Connect ${server.name}`}
                                title="Connect"
                              >
                                <PlugsIcon className="size-3.5" aria-hidden="true" />
                              </Button>
                            ) : (
                              <Button
                                variant="ghost"
                                size="icon-sm"
                                onClick={() => void disconnect(server.id)}
                                disabled={busy}
                                aria-label={`Disconnect ${server.name}`}
                                title={
                                  connectionState === "connected"
                                    ? "Disconnect"
                                    : "Cancel connection"
                                }
                              >
                                <LinkBreakIcon className="size-3.5" aria-hidden="true" />
                              </Button>
                            )
                          ) : null}
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => setKnownHostsTarget(server)}
                            aria-label={`Manage trusted host keys for ${server.name}`}
                            title="Trusted host keys"
                          >
                            <FingerprintIcon className="size-3.5" aria-hidden="true" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => openEditDialog(server)}
                            aria-label={`Edit ${server.name}`}
                            title="Edit server"
                          >
                            <PencilSimpleIcon className="size-3.5" aria-hidden="true" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => setDeleteTarget(server)}
                            aria-label={`Delete ${server.name}`}
                            title="Delete server"
                          >
                            <TrashIcon className="size-3.5" aria-hidden="true" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>

            <div className="flex flex-col gap-3 border-t border-border px-4 py-3 text-[11px] text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
              <div className="flex items-center gap-2">
                <span>Per page</span>
                <Select
                  value={String(pageSize)}
                  onValueChange={(value) => {
                    setPageSize(Number(value));
                    setPage(1);
                  }}
                >
                  <SelectTrigger aria-label="Rows per page" className="h-8 w-20">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="10">10</SelectItem>
                    <SelectItem value="25">25</SelectItem>
                    <SelectItem value="50">50</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="mr-1">
                  Page {currentPage} of {totalPages}
                </span>
                <Button
                  variant="secondary"
                  size="icon-sm"
                  onClick={() => setPage(1)}
                  disabled={currentPage <= 1}
                  aria-label="First page"
                >
                  <CaretDoubleLeftIcon className="size-3.5" aria-hidden="true" />
                </Button>
                <Button
                  variant="secondary"
                  size="icon-sm"
                  onClick={() => setPage(currentPage - 1)}
                  disabled={currentPage <= 1}
                  aria-label="Previous page"
                >
                  <CaretLeftIcon className="size-3.5" aria-hidden="true" />
                </Button>
                <Button
                  variant="secondary"
                  size="icon-sm"
                  onClick={() => setPage(currentPage + 1)}
                  disabled={currentPage >= totalPages}
                  aria-label="Next page"
                >
                  <CaretRightIcon className="size-3.5" aria-hidden="true" />
                </Button>
                <Button
                  variant="secondary"
                  size="icon-sm"
                  onClick={() => setPage(totalPages)}
                  disabled={currentPage >= totalPages}
                  aria-label="Last page"
                >
                  <CaretDoubleRightIcon className="size-3.5" aria-hidden="true" />
                </Button>
              </div>
            </div>
          </>
        ) : null}
      </div>

      {dialogOpen ? (
        <ServerProfileDialog
          key={editing?.id ?? "new"}
          server={editing}
          onOpenChange={setDialogOpen}
        />
      ) : null}

      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deletePending) {
            setDeleteTarget(null);
          }
        }}
        title={`Delete ${deleteTarget?.name ?? "server"}?`}
        description="The profile and its local metadata will be removed. This does not change the remote server."
        confirmLabel="Delete server"
        details={
          deleteTarget
            ? [
                { label: "Server", value: deleteTarget.name },
                { label: "Host", value: `${deleteTarget.host}:${deleteTarget.port}` },
                { label: "User", value: deleteTarget.username },
              ]
            : undefined
        }
        onConfirm={() => void handleDelete()}
        pending={deletePending}
      />

      {verificationTarget ? (
        <HostKeyVerificationDialog
          serverName={verificationTarget.server.name}
          host={`${verificationTarget.server.host}:${verificationTarget.server.port}`}
          hostKey={verificationTarget.hostKey}
          pending={connectionBusy[verificationTarget.server.id] ?? false}
          onTrust={() =>
            void trustHostKey(
              verificationTarget.server.id,
              verificationTarget.hostKey.keyType,
              verificationTarget.hostKey.fingerprint,
            )
          }
          onReject={() =>
            void rejectHostKey(
              verificationTarget.server.id,
              verificationTarget.hostKey.keyType,
              verificationTarget.hostKey.fingerprint,
            )
          }
        />
      ) : null}

      {!verificationTarget && changedKeyTarget ? (
        <HostKeyChangedDialog
          serverName={changedKeyTarget.server.name}
          host={`${changedKeyTarget.server.host}:${changedKeyTarget.server.port}`}
          hostKey={changedKeyTarget.hostKey}
          onDismiss={() => {
            setDismissedChangedKeys((current) => ({
              ...current,
              [changedKeyTarget.server.id]: changedKeyTarget.hostKey.fingerprint,
            }));
          }}
          onManageKeys={() => {
            setDismissedChangedKeys((current) => ({
              ...current,
              [changedKeyTarget.server.id]: changedKeyTarget.hostKey.fingerprint,
            }));
            setKnownHostsTarget(changedKeyTarget.server);
          }}
        />
      ) : null}

      {knownHostsTarget ? (
        <KnownHostsDialog
          key={knownHostsTarget.id}
          server={knownHostsTarget}
          onOpenChange={(open) => {
            if (!open) {
              setKnownHostsTarget(null);
            }
          }}
        />
      ) : null}
    </div>
  );
}

export { ServersPage };
