import { AlertTriangle, XCircle } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import type { ActiveAlert, AlertSeverity } from "@/api/types";
import { toneText, type Tone } from "@/components/status";
import { fmtDateTime, timeAgo } from "@/lib/format";

export const SEVERITY_META: Record<AlertSeverity, { label: string; tone: Tone; icon: typeof XCircle }> = {
  critical: { label: "Critical", tone: "bad", icon: XCircle },
  warning: { label: "Warning", tone: "warn", icon: AlertTriangle },
};

/** Alerts firing now: what, since when, and what to do — severity as icon + words. */
export function AlertList({ alerts, compact }: { alerts: ActiveAlert[]; compact?: boolean }) {
  return (
    <ul className="divide-y divide-border/50">
      {alerts.map((a) => {
        const m = SEVERITY_META[a.severity];
        return (
          <li key={a.id} className={cn("flex gap-3", compact ? "py-2.5" : "py-3.5")}>
            <m.icon aria-hidden strokeWidth={1.75} className={cn("mt-0.5 h-4 w-4 shrink-0", toneText(m.tone))} />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <p className="text-sm font-medium text-foreground/90">{a.title}</p>
                <span className="text-[11px] text-muted-foreground">
                  <span className={cn("font-medium", toneText(m.tone))}>{m.label}</span> ·{" "}
                  <time dateTime={a.since} title={fmtDateTime(a.since)}>
                    started {timeAgo(a.since)}
                  </time>
                </span>
              </div>
              <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{a.summary}</p>
              <p className="mt-1.5 text-xs leading-relaxed text-foreground/85">
                <span className="font-medium">What to do: </span>
                {a.remedy}
              </p>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
