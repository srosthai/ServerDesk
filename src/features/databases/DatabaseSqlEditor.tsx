import { useMemo, useRef } from "react";
import { tokenizeSql } from "@/features/databases/databaseSqlWorkspaceModel";
import type { DatabaseEngine } from "@/types/contracts";

type DatabaseSqlEditorProps = {
  id: string;
  ariaLabel: string;
  value: string;
  engine: DatabaseEngine;
  placeholder: string;
  selectionStart: number;
  onChange: (value: string) => void;
  onSelectionChange: (start: number, end: number) => void;
  onRun: () => void;
  onRunAll: () => void;
};

export function DatabaseSqlEditor({
  id,
  ariaLabel,
  value,
  engine,
  placeholder,
  selectionStart,
  onChange,
  onSelectionChange,
  onRun,
  onRunAll,
}: DatabaseSqlEditorProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const highlightRef = useRef<HTMLPreElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);
  const tokens = useMemo(() => tokenizeSql(value, engine), [value, engine]);
  const lines = Math.max(12, value.split("\n").length);
  const activeLine = value
    .slice(0, Math.max(0, Math.min(selectionStart, value.length)))
    .split("\n").length;

  function syncScroll() {
    const textarea = textareaRef.current;
    if (!textarea) {
      return;
    }
    if (highlightRef.current) {
      highlightRef.current.scrollTop = textarea.scrollTop;
      highlightRef.current.scrollLeft = textarea.scrollLeft;
    }
    if (gutterRef.current) {
      gutterRef.current.scrollTop = textarea.scrollTop;
    }
  }

  function reportSelection() {
    const textarea = textareaRef.current;
    if (!textarea) {
      return;
    }
    onSelectionChange(textarea.selectionStart, textarea.selectionEnd);
  }

  return (
    <div className="flex h-64 min-h-56 overflow-hidden border-b border-border bg-[color:var(--card)] focus-within:ring-1 focus-within:ring-ring/40">
      <div
        ref={gutterRef}
        aria-hidden="true"
        className="w-11 shrink-0 overflow-hidden border-r border-border bg-muted/20 py-3 text-right font-mono text-[11px] leading-5 text-muted-foreground/55 select-none"
      >
        {Array.from({ length: lines }, (_, index) => (
          <div
            key={index}
            className={
              index + 1 === activeLine
                ? "bg-muted/60 pr-2 text-foreground tabular-nums"
                : "pr-2 tabular-nums"
            }
          >
            {index + 1}
          </div>
        ))}
      </div>
      <div className="relative min-w-0 flex-1">
        <pre
          ref={highlightRef}
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 overflow-hidden px-3 py-3 font-mono text-[12px] leading-5 whitespace-pre text-foreground"
        >
          {value.length === 0 ? (
            <span className="text-muted-foreground/70">{placeholder}</span>
          ) : (
            tokens.map((token, index) => (
              <span
                key={index}
                className={
                  token.kind === "comment"
                    ? "text-muted-foreground/75 italic"
                    : token.kind === "keyword"
                      ? "font-medium text-primary"
                      : token.kind === "function"
                        ? "text-info"
                        : token.kind === "string"
                          ? "text-success"
                          : token.kind === "number"
                            ? "text-warning"
                            : token.kind === "identifier"
                              ? "text-foreground"
                              : token.kind === "operator" || token.kind === "punctuation"
                                ? "text-muted-foreground"
                                : undefined
                }
              >
                {token.text}
              </span>
            ))
          )}
          {"\n "}
        </pre>
        <textarea
          ref={textareaRef}
          id={id}
          aria-label={ariaLabel}
          aria-multiline="true"
          value={value}
          onChange={(event) => {
            onChange(event.currentTarget.value);
            onSelectionChange(event.currentTarget.selectionStart, event.currentTarget.selectionEnd);
          }}
          onSelect={reportSelection}
          onFocus={reportSelection}
          onClick={reportSelection}
          onKeyUp={reportSelection}
          onScroll={syncScroll}
          onKeyDown={(event) => {
            if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
              event.preventDefault();
              if (event.shiftKey) {
                onRunAll();
              } else {
                onRun();
              }
            }
          }}
          placeholder={placeholder}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          wrap="off"
          className="absolute inset-0 h-full w-full resize-none overflow-auto bg-transparent px-3 py-3 font-mono text-[12px] leading-5 whitespace-pre text-transparent caret-[color:var(--foreground)] outline-none placeholder:text-transparent selection:bg-primary/25"
          style={{ WebkitTextFillColor: "transparent" }}
        />
      </div>
    </div>
  );
}
