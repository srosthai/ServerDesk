import { CheckIcon } from "@phosphor-icons/react";
import { cn } from "@/lib/utils";

export function TransferSteps({
  steps,
  activeIndex,
  label,
}: {
  steps: string[];
  activeIndex: number;
  label: string;
}) {
  return (
    <ol aria-label={label} className="flex flex-wrap items-center gap-1.5">
      {steps.map((step, index) => {
        const complete = index < activeIndex;
        const active = index === activeIndex;
        return (
          <li key={step} className="flex items-center gap-1.5">
            <span
              aria-current={active ? "step" : undefined}
              className={cn(
                "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[10px] font-medium",
                active
                  ? "border-primary/40 bg-primary/10 text-primary"
                  : complete
                    ? "border-success/30 bg-success/10 text-success"
                    : "border-border bg-muted/30 text-muted-foreground",
              )}
            >
              <span className="grid size-3.5 place-items-center rounded-full border border-current text-[8px]">
                {complete ? <CheckIcon className="size-2.5" aria-hidden="true" /> : index + 1}
              </span>
              {step}
            </span>
            {index < steps.length - 1 ? (
              <span aria-hidden="true" className="h-px w-3 bg-border" />
            ) : null}
          </li>
        );
      })}
    </ol>
  );
}
