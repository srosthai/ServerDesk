import type * as React from "react";
import { CopyButton } from "@/components/ui/copy-button";
import { cn } from "@/lib/utils";

type OutputPanelProps = {
  title?: string;
  content: string;
  emptyText?: string;
  className?: string;
  maxHeightClassName?: string;
  meta?: React.ReactNode;
};

function OutputPanel({
  title = "Output",
  content,
  emptyText = "No output yet.",
  className,
  maxHeightClassName = "max-h-72",
  meta,
}: OutputPanelProps) {
  return (
    <section
      className={cn(
        "overflow-hidden rounded-md border border-[#1b241f] bg-[#050806] text-[#e2eae4]",
        className,
      )}
    >
      <header className="flex h-9 items-center justify-between gap-3 border-b border-[#1b241f] px-3">
        <span className="text-[11px] font-medium text-[#98a79e]">{title}</span>
        <div className="flex items-center gap-2">
          {meta ? <span className="font-mono text-[10px] text-[#98a79e]">{meta}</span> : null}
          <CopyButton
            value={content}
            className="text-[#98a79e] hover:bg-[#101713] hover:text-[#e2eae4]"
          />
        </div>
      </header>
      <pre
        className={cn(
          "overflow-auto px-3 py-2.5 font-mono text-[12px] leading-5 whitespace-pre-wrap break-words",
          maxHeightClassName,
        )}
      >
        {content || <span className="text-[#7d8d84]">{emptyText}</span>}
      </pre>
    </section>
  );
}

export { OutputPanel };
