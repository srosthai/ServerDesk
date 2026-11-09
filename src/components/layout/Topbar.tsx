import { useEffect } from "react";
import { useLocation } from "react-router-dom";
import { ListIcon, SidebarSimpleIcon } from "@phosphor-icons/react";
import { allNavItems, navGroups } from "@/app/navigation";
import { StatusBadge } from "@/components/ui/status-badge";
import { useActiveServer } from "@/hooks/useActiveServer";

type TopbarProps = {
  sidebarCollapsed: boolean;
  onOpenNavigation: () => void;
  onToggleSidebar: () => void;
};

function Topbar({ sidebarCollapsed, onOpenNavigation, onToggleSidebar }: TopbarProps) {
  const location = useLocation();
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
