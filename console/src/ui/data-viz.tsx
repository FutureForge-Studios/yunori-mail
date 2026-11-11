import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import { ArrowUpRight, type LucideIcon } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { TONE_DOT, TONE_FILL, TONE_STROKE, TONE_TEXT, type Tone } from "./tone";

/**
 * Small, honest charts: a bar meter, a ring, a ranked bar list and a stacked
 * distribution. Every mark has its number in text next to it, so the visual
 * is a reading aid, never the only carrier of meaning.
 */

/** Tone for a usage percentage: amber past 90%. */
export const usageTone = (p: number | null): Tone => (p !== null && p >= 90 ? "warn" : "muted");

export function Meter({
  value,
  label,
  valueText,
  tone,
  className,
  size = "sm",
}: {
  /** 0–100, or null when unknown/unlimited. */
  value: number | null;
  label: string;
  valueText: string;
  tone?: Tone;
  className?: string;
  size?: "sm" | "md";
}) {
  const t = tone ?? usageTone(value);
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value === null ? undefined : Math.round(value)}
      aria-valuetext={valueText}
      className={cn("w-full overflow-hidden rounded-full bg-muted", size === "md" ? "h-2" : "h-1.5", className)}
    >
      {value !== null && (
        <div
          className={cn("h-full rounded-full transition-[width] duration-500 motion-reduce:transition-none", TONE_FILL[t])}
          style={{ width: `${Math.max(value, value > 0 ? 2 : 0)}%` }}
        />
      )}
    </div>
  );
}

/** Donut gauge with the figure in the middle. */
export function RingMeter({
  value,
  label,
  valueText,
  tone,
  size = 88,
  stroke = 8,
  center,
  className,
}: {
  value: number | null;
  label: string;
  valueText: string;
  tone?: Tone;
  size?: number;
  stroke?: number;
  /** What to show in the middle (defaults to the rounded percentage). */
  center?: ReactNode;
  className?: string;
}) {
  const t = tone ?? usageTone(value);
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const v = value === null ? 0 : Math.min(100, Math.max(0, value));
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value === null ? undefined : Math.round(v)}
      aria-valuetext={valueText}
      className={cn("relative inline-grid shrink-0 place-items-center", className)}
      style={{ width: size, height: size }}
    >
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="-rotate-90" aria-hidden>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="hsl(var(--muted))" strokeWidth={stroke} />
        {value !== null && (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke={TONE_STROKE[t]}
            strokeWidth={stroke}
            strokeLinecap="round"
            strokeDasharray={`${(v / 100) * c} ${c}`}
            className="transition-[stroke-dasharray] duration-700 motion-reduce:transition-none"
          />
        )}
      </svg>
      <span aria-hidden className="absolute inset-0 grid place-items-center text-center">
        {center ?? <span className="display-number text-lg leading-none">{value === null ? "—" : `${Math.round(v)}%`}</span>}
      </span>
    </div>
  );
}

/** Ranked horizontal bars with label and value (storage per mailbox, queue reasons). */
export function BarList({
  items,
  label,
  className,
}: {
  items: { key: string; label: ReactNode; value: number; display: string; tone?: Tone; hint?: ReactNode }[];
  label: string;
  className?: string;
}) {
  const max = Math.max(1, ...items.map((i) => i.value));
  return (
    <ul aria-label={label} className={cn("space-y-3", className)}>
      {items.map((i) => (
        <li key={i.key}>
          <div className="mb-1.5 flex items-baseline justify-between gap-3 text-[13px]">
            <span className="min-w-0 truncate text-foreground/90">{i.label}</span>
            <span className="shrink-0 tabular-nums text-muted-foreground">{i.display}</span>
          </div>
          <div aria-hidden className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
            <div className={cn("h-full rounded-full", TONE_FILL[i.tone ?? "muted"])} style={{ width: `${Math.max(2, (i.value / max) * 100)}%` }} />
          </div>
          {i.hint && <p className="mt-1 text-xs text-muted-foreground">{i.hint}</p>}
        </li>
      ))}
    </ul>
  );
}

/** One stacked bar showing how a total splits, with a legend. */
export function SegmentBar({
  segments,
  label,
  className,
}: {
  segments: { key: string; label: string; value: number; tone: Tone }[];
  label: string;
  className?: string;
}) {
  const total = segments.reduce((n, s) => n + s.value, 0);
  return (
    <figure className={className}>
      <div aria-hidden className="flex h-2 w-full gap-0.5 overflow-hidden rounded-full bg-muted">
        {total > 0 &&
          segments
            .filter((s) => s.value > 0)
            .map((s) => <div key={s.key} className={cn("h-full first:rounded-l-full last:rounded-r-full", TONE_FILL[s.tone])} style={{ width: `${(s.value / total) * 100}%` }} />)}
      </div>
      <figcaption className="mt-3">
        <span className="sr-only">{label}: </span>
        <ul className="flex flex-wrap gap-x-4 gap-y-1.5 text-xs">
          {segments.map((s) => (
            <li key={s.key} className="flex items-center gap-1.5 text-muted-foreground">
              <span aria-hidden className={cn("h-2 w-2 rounded-full", TONE_DOT[s.tone])} />
              {s.label}
              <span className="tabular-nums text-foreground">{s.value}</span>
            </li>
          ))}
        </ul>
      </figcaption>
    </figure>
  );
}

/** Stat tile: label, Fraunces number, a hint, and an optional visual or link. */
export function StatCard({
  label,
  value,
  hint,
  icon: Icon,
  to,
  tone,
  children,
  className,
}: {
  label: ReactNode;
  value: ReactNode;
  hint?: ReactNode;
  icon?: LucideIcon;
  /** Makes the whole tile a link. */
  to?: string;
  /** Tints the hint (e.g. warn when something waits). */
  tone?: Tone;
  children?: ReactNode;
  className?: string;
}) {
  const body = (
    <>
      <div className="flex items-center justify-between gap-2">
        <span className="text-[13px] font-medium text-muted-foreground">{label}</span>
        {Icon && <Icon aria-hidden className="h-4 w-4 text-icon" strokeWidth={1.75} />}
        {to && !Icon && <ArrowUpRight aria-hidden className="h-4 w-4 text-icon opacity-0 transition-opacity group-hover:opacity-100" />}
      </div>
      <div className="mt-3 flex items-end justify-between gap-3">
        <div className="min-w-0">
          <p className="display-number text-[30px] leading-none text-foreground">{value}</p>
          {hint && (
            <p className={cn("mt-2 flex items-start gap-1.5 text-xs leading-snug", tone && tone !== "muted" ? TONE_TEXT[tone] : "text-muted-foreground")}>
              {tone && tone !== "muted" && <span aria-hidden className={cn("mt-[5px] h-1.5 w-1.5 shrink-0 rounded-full", TONE_DOT[tone])} />}
              {hint}
            </p>
          )}
        </div>
        {children}
      </div>
    </>
  );
  const cls = cn("group block min-w-0 rounded-xl border border-border bg-card p-4 shadow-card sm:p-5", className);
  if (to)
    return (
      <Link
        to={to}
        className={cn(
          cls,
          "transition-[border-color,box-shadow] hover:border-border-strong hover:shadow-pop focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 focus-visible:ring-offset-2 focus-visible:ring-offset-background",
        )}
      >
        {body}
      </Link>
    );
  return <div className={cls}>{body}</div>;
}
