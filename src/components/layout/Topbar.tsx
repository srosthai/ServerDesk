import { useEffect } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { ArrowCircleUpIcon, ListIcon, SidebarSimpleIcon } from "@phosphor-icons/react";
import { allNavItems, navGroups } from "@/app/navigation";
import { StatusBadge } from "@/components/ui/status-badge";
import { useActiveServer } from "@/hooks/useActiveServer";
import { formatVersionLabel } from "@/lib/updates";
import { useUpdateStore } from "@/stores/updateStore";

type TopbarProps = {
  sidebarCollapsed: boolean;
  onOpenNavigation: () => void;
  onToggleSidebar: () => void;
};

function Topbar({ sidebarCollapsed, onOpenNavigation, onToggleSidebar }: TopbarProps) {
  const location = useLocation();
  const navigate = useNavigate();
  const updateStatus = useUpdateStore((state) => state.status);
  const availableVersion = useUpdateStore((state) =>
    state.status === "available" && state.available ? state.available.version : null,
  );
  const current = allNavItems.find(
    (item) =>
      location.pathname === item.to || (!item.end && location.pathname.startsWith(`${item.to}/`)),
  );
  const currentGroup = navGroups.find((group) =>
    group.items.some((item) => item.to === current?.to),
  );
  const { activeServer, connection } = useActiveServer();
  const connectionState = connection?.state ?? "disconnected";

  useEffect(() => {
    document.title = current ? `${current.label} · ServerDesk` : "ServerDesk";
  }, [current]);

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center justify-between gap-3 border-b border-border bg-background/90 px-4 backdrop-blur-sm md:px-6">
      <div className="flex min-w-0 items-center gap-3">
        <button
          type="button"
          onClick={onOpenNavigation}
          className="rounded-md p-1.5 text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40 md:hidden"
          aria-label="Open navigation"
        >
          <ListIcon className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          onClick={onToggleSidebar}
          aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          aria-pressed={sidebarCollapsed}
          className="hidden rounded-md p-1.5 text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40 md:inline-flex"
        >
          <SidebarSimpleIcon className="size-4" aria-hidden="true" />
        </button>
        <div className="min-w-0">
          <div className="truncate text-[12px] font-medium text-foreground">
            {current?.label ?? "ServerDesk"}
          </div>
          <div className="truncate text-[10px] text-muted-foreground">
            {currentGroup?.label ?? "Desktop workspace"}
          </div>
        </div>
      </div>

      <div className="flex min-w-0 shrink-0 items-center gap-2">
        {availableVersion ? (
          <button
            type="button"
            onClick={() => navigate("/settings")}
            className="inline-flex h-7 items-center gap-1.5 rounded-sm border border-primary/30 bg-primary/10 px-2 text-[11px] font-medium text-primary outline-none transition-colors hover:bg-primary/15 focus-visible:ring-2 focus-visible:ring-ring/40"
            aria-label={`Update ${formatVersionLabel(availableVersion)} available. Open Settings to install it.`}
            aria-live={updateStatus === "available" ? "polite" : undefined}
          >
            <ArrowCircleUpIcon className="size-3.5" aria-hidden="true" />
            <span className="hidden sm:inline">
              {formatVersionLabel(availableVersion)} available
            </span>
            <span className="sm:hidden">Update</span>
          </button>
        ) : null}
        {activeServer ? (
          <span className="hidden max-w-40 truncate text-[11px] font-medium text-muted-foreground sm:inline">
            {activeServer.name}
          </span>
        ) : null}
        <StatusBadge
          state={activeServer ? connectionState : "disconnected"}
          label={activeServer ? undefined : "No server"}
        />
      </div>
    </header>
  );
}

export { Topbar };
