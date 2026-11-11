import type { Icon } from "@phosphor-icons/react";
import {
  ChartLineIcon,
  ClockCounterClockwiseIcon,
  DatabaseIcon,
  FilesIcon,
  FolderSimpleIcon,
  GaugeIcon,
  HardDrivesIcon,
  ListMagnifyingGlassIcon,
  ShieldCheckIcon,
  SlidersHorizontalIcon,
  SquaresFourIcon,
  TerminalWindowIcon,
} from "@phosphor-icons/react";

export type NavItem = {
  label: string;
  to: string;
  icon: Icon;
  end?: boolean;
};

export type NavGroup = {
  label: string;
  items: NavItem[];
};

export const navGroups: NavGroup[] = [
  {
    label: "Workspace",
    items: [
      { label: "Overview", to: "/", icon: SquaresFourIcon, end: true },
      { label: "Servers", to: "/servers", icon: HardDrivesIcon },
      { label: "Projects", to: "/projects", icon: FolderSimpleIcon },
      { label: "Monitoring", to: "/monitoring", icon: ChartLineIcon },
    ],
  },
  {
    label: "Server",
    items: [
      { label: "Services", to: "/services", icon: GaugeIcon },
      { label: "Databases", to: "/databases", icon: DatabaseIcon },
      { label: "File Manager", to: "/files", icon: FilesIcon },
      { label: "Terminal", to: "/terminal", icon: TerminalWindowIcon },
      { label: "Logs", to: "/logs", icon: ListMagnifyingGlassIcon },
    ],
  },
  {
    label: "Application",
    items: [
      { label: "Command History", to: "/history", icon: ClockCounterClockwiseIcon },
      { label: "Security & Credentials", to: "/security", icon: ShieldCheckIcon },
      { label: "Settings", to: "/settings", icon: SlidersHorizontalIcon },
    ],
  },
];

export const allNavItems = navGroups.flatMap((group) => group.items);
