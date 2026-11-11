import type { ReactNode } from "react";
import { AlertTriangle, CheckCircle2, Info, RotateCw, XCircle, type LucideIcon } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { errorMessage } from "@/api/client";
import { Button } from "./button";
import { TONE_SOFT, TONE_TEXT, type Tone } from "./tone";

export function Skeleton({ className }: { className?: string }) {
  return <div aria-hidden className={cn("yu-skeleton rounded-lg", className)} />;
}

/** Honest empty state: icon, one sentence, a hint, and the next action. */
export function EmptyState({
  title,
  hint,
  action,
  icon: Icon,
  className,
  compact,
}: {
  title: ReactNode;
  hint?: ReactNode;
  action?: ReactNode;
  icon?: LucideIcon;
  className?: string;
  compact?: boolean;
}) {
  return (
    <div
      className={cn(
        "grid place-items-center rounded-xl border border-dashed border-border-strong/70 bg-card/40 px-6 text-center",
        compact ? "py-8" : "py-14",
        className,
      )}
    >
      <div className="max-w-sm">
        {Icon && (
          <span aria-hidden className="mx-auto mb-3 grid h-10 w-10 place-items-center rounded-full border border-border bg-card text-icon shadow-card">
            <Icon className="h-[18px] w-[18px]" strokeWidth={1.75} />
          </span>
        )}
        <p className="text-sm font-medium text-foreground">{title}</p>
        {hint && <p className="mx-auto mt-1 text-[13px] leading-relaxed text-muted-foreground">{hint}</p>}
        {action && <div className="mt-4 flex justify-center gap-2">{action}</div>}
      </div>
    </div>
  );
}

/** Load failure with a retry — never a blank page. */
export function LoadError({ error, onRetry, what }: { error: unknown; onRetry: () => void; what: string }) {
  return (
    <EmptyState
      icon={AlertTriangle}
      title={`${what} couldn't load`}
      hint={errorMessage(error, "The server didn't answer. Check that yunori-mail is running.")}
      action={
        <Button variant="outline" size="sm" onClick={onRetry}>
          <RotateCw className="h-3.5 w-3.5" />
          Try again
        </Button>
      }
    />
  );
}

const CALLOUT_ICON: Record<Tone, LucideIcon> = { ok: CheckCircle2, warn: AlertTriangle, bad: XCircle, info: Info, muted: Info };

/** Inline banner: what happened, what to do, and an optional action. */
export function Callout({
  tone = "info",
  title,
  children,
  action,
  icon,
  className,
  role,
}: {
  tone?: Tone;
  title?: ReactNode;
  children?: ReactNode;
  action?: ReactNode;
  icon?: LucideIcon;
  className?: string;
  role?: "status" | "alert";
}) {
  const Icon = icon ?? CALLOUT_ICON[tone];
  return (
    <div role={role} className={cn("flex flex-col gap-3 rounded-xl border px-4 py-3.5 sm:flex-row sm:items-start", TONE_SOFT[tone], "text-foreground", className)}>
      <div className="flex min-w-0 flex-1 gap-3">
        <Icon aria-hidden className={cn("mt-0.5 h-4 w-4 shrink-0", TONE_TEXT[tone])} strokeWidth={2} />
        <div className="min-w-0 flex-1 text-[13px] leading-relaxed">
          {title && <p className="text-sm font-medium text-foreground">{title}</p>}
          {children && <div className={cn("text-foreground/80", title && "mt-0.5")}>{children}</div>}
        </div>
      </div>
      {action && <div className="flex shrink-0 flex-wrap items-center gap-2 pl-7 sm:pl-0">{action}</div>}
    </div>
  );
}

/** Form-level error (non-field), announced. */
export function FormError({ children, className }: { children: ReactNode; className?: string }) {
  if (!children) return null;
  return (
    <p role="alert" className={cn("flex items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/[0.07] px-3 py-2.5 text-[13px] leading-relaxed text-foreground", className)}>
      <XCircle aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-danger-ink" />
      <span className="min-w-0">{children}</span>
    </p>
  );
}

/** A small "loading…" line for panels that load after the page. */
export function InlineLoading({ label = "Loading…", className }: { label?: string; className?: string }) {
  return (
    <p className={cn("flex items-center gap-2 text-[13px] text-muted-foreground", className)} aria-busy="true">
      <span aria-hidden className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-r-transparent motion-reduce:animate-none" />
      {label}
    </p>
  );
}
