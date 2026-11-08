import { useEffect, useState } from "react";
import {
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
import { StatusBadge } from "@/components/ui/status-badge";
import { ServerProfileDialog } from "@/features/servers/ServerProfileDialog";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { useServerStore } from "@/stores/serverStore";
import type { AuthType, ServerProfile } from "@/types/contracts";

const authLabels: Record<AuthType, string> = {
  password: "Password",
  private_key: "Private key",
};

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
  const clearConnectionError = useConnectionStore((state) => state.clearError);

  const [query, setQuery] = useState("");
  const [groupFilter, setGroupFilter] = useState("all");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<ServerProfile | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ServerProfile | null>(null);
  const [deletePending, setDeletePending] = useState(false);

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
      <PageHeader
        title="Servers"
        description="Saved VPS profiles, connection details, and grouping."
        actions={
          <Button size="sm" onClick={openAddDialog}>
            <PlusIcon className="size-3.5" aria-hidden="true" />
            Add server
          </Button>
        }
      />

      {!isTauriRuntime() ? (
        <Alert variant="info">
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
        <Alert variant="destructive">
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
        <Alert variant="destructive">
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
        <div className="flex flex-col gap-2 sm:flex-row">
          <div className="relative flex-1">
            <MagnifyingGlassIcon
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <Input
              aria-label="Search servers"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Search by name, host, user, group, or tag"
              className="pl-8"
            />
          </div>
          <Select value={groupFilter} onValueChange={setGroupFilter}>
            <SelectTrigger aria-label="Filter by group" className="sm:w-52">
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

      {status === "loading" && servers.length === 0 ? <LoadingState rows={4} /> : null}

      {status === "error" && servers.length === 0 ? (
        <ErrorState message={error ?? "Could not load server profiles."} onRetry={fetchServers} />
      ) : null}

      {status !== "loading" && servers.length === 0 ? (
        <EmptyState
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
        <div className="rounded-md border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Server</TableHead>
                <TableHead>User</TableHead>
                <TableHead>Auth</TableHead>
                <TableHead>Group</TableHead>
                <TableHead className="hidden xl:table-cell">Tags</TableHead>
                <TableHead>Connection</TableHead>
                <TableHead className="w-24 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.map((server) => {
                const connection = connections[server.id];
                const connectionState = connection?.state ?? "disconnected";
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
                        {connection?.error ? (
                          <div
                            className="truncate text-[11px] text-destructive"
                            title={connection.error.message}
                          >
                            {connection.error.message}
                          </div>
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
                                connectionState === "connected" ? "Disconnect" : "Cancel connection"
                              }
                            >
                              <LinkBreakIcon className="size-3.5" aria-hidden="true" />
                            </Button>
                          )
                        ) : null}
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
        </div>
      ) : null}

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
    </div>
  );
}

export { ServersPage };
