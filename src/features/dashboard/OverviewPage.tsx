import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  HardDrivesIcon,
  InfoIcon,
  PlusIcon,
  SquaresFourIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { ErrorState, LoadingState } from "@/components/ui/states";
import { ServerInspectionPanel } from "@/features/dashboard/ServerInspectionPanel";
import { ServerProfileDialog } from "@/features/servers/ServerProfileDialog";
import { isTauriRuntime } from "@/lib/tauri";
import { useConnectionStore } from "@/stores/connectionStore";
import { useServerStore } from "@/stores/serverStore";
import { cn } from "@/lib/utils";

function OverviewPage() {
  const servers = useServerStore((state) => state.servers);
  const status = useServerStore((state) => state.status);
  const error = useServerStore((state) => state.error);
  const fetchServers = useServerStore((state) => state.fetchServers);
  const refreshConnectionStates = useConnectionStore((state) => state.refreshStates);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

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

  const selected = servers.find((server) => server.id === selectedId) ?? servers[0] ?? null;

  return (
    <div className="space-y-6">
      <PageHeader
        title="Overview"
        description="Server health at a glance: connection state, identity, resource metrics, and sample freshness."
        actions={
          <>
            <Button asChild variant="secondary" size="sm">
              <Link to="/servers">Manage servers</Link>
            </Button>
            <Button size="sm" onClick={() => setDialogOpen(true)}>
              <PlusIcon className="size-3.5" aria-hidden="true" />
              Add server
            </Button>
          </>
        }
      />

      {!isTauriRuntime() ? (
        <Alert variant="info">
          <InfoIcon className="mt-0.5 size-4 shrink-0 text-info" aria-hidden="true" />
          <div>
            <AlertTitle>Web preview</AlertTitle>
            <AlertDescription>
              Server profiles are saved in browser storage here. Open the desktop build to verify
              host keys, authenticate, and run remote commands over SSH.
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
          icon={<SquaresFourIcon className="size-4" aria-hidden="true" />}
          title="No server profiles yet"
          description="Add a server profile, then connect, verify its host key, authenticate, inspect its operating system identity, and run hostname and uptime."
          action={
            <Button size="sm" onClick={() => setDialogOpen(true)}>
              <PlusIcon className="size-3.5" aria-hidden="true" />
              Add server
            </Button>
          }
        />
      ) : null}

      {servers.length > 0 ? (
        <div className="grid gap-4 xl:grid-cols-[250px_minmax(0,1fr)]">
          <aside className="h-fit rounded-md border border-border bg-card p-2">
            <div className="flex items-center gap-1.5 px-2 py-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
              <HardDrivesIcon className="size-3.5" aria-hidden="true" />
              Servers
            </div>
            <div className="space-y-1">
              {servers.map((server) => {
                const active = server.id === selected?.id;
                return (
                  <button
                    key={server.id}
                    type="button"
                    onClick={() => setSelectedId(server.id)}
                    aria-pressed={active}
                    className={cn(
                      "w-full rounded-sm border px-2.5 py-2 text-left transition-colors",
                      active
                        ? "border-border bg-muted text-foreground"
                        : "border-transparent text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                    )}
                  >
                    <span className="block truncate text-[12px] font-medium">{server.name}</span>
                    <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground">
                      {server.host}:{server.port}
                    </span>
                  </button>
                );
              })}
            </div>
          </aside>

          {selected ? <ServerInspectionPanel key={selected.id} server={selected} /> : null}
        </div>
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

      {dialogOpen ? <ServerProfileDialog server={null} onOpenChange={setDialogOpen} /> : null}
    </div>
  );
}

export { OverviewPage };
