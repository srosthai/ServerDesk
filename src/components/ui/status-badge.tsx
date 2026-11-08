import { cn } from "@/lib/utils";
import type { ConnectionState } from "@/types/contracts";

export type { ConnectionState };

const stateConfig: Record<
  ConnectionState,
  { label: string; dot: string; text: string; container: string; pulse?: boolean }
> = {
  disconnected: {
    label: "Disconnected",
    dot: "bg-muted-foreground/45",
    text: "text-muted-foreground",
    container: "border-border bg-muted/50",
  },
  connecting: {
    label: "Connecting",
    dot: "bg-warning",
    text: "text-warning",
    container: "border-warning/25 bg-warning/8",
    pulse: true,
  },
  "host-key-verification": {
    label: "Verify host key",
    dot: "bg-warning",
    text: "text-warning",
    container: "border-warning/25 bg-warning/8",
  },
  connected: {
    label: "Connected",
    dot: "bg-success",
    text: "text-success",
    container: "border-success/25 bg-success/8",
  },
  reconnecting: {
    label: "Reconnecting",
    dot: "bg-warning",
    text: "text-warning",
    container: "border-warning/25 bg-warning/8",
    pulse: true,
  },
  error: {
    label: "Connection error",
    dot: "bg-destructive",
    text: "text-destructive",
    container: "border-destructive/25 bg-destructive/8",
  },
};

type StatusBadgeProps = {
  state: ConnectionState;
  label?: string;
  className?: string;
};

function StatusBadge({ state, label, className }: StatusBadgeProps) {
  const config = stateConfig[state];

  return (
    <span
      className={cn(
        "inline-flex w-fit items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium",
        config.container,
        config.text,
        className,
      )}
    >
      <span
        aria-hidden="true"
        className={cn("size-1.5 rounded-full", config.dot, config.pulse && "animate-pulse")}
      />
      {label ?? config.label}
    </span>
  );
}

export { StatusBadge };
