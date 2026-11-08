import * as React from "react";
import { useState } from "react";
import { CheckCircle, HardDrives, Info, Moon, Sun, WarningCircle } from "@phosphor-icons/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { OutputPanel } from "@/components/ui/output-panel";
import { PageHeader } from "@/components/ui/page-header";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { StatusBadge, type ConnectionState } from "@/components/ui/status-badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

const palette = [
  { name: "Canvas", token: "--background", className: "bg-background" },
  { name: "Surface", token: "--card", className: "bg-card" },
  { name: "Ink", token: "--foreground", className: "bg-foreground" },
  { name: "Muted", token: "--muted-foreground", className: "bg-muted-foreground" },
  { name: "Signal", token: "--primary", className: "bg-primary" },
  { name: "Sidebar", token: "--sidebar", className: "bg-sidebar" },
];

const serviceRows = [
  { name: "nginx.service", state: "Active", tone: "success" as const },
  { name: "php8.4-fpm.service", state: "Active", tone: "success" as const },
  { name: "postgresql.service", state: "Failed", tone: "danger" as const },
  { name: "redis.service", state: "Inactive", tone: "secondary" as const },
];

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="grid gap-5 border-t border-border pt-7 lg:grid-cols-[180px_minmax(0,1fr)] lg:gap-10">
      <div>
        <h2 className="text-[14px] font-semibold tracking-[-0.01em]">{title}</h2>
        <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{description}</p>
      </div>
      <div className="min-w-0">{children}</div>
    </section>
  );
}

function StyleGuidePage() {
  const [dark, setDark] = useState(() => document.documentElement.classList.contains("dark"));
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [service, setService] = useState("nginx.service");

  function toggleTheme() {
    const next = !dark;
    setDark(next);
    document.documentElement.classList.toggle("dark", next);
    localStorage.setItem("serverdesk-theme", next ? "dark" : "light");
  }

  return (
    <TooltipProvider>
      <main className="min-h-dvh bg-background px-6 py-8 text-foreground lg:px-10">
        <div className="mx-auto max-w-[1120px] space-y-8">
          <PageHeader
            title="ServerDesk design system"
            description="The shared visual language for the desktop workspace. Tokens and primitives are composed from the same source used by feature screens."
            actions={
              <Button variant="secondary" size="sm" onClick={toggleTheme}>
                {dark ? (
                  <Sun className="size-4" aria-hidden="true" />
                ) : (
                  <Moon className="size-4" aria-hidden="true" />
                )}
                {dark ? "Light theme" : "Dark theme"}
              </Button>
            }
          />

          <Section
            title="Color"
            description="Cool surfaces, near-black ink, and one signal accent. Status colors carry meaning only."
          >
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              {palette.map((color) => (
                <div key={color.name} className="space-y-2">
                  <div
                    className={cn(
                      "h-16 rounded-md border border-border",
                      color.className,
                      color.name === "Ink" && "border-foreground/20",
                    )}
                  />
                  <div>
                    <div className="text-[12px] font-medium">{color.name}</div>
                    <div className="font-mono text-[10px] text-muted-foreground">{color.token}</div>
                  </div>
                </div>
              ))}
            </div>
          </Section>

          <Section
            title="Typography"
            description="IBM Plex Sans for interface text. IBM Plex Mono for identifiers, metrics, and terminal output."
          >
            <div className="space-y-5">
              <div>
                <div className="mb-2 font-mono text-[10px] text-muted-foreground">Display / 32</div>
                <div className="text-[32px] font-semibold leading-tight tracking-[-0.04em]">
                  Server health at a glance
                </div>
              </div>
              <div>
                <div className="mb-2 font-mono text-[10px] text-muted-foreground">Heading / 22</div>
                <div className="text-[22px] font-semibold leading-tight tracking-[-0.025em]">
                  Deployments and services
                </div>
              </div>
              <div>
                <div className="mb-2 font-mono text-[10px] text-muted-foreground">Body / 13</div>
                <p className="max-w-[62ch] leading-relaxed text-muted-foreground">
                  Connect to a server, inspect its current state, and act on services without
                  leaving the workspace. Values always show their unit and last sample time.
                </p>
              </div>
              <div className="rounded-md border border-border bg-muted/45 px-3 py-2.5 font-mono text-[12px]">
                198.51.100.24 · 4 vCPU · 8 GB · 42 days
              </div>
            </div>
          </Section>

          <Section
            title="Actions"
            description="One primary action per view. Secondary actions stay quiet until needed."
          >
            <div className="flex flex-wrap items-center gap-2">
              <Button>Add server</Button>
              <Button variant="secondary">Test connection</Button>
              <Button variant="outline">Refresh</Button>
              <Button variant="ghost">Cancel</Button>
              <Button variant="destructive">Stop service</Button>
              <Button variant="link">View logs</Button>
            </div>
            <div className="mt-4 flex flex-wrap items-center gap-2">
              <Button size="sm">Small</Button>
              <Button size="icon" aria-label="Refresh">
                <HardDrives className="size-4" aria-hidden="true" />
              </Button>
              <Button disabled>Disabled</Button>
            </div>
          </Section>

          <Section
            title="Forms"
            description="Labels sit above inputs. Helper and error text sit below. No placeholder-only labels."
          >
            <div className="grid max-w-2xl gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="host">Host</Label>
                <Input id="host" defaultValue="198.51.100.24" />
              </div>
              <div className="space-y-2">
                <Label htmlFor="port">SSH port</Label>
                <Input id="port" defaultValue="22" inputMode="numeric" />
              </div>
              <div className="space-y-2">
                <Label htmlFor="service">Service</Label>
                <Select value={service} onValueChange={setService}>
                  <SelectTrigger id="service">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="nginx.service">nginx.service</SelectItem>
                    <SelectItem value="postgresql.service">postgresql.service</SelectItem>
                    <SelectItem value="redis.service">redis.service</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label htmlFor="note">Note</Label>
                <Input id="note" aria-invalid="true" defaultValue="Failed to resolve host" />
                <p className="text-[11px] text-destructive">Check the hostname or DNS settings.</p>
              </div>
              <div className="space-y-2 sm:col-span-2">
                <Label htmlFor="notes">Notes</Label>
                <Textarea id="notes" placeholder="Optional non-sensitive notes" />
              </div>
            </div>
          </Section>

          <Section
            title="Status"
            description="Connection state and service state use text plus color. Color never carries meaning alone."
          >
            <div className="flex flex-wrap gap-2">
              {(
                [
                  "connected",
                  "connecting",
                  "host-key-verification",
                  "reconnecting",
                  "disconnected",
                  "error",
                ] as ConnectionState[]
              ).map((state) => (
                <StatusBadge key={state} state={state} />
              ))}
            </div>
            <div className="mt-4 flex flex-wrap gap-2">
              <Badge variant="success">Active</Badge>
              <Badge variant="warning">Degraded</Badge>
              <Badge variant="danger">Failed</Badge>
              <Badge variant="info">Update available</Badge>
              <Badge variant="secondary">Inactive</Badge>
              <Badge variant="outline">Unknown</Badge>
            </div>
          </Section>

          <Section
            title="Alerts"
            description="Inline messages explain what happened and what to do next."
          >
            <div className="space-y-2.5">
              <Alert>
                <Info className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                <div>
                  <AlertTitle>Monitoring uses snapshots</AlertTitle>
                  <AlertDescription>
                    Values refresh on demand or at the configured interval.
                  </AlertDescription>
                </div>
              </Alert>
              <Alert variant="success">
                <CheckCircle className="mt-0.5 size-4 shrink-0 text-success" aria-hidden="true" />
                <div>
                  <AlertTitle>nginx.service is active</AlertTitle>
                  <AlertDescription>Verified after restart at 14:32:08.</AlertDescription>
                </div>
              </Alert>
              <Alert variant="destructive">
                <WarningCircle
                  className="mt-0.5 size-4 shrink-0 text-destructive"
                  aria-hidden="true"
                />
                <div>
                  <AlertTitle>Host key changed</AlertTitle>
                  <AlertDescription>
                    Connection blocked. Remove the trusted fingerprint only after verifying the
                    server out of band.
                  </AlertDescription>
                </div>
              </Alert>
            </div>
          </Section>

          <Section
            title="Data"
            description="Tables use rules and spacing instead of elevated cards. Numbers use the mono face."
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Service</TableHead>
                  <TableHead>State</TableHead>
                  <TableHead className="text-right">Memory</TableHead>
                  <TableHead className="text-right">Updated</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {serviceRows.map((row) => (
                  <TableRow key={row.name}>
                    <TableCell className="font-mono text-[12px]">{row.name}</TableCell>
                    <TableCell>
                      <Badge variant={row.tone}>{row.state}</Badge>
                    </TableCell>
                    <TableCell className="text-right font-mono text-[12px] text-muted-foreground">
                      {row.name.includes("postgresql") ? "128 MB" : "42 MB"}
                    </TableCell>
                    <TableCell className="text-right font-mono text-[12px] text-muted-foreground">
                      14:32
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            <Separator className="my-5" />
            <Tabs defaultValue="overview">
              <TabsList>
                <TabsTrigger value="overview">Overview</TabsTrigger>
                <TabsTrigger value="services">Services</TabsTrigger>
                <TabsTrigger value="logs">Logs</TabsTrigger>
              </TabsList>
              <TabsContent value="overview" className="text-[13px] text-muted-foreground">
                Server identity and resource summary.
              </TabsContent>
              <TabsContent value="services" className="text-[13px] text-muted-foreground">
                Discovered systemd units and verified actions.
              </TabsContent>
              <TabsContent value="logs" className="text-[13px] text-muted-foreground">
                Bounded log tails with retrieval timestamps.
              </TabsContent>
            </Tabs>
          </Section>

          <Section
            title="Terminal output"
            description="Command output stays readable, copyable, and bounded."
          >
            <OutputPanel
              title="nginx.service status"
              content={`● nginx.service - A high performance web server
     Loaded: loaded (/usr/lib/systemd/system/nginx.service; enabled)
     Active: active (running) since Tue 2026-11-03 09:12:44 UTC
   Main PID: 842 (nginx)
      Tasks: 3 (limit: 9488)
     Memory: 18.4M`}
            />
          </Section>

          <Section
            title="Empty and confirmation"
            description="Empty states invite the next action. Destructive actions name the server and target."
          >
            <EmptyState
              icon={<HardDrives className="size-4" aria-hidden="true" />}
              title="No servers yet"
              description="Add your first VPS profile to connect over SSH and see its live state."
              action={<Button size="sm">Add server</Button>}
            />
            <div className="mt-4 flex flex-wrap gap-2">
              <Button variant="destructive" onClick={() => setConfirmOpen(true)}>
                Stop production service
              </Button>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="secondary">Hover for detail</Button>
                </TooltipTrigger>
                <TooltipContent>Uses 42 MB of 8 GB available memory.</TooltipContent>
              </Tooltip>
            </div>
            <ConfirmDialog
              open={confirmOpen}
              onOpenChange={setConfirmOpen}
              title="Stop nginx.service?"
              description="Active connections will close and the site will stop responding until the service starts again."
              confirmLabel="Stop service"
              details={[
                { label: "Server", value: "Production · 198.51.100.24" },
                { label: "Service", value: "nginx.service" },
                { label: "Action", value: "systemctl stop nginx.service" },
              ]}
              onConfirm={() => setConfirmOpen(false)}
            />
          </Section>

          <footer className="flex items-center justify-between border-t border-border pt-5 font-mono text-[10px] text-muted-foreground">
            <span>ServerDesk design system</span>
            <span>v0.1.0</span>
          </footer>
        </div>
      </main>
    </TooltipProvider>
  );
}

export { StyleGuidePage };
