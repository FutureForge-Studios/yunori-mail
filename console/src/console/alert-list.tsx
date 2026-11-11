import { AlertTriangle, XCircle } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import type { ActiveAlert, AlertSeverity } from "@/api/types";
import { ToneIcon, toneText, type Tone } from "@/components/status";
import { fmtDateTime, timeAgo } from "@/lib/format";

export const SEVERITY_META: Record<AlertSeverity, { label: string; tone: Tone; icon: typeof XCircle }> = {
  critical: { label: "Critical", tone: "bad", icon: XCircle },
  warning: { label: "Warning", tone: "warn", icon: AlertTriangle },
};

/** Alerts firing now: what, since when, and what to do — severity as icon + words. */
export function AlertList({ alerts, compact }: { alerts: ActiveAlert[]; compact?: boolean }) {
  return (
    <ul className="divide-y divide-border/60">
      {alerts.map((a) => {
        const m = SEVERITY_META[a.severity];
        return (
          <li key={a.id} className={cn("flex gap-3", compact ? "py-3" : "py-4")}>
            <ToneIcon tone={m.tone} icon={m.icon} />
            <div className="min-w-0 flex-1 pt-0.5">
              <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <p className="text-sm font-medium text-foreground">{a.title}</p>
                <span className="text-xs text-muted-foreground">
                  <span className={cn("font-medium", toneText(m.tone))}>{m.label}</span> ·{" "}
                  <time dateTime={a.since} title={fmtDateTime(a.since)}>
                    started {timeAgo(a.since)}
                  </time>
                </span>
              </div>
              <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">{a.summary}</p>
              <p className="mt-2 rounded-lg bg-muted/60 px-3 py-2 text-[13px] leading-relaxed text-foreground/90">
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
