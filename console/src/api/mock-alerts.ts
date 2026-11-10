/**
 * Mock of the alerts endpoints (GET /alerts, PUT /alerts/settings,
 * POST /alerts/test, POST /alerts/check) for `npm run dev:mock`. Kept apart
 * from ./mock.ts; its state lives on the mock DB as `alerts`.
 *
 * Scenarios: "healthy" has quiet history; "degraded" (port 25 blocked) has a
 * stuck-queue alert firing, as the real server would after six hours.
 */
import type { ActiveAlert, AlertDelivery, AlertHistoryEntry, AlertsResponse, WebhookKind } from "./types";

export interface MockAlerts {
  active: ActiveAlert[];
  history: AlertHistoryEntry[];
  evaluatedAt: string;
  emailEnabled: boolean;
  recipient: string | null;
  webhook: { enabled: boolean; kind: WebhookKind; host: string | null; url: string | null; token: string | null };
}

type Fail = (status: number, code: string, message: string, field?: string) => never;
type Audit = (action: string, target: string | null, detail?: string | null) => void;

const HOST = "mail.example.com";
const DEFAULT_RECIPIENT = "aiko@studio-mail.example";
const SENDER = "yunori-alerts@example.com";
const HOSTED = ["example.com", "example.net"];
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const iso = (msAgo = 0) => new Date(Date.now() - msAgo).toISOString();

const STUCK: Omit<ActiveAlert, "since" | "lastNotified"> = {
  id: "queue",
  severity: "warning",
  title: "Outgoing mail is stuck",
  summary: "14 outgoing messages have been waiting more than 6 hours to be delivered.",
  remedy: "Other servers could not be reached — often a blocked outbound port 25. Switch sending to a relay.",
  notifiedSeverity: "warning",
  clearTicks: 0,
};

let seq = 50;
function entry(msAgo: number, e: Omit<AlertHistoryEntry, "id" | "at">): AlertHistoryEntry {
  seq += 1;
  return { id: seq, at: iso(msAgo), ...e };
}
const sent = (detail = `sent to ${DEFAULT_RECIPIENT}`): AlertDelivery[] => [{ channel: "email", ok: true, detail }];

export function seedAlerts(scenario: string): MockAlerts {
  const history: AlertHistoryEntry[] = [
    entry(3 * DAY, {
      alertId: "certificate.renewal",
      kind: "resolved",
      severity: "warning",
      title: "Certificate renewal failing",
      summary: "Automatic certificate renewal has failed 3 times in a row: Let's Encrypt could not reach mail.example.com on port 80.",
      deliveries: sent(),
    }),
    entry(3 * DAY + 5 * HOUR, {
      alertId: "certificate.renewal",
      kind: "firing",
      severity: "warning",
      title: "Certificate renewal failing",
      summary: "Automatic certificate renewal has failed 3 times in a row: Let's Encrypt could not reach mail.example.com on port 80.",
      deliveries: sent(),
    }),
    entry(12 * DAY, {
      alertId: "test",
      kind: "test",
      severity: "warning",
      title: "Test alert",
      summary: `This is a test alert from Yunori on ${HOST}, sent from the console.`,
      deliveries: sent(),
    }),
  ];
  const base: MockAlerts = {
    active: [],
    history,
    evaluatedAt: iso(4 * MIN),
    emailEnabled: true,
    recipient: null,
    webhook: { enabled: false, kind: "ntfy", host: null, url: null, token: null },
  };
  if (scenario !== "degraded") return base;
  return {
    ...base,
    active: [{ ...STUCK, since: iso(7 * HOUR), lastNotified: iso(HOUR) }],
    history: [
      entry(HOUR, { alertId: "queue", kind: "firing", severity: "warning", title: STUCK.title, summary: STUCK.summary, deliveries: sent() }),
      ...history,
    ],
  };
}

function view(a: MockAlerts): AlertsResponse {
  return {
    active: a.active,
    history: a.history.slice(0, 100),
    evaluatedAt: a.evaluatedAt,
    settings: {
      emailEnabled: a.emailEnabled,
      recipient: a.recipient ?? DEFAULT_RECIPIENT,
      recipientIsDefault: a.recipient === null,
      defaultRecipient: DEFAULT_RECIPIENT,
      sender: SENDER,
      webhook: { enabled: a.webhook.enabled, kind: a.webhook.kind, host: a.webhook.host, hasUrl: !!a.webhook.url, hasToken: !!a.webhook.token },
    },
  };
}

function deliveries(a: MockAlerts): AlertDelivery[] {
  const out: AlertDelivery[] = [];
  if (a.emailEnabled) out.push({ channel: "email", ok: true, detail: `sent to ${a.recipient ?? DEFAULT_RECIPIENT}` });
  if (a.webhook.enabled) {
    const failing = a.webhook.host?.includes("fail");
    out.push({
      channel: a.webhook.kind === "ntfy" ? "ntfy" : "webhook",
      ok: !failing,
      detail: failing ? "the receiver answered HTTP 404" : "delivered (HTTP 200)",
    });
  }
  return out;
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** The alerts routes; `null` when the path is not one of them. */
export function routeAlerts(
  a: MockAlerts,
  method: string,
  path: string,
  body: Record<string, unknown>,
  fail: Fail,
  audit: Audit,
): [number, unknown] | null {
  if (method === "GET" && path === "/alerts") return [200, view(a)];

  if (method === "PUT" && path === "/alerts/settings") {
    const recipient = typeof body.recipient === "string" ? body.recipient.trim().toLowerCase() : "";
    if (recipient && !EMAIL_RE.test(recipient)) fail(400, "invalid_request", "Enter a valid email address.", "recipient");
    if (recipient && HOSTED.includes(recipient.split("@")[1] ?? ""))
      fail(400, "invalid_request", "Use an address outside this server: if this server has a problem, mail to it may not arrive.", "recipient");
    const w = (body.webhook ?? {}) as { enabled?: boolean; kind?: WebhookKind; url?: string; token?: string };
    const next = { ...a.webhook, enabled: !!w.enabled, kind: w.kind === "ntfy" ? "ntfy" : ("json" as WebhookKind) };
    if (typeof w.url === "string") {
      const raw = w.url.trim();
      if (raw === "") Object.assign(next, { url: null, host: null, token: null });
      else {
        let u: URL;
        try {
          u = new URL(raw);
        } catch {
          return fail(400, "invalid_request", "Enter a full URL, such as https://ntfy.sh/your-topic.", "webhookUrl");
        }
        if (u.protocol !== "https:" && u.protocol !== "http:")
          fail(400, "invalid_request", "The URL must start with https:// (or http:// on your own network).", "webhookUrl");
        if (raw !== next.url && w.token === undefined) next.token = null;
        Object.assign(next, { url: raw, host: u.hostname });
      }
    }
    if (typeof w.token === "string") next.token = w.token.trim() || null;
    if (next.enabled && !next.url) fail(400, "invalid_request", "Enter the URL to send alerts to.", "webhookUrl");
    a.emailEnabled = !!body.emailEnabled;
    a.recipient = recipient || null;
    a.webhook = next;
    audit("alerts.settings", null, `email ${a.emailEnabled ? "on" : "off"}; ${next.kind === "ntfy" ? "ntfy" : "webhook"} ${next.enabled ? "on" : "off"}`);
    return [200, view(a)];
  }

  if (method === "POST" && path === "/alerts/test") {
    if (!a.emailEnabled && !a.webhook.enabled) fail(409, "conflict", "Turn on email or a webhook first, then send a test.");
    const d = deliveries(a);
    a.history = [
      entry(0, { alertId: "test", kind: "test", severity: "warning", title: "Test alert", summary: `This is a test alert from Yunori on ${HOST}, sent from the console.`, deliveries: d }),
      ...a.history,
    ];
    audit("alert.test", "test", d.map((x) => `${x.channel}: ${x.detail}`).join("; "));
    return [200, { deliveries: d }];
  }

  if (method === "POST" && path === "/alerts/check") {
    a.evaluatedAt = iso();
    audit("alerts.check", null, "0 change(s)");
    return [200, view(a)];
  }

  return null;
}
