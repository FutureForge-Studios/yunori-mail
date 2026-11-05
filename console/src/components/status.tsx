import type { ReactNode } from "react";
import { AlertTriangle, CheckCircle2, CircleDashed, ExternalLink, Info, XCircle, type LucideIcon } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import type { Check, CheckStatus } from "@/api/types";

/**
 * Status vocabulary. Status color is reserved for state and ALWAYS ships with
 * an icon and a text label — never color alone (WCAG 1.4.1). Same tones as the
 * client's admin StatusPill; the "bad" text lifts in dark mode so it clears
 * 4.5:1 on the night canvas (torii-dark on night is ~3:1 at 11px).
 */
export type Tone = "ok" | "warn" | "bad" | "muted";

const TONE_TEXT: Record<Tone, string> = {
  ok: "text-emerald-700 dark:text-emerald-400",
  warn: "text-amber-700 dark:text-amber-400",
  bad: "text-torii dark:text-[hsl(4_80%_70%)]",
  muted: "text-muted-foreground",
};
const TONE_BORDER: Record<Tone, string> = {
  ok: "border-emerald-600/30",
  warn: "border-amber-600/30",
  bad: "border-torii/40",
  muted: "border-border/70",
};
const TONE_DOT: Record<Tone, string> = {
  ok: "bg-emerald-600 dark:bg-emerald-400",
  warn: "bg-amber-600 dark:bg-amber-400",
  bad: "bg-torii",
  muted: "bg-muted-foreground/60",
};

export const toneText = (t: Tone) => TONE_TEXT[t];
export const toneDot = (t: Tone) => TONE_DOT[t];

export const CHECK_META: Record<CheckStatus, { label: string; tone: Tone; icon: LucideIcon }> = {
  pass: { label: "Pass", tone: "ok", icon: CheckCircle2 },
  warn: { label: "Warning", tone: "warn", icon: AlertTriangle },
  fail: { label: "Failing", tone: "bad", icon: XCircle },
  info: { label: "Info", tone: "muted", icon: Info },
  unknown: { label: "Not checked", tone: "muted", icon: CircleDashed },
};

/** Pill with an icon + label. */
export function StatusBadge({
  tone,
  icon: Icon,
  children,
  className,
}: {
  tone: Tone;
  icon?: LucideIcon;
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-2 py-0.5 text-[11px]",
        TONE_BORDER[tone],
        TONE_TEXT[tone],
        className,
      )}
    >
      {Icon ? (
        <Icon aria-hidden className="h-3 w-3" strokeWidth={2} />
      ) : (
        <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", TONE_DOT[tone])} />
      )}
      {children}
    </span>
  );
}

export function CheckPill({ status, label }: { status: CheckStatus; label?: string }) {
  const m = CHECK_META[status];
  return (
    <StatusBadge tone={m.tone} icon={m.icon}>
      {label ?? m.label}
    </StatusBadge>
  );
}

/** One diagnostic: label, what we saw, what to do — status as icon + words. */
export function CheckRow({ check, compact }: { check: Check; compact?: boolean }) {
  const m = CHECK_META[check.status];
  return (
    <li className={cn("flex gap-3", compact ? "py-2.5" : "py-3.5")}>
      <m.icon aria-hidden strokeWidth={1.75} className={cn("mt-0.5 h-4 w-4 shrink-0", TONE_TEXT[m.tone])} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
          <p className="text-sm font-medium text-foreground/90">{check.label}</p>
          <span className={cn("text-[11px] font-medium", TONE_TEXT[m.tone])}>{m.label}</span>
        </div>
        <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{check.detail}</p>
        {check.remedy && (
          <p className="mt-1.5 text-xs leading-relaxed text-foreground/85">
            <span className="font-medium">What to do: </span>
            {check.remedy}
          </p>
        )}
        {check.helpUrl && (
          <a
            href={check.helpUrl}
            target="_blank"
            rel="noreferrer noopener"
            className="mt-1.5 inline-flex items-center gap-1 text-[11px] text-muted-foreground underline underline-offset-2 transition-colors hover:text-foreground"
          >
            Learn more <ExternalLink aria-hidden className="h-3 w-3" />
            <span className="sr-only">(opens in a new tab)</span>
          </a>
        )}
      </div>
    </li>
  );
}

/** A divided list of checks, worst first. */
export function CheckList({ checks, compact }: { checks: Check[]; compact?: boolean }) {
  return (
    <ul className="divide-y divide-border/50">
      {sortChecks(checks).map((c) => (
        <CheckRow key={c.id} check={c} compact={compact} />
      ))}
    </ul>
  );
}

const ORDER: Record<CheckStatus, number> = { fail: 0, warn: 1, unknown: 2, info: 3, pass: 4 };
export function sortChecks(checks: Check[]): Check[] {
  return [...checks].sort((a, b) => ORDER[a.status] - ORDER[b.status]);
}

/** Worst status in a set (for summaries). */
export function worstStatus(statuses: CheckStatus[]): CheckStatus {
  return statuses.reduce<CheckStatus>((w, s) => (ORDER[s] < ORDER[w] ? s : w), "pass");
}
