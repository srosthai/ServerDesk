import { NavLink } from "react-router-dom";
import { CaretUpDownIcon, HardDrivesIcon, XIcon } from "@phosphor-icons/react";
import serverdeskMark from "@/assets/brand/serverdesk-mark.png";
import { navGroups } from "@/app/navigation";
import { useActiveServer } from "@/hooks/useActiveServer";
import { useAppInfo } from "@/hooks/useAppInfo";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { cn } from "@/lib/utils";

type SidebarProps = {
  open: boolean;
  collapsed: boolean;
  onClose: () => void;
};

function Sidebar({ open, collapsed, onClose }: SidebarProps) {
  const isDesktop = useMediaQuery("(min-width: 768px)");
  const appInfo = useAppInfo();
  const version = appInfo.status === "ready" ? `v${appInfo.info.appVersion}` : "v0.1.0";
  const { activeServer, connection } = useActiveServer();
  const connectionState = connection?.state ?? "disconnected";
  const activeServerName = activeServer?.name ?? "None selected";
  const connectionLabel =
    connectionState === "host-key-verification"
      ? "Verify host key"
      : connectionState === "connected"
        ? "Connected"
        : connectionState === "connecting"
          ? "Connecting"
          : connectionState === "reconnecting"
            ? "Reconnecting"
            : connectionState === "error"
              ? "Connection error"
              : "Disconnected";
  const connectionDot =
    connectionState === "connected"
      ? "bg-success"
      : connectionState === "error"
        ? "bg-destructive"
        : connectionState === "disconnected"
          ? "bg-sidebar-muted/60"
          : "bg-warning";

  return (
    <aside
      aria-label="Primary"
      inert={!isDesktop && !open}
      className={cn(
        "fixed inset-y-0 left-0 z-50 flex w-60 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground transition-[width,transform] duration-200 ease-out md:translate-x-0",
        collapsed ? "md:w-16" : "md:w-60",
        open ? "translate-x-0" : "-translate-x-full",
      )}
    >
      <div
        className={cn(
          "flex h-14 items-center justify-between border-b border-sidebar-border px-4",
          collapsed && "md:justify-center md:px-0",
        )}
      >
        <NavLink
          to="/"
          onClick={onClose}
          className={cn(
            "flex min-w-0 items-center gap-2.5 rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-sidebar-accent/60",
            collapsed && "md:justify-center",
          )}
        >
          <img
            src={serverdeskMark}
            alt=""
            className="size-8 shrink-0 rounded-md object-cover ring-1 ring-sidebar-border"
          />
          <span
            className={cn(
              "truncate text-[14px] font-semibold tracking-[-0.015em]",
              collapsed && "md:hidden",
            )}
          >
            ServerDesk
          </span>
        </NavLink>
        <button
          type="button"
          onClick={onClose}
          className="rounded-sm p-1.5 text-sidebar-muted outline-none hover:bg-sidebar-active hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-sidebar-accent/60 md:hidden"
          aria-label="Close navigation"
        >
          <XIcon className="size-4" aria-hidden="true" />
        </button>
      </div>

      <div className={cn("px-3 py-3", collapsed && "md:px-2")}>
        <NavLink
          to="/servers"
          onClick={onClose}
          aria-label={
            collapsed ? `Active server: ${activeServerName}, ${connectionLabel}` : undefined
          }
          title={collapsed ? `Active server: ${activeServerName}, ${connectionLabel}` : undefined}
          className={cn(
            "group flex w-full items-center gap-2.5 rounded-md border border-sidebar-border bg-sidebar-active/45 px-2.5 py-2 text-left outline-none transition-colors hover:bg-sidebar-active focus-visible:ring-2 focus-visible:ring-sidebar-accent/60",
            collapsed && "md:justify-center md:px-0",
          )}
        >
          <span className="grid size-7 shrink-0 place-items-center rounded-sm bg-sidebar-active text-sidebar-muted">
            <HardDrivesIcon className="size-3.5" aria-hidden="true" />
          </span>
          <span className={cn("min-w-0 flex-1", collapsed && "md:hidden")}>
            <span className="flex items-center gap-1.5 text-[10px] leading-4 text-sidebar-muted">
              <span className={cn("size-1.5 rounded-full", connectionDot)} aria-hidden="true" />
              Active server
            </span>
            <span className="block truncate text-[12px] font-medium leading-4">
              {activeServerName}
            </span>
          </span>
          <CaretUpDownIcon
            className={cn(
              "size-3.5 shrink-0 text-sidebar-muted transition-colors group-hover:text-sidebar-foreground",
              collapsed && "md:hidden",
            )}
            aria-hidden="true"
          />
        </NavLink>
      </div>

      <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">
        {navGroups.map((group) => (
          <div key={group.label} className="mb-4 last:mb-0">
            <div
              className={cn(
                "px-2 pb-1.5 text-[10px] font-medium leading-4 text-sidebar-muted",
                collapsed && "md:hidden",
              )}
            >
              {group.label}
            </div>
            <ul className="space-y-0.5">
              {group.items.map((item) => {
                const Icon = item.icon;

                return (
                  <li key={item.to}>
                    <NavLink
                      to={item.to}
                      end={item.end}
                      onClick={onClose}
                      title={collapsed ? item.label : undefined}
                      className={({ isActive }) =>
                        cn(
                          "group relative flex h-8 items-center gap-2.5 rounded-sm px-2 text-[12px] font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-sidebar-accent/60",
                          collapsed && "md:justify-center md:px-0",
                          isActive
                            ? "bg-sidebar-active text-sidebar-foreground"
                            : "text-sidebar-muted hover:bg-sidebar-active/60 hover:text-sidebar-foreground",
                        )
                      }
                    >
                      {({ isActive }) => (
                        <>
                          <span
                            aria-hidden="true"
                            className={cn(
                              "absolute inset-y-1 left-0 w-0.5 rounded-full bg-sidebar-accent transition-opacity",
                              isActive ? "opacity-100" : "opacity-0",
                            )}
                          />
                          <Icon
                            className={cn(
                              "size-4 shrink-0",
                              isActive ? "text-sidebar-accent" : "text-sidebar-muted",
                            )}
                            aria-hidden="true"
                          />
                          <span className={cn("truncate", collapsed && "md:sr-only")}>
                            {item.label}
                          </span>
                        </>
                      )}
                    </NavLink>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </nav>

      <div className={cn("border-t border-sidebar-border px-4 py-3", collapsed && "md:px-2")}>
        <div
          className={cn(
            "flex items-center gap-2 text-[11px] text-sidebar-muted",
            collapsed && "md:justify-center",
          )}
          title={collapsed ? `${activeServerName}, ${connectionLabel}` : undefined}
        >
          <span className={cn("size-1.5 rounded-full", connectionDot)} aria-hidden="true" />
          <span className={cn("truncate", collapsed && "md:hidden")}>
            {activeServer ? activeServerName : "No server selected"}
          </span>
        </div>
        <div
          className={cn(
            "mt-2 font-mono text-[10px] text-sidebar-muted/80",
            collapsed && "md:hidden",
          )}
        >
          ServerDesk {version}
        </div>
      </div>
    </aside>
  );
}

export { Sidebar };
