import { AlertTriangle, CheckCircle2, CircleDashed, ExternalLink, Info, XCircle, type LucideIcon } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import type { Check, CheckStatus } from "@/api/types";
import { StatusBadge } from "@/ui/badge";
import { TONE_DOT, TONE_TEXT, type Tone } from "@/ui/tone";

/**
 * Status vocabulary for checks. Status colour always ships with an icon and a
 * word — never colour alone (WCAG 1.4.1). Tones live in src/ui/tone.ts.
 */
export type { Tone };
export { StatusBadge };

export const toneText = (t: Tone) => TONE_TEXT[t];
export const toneDot = (t: Tone) => TONE_DOT[t];

const ICON_WELL: Record<Tone, string> = {
  ok: "bg-success/10",
  warn: "bg-warning/15",
  bad: "bg-destructive/10",
  info: "bg-info/10",
  muted: "bg-muted",
};

export const CHECK_META: Record<CheckStatus, { label: string; tone: Tone; icon: LucideIcon }> = {
  pass: { label: "Pass", tone: "ok", icon: CheckCircle2 },
  warn: { label: "Warning", tone: "warn", icon: AlertTriangle },
  fail: { label: "Failing", tone: "bad", icon: XCircle },
  info: { label: "Info", tone: "info", icon: Info },
  unknown: { label: "Not checked", tone: "muted", icon: CircleDashed },
};

export function CheckPill({ status, label }: { status: CheckStatus; label?: string }) {
  const m = CHECK_META[status];
  return (
    <StatusBadge tone={m.tone} icon={m.icon}>
      {label ?? m.label}
    </StatusBadge>
  );
}

/** Round icon well in the tone of a status (check rows, alert rows). */
export function ToneIcon({ tone, icon: Icon, className }: { tone: Tone; icon: LucideIcon; className?: string }) {
  return (
    <span aria-hidden className={cn("grid h-7 w-7 shrink-0 place-items-center rounded-full", ICON_WELL[tone], className)}>
      <Icon strokeWidth={2} className={cn("h-3.5 w-3.5", TONE_TEXT[tone])} />
    </span>
  );
}

/** One diagnostic: label, what we saw, what to do — status as icon + words. */
export function CheckRow({ check, compact }: { check: Check; compact?: boolean }) {
  const m = CHECK_META[check.status];
  return (
    <li className={cn("flex gap-3", compact ? "py-3" : "py-4")}>
      <ToneIcon tone={m.tone} icon={m.icon} />
      <div className="min-w-0 flex-1 pt-0.5">
        <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
          <p className="text-sm font-medium text-foreground">{check.label}</p>
          <span className={cn("text-xs font-medium", TONE_TEXT[m.tone])}>{m.label}</span>
        </div>
        <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">{check.detail}</p>
        {check.remedy && (
          <p className="mt-2 rounded-lg bg-muted/60 px-3 py-2 text-[13px] leading-relaxed text-foreground/90">
            <span className="font-medium">What to do: </span>
            {check.remedy}
          </p>
        )}
        {check.helpUrl && (
          <a
            href={check.helpUrl}
            target="_blank"
            rel="noreferrer noopener"
            className="mt-1.5 inline-flex items-center gap-1 rounded-sm text-xs text-muted-foreground underline underline-offset-2 transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
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
export function CheckList({ checks, compact, className }: { checks: Check[]; compact?: boolean; className?: string }) {
  return (
    <ul className={cn("divide-y divide-border/60", className)}>
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
