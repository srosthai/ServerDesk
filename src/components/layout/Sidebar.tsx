import { NavLink } from "react-router-dom";
import { CaretUpDownIcon, HardDrivesIcon, XIcon } from "@phosphor-icons/react";
import { navGroups } from "@/app/navigation";
import { useAppInfo } from "@/hooks/useAppInfo";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { cn } from "@/lib/utils";

type SidebarProps = {
  open: boolean;
  onClose: () => void;
};

function Sidebar({ open, onClose }: SidebarProps) {
  const isDesktop = useMediaQuery("(min-width: 768px)");
  const appInfo = useAppInfo();
  const version = appInfo.status === "ready" ? `v${appInfo.info.appVersion}` : "v0.1.0";
  const storageLabel =
    appInfo.status === "ready"
      ? `Database v${appInfo.info.databaseSchemaVersion}`
      : appInfo.status === "error"
        ? "Database error"
        : appInfo.status === "unavailable"
          ? "Web preview"
          : "Checking database";
  const storageTitle =
    appInfo.status === "error" ? appInfo.message : "Local SQLite storage is initialized on launch.";

  return (
    <aside
      aria-label="Primary"
      inert={!isDesktop && !open}
      className={cn(
        "fixed inset-y-0 left-0 z-50 flex w-60 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground transition-transform duration-200 ease-out md:translate-x-0",
        open ? "translate-x-0" : "-translate-x-full",
      )}
    >
      <div className="flex h-14 items-center justify-between border-b border-sidebar-border px-4">
        <NavLink
          to="/"
          onClick={onClose}
          className="flex min-w-0 items-center gap-2.5 rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-sidebar-accent/60"
        >
          <span className="grid size-7 shrink-0 place-items-center rounded-sm bg-sidebar-foreground text-[12px] font-bold text-sidebar">
            S
          </span>
          <span className="truncate text-[14px] font-semibold tracking-[-0.015em]">ServerDesk</span>
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

      <div className="px-3 py-3">
        <NavLink
          to="/servers"
          onClick={onClose}
          className="group flex w-full items-center gap-2.5 rounded-md border border-sidebar-border bg-sidebar-active/45 px-2.5 py-2 text-left outline-none transition-colors hover:bg-sidebar-active focus-visible:ring-2 focus-visible:ring-sidebar-accent/60"
        >
          <span className="grid size-7 shrink-0 place-items-center rounded-sm bg-sidebar-active text-sidebar-muted">
            <HardDrivesIcon className="size-3.5" aria-hidden="true" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-[10px] leading-4 text-sidebar-muted">Active server</span>
            <span className="block truncate text-[12px] font-medium leading-4">None selected</span>
          </span>
          <CaretUpDownIcon
            className="size-3.5 shrink-0 text-sidebar-muted transition-colors group-hover:text-sidebar-foreground"
            aria-hidden="true"
          />
        </NavLink>
      </div>

      <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">
        {navGroups.map((group) => (
          <div key={group.label} className="mb-4 last:mb-0">
            <div className="px-2 pb-1.5 text-[10px] font-medium leading-4 text-sidebar-muted">
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
                      className={({ isActive }) =>
                        cn(
                          "group relative flex h-8 items-center gap-2.5 rounded-sm px-2 text-[12px] font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-sidebar-accent/60",
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
                          <span className="truncate">{item.label}</span>
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

      <div className="border-t border-sidebar-border px-4 py-3">
        <div className="flex items-center gap-2 text-[11px] text-sidebar-muted">
          <span className="size-1.5 rounded-full bg-sidebar-muted/60" aria-hidden="true" />
          No server selected
        </div>
        <div className="mt-2 flex items-center justify-between gap-2 font-mono text-[10px] text-sidebar-muted/80">
          <span>{version}</span>
          <span
            className={appInfo.status === "error" ? "text-destructive" : undefined}
            title={storageTitle}
          >
            {storageLabel}
          </span>
        </div>
      </div>
    </aside>
  );
}

export { Sidebar };
