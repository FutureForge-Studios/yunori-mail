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
import { Button } from "@/design/vendor/ui/button";
import { Card, Skeleton } from "@/design/vendor/admin/ui";
import { errorMessage } from "@/api/client";
import { queueApi, type QueueProblemKind, type QueueRecipient, type QueueRecipientStatus, type QueuedMessage } from "@/api/queue";
import { ConfirmAction } from "@/components/confirm";
import { EmptyState, FilterPills, LoadError, Page, usePageTitle } from "@/components/kit";
import { StatusBadge, type Tone } from "@/components/status";
import { fmtBytes, fmtDateTime, fmtNumber, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";

const KINDS: Record<QueueProblemKind, { label: string; icon: LucideIcon }> = {
  auth: { label: "Sign-in", icon: KeyRound },
  tls: { label: "TLS", icon: Lock },
  network: { label: "Network", icon: Network },
  rejected: { label: "Refused by recipient", icon: ShieldX },
  address: { label: "Address", icon: MailX },
  other: { label: "Deferred", icon: HelpCircle },
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
        <div role="status" className="mb-5 flex flex-wrap items-start gap-3 rounded-xl border border-amber-600/30 bg-amber-500/5 px-5 py-4">
          <Pause aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-amber-700 dark:text-amber-400" />
          <div className="min-w-0 flex-1 text-xs leading-relaxed">
            <p className="text-sm font-medium text-foreground/90">Outgoing mail is paused{data.pausedAt ? ` since ${fmtDateTime(data.pausedAt)}` : ""}.</p>
            <p className="mt-0.5 text-muted-foreground">New mail waits here and nothing is delivered to other servers until you resume. Incoming mail and delivery between mailboxes on this server are unaffected.</p>
          </div>
        </div>
      )}

      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <FilterPills
          label="Show"
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: `All${data ? ` · ${fmtNumber(count)}` : ""}` },
            { value: "problems", label: `With problems${data ? ` · ${fmtNumber(problems)}` : ""}` },
            { value: "waiting", label: "Waiting normally" },
          ]}
        />
        <Button size="sm" variant="ghost" onClick={() => void reload()} disabled={loading}>
          <RotateCw className="h-3.5 w-3.5" />
          Refresh
        </Button>
      </div>

      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="The queue" />
      ) : !data ? (
        <div className="space-y-3">
          <Skeleton className="h-36" />
          <Skeleton className="h-36" />
        </div>
      ) : rows.length === 0 ? (
        <EmptyState
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

      <p className="mt-4 text-[11px] text-muted-foreground">
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
      <Card className="p-0">
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 pt-4">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium text-foreground/90">{m.sender || <span className="text-muted-foreground">Bounce notice (no sender)</span>}</p>
            <p className="mt-0.5 text-[11px] text-muted-foreground">
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
              <span className="ml-1 font-mono text-[10px] text-muted-foreground/80">{m.id}</span>
            </p>
          </div>
          <div className="flex items-center gap-1">
            <Button size="sm" variant="ghost" onClick={onRetry} disabled={busy !== null || pending.length === 0} aria-label={`Retry delivery of the message from ${m.sender || "the bounce notice"} now`}>
              <RotateCw className={busy === m.id ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
              Retry now
            </Button>
            <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy !== null} aria-label={`Cancel delivery of the message from ${m.sender || "the bounce notice"}`} className="hover:text-destructive">
              <Trash2 className="h-3.5 w-3.5" />
              Cancel
            </Button>
          </div>
        </div>

        <dl className="mt-3 grid grid-cols-2 gap-x-6 gap-y-1 px-5 text-[11px] sm:grid-cols-3">
          <div>
            <dt className="text-muted-foreground">Next attempt</dt>
            <dd className="text-foreground/85">{pending.length === 0 ? "—" : paused ? "Paused" : <time dateTime={next ?? undefined} title={fmtDateTime(next)}>{relative(next)}</time>}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Gives up</dt>
            <dd className="text-foreground/85">{m.expiresAt ? <time dateTime={m.expiresAt}>{fmtDateTime(m.expiresAt)}</time> : pending[0]?.expiresAfterAttempts ? `after ${pending[0].expiresAfterAttempts} attempts` : "—"}</dd>
          </div>
        </dl>

        <ul className="mt-3 divide-y divide-border/50 border-t border-border/50" aria-label="Recipients">
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
        <p className="min-w-0 truncate text-xs text-foreground/90">{r.address}</p>
        <div className="flex items-center gap-2">
          {r.attempts > 0 && (
            <span className="text-[11px] tabular-nums text-muted-foreground">
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
          <div className="min-w-0 flex-1 text-xs leading-relaxed">
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
                <summary className="cursor-pointer text-[11px] text-muted-foreground hover:text-foreground">What the server said{r.host ? ` (${r.host})` : ""}</summary>
                <code className="mt-1 block break-words rounded-md border border-border/60 bg-muted/30 px-2.5 py-1.5 font-mono text-[11px] text-foreground/80">{r.response}</code>
              </details>
            )}
          </div>
        </div>
      )}
    </li>
  );
}
