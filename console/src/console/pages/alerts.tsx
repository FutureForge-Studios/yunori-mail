import { useEffect, useId, useState, type FormEvent } from "react";
import { Activity, BellRing, CheckCircle2, Eye, History as HistoryIcon, Mail, RefreshCw, Send, Webhook } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/ui/button";
import { Card, CardBody, CardHeader, SectionHeader } from "@/ui/card";
import { Skeleton } from "@/ui/feedback";
import { CheckboxRow, Input, RadioCard, Switch } from "@/ui/form";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { AlertDelivery, AlertEventKind, AlertHistoryEntry, AlertsResponse, UpdateAlertSettingsRequest, WebhookKind } from "@/api/types";
import { DataTable, FieldError, LoadError, Page, usePageTitle, type Column } from "@/components/kit";
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
        <div className="grid gap-4 xl:grid-cols-12" aria-busy="true">
          <Skeleton className="h-48 rounded-xl xl:col-span-8" />
          <Skeleton className="h-48 rounded-xl xl:col-span-4" />
          <Skeleton className="h-72 rounded-xl xl:col-span-12" />
        </div>
      ) : (
        <>
          <div className="grid gap-4 xl:grid-cols-12">
            <Card padded={false} className="xl:col-span-8 xl:self-start" aria-labelledby="al-now">
              <CardHeader
                id="al-now"
                title="Happening now"
                icon={Activity}
                divider
                actions={
                <span className="text-xs text-muted-foreground">
                  {data.evaluatedAt ? (
                    <time dateTime={data.evaluatedAt} title={fmtDateTime(data.evaluatedAt)}>
                      Checked {timeAgo(data.evaluatedAt)}
                    </time>
                  ) : (
                    "Not checked yet"
                  )}
                </span>
                }
              />
              <CardBody className="pb-2">
              {data.active.length > 0 ? (
                <AlertList alerts={data.active} />
              ) : (
                <div className="flex items-start gap-3 py-5">
                  <span aria-hidden className="grid h-9 w-9 shrink-0 place-items-center rounded-full bg-success/10">
                    <CheckCircle2 strokeWidth={2} className="h-[18px] w-[18px] text-success-ink" />
                  </span>
                  <p className="text-sm font-medium text-foreground">
                    Nothing needs your attention.
                    <span className="mt-0.5 block text-[13px] font-normal text-muted-foreground">
                      {data.evaluatedAt ? "Every check passed on the last run." : "The first check runs a few minutes after the server starts."}
                    </span>
                  </p>
                </div>
              )}
              </CardBody>
            </Card>
            <Card padded={false} className="xl:col-span-4" aria-labelledby="al-watch">
              <CardHeader id="al-watch" title="What Yunori watches" icon={Eye} description="Checked every 10 minutes." divider />
              <CardBody className="pt-1">
              <dl className="divide-y divide-border/60">
                {WATCHED.map(([k, v]) => (
                  <div key={k} className="py-2.5">
                    <dt className="text-[13px] font-medium text-foreground">{k}</dt>
                    <dd className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{v}</dd>
                  </div>
                ))}
              </dl>
              </CardBody>
            </Card>
          </div>

          <SectionHeader className="mt-10" title="Where alerts go" description="Use both: email for the record, a push for when mail itself is the problem." />
          <Channels data={data} onSaved={setData} />

          <SectionHeader className="mt-10" title="History" description="Every alert Yunori sent, and every test." />
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
    <form onSubmit={(e) => void save(e)} className="grid gap-4 lg:grid-cols-2" noValidate>
      <Card padded={false}>
        <CardHeader
          title="Email"
          icon={Mail}
          description={
            <>
              Sent from <span className="font-mono text-foreground/85">{s.sender ?? "a mailbox Yunori manages"}</span>, a mailbox Yunori creates and manages on this
              server. If the mail engine itself is down, email cannot go out, so add a webhook too.
            </>
          }
          actions={<Switch id={`${id}-email`} checked={form.emailEnabled} onCheckedChange={(v) => set("emailEnabled", v)} aria-label="Email alerts" />}
        />
        <CardBody>
        <div className="space-y-1.5">
          <label htmlFor={`${id}-recipient`} className="block text-[13px] font-medium text-foreground/90">
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
          <p id={`${id}-recipient-hint`} className="text-xs leading-relaxed text-muted-foreground">
            Leave empty for the admin address from setup{s.defaultRecipient ? ` (${s.defaultRecipient})` : ""}. Use an address outside this server.
          </p>
          <FieldError id={`${id}-recipient-err`}>{errors.recipient}</FieldError>
        </div>
        </CardBody>
      </Card>

      <Card padded={false}>
        <CardHeader
          title="Webhook or ntfy"
          icon={Webhook}
          description="A push to your phone (ntfy) or a message in your chat or incident tool. It works even when mail does not."
          actions={<Switch id={`${id}-webhook`} checked={form.webhookEnabled} onCheckedChange={(v) => set("webhookEnabled", v)} aria-label="Webhook alerts" />}
        />
        <CardBody>
        <fieldset className={cn("grid gap-2 sm:grid-cols-2", !form.webhookEnabled && "opacity-60")} disabled={!form.webhookEnabled}>
          <legend className="mb-2 text-[13px] font-medium text-foreground/90">Kind</legend>
          <RadioCard compact name={`${id}-kind`} value="ntfy" checked={form.kind === "ntfy"} onChange={() => set("kind", "ntfy")} title="ntfy topic" description="Title, priority and text, as ntfy shows them." />
          <RadioCard compact name={`${id}-kind`} value="json" checked={form.kind === "json"} onChange={() => set("kind", "json")} title="JSON webhook" description="A POST with the alert as JSON." />
        </fieldset>
        <div className="mt-4 space-y-1.5">
          <label htmlFor={`${id}-url`} className="block text-[13px] font-medium text-foreground/90">
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
          <p id={`${id}-url-hint`} className="text-xs leading-relaxed text-muted-foreground">
            {urlHint}
          </p>
          <FieldError id={`${id}-url-err`}>{errors.webhookUrl}</FieldError>
        </div>
        <div className="mt-4 space-y-1.5">
          <label htmlFor={`${id}-token`} className="block text-[13px] font-medium text-foreground/90">
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
            <CheckboxRow
              className="pt-1 text-[13px]"
              checked={form.removeToken}
              onChange={(v) => set("removeToken", v)}
              disabled={!form.webhookEnabled || !!form.token}
              label="Remove the saved token"
            />
          )}
        </div>
        {s.webhook.hasUrl && (
          <button
            type="button"
            onClick={(e) => void save(e, true)}
            disabled={busy}
            className="mt-4 rounded-sm text-xs text-muted-foreground underline underline-offset-2 transition-colors hover:text-danger-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
          >
            Remove the saved webhook
          </button>
        )}
        </CardBody>
      </Card>

      <div className="flex flex-wrap items-center gap-3 lg:col-span-2">
        <Button type="submit" size="sm" disabled={!dirty} loading={busy}>
          {!busy && <BellRing aria-hidden className="h-4 w-4" />}
          {busy ? "Saving…" : "Save alert settings"}
        </Button>
        {!form.emailEnabled && !form.webhookEnabled && (
          <span className="text-[13px] text-muted-foreground">With both off, alerts still appear here and on the Overview.</span>
        )}
        {formError && (
          <p role="alert" className="text-[13px] text-danger-ink">
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
        <time dateTime={e.at} title={fmtDateTime(e.at)} className="whitespace-nowrap text-[13px] text-muted-foreground">
          {timeAgo(e.at)}
        </time>
      ),
    },
    {
      key: "kind",
      header: "Event",
      mobile: "aside",
      cell: (e) => <StatusBadge tone={EVENT_META[e.kind].tone}>{EVENT_META[e.kind].label}</StatusBadge>,
    },
    {
      key: "title",
      header: "Alert",
      mobile: "title",
      cell: (e) => (
        <div className="min-w-0">
          <p className="text-[13px] font-medium text-foreground">{e.title}</p>
          <p className="text-xs leading-relaxed text-muted-foreground">{e.summary}</p>
        </div>
      ),
    },
    {
      key: "delivery",
      header: "Delivery",
      hideBelowMd: true,
      cell: (e) =>
        e.deliveries.length === 0 ? (
          <span className="text-xs text-muted-foreground">Shown here only</span>
        ) : (
          <ul className="space-y-0.5">
            {e.deliveries.map((d, i) => (
              <li key={i} className={cn("text-xs", d.ok ? "text-muted-foreground" : "text-danger-ink")}>
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
      caption="Alert history"
      emptyIcon={HistoryIcon}
      emptyTitle="No alerts yet"
      emptyHint="Every alert Yunori sends, and every test, is listed here."
    />
  );
}
