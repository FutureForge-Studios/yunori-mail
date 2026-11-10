import { useEffect, useId, useState, type FormEvent } from "react";
import { BellRing, CheckCircle2, RefreshCw, Send } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Switch } from "@/design/vendor/ui/switch";
import { Card, SectionLabel, Skeleton } from "@/design/vendor/admin/ui";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { AlertDelivery, AlertEventKind, AlertHistoryEntry, AlertsResponse, UpdateAlertSettingsRequest, WebhookKind } from "@/api/types";
import { DataTable, FieldError, LoadError, Page, RadioRow, usePageTitle, type Column } from "@/components/kit";
import { StatusBadge, type Tone } from "@/components/status";
import { fmtDateTime, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { AlertList } from "../alert-list";

const EVENT_META: Record<AlertEventKind, { label: string; tone: Tone }> = {
  firing: { label: "Started", tone: "warn" },
  worse: { label: "Got worse", tone: "bad" },
  reminder: { label: "Reminder", tone: "warn" },
  resolved: { label: "Resolved", tone: "ok" },
  test: { label: "Test", tone: "muted" },
};

/** What the server watches, in the same words the alerts use. */
const WATCHED: [string, string][] = [
  ["Mail engine", "Stalwart stops responding."],
  ["Certificate", "Fewer than 14 days left, or renewal failing three times in a row."],
  ["Outgoing mail", "Messages waiting more than 6 hours to be delivered."],
  ["Blocklists", "This server's IP appears on Spamhaus, Barracuda or SpamCop."],
  ["DNS", "A required record that was correct now fails on public resolvers."],
  ["Disk", "The mail disk is over 90% full (critical over 97%)."],
  ["Backups", "More than 7 days since the last backup, once you have made one."],
];

interface Form {
  emailEnabled: boolean;
  recipient: string;
  webhookEnabled: boolean;
  kind: WebhookKind;
  url: string;
  token: string;
  removeToken: boolean;
}

function toForm(d: AlertsResponse): Form {
  const s = d.settings;
  return {
    emailEnabled: s.emailEnabled,
    recipient: s.recipientIsDefault ? "" : (s.recipient ?? ""),
    webhookEnabled: s.webhook.enabled,
    kind: s.webhook.kind,
    url: "",
    token: "",
    removeToken: false,
  };
}

/** Problems happening now, where alerts go, and every message sent. */
export default function AlertsPage() {
  usePageTitle("Alerts");
  const { data, error, reload, setData } = useApi("alerts", api.alerts);
  const [checking, setChecking] = useState(false);
  const [testing, setTesting] = useState(false);

  async function checkNow() {
    setChecking(true);
    try {
      const next = await api.checkAlerts();
      setData(next);
      toast.success(next.active.length === 0 ? "Checked: nothing needs your attention." : `Checked: ${next.active.length} alert${next.active.length === 1 ? "" : "s"} firing.`);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setChecking(false);
    }
  }

  async function sendTest() {
    setTesting(true);
    try {
      const r = await api.sendTestAlert();
      const failed = r.deliveries.filter((d) => !d.ok);
      if (failed.length === 0) toast.success(`Test alert sent: ${r.deliveries.map(describe).join("; ")}.`);
      else toast.error(`Not delivered: ${failed.map(describe).join("; ")}.`);
      void reload();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setTesting(false);
    }
  }

  const actions = (
    <div className="flex flex-wrap gap-2">
      <Button size="sm" variant="outline" onClick={() => void checkNow()} disabled={checking || !data}>
        <RefreshCw aria-hidden className={cn("h-4 w-4", checking && "animate-spin motion-reduce:animate-none")} />
        {checking ? "Checking…" : "Check now"}
      </Button>
      <Button size="sm" variant="outline" onClick={() => void sendTest()} disabled={testing || !data}>
        <Send aria-hidden className="h-4 w-4" />
        {testing ? "Sending…" : "Send test alert"}
      </Button>
    </div>
  );

  return (
    <Page
      title="Alerts"
      blurb="Yunori checks this server every 10 minutes and tells you when something needs you: once when a problem starts, once when it ends, and at most a daily reminder in between."
      actions={actions}
    >
      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="Alerts" />
      ) : !data ? (
        <div className="grid gap-4 lg:grid-cols-3" aria-busy="true">
          <Skeleton className="h-40 lg:col-span-2" />
          <Skeleton className="h-40" />
          <Skeleton className="h-72 lg:col-span-2" />
        </div>
      ) : (
        <>
          <div className="grid gap-4 lg:grid-cols-3">
            <Card className="lg:col-span-2">
              <div className="flex items-baseline justify-between gap-3">
                <SectionLabel>Happening now</SectionLabel>
                <span className="text-[11px] text-muted-foreground">
                  {data.evaluatedAt ? (
                    <time dateTime={data.evaluatedAt} title={fmtDateTime(data.evaluatedAt)}>
                      Checked {timeAgo(data.evaluatedAt)}
                    </time>
                  ) : (
                    "Not checked yet"
                  )}
                </span>
              </div>
              {data.active.length > 0 ? (
                <AlertList alerts={data.active} />
              ) : (
                <div className="flex items-start gap-3 py-3">
                  <CheckCircle2 aria-hidden strokeWidth={1.75} className="mt-0.5 h-4 w-4 shrink-0 text-emerald-700 dark:text-emerald-400" />
                  <p className="text-sm text-foreground/85">
                    Nothing needs your attention.
                    <span className="mt-0.5 block text-xs text-muted-foreground">
                      {data.evaluatedAt ? "Every check passed on the last run." : "The first check runs a few minutes after the server starts."}
                    </span>
                  </p>
                </div>
              )}
            </Card>
            <Card>
              <SectionLabel>What Yunori watches</SectionLabel>
              <dl className="space-y-2">
                {WATCHED.map(([k, v]) => (
                  <div key={k}>
                    <dt className="text-xs font-medium text-foreground/85">{k}</dt>
                    <dd className="text-[11px] leading-relaxed text-muted-foreground">{v}</dd>
                  </div>
                ))}
              </dl>
            </Card>
          </div>

          <Channels data={data} onSaved={setData} />

          <h2 className="mb-3 mt-8 text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">History</h2>
          <History rows={data.history} />
        </>
      )}
    </Page>
  );
}

function describe(d: AlertDelivery) {
  const name = d.channel === "email" ? "Email" : d.channel === "ntfy" ? "ntfy" : "Webhook";
  return `${name} ${d.detail}`;
}

function Channels({ data, onSaved }: { data: AlertsResponse; onSaved: (d: AlertsResponse) => void }) {
  const id = useId();
  const s = data.settings;
  const [form, setForm] = useState<Form>(() => toForm(data));
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [formError, setFormError] = useState<string | null>(null);
  useEffect(() => setForm(toForm(data)), [data]);
  const set = <K extends keyof Form>(k: K, v: Form[K]) => setForm((f) => ({ ...f, [k]: v }));

  const dirty =
    form.emailEnabled !== s.emailEnabled ||
    form.recipient.trim() !== (s.recipientIsDefault ? "" : (s.recipient ?? "")) ||
    form.webhookEnabled !== s.webhook.enabled ||
    form.kind !== s.webhook.kind ||
    form.url.trim() !== "" ||
    form.token.trim() !== "" ||
    form.removeToken;

  async function save(e: FormEvent, removeUrl = false) {
    e.preventDefault();
    setBusy(true);
    setErrors({});
    setFormError(null);
    const body: UpdateAlertSettingsRequest = {
      emailEnabled: form.emailEnabled,
      recipient: form.recipient.trim() || null,
      webhook: {
        enabled: removeUrl ? false : form.webhookEnabled,
        kind: form.kind,
        ...(removeUrl ? { url: "" } : form.url.trim() ? { url: form.url.trim() } : {}),
        ...(form.token.trim() ? { token: form.token.trim() } : form.removeToken ? { token: "" } : {}),
      },
    };
    try {
      const next = await api.updateAlertSettings(body);
      onSaved(next);
      toast.success(removeUrl ? "Webhook removed." : "Alert settings saved.");
    } catch (err) {
      if (err instanceof ApiRequestError && err.field) setErrors({ [err.field]: err.message });
      else setFormError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  const urlHint = s.webhook.hasUrl
    ? `Saved for ${s.webhook.host ?? "your receiver"}. The full URL is never shown again; type a new one to replace it.`
    : form.kind === "ntfy"
      ? "Your ntfy topic, such as https://ntfy.sh/a-long-private-topic. Anyone who knows a public topic can read it, so make it hard to guess."
      : "Yunori POSTs JSON with the alert's id, severity, summary and remedy (and a Slack-style text field).";

  return (
    <form onSubmit={(e) => void save(e)} className="mt-4 grid gap-4 lg:grid-cols-2" noValidate>
      <Card>
        <div className="flex items-start justify-between gap-3">
          <div>
            <SectionLabel>Email</SectionLabel>
            <p className="-mt-1 text-xs leading-relaxed text-muted-foreground">
              Sent from <span className="font-mono text-foreground/85">{s.sender ?? "a mailbox Yunori manages"}</span>, a mailbox Yunori creates
              and manages on this server. If the mail engine itself is down, email cannot go out, so add a webhook too.
            </p>
          </div>
          <Switch
            id={`${id}-email`}
            checked={form.emailEnabled}
            onCheckedChange={(v) => set("emailEnabled", v)}
            aria-label="Email alerts"
            className="mt-0.5"
          />
        </div>
        <div className="mt-4 space-y-1.5">
          <label htmlFor={`${id}-recipient`} className="block text-xs font-medium text-foreground/80">
            Send to
          </label>
          <Input
            id={`${id}-recipient`}
            type="email"
            value={form.recipient}
            onChange={(e) => set("recipient", e.target.value)}
            placeholder={s.defaultRecipient ?? "you@example.org"}
            disabled={!form.emailEnabled}
            aria-invalid={!!errors.recipient}
            aria-describedby={`${id}-recipient-hint ${id}-recipient-err`}
          />
          <p id={`${id}-recipient-hint`} className="text-[11px] leading-relaxed text-muted-foreground">
            Leave empty for the admin address from setup{s.defaultRecipient ? ` (${s.defaultRecipient})` : ""}. Use an address outside this server.
          </p>
          <FieldError id={`${id}-recipient-err`}>{errors.recipient}</FieldError>
        </div>
      </Card>

      <Card>
        <div className="flex items-start justify-between gap-3">
          <div>
            <SectionLabel>Webhook or ntfy</SectionLabel>
            <p className="-mt-1 text-xs leading-relaxed text-muted-foreground">
              A push to your phone (ntfy) or a message in your chat or incident tool. It works even when mail does not.
            </p>
          </div>
          <Switch
            id={`${id}-webhook`}
            checked={form.webhookEnabled}
            onCheckedChange={(v) => set("webhookEnabled", v)}
            aria-label="Webhook alerts"
            className="mt-0.5"
          />
        </div>
        <fieldset className="mt-4 space-y-2" disabled={!form.webhookEnabled}>
          <legend className="mb-1 text-xs font-medium text-foreground/80">Kind</legend>
          <RadioRow name={`${id}-kind`} checked={form.kind === "ntfy"} onChange={() => set("kind", "ntfy")} label="ntfy topic" hint="Title, priority and text, as ntfy shows them." />
          <RadioRow name={`${id}-kind`} checked={form.kind === "json"} onChange={() => set("kind", "json")} label="JSON webhook" hint="A POST with the alert as JSON." />
        </fieldset>
        <div className="mt-4 space-y-1.5">
          <label htmlFor={`${id}-url`} className="block text-xs font-medium text-foreground/80">
            URL
          </label>
          <Input
            id={`${id}-url`}
            type="url"
            inputMode="url"
            autoComplete="off"
            spellCheck={false}
            value={form.url}
            onChange={(e) => set("url", e.target.value)}
            placeholder={s.webhook.hasUrl ? `https://${s.webhook.host ?? "…"}/… (saved)` : form.kind === "ntfy" ? "https://ntfy.sh/your-topic" : "https://hooks.example.com/…"}
            disabled={!form.webhookEnabled}
            aria-invalid={!!errors.webhookUrl}
            aria-describedby={`${id}-url-hint ${id}-url-err`}
          />
          <p id={`${id}-url-hint`} className="text-[11px] leading-relaxed text-muted-foreground">
            {urlHint}
          </p>
          <FieldError id={`${id}-url-err`}>{errors.webhookUrl}</FieldError>
        </div>
        <div className="mt-4 space-y-1.5">
          <label htmlFor={`${id}-token`} className="block text-xs font-medium text-foreground/80">
            Access token <span className="font-normal text-muted-foreground">(optional)</span>
          </label>
          <Input
            id={`${id}-token`}
            type="password"
            autoComplete="off"
            value={form.token}
            onChange={(e) => {
              set("token", e.target.value);
              if (e.target.value) set("removeToken", false);
            }}
            placeholder={s.webhook.hasToken ? "•••••••• (saved)" : "Sent as Authorization: Bearer …"}
            disabled={!form.webhookEnabled}
            aria-invalid={!!errors.webhookToken}
            aria-describedby={`${id}-token-err`}
          />
          <FieldError id={`${id}-token-err`}>{errors.webhookToken}</FieldError>
          {s.webhook.hasToken && (
            <label className="flex items-center gap-2 text-[11px] text-muted-foreground">
              <input
                type="checkbox"
                checked={form.removeToken}
                onChange={(e) => set("removeToken", e.target.checked)}
                disabled={!form.webhookEnabled || !!form.token}
                className="h-3.5 w-3.5 accent-foreground"
              />
              Remove the saved token
            </label>
          )}
        </div>
        {s.webhook.hasUrl && (
          <button
            type="button"
            onClick={(e) => void save(e, true)}
            disabled={busy}
            className="mt-3 rounded-sm text-[11px] text-muted-foreground underline underline-offset-2 transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            Remove the saved webhook
          </button>
        )}
      </Card>

      <div className="flex flex-wrap items-center gap-3 lg:col-span-2">
        <Button type="submit" size="sm" disabled={busy || !dirty}>
          <BellRing aria-hidden className="h-4 w-4" />
          {busy ? "Saving…" : "Save alert settings"}
        </Button>
        {!form.emailEnabled && !form.webhookEnabled && (
          <span className="text-xs text-muted-foreground">With both off, alerts still appear here and on the Overview.</span>
        )}
        {formError && (
          <p role="alert" className="text-xs text-torii dark:text-[hsl(4_80%_70%)]">
            {formError}
          </p>
        )}
      </div>
    </form>
  );
}

function History({ rows }: { rows: AlertHistoryEntry[] }) {
  const columns: Column<AlertHistoryEntry>[] = [
    {
      key: "at",
      header: "When",
      cell: (e) => (
        <time dateTime={e.at} title={fmtDateTime(e.at)} className="whitespace-nowrap text-xs text-muted-foreground">
          {timeAgo(e.at)}
        </time>
      ),
    },
    {
      key: "kind",
      header: "Event",
      cell: (e) => <StatusBadge tone={EVENT_META[e.kind].tone}>{EVENT_META[e.kind].label}</StatusBadge>,
    },
    {
      key: "title",
      header: "Alert",
      cell: (e) => (
        <div className="min-w-0">
          <p className="text-xs text-foreground/90">{e.title}</p>
          <p className="text-[11px] leading-relaxed text-muted-foreground">{e.summary}</p>
        </div>
      ),
    },
    {
      key: "delivery",
      header: "Delivery",
      hideBelowMd: true,
      cell: (e) =>
        e.deliveries.length === 0 ? (
          <span className="text-[11px] text-muted-foreground">Shown here only</span>
        ) : (
          <ul className="space-y-0.5">
            {e.deliveries.map((d, i) => (
              <li key={i} className={cn("text-[11px]", d.ok ? "text-muted-foreground" : "text-torii dark:text-[hsl(4_80%_70%)]")}>
                {d.ok ? "" : "Failed: "}
                {describe(d)}
              </li>
            ))}
          </ul>
        ),
    },
  ];
  return (
    <DataTable
      columns={columns}
      rows={rows}
      rowKey={(e) => String(e.id)}
      emptyTitle="No alerts yet"
      emptyHint="Every alert Yunori sends, and every test, is listed here."
    />
  );
}
