import { Link } from "react-router-dom";
import { ArrowRight, HelpCircle, KeyRound, ListOrdered, Lock, MailCheck, Network, ShieldCheck, ShieldX, type LucideIcon } from "lucide-react";
import { api } from "@/api/client";
import type { QueueSummary } from "@/api/types";
import { LoadError, Page, usePageTitle } from "@/components/kit";
import { CheckList } from "@/components/status";
import { fmtNumber, timeAgo } from "@/lib/format";
import { useApi } from "@/lib/use-api";
import { Button } from "@/ui/button";
import { Card, CardBody, CardHeader } from "@/ui/card";
import { RingMeter, SegmentBar, StatCard } from "@/ui/data-viz";
import { EmptyState, Skeleton } from "@/ui/feedback";
import { Step, Steps } from "@/ui/steps";
import type { Tone } from "@/ui/tone";

const REASONS: Record<QueueSummary["groups"][number]["reason"], { label: string; icon: LucideIcon; tone: Tone }> = {
  auth: { label: "Authentication", icon: KeyRound, tone: "bad" },
  tls: { label: "TLS", icon: Lock, tone: "warn" },
  network: { label: "Network", icon: Network, tone: "bad" },
  rejected: { label: "Rejected by recipient", icon: ShieldX, tone: "info" },
  other: { label: "Other", icon: HelpCircle, tone: "muted" },
};

const LADDER: { key: "none" | "quarantine" | "reject"; label: string; text: string }[] = [
  { key: "none", label: "p=none", text: "Monitor only. Receivers report failures but still deliver them." },
  { key: "quarantine", label: "p=quarantine", text: "Mail that fails SPF and DKIM goes to spam." },
  { key: "reject", label: "p=reject", text: "Failing mail is refused outright — the strongest protection against spoofing." },
];

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
        <div className="grid gap-4 sm:grid-cols-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={i} className="h-28 rounded-xl" />
          ))}
        </div>
        <div className="mt-4 grid gap-4 xl:grid-cols-3">
          <Skeleton className="h-80 rounded-xl xl:col-span-2" />
          <Skeleton className="h-80 rounded-xl" />
        </div>
      </Page>
    );
  }

  const passing = data.checks.filter((c) => c.status === "pass").length;
  const tone: Tone = data.checks.some((c) => c.status === "fail") ? "bad" : data.checks.some((c) => c.status === "warn") ? "warn" : "ok";
  const dmarcStep = LADDER.findIndex((s) => s.key === data.dmarcPolicy);

  return (
    <Page
      title="Deliverability"
      blurb={`Whether your mail reaches inboxes rather than spam folders${data.checkedAt ? ` — checked ${timeAgo(data.checkedAt)}` : ""}.`}
      actions={
        <Button asChild variant="outline" size="sm">
          <Link to="/console/queue">
            <ListOrdered className="h-4 w-4" />
            Open the queue
          </Link>
        </Button>
      }
    >
      <div className="grid gap-3 sm:grid-cols-3 sm:gap-4">
        <StatCard
          label="Reputation checks"
          value={
            <>
              {passing}
              <span className="text-lg text-muted-foreground">/{data.checks.length}</span>
            </>
          }
          hint={tone === "ok" ? "Everything passes" : "Something needs a look"}
          tone={tone === "ok" ? undefined : tone}
        >
          <RingMeter
            value={data.checks.length ? (passing / data.checks.length) * 100 : null}
            tone={tone}
            label="Reputation checks passing"
            valueText={`${passing} of ${data.checks.length}`}
            size={44}
            stroke={5}
            center={<span />}
          />
        </StatCard>
        <StatCard
          label="DMARC policy"
          icon={ShieldCheck}
          value={<span className="font-mono text-2xl">{data.dmarcPolicy === "unknown" ? "—" : `p=${data.dmarcPolicy}`}</span>}
          hint={dmarcStep >= 0 ? `Step ${dmarcStep + 1} of ${LADDER.length}` : "Not readable yet"}
        />
        <StatCard
          to="/console/queue"
          label="Outgoing queue"
          value={fmtNumber(data.queue.total)}
          hint={data.queue.deferred > 0 ? `${fmtNumber(data.queue.deferred)} deferred — retried automatically` : "Nothing deferred"}
          tone={data.queue.deferred > 0 ? "warn" : undefined}
        />
      </div>

      <div className="mt-4 grid gap-4 xl:grid-cols-12">
        <Card padded={false} className="xl:col-span-8" aria-labelledby="dl-checks">
          <CardHeader id="dl-checks" title="Reputation & transport" icon={MailCheck} description="Reverse DNS, blocklists, TLS and authentication — worst first." divider />
          <CardBody className="pb-2">
            <CheckList checks={data.checks} compact />
          </CardBody>
        </Card>
        <Card padded={false} className="xl:col-span-4" aria-labelledby="dl-dmarc">
          <CardHeader
            id="dl-dmarc"
            title="DMARC policy"
            icon={ShieldCheck}
            description="What other servers do with mail that claims to be from your domain but fails its checks. Climb one step at a time."
          />
          <CardBody>
            <Steps label="DMARC policy steps">
              {LADDER.map((s, i) => (
                <Step
                  key={s.key}
                  index={i + 1}
                  last={i === LADDER.length - 1}
                  state={i === dmarcStep ? "current" : dmarcStep >= 0 && i < dmarcStep ? "done" : "pending"}
                  title={<code className="font-mono text-[13px]">{s.label}</code>}
                  meta={i === dmarcStep ? "Current policy" : undefined}
                >
                  {s.text}
                </Step>
              ))}
            </Steps>
            <p className="mt-5 rounded-lg bg-muted/60 px-3 py-2.5 text-[13px] leading-relaxed text-foreground/90">
              {data.dmarcPolicy === "unknown"
                ? "We couldn't read a DMARC record for the primary domain yet. Publish it from the domain's DNS table."
                : data.dmarcPolicy === "reject"
                  ? "You're at the top: spoofed mail claiming to be you is refused."
                  : data.dmarcPolicy === "quarantine"
                    ? "Next step: p=reject, once a few weeks of reports show nothing unexpected."
                    : "Next step: p=quarantine, once reports show only your own servers sending."}
            </p>
          </CardBody>
        </Card>
      </div>

      <QueueCard queue={data.queue} />
    </Page>
  );
}

function QueueCard({ queue }: { queue: QueueSummary }) {
  return (
    <Card padded={false} className="mt-4" aria-labelledby="queue-title">
      <CardHeader
        id="queue-title"
        title="Why mail is waiting"
        icon={ListOrdered}
        description="Only envelopes and server responses are shown here — never message content."
        actions={
          <Button asChild variant="ghost" size="sm">
            <Link to="/console/queue">
              Open the queue
              <ArrowRight className="h-3.5 w-3.5" />
            </Link>
          </Button>
        }
      >
        {queue.groups.length > 0 && (
          <SegmentBar
            className="mt-5"
            label="Deferred messages by reason"
            segments={queue.groups.map((g) => ({ key: g.reason, label: REASONS[g.reason].label, value: g.count, tone: REASONS[g.reason].tone }))}
          />
        )}
      </CardHeader>
      {queue.groups.length > 0 ? (
        <ul className="divide-y divide-border/60 border-t border-border/70">
          {queue.groups.map((g) => {
            const r = REASONS[g.reason];
            return (
              <li key={g.reason} className="flex gap-3 px-5 py-4">
                <span aria-hidden className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-muted text-icon">
                  <r.icon className="h-3.5 w-3.5" strokeWidth={2} />
                </span>
                <div className="min-w-0 flex-1 pt-0.5">
                  <div className="flex flex-wrap items-baseline justify-between gap-2">
                    <p className="text-sm font-medium text-foreground">{r.label}</p>
                    <span className="text-xs tabular-nums text-muted-foreground">
                      {fmtNumber(g.count)} message{g.count === 1 ? "" : "s"}
                    </span>
                  </div>
                  <p className="mt-2 text-xs text-muted-foreground">Example server response</p>
                  <code className="mt-1 block break-words rounded-lg border border-border bg-muted/40 px-3 py-2 font-mono text-xs leading-relaxed text-foreground/85">{g.example}</code>
                  <p className="mt-2 text-[13px] leading-relaxed text-foreground/90">
                    <span className="font-medium">What to do: </span>
                    {g.remedy}
                  </p>
                </div>
              </li>
            );
          })}
        </ul>
      ) : (
        <CardBody>
          <EmptyState compact icon={MailCheck} title="No deferrals" hint="Mail is leaving as fast as it arrives." />
        </CardBody>
      )}
    </Card>
  );
}

