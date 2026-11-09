import { useEffect, useState } from "react";
import { Outlet, useLocation } from "react-router-dom";
import { Sidebar } from "@/components/layout/Sidebar";
import { Topbar } from "@/components/layout/Topbar";
import { useConnectionEvents } from "@/hooks/useConnectionEvents";
import { useTerminalEvents } from "@/hooks/useTerminalEvents";
import { useServerStore } from "@/stores/serverStore";
import { cn } from "@/lib/utils";

function AppShell() {
  const [navigationOpen, setNavigationOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(
    () => localStorage.getItem("serverdesk-sidebar-collapsed") === "true",
  );
  const location = useLocation();
  const serverStatus = useServerStore((state) => state.status);
  const fetchServers = useServerStore((state) => state.fetchServers);

  useConnectionEvents();
  useTerminalEvents();

  useEffect(() => {
    window.scrollTo({ top: 0 });
  }, [location.pathname]);

  useEffect(() => {
    if (serverStatus === "idle") {
      void fetchServers();
    }
  }, [serverStatus, fetchServers]);

  function toggleSidebar() {
    setSidebarCollapsed((collapsed) => {
      const next = !collapsed;
      localStorage.setItem("serverdesk-sidebar-collapsed", String(next));
      return next;
    });
  }

  return (
    <div className="min-h-dvh bg-background text-foreground">
      <a
        href="#main-content"
        className="sr-only z-[60] rounded-md bg-primary px-3 py-2 text-[13px] font-medium text-primary-foreground focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
      >
        Skip to content
      </a>

      <Sidebar
        open={navigationOpen}
        collapsed={sidebarCollapsed}
        onClose={() => setNavigationOpen(false)}
      />

      {navigationOpen ? (
        <button
          type="button"
          aria-label="Close navigation"
          className="fixed inset-0 z-40 bg-black/60 backdrop-blur-[1px] md:hidden"
          onClick={() => setNavigationOpen(false)}
        />
      ) : null}

      <div
        className={cn(
          "min-h-dvh transition-[padding] duration-200",
          sidebarCollapsed ? "md:pl-16" : "md:pl-60",
        )}
      >
        <Topbar
          sidebarCollapsed={sidebarCollapsed}
          onOpenNavigation={() => setNavigationOpen(true)}
          onToggleSidebar={toggleSidebar}
        />
        <main id="main-content" className="mx-auto w-full max-w-[1440px] px-5 py-6 md:px-8 md:py-7">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

export { AppShell };
