import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const alertVariants = cva("relative flex gap-3 rounded-md border px-3.5 py-3 text-[13px]", {
  variants: {
    variant: {
      default: "border-border bg-card text-foreground",
      info: "border-info/25 bg-info/8 text-foreground dark:border-info/30 dark:bg-info/10",
      success:
        "border-success/25 bg-success/8 text-foreground dark:border-success/30 dark:bg-success/10",
      warning:
        "border-warning/30 bg-warning/8 text-foreground dark:border-warning/35 dark:bg-warning/10",
      destructive:
        "border-destructive/30 bg-destructive/8 text-foreground dark:border-destructive/35 dark:bg-destructive/10",
    },
  },
  defaultVariants: {
    variant: "default",
  },
});

function Alert({
  className,
  variant,
  ...props
}: React.ComponentProps<"div"> & VariantProps<typeof alertVariants>) {
  return (
    <div
      data-slot="alert"
      role="alert"
      className={cn(alertVariants({ variant }), className)}
      {...props}
    />
  );
}

function AlertTitle({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-title"
      className={cn("font-medium leading-5 text-foreground", className)}
      {...props}
    />
  );
}

function AlertDescription({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-description"
      className={cn("text-[12px] leading-5 text-muted-foreground", className)}
      {...props}
    />
  );
}

export { Alert, AlertDescription, AlertTitle };
