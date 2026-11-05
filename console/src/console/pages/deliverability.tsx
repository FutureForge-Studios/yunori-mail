import { Check as CheckIcon, KeyRound, Lock, Network, ShieldX, HelpCircle, type LucideIcon } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Card, SectionLabel, Skeleton } from "@/design/vendor/admin/ui";
import { api } from "@/api/client";
import type { DeliverabilityResponse, QueueSummary } from "@/api/types";
import { LoadError, Page, usePageTitle } from "@/components/kit";
import { CheckList } from "@/components/status";
import { fmtNumber, timeAgo } from "@/lib/format";
import { useApi } from "@/lib/use-api";

const REASONS: Record<QueueSummary["groups"][number]["reason"], { label: string; icon: LucideIcon }> = {
  auth: { label: "Authentication", icon: KeyRound },
  tls: { label: "TLS", icon: Lock },
  network: { label: "Network", icon: Network },
  rejected: { label: "Rejected by recipient", icon: ShieldX },
  other: { label: "Other", icon: HelpCircle },
};

/** Will mail reach inboxes? Reputation checks, queue health, DMARC ladder. */
export default function DeliverabilityPage() {
  usePageTitle("Deliverability");
  const { data, error, reload } = useApi("deliverability", api.deliverability);

  if (error && !data) {
    return (
      <Page title="Deliverability">
        <LoadError error={error} onRetry={() => void reload()} what="Deliverability" />
      </Page>
    );
  }
  if (!data) {
    return (
      <Page title="Deliverability" blurb="Whether your mail reaches inboxes.">
        <div className="grid gap-4 lg:grid-cols-3">
          <Skeleton className="h-80 lg:col-span-2" />
          <Skeleton className="h-80" />
        </div>
      </Page>
    );
  }

  return (
    <Page
      title="Deliverability"
      blurb={`Whether your mail reaches inboxes rather than spam folders${data.checkedAt ? ` — checked ${timeAgo(data.checkedAt)}` : ""}.`}
    >
      <div className="grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <SectionLabel>Reputation & transport</SectionLabel>
          <CheckList checks={data.checks} compact />
        </Card>
        <Card>
          <SectionLabel>DMARC policy</SectionLabel>
          <DmarcLadder policy={data.dmarcPolicy} />
        </Card>
      </div>

      <QueueCard queue={data.queue} />
    </Page>
  );
}

function QueueCard({ queue }: { queue: QueueSummary }) {
  return (
    <section className="mt-8" aria-labelledby="queue-title">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 id="queue-title" className="text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
          Outgoing queue
        </h2>
        <p className="text-xs tabular-nums text-muted-foreground">
          <span className="text-foreground/90">{fmtNumber(queue.total)}</span> queued ·{" "}
          <span className="text-foreground/90">{fmtNumber(queue.deferred)}</span> deferred
          {queue.deferred > 0 && " — retried automatically"}
        </p>
      </div>
      {queue.groups.length > 0 ? (
        <ul className="mt-3 divide-y divide-border/50 rounded-xl border border-border/60 bg-card/40">
          {queue.groups.map((g) => {
            const r = REASONS[g.reason];
            return (
              <li key={g.reason} className="flex gap-3 p-5">
                <r.icon aria-hidden strokeWidth={1.75} className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-baseline justify-between gap-2">
                    <p className="text-sm font-medium text-foreground/90">{r.label}</p>
                    <span className="text-xs tabular-nums text-muted-foreground">
                      {fmtNumber(g.count)} message{g.count === 1 ? "" : "s"}
                    </span>
                  </div>
                  <p className="mt-1.5 text-[11px] text-muted-foreground">Example server response</p>
                  <code className="mt-0.5 block break-words rounded-md border border-border/60 bg-muted/30 px-2.5 py-1.5 font-mono text-[11px] text-foreground/80">
                    {g.example}
                  </code>
                  <p className="mt-2 text-xs leading-relaxed text-foreground/85">
                    <span className="font-medium">What to do: </span>
                    {g.remedy}
                  </p>
                </div>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="mt-3 rounded-xl border border-dashed border-border/60 px-5 py-6 text-center text-xs text-muted-foreground">No deferrals. Mail is leaving as fast as it arrives.</p>
      )}
      <p className="mt-3 text-[11px] text-muted-foreground">Only envelopes and server responses are shown here — never message content.</p>
    </section>
  );
}

const LADDER: { key: "none" | "quarantine" | "reject"; label: string; text: string }[] = [
  { key: "none", label: "p=none", text: "Monitor only. Receivers report failures but still deliver them." },
  { key: "quarantine", label: "p=quarantine", text: "Mail that fails SPF and DKIM goes to spam." },
  { key: "reject", label: "p=reject", text: "Failing mail is refused outright — the strongest protection against spoofing." },
];

function DmarcLadder({ policy }: { policy: DeliverabilityResponse["dmarcPolicy"] }) {
  const current = LADDER.findIndex((s) => s.key === policy);
  return (
    <div>
      <p className="-mt-1 text-xs leading-relaxed text-muted-foreground">
        DMARC tells other servers what to do with mail that claims to be from your domain but fails its checks. Climb one step at a time,
        once reports show only your own servers sending.
      </p>
      <ol className="mt-5 space-y-0" aria-label="DMARC policy steps">
        {LADDER.map((s, i) => {
          const isCurrent = i === current;
          const passed = current >= 0 && i < current;
          return (
            <li key={s.key} className="relative flex gap-3 pb-5 last:pb-0" aria-current={isCurrent ? "step" : undefined}>
              {i < LADDER.length - 1 && (
                <span aria-hidden className={cn("absolute left-[9px] top-6 h-[calc(100%-1.25rem)] w-px", passed ? "bg-foreground/40" : "bg-border")} />
              )}
              <span
                aria-hidden
                className={cn(
                  "relative z-[1] mt-0.5 grid h-[19px] w-[19px] shrink-0 place-items-center rounded-full border",
                  isCurrent && "border-foreground bg-foreground text-background",
                  passed && "border-foreground/40 bg-card text-foreground/70",
                  !isCurrent && !passed && "border-border bg-card",
                )}
              >
                {passed ? <CheckIcon className="h-3 w-3" /> : isCurrent ? <span className="h-1.5 w-1.5 rounded-full bg-background" /> : null}
              </span>
              <div className="min-w-0">
                <p className="flex flex-wrap items-baseline gap-x-2">
                  <code className={cn("font-mono text-xs", isCurrent ? "font-medium text-foreground" : "text-foreground/75")}>{s.label}</code>
                  {isCurrent && <span className="text-[11px] text-muted-foreground">Current policy</span>}
                  {passed && <span className="sr-only">(passed)</span>}
                </p>
                <p className={cn("mt-0.5 text-xs leading-relaxed", isCurrent ? "text-foreground/85" : "text-muted-foreground")}>{s.text}</p>
              </div>
            </li>
          );
        })}
      </ol>
      <p className="mt-5 border-t border-border/50 pt-3 text-xs leading-relaxed text-foreground/85">
        {policy === "unknown"
          ? "We couldn't read a DMARC record for the primary domain yet. Publish it from the domain's DNS table."
          : policy === "reject"
            ? "You're at the top: spoofed mail claiming to be you is refused."
            : policy === "quarantine"
              ? "Next step: p=reject, once a few weeks of reports show nothing unexpected."
              : "Next step: p=quarantine, once reports show only your own servers sending."}
      </p>
    </div>
  );
}
