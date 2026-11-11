import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { TONE_DOT, TONE_SOFT, TONE_TEXT, type Tone } from "./tone";

/** Status pill: soft tint + icon (or dot) + words. Never colour alone. */
export function StatusBadge({
  tone,
  icon: Icon,
  children,
  className,
  size = "sm",
}: {
  tone: Tone;
  icon?: LucideIcon;
  children: ReactNode;
  className?: string;
  size?: "xs" | "sm";
}) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full border font-medium",
        size === "xs" ? "px-1.5 py-px text-2xs" : "px-2 py-0.5 text-xs",
        TONE_SOFT[tone],
        className,
      )}
    >
      {Icon ? (
        <Icon aria-hidden className={size === "xs" ? "h-3 w-3" : "h-3.5 w-3.5"} strokeWidth={2} />
      ) : (
        <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", TONE_DOT[tone])} />
      )}
      {children}
    </span>
  );
}

/** Neutral label (Primary, Admin, System). */
export function Badge({ children, className, tone }: { children: ReactNode; className?: string; tone?: Tone }) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center whitespace-nowrap rounded-md border px-1.5 py-px text-2xs font-medium",
        tone ? TONE_SOFT[tone] : "border-border bg-muted/60 text-foreground/75",
        className,
      )}
    >
      {children}
    </span>
  );
}

/** Monospace chip (record types, hostnames, audit actions). */
export function Chip({ children, onRemove, className }: { children: ReactNode; onRemove?: () => void; className?: string }) {
  return (
    <span className={cn("inline-flex max-w-full items-center gap-1 rounded-md border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-2xs text-foreground/85", className)}>
      <span className="truncate">{children}</span>
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Remove ${typeof children === "string" ? children : "item"}`}
          className="rounded text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
        >
          ×
        </button>
      )}
    </span>
  );
}

export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <kbd className={cn("inline-flex h-5 min-w-5 items-center justify-center rounded border border-border bg-muted/60 px-1 font-sans text-2xs font-medium text-muted-foreground", className)}>
      {children}
    </kbd>
  );
}

/** A dot + word, for dense rows. */
export function StatusDot({ tone, children, className }: { tone: Tone; children?: ReactNode; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-xs", TONE_TEXT[tone], className)}>
      <span aria-hidden className={cn("h-2 w-2 rounded-full", TONE_DOT[tone])} />
      {children}
    </span>
  );
}

/** Initials avatar on a quiet tile. */
export function Avatar({ name, className }: { name: string; className?: string }) {
  const initials =
    name
      .replace(/@.*/, "")
      .split(/[\s._-]+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((p) => p[0]!.toUpperCase())
      .join("") || "?";
  // A stable warm hue per name, low chroma so it never competes with status.
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) % 360;
  return (
    <span
      aria-hidden
      className={cn("grid h-8 w-8 shrink-0 place-items-center rounded-full text-xs font-semibold text-foreground/80 ring-1 ring-inset ring-foreground/10", className)}
      style={{ background: `hsl(${h} 32% 50% / 0.16)` }}
    >
      {initials}
    </span>
  );
}
