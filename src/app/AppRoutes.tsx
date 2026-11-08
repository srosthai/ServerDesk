import { Link, Route, Routes } from "react-router-dom";
import { AppShell } from "@/app/AppShell";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { OverviewPage } from "@/features/dashboard/OverviewPage";
import { FileManagerPage } from "@/features/files/FileManagerPage";
import { HistoryPage } from "@/features/history/HistoryPage";
import { LogsPage } from "@/features/logs/LogsPage";
import { MonitoringPage } from "@/features/monitoring/MonitoringPage";
import { ProjectsPage } from "@/features/projects/ProjectsPage";
import { SecurityPage } from "@/features/security/SecurityPage";
import { ServersPage } from "@/features/servers/ServersPage";
import { ServicesPage } from "@/features/services/ServicesPage";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { StyleGuidePage } from "@/features/style-guide/StyleGuidePage";
import { TerminalPage } from "@/features/terminal/TerminalPage";

function NotFoundPage() {
  return (
    <div className="space-y-6">
      <PageHeader title="Page not found" description="This route does not exist." />
      <EmptyState
        title="Nothing here"
        description="The page may have moved, or the link may be incomplete."
        action={
          <Button asChild size="sm">
            <Link to="/">Back to overview</Link>
          </Button>
        }
      />
    </div>
  );
}

function AppRoutes() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<OverviewPage />} />
        <Route path="servers" element={<ServersPage />} />
        <Route path="projects" element={<ProjectsPage />} />
        <Route path="monitoring" element={<MonitoringPage />} />
        <Route path="services" element={<ServicesPage />} />
        <Route path="files" element={<FileManagerPage />} />
        <Route path="terminal" element={<TerminalPage />} />
        <Route path="logs" element={<LogsPage />} />
        <Route path="history" element={<HistoryPage />} />
        <Route path="security" element={<SecurityPage />} />
        <Route path="settings" element={<SettingsPage />} />
        {import.meta.env.DEV ? <Route path="design" element={<StyleGuidePage />} /> : null}
        <Route path="*" element={<NotFoundPage />} />
      </Route>
    </Routes>
  );
}

export { AppRoutes };
