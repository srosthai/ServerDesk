import { useEffect, useState } from "react";
import { Outlet, useLocation } from "react-router-dom";
import { Sidebar } from "@/components/layout/Sidebar";
import { Topbar } from "@/components/layout/Topbar";
import { useConnectionEvents } from "@/hooks/useConnectionEvents";
import { useTerminalEvents } from "@/hooks/useTerminalEvents";

function AppShell() {
  const [navigationOpen, setNavigationOpen] = useState(false);
  const location = useLocation();

  useConnectionEvents();
  useTerminalEvents();

  useEffect(() => {
    window.scrollTo({ top: 0 });
  }, [location.pathname]);

  return (
    <div className="min-h-dvh bg-background text-foreground">
      <a
        href="#main-content"
        className="sr-only z-[60] rounded-md bg-primary px-3 py-2 text-[13px] font-medium text-primary-foreground focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
      >
        Skip to content
      </a>

      <Sidebar open={navigationOpen} onClose={() => setNavigationOpen(false)} />

      {navigationOpen ? (
        <button
          type="button"
          aria-label="Close navigation"
          className="fixed inset-0 z-40 bg-[#07100c]/50 backdrop-blur-[1px] md:hidden"
          onClick={() => setNavigationOpen(false)}
        />
      ) : null}

      <div className="min-h-dvh md:pl-60">
        <Topbar onOpenNavigation={() => setNavigationOpen(true)} />
        <main id="main-content" className="mx-auto w-full max-w-[1440px] px-5 py-6 md:px-8 md:py-7">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

export { AppShell };
