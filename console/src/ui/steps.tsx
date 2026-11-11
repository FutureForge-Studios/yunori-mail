import type { ReactNode } from "react";
import { Check, X } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";

export type StepState = "done" | "current" | "pending" | "failed" | "waiting" | "skipped";

/**
 * Vertical step list (install checklist, DMARC ladder, any "where am I in
 * this process"). The marker carries state as a glyph and the row carries it
 * as words; the rail between markers fills in as steps complete.
 */
export function Steps({ children, label, className }: { children: ReactNode; label: string; className?: string }) {
  return (
    <ol aria-label={label} className={cn("relative", className)}>
      {children}
    </ol>
  );
}

export function Step({
  state,
  title,
  meta,
  children,
  last,
  index,
}: {
  state: StepState;
  title: ReactNode;
  /** Right-aligned status words or a timestamp. */
  meta?: ReactNode;
  children?: ReactNode;
  last?: boolean;
  /** Shown in pending markers. */
  index?: number;
}) {
  const filled = state === "done" || state === "skipped";
  return (
    <li className="relative flex gap-3.5 pb-6 last:pb-0" aria-current={state === "current" || state === "waiting" ? "step" : undefined}>
      {!last && <span aria-hidden className={cn("absolute left-[11px] top-7 h-[calc(100%-1.75rem)] w-px", filled ? "bg-foreground/35" : "bg-border")} />}
      <span
        aria-hidden
        className={cn(
          "relative z-[1] mt-px grid h-6 w-6 shrink-0 place-items-center rounded-full border text-2xs font-semibold tabular-nums",
          state === "done" && "border-foreground bg-foreground text-background",
          state === "skipped" && "border-border-strong bg-muted text-muted-foreground",
          state === "current" && "border-foreground bg-card text-foreground ring-4 ring-foreground/10",
          state === "waiting" && "border-warning bg-warning/10 text-warning-ink ring-4 ring-warning/15",
          state === "failed" && "border-destructive bg-destructive/10 text-danger-ink",
          state === "pending" && "border-border-strong bg-card text-muted-foreground",
        )}
      >
        {state === "done" ? (
          <Check className="h-3.5 w-3.5" strokeWidth={2.5} />
        ) : state === "failed" ? (
          <X className="h-3.5 w-3.5" strokeWidth={2.5} />
        ) : state === "current" || state === "waiting" ? (
          <span className={cn("h-2 w-2 rounded-full", state === "waiting" ? "bg-warning" : "bg-foreground")} />
        ) : (
          index ?? ""
        )}
      </span>
      <div className="min-w-0 flex-1 pt-0.5">
        <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
          <p className={cn("text-sm font-medium", state === "pending" || state === "skipped" ? "text-muted-foreground" : "text-foreground")}>{title}</p>
          {meta && <span className="text-xs text-muted-foreground">{meta}</span>}
        </div>
        {children && <div className="mt-1 text-[13px] leading-relaxed text-muted-foreground">{children}</div>}
      </div>
    </li>
  );
}
