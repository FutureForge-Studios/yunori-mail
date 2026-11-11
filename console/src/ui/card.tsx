import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";

/**
 * Surfaces. A Card is the card token on the canvas with a hairline and a
 * whisper shadow. Use CardHeader/CardBody/CardFooter for sectioned cards, or
 * pass `padded` (default) for a simple padded box.
 */
export function Card({
  className,
  children,
  padded = true,
  as: As = "div",
  id,
  "aria-labelledby": labelledBy,
}: {
  className?: string;
  children: ReactNode;
  padded?: boolean;
  as?: "div" | "section" | "li" | "article";
  id?: string;
  "aria-labelledby"?: string;
}) {
  return (
    <As id={id} aria-labelledby={labelledBy} className={cn("min-w-0 rounded-xl border border-border bg-card text-card-foreground shadow-card", padded && "p-5", className)}>
      {children}
    </As>
  );
}

export function CardHeader({
  title,
  description,
  actions,
  icon: Icon,
  id,
  className,
  children,
  divider = false,
}: {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  icon?: LucideIcon;
  id?: string;
  className?: string;
  children?: ReactNode;
  /** A rule under the header (for cards whose body is a list). */
  divider?: boolean;
}) {
  return (
    <div className={cn("px-5 pb-4 pt-5", divider && "border-b border-border/70", className)}>
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
        <div className="flex min-w-0 items-start gap-3">
          {Icon && (
            <span aria-hidden className="mt-px grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-border bg-muted/50 text-icon">
              <Icon className="h-4 w-4" strokeWidth={1.75} />
            </span>
          )}
          <div className="min-w-0">
            <h2 id={id} className="text-[15px] font-semibold leading-6 tracking-[-0.005em] text-foreground">
              {title}
            </h2>
            {description && <p className="mt-0.5 max-w-prose text-[13px] leading-relaxed text-muted-foreground">{description}</p>}
          </div>
        </div>
        {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
      </div>
      {children}
    </div>
  );
}

export function CardBody({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("px-5 pb-5", className)}>{children}</div>;
}

export function CardFooter({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("flex flex-wrap items-center gap-2 rounded-b-xl border-t border-border/70 bg-muted/25 px-5 py-3", className)}>{children}</div>;
}

/** Small heading inside a card or page section. */
export function SectionLabel({ children, className, id }: { children: ReactNode; className?: string; id?: string }) {
  return (
    <h3 id={id} className={cn("mb-3 text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground", className)}>
      {children}
    </h3>
  );
}

/** A heading row between groups of cards on a page. */
export function SectionHeader({
  title,
  description,
  actions,
  id,
  className,
}: {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  id?: string;
  className?: string;
}) {
  return (
    <div className={cn("mb-4 flex flex-wrap items-end justify-between gap-x-4 gap-y-2", className)}>
      <div className="min-w-0">
        <h2 id={id} className="text-base font-semibold tracking-[-0.01em] text-foreground">
          {title}
        </h2>
        {description && <p className="mt-0.5 max-w-3xl text-[13px] leading-relaxed text-muted-foreground">{description}</p>}
      </div>
      {actions && <div className="flex min-w-0 max-w-full flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}

/** Label/value row used across detail panes. */
export function KV({ label, children, className }: { label: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex items-baseline justify-between gap-4 py-2", className)}>
      <dt className="shrink-0 text-[13px] text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate text-right text-[13px] text-foreground">{children}</dd>
    </div>
  );
}

/** <dl> wrapper with dividers for KV rows. */
export function KVList({ children, className }: { children: ReactNode; className?: string }) {
  return <dl className={cn("divide-y divide-border/60", className)}>{children}</dl>;
}
