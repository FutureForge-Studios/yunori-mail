import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import {
  CheckCircle2,
  CircleDashed,
  HelpCircle,
  KeyRound,
  Lock,
  MailX,
  Network,
  Pause,
  Play,
  RotateCw,
  ShieldX,
  Trash2,
  XCircle,
  type LucideIcon,
} from "lucide-react";
import { Button } from "@/ui/button";
import { Card } from "@/ui/card";
import { SegmentBar, StatCard } from "@/ui/data-viz";
import { Callout, Skeleton } from "@/ui/feedback";
import { Segmented } from "@/ui/segmented";
import type { Tone as UiTone } from "@/ui/tone";
import { errorMessage } from "@/api/client";
import { queueApi, type QueueProblemKind, type QueueRecipient, type QueueRecipientStatus, type QueuedMessage } from "@/api/queue";
import { ConfirmAction } from "@/components/confirm";
import { EmptyState, LoadError, Page, usePageTitle } from "@/components/kit";
import { StatusBadge, type Tone } from "@/components/status";
import { fmtBytes, fmtDateTime, fmtNumber, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";

const KINDS: Record<QueueProblemKind, { label: string; icon: LucideIcon; tone: UiTone }> = {
  auth: { label: "Sign-in", icon: KeyRound, tone: "bad" },
  tls: { label: "TLS", icon: Lock, tone: "warn" },
  network: { label: "Network", icon: Network, tone: "bad" },
  rejected: { label: "Refused by recipient", icon: ShieldX, tone: "info" },
  address: { label: "Address", icon: MailX, tone: "warn" },
  other: { label: "Deferred", icon: HelpCircle, tone: "muted" },
};

const STATUS: Record<QueueRecipientStatus, { label: string; tone: Tone; icon: LucideIcon }> = {
  waiting: { label: "Waiting", tone: "muted", icon: CircleDashed },
  retrying: { label: "Retrying", tone: "warn", icon: RotateCw },
  failed: { label: "Gave up", tone: "bad", icon: XCircle },
  delivered: { label: "Delivered", tone: "ok", icon: CheckCircle2 },
};

type Filter = "all" | "problems" | "waiting";

/** "in 4 minutes" / "4 minutes ago" for a timestamp either side of now. */
function relative(iso: string | null): string {
  if (!iso) return "—";
  const ms = new Date(iso).getTime() - Date.now();
  if (Number.isNaN(ms)) return "—";
  if (ms <= 0) return "due now";
  const min = Math.round(ms / 60_000);
  if (min < 1) return "in under a minute";
  if (min < 90) return `in ${min} minute${min === 1 ? "" : "s"}`;
  const h = Math.round(min / 60);
  if (h < 48) return `in ${h} hours`;
  return `in ${Math.round(h / 24)} days`;
}

const hasProblem = (m: QueuedMessage) => m.recipients.some((r) => r.kind !== null);

/** Outgoing mail that hasn't left yet: why, when it's tried next, and controls. */
export default function QueuePage() {
  usePageTitle("Queue");
  const { data, error, loading, reload } = useApi("queue", queueApi.list);
  const [filter, setFilter] = useState<Filter>("all");
  const [busy, setBusy] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState<QueuedMessage | null>(null);
  const [pauseOpen, setPauseOpen] = useState(false);

  const rows = useMemo(() => {
    const all = data?.messages ?? [];
    if (filter === "problems") return all.filter(hasProblem);
    if (filter === "waiting") return all.filter((m) => !hasProblem(m));
    return all;
  }, [data, filter]);

  async function run(key: string, fn: () => Promise<unknown>, ok: string) {
    setBusy(key);
    try {
      await fn();
      toast.success(ok);
      await reload();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy(null);
    }
  }

  const count = data?.messages.length ?? 0;
  const problems = data ? data.messages.filter(hasProblem).length : 0;

  return (
    <Page
      title="Queue"
      blurb={
        data
          ? count === 0
            ? "Nothing is waiting to leave this server."
            : `${fmtNumber(count)} message${count === 1 ? "" : "s"} waiting to leave this server${problems ? `, ${fmtNumber(problems)} with a delivery problem` : ""}. Each is retried automatically until it expires.`
          : "Mail waiting to leave this server."
      }
      actions={
        data && (
          <>
            <Button
              size="sm"
              variant="outline"
              disabled={count === 0 || busy !== null}
              onClick={() => void run("all", () => queueApi.retry(), data.paused ? "Queued for a retry once you resume." : "Retrying every message now.")}
            >
              <RotateCw className="h-3.5 w-3.5" />
              Retry all
            </Button>
            {data.paused ? (
              <Button size="sm" disabled={busy !== null} onClick={() => void run("pause", queueApi.resume, "Outgoing mail is flowing again.")}>
                <Play className="h-3.5 w-3.5" />
                Resume sending
              </Button>
            ) : (
              <Button size="sm" variant="outline" disabled={busy !== null} onClick={() => setPauseOpen(true)}>
                <Pause className="h-3.5 w-3.5" />
                Pause sending
              </Button>
            )}
          </>
        )
      }
    >
      {data?.paused && (
        <Callout role="status" tone="warn" icon={Pause} className="mb-5" title={`Outgoing mail is paused${data.pausedAt ? ` since ${fmtDateTime(data.pausedAt)}` : ""}.`}>
          New mail waits here and nothing is delivered to other servers until you resume. Incoming mail and delivery between mailboxes on this server are unaffected.
        </Callout>
      )}

      {data && count > 0 && <QueueSummaryRow messages={data.messages} paused={data.paused} />}

      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <Segmented
          label="Show"
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: "All", count: data ? count : undefined },
            { value: "problems", label: "With problems", count: data ? problems : undefined },
            { value: "waiting", label: "Waiting normally" },
          ]}
        />
        <Button size="sm" variant="ghost" onClick={() => void reload()} disabled={loading}>
          <RotateCw className={loading ? "h-3.5 w-3.5 animate-spin motion-reduce:animate-none" : "h-3.5 w-3.5"} />
          Refresh
        </Button>
      </div>

      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="The queue" />
      ) : !data ? (
        <div className="space-y-3">
          <Skeleton className="h-40 rounded-xl" />
          <Skeleton className="h-40 rounded-xl" />
        </div>
      ) : rows.length === 0 ? (
        <EmptyState
          icon={count === 0 ? CheckCircle2 : undefined}
          title={count === 0 ? "The queue is empty" : "Nothing matches this filter"}
          hint={count === 0 ? "Mail is leaving as fast as it arrives." : "Try another filter."}
        />
      ) : (
        <ul className="space-y-3" aria-label="Queued messages">
          {rows.map((m) => (
            <MessageCard
              key={m.id}
              m={m}
              busy={busy}
              paused={data.paused}
              onRetry={() => void run(m.id, () => queueApi.retry(m.id), data.paused ? "It will be retried once you resume." : "Retrying now.")}
              onCancel={() => setCancelling(m)}
            />
          ))}
        </ul>
      )}

      <p className="mt-4 text-xs text-muted-foreground">
        Only envelopes and server responses are shown — never message content. Reputation and DNS checks live under{" "}
        <Link to="/console/deliverability" className="underline underline-offset-2 hover:text-foreground">
          Deliverability
        </Link>
        .
      </p>

      <ConfirmAction
        open={pauseOpen}
        onOpenChange={setPauseOpen}
        title="Pause all outgoing mail?"
        description="Mail to other servers waits in the queue until you resume — useful while you fix DNS or the relay, or stop a compromised mailbox from sending. Incoming mail keeps arriving."
        confirmLabel="Pause sending"
        onConfirm={async () => {
          await queueApi.pause();
          toast.success("Outgoing mail is paused.");
          void reload();
        }}
      />

      <ConfirmAction
        open={!!cancelling}
        onOpenChange={(o) => !o && setCancelling(null)}
        title="Cancel this message?"
        description={
          cancelling
            ? `It is removed from the queue and won't reach ${cancelling.recipients.filter((r) => r.status !== "delivered").map((r) => r.address).join(", ")}. The sender is not notified.`
            : undefined
        }
        confirmLabel="Cancel delivery"
        destructive
        onConfirm={async () => {
          await queueApi.cancel(cancelling!.id);
          toast.success("Delivery cancelled.");
          void reload();
        }}
      />
    </Page>
  );
}

function MessageCard({
  m,
  busy,
  paused,
  onRetry,
  onCancel,
}: {
  m: QueuedMessage;
  busy: string | null;
  paused: boolean;
  onRetry: () => void;
  onCancel: () => void;
}) {
  const pending = m.recipients.filter((r) => r.status === "waiting" || r.status === "retrying");
  const next = pending.map((r) => r.nextRetry).filter((x): x is string => !!x).sort()[0] ?? m.nextRetry;
  return (
    <li>
      <Card padded={false}>
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 pt-4">
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-foreground">{m.sender || <span className="text-muted-foreground">Bounce notice (no sender)</span>}</p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {m.recipients.length} recipient{m.recipients.length === 1 ? "" : "s"}
              {m.size != null && ` · ${fmtBytes(m.size)}`}
              {m.createdAt && (
                <>
                  {" · queued "}
                  <time dateTime={m.createdAt} title={fmtDateTime(m.createdAt)}>
                    {timeAgo(m.createdAt)}
                  </time>
                </>
              )}
              <span className="ml-1.5 font-mono text-2xs text-muted-foreground/90">{m.id}</span>
            </p>
          </div>
          <div className="flex items-center gap-1">
            <Button size="sm" variant="ghost" onClick={onRetry} disabled={busy !== null || pending.length === 0} aria-label={`Retry delivery of the message from ${m.sender || "the bounce notice"} now`}>
              <RotateCw className={busy === m.id ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
              Retry now
            </Button>
            <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy !== null} aria-label={`Cancel delivery of the message from ${m.sender || "the bounce notice"}`} className="hover:text-danger-ink">
              <Trash2 className="h-3.5 w-3.5" />
              Cancel
            </Button>
          </div>
        </div>

        <dl className="mx-5 mt-3 grid grid-cols-2 gap-x-6 gap-y-1 rounded-lg bg-muted/50 px-3 py-2 text-xs sm:grid-cols-3">
          <div>
            <dt className="text-muted-foreground">Next attempt</dt>
            <dd className="font-medium text-foreground">{pending.length === 0 ? "—" : paused ? "Paused" : <time dateTime={next ?? undefined} title={fmtDateTime(next)}>{relative(next)}</time>}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Gives up</dt>
            <dd className="font-medium text-foreground">{m.expiresAt ? <time dateTime={m.expiresAt}>{fmtDateTime(m.expiresAt)}</time> : pending[0]?.expiresAfterAttempts ? `after ${pending[0].expiresAfterAttempts} attempts` : "—"}</dd>
          </div>
        </dl>

        <ul className="mt-4 divide-y divide-border/60 border-t border-border/70" aria-label="Recipients">
          {m.recipients.map((r) => (
            <RecipientRow key={r.address} r={r} />
          ))}
        </ul>
      </Card>
    </li>
  );
}

function RecipientRow({ r }: { r: QueueRecipient }) {
  const s = STATUS[r.status];
  const k = r.kind ? KINDS[r.kind] : null;
  return (
    <li className="px-5 py-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="min-w-0 truncate text-[13px] font-medium text-foreground">{r.address}</p>
        <div className="flex items-center gap-2">
          {r.attempts > 0 && (
            <span className="text-xs tabular-nums text-muted-foreground">
              {r.attempts} attempt{r.attempts === 1 ? "" : "s"}
            </span>
          )}
          <StatusBadge tone={s.tone} icon={s.icon}>
            {s.label}
          </StatusBadge>
        </div>
      </div>
      {r.problem && (
        <div className="mt-2 flex gap-2.5">
          {k && <k.icon aria-hidden strokeWidth={1.75} className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />}
          <div className="min-w-0 flex-1 text-[13px] leading-relaxed">
            <p className="text-foreground/85">
              {k && <span className="sr-only">{k.label}: </span>}
              {r.problem}
            </p>
            {r.remedy && (
              <p className="mt-1 text-muted-foreground">
                <span className="font-medium text-foreground/80">What to do: </span>
                {r.remedy}
              </p>
            )}
            {r.response && (
              <details className="mt-1.5 group">
                <summary className="cursor-pointer rounded-sm text-xs text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60">What the server said{r.host ? ` (${r.host})` : ""}</summary>
                <code className="mt-1 block break-words rounded-lg border border-border bg-muted/40 px-3 py-2 font-mono text-xs text-foreground/85">{r.response}</code>
              </details>
            )}
          </div>
        </div>
      )}
    </li>
  );
}

/** Counts and the problem mix across every queued recipient. */
function QueueSummaryRow({ messages, paused }: { messages: QueuedMessage[]; paused: boolean }) {
  const recipients = messages.flatMap((m) => m.recipients).filter((r) => r.status !== "delivered");
  const byKind = new Map<QueueProblemKind, number>();
  for (const r of recipients) if (r.kind) byKind.set(r.kind, (byKind.get(r.kind) ?? 0) + 1);
  const withProblem = messages.filter(hasProblem).length;
  return (
    <div className="mb-6 grid gap-3 sm:gap-4 lg:grid-cols-12">
      <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:col-span-6">
        <StatCard label="Messages waiting" value={fmtNumber(messages.length)} hint={`${fmtNumber(recipients.length)} recipient${recipients.length === 1 ? "" : "s"} to go`} />
        <StatCard
          label="With a problem"
          value={fmtNumber(withProblem)}
          hint={paused ? "Sending is paused" : withProblem ? "Retried automatically" : "All waiting normally"}
          tone={withProblem ? "warn" : undefined}
        />
      </div>
      <Card className="lg:col-span-6">
        <p className="text-[13px] font-medium text-muted-foreground">Why recipients are waiting</p>
        {byKind.size > 0 ? (
          <SegmentBar
            className="mt-4"
            label="Waiting recipients by problem"
            segments={[...byKind.entries()].map(([k, v]) => ({ key: k, label: KINDS[k].label, value: v, tone: KINDS[k].tone }))}
          />
        ) : (
          <p className="mt-4 text-[13px] text-foreground/85">No delivery problems — just waiting their turn.</p>
        )}
      </Card>
    </div>
  );
}
