import type { AccessResponse, AllowedAddress, BlockedAddress, QueuedMessage, QueueRecipient, QueueResponse } from "./queue";

/**
 * In-memory queue manager + blocked/allowed addresses for `npm run dev:mock`.
 * Mirrors `src/mailqueue.rs` / `src/ipaccess.rs` (same shapes, sentences and
 * status codes). Kept per scenario in sessionStorage next to the main mock DB.
 */

type Fail = (status: number, code: string, message: string, field?: string) => never;
type Audit = (action: string, target: string | null, detail?: string | null) => void;

interface State {
  scenario: string;
  paused: string | null;
  messages: QueuedMessage[];
  blocked: BlockedAddress[];
  allowed: AllowedAddress[];
  seq: number;
}

const KEY = "yunori:mock:queue";
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const at = (offsetMs: number) => new Date(Date.now() + offsetMs).toISOString();

function rcpt(address: string, r: Partial<QueueRecipient>): QueueRecipient {
  return {
    address,
    status: "retrying",
    kind: null,
    problem: null,
    remedy: null,
    response: null,
    host: null,
    attempts: 0,
    nextRetry: null,
    expiresAt: at(3 * DAY),
    expiresAfterAttempts: null,
    ...r,
  };
}

const NETWORK_REMEDY = "If this happens for every destination, your provider probably blocks outbound port 25: switch sending to a relay.";

function seed(scenario: string): State {
  const s: State = { scenario, paused: null, messages: [], blocked: [], allowed: [], seq: 10 };
  if (scenario === "setup" || scenario === "installing" || scenario === "certificate") return s;
  s.blocked = [
    { id: "blk_1", address: "198.51.100.23", reason: "authFailure", reasonText: "Too many failed sign-ins", createdAt: at(-2 * HOUR), expiresAt: at(22 * HOUR) },
    { id: "blk_2", address: "203.0.113.200", reason: "portScanning", reasonText: "Probed the web server for known exploits", createdAt: at(-3 * DAY), expiresAt: null },
    { id: "blk_3", address: "2001:db8:bad::17", reason: "rcptToFailure", reasonText: "Too many attempts to send to addresses that don't exist", createdAt: at(-9 * HOUR), expiresAt: null },
  ];
  s.allowed = [{ id: "alw_1", address: "192.0.2.0/24", reason: "Studio office", createdAt: at(-10 * DAY) }];
  if (scenario === "degraded") {
    s.messages = ["gmail.com", "outlook.com", "icloud.com", "example.org"].map((d, i) => ({
      id: `jn93q${i}aaaaa`,
      sender: i % 2 ? "ren@example.com" : "aiko@example.com",
      size: 12_000 + i * 4_100,
      createdAt: at(-(5 + i) * HOUR),
      nextRetry: at((4 + i * 3) * MIN),
      expiresAt: at(3 * DAY - (5 + i) * HOUR),
      recipients: [
        rcpt(`friend${i}@${d}`, {
          kind: "network",
          problem: `Couldn't connect to mx.${d}: the connection was refused or timed out.`,
          remedy: NETWORK_REMEDY,
          response: "I/O error: Connection timed out (os error 110)",
          host: `mx.${d}`,
          attempts: 6 + i,
          nextRetry: at((4 + i * 3) * MIN),
          expiresAt: at(3 * DAY - (5 + i) * HOUR),
        }),
      ],
    }));
    return s;
  }
  s.messages = [
    {
      id: "jn93tqlaaaaa",
      sender: "aiko@example.com",
      size: 48_213,
      createdAt: at(-47 * MIN),
      nextRetry: at(13 * MIN),
      expiresAt: at(3 * DAY - 47 * MIN),
      recipients: [
        rcpt("partner@outlook.example", {
          kind: "other",
          problem: "outlook-com.olc.protection.outlook.example asked to try again later (greylisting, a rate limit or a reputation check).",
          remedy: "Nothing to do: it is retried automatically.",
          response: "451 4.7.650 The mail server [203.0.113.24] has been temporarily rate limited due to IP reputation",
          host: "outlook-com.olc.protection.outlook.example",
          attempts: 3,
          nextRetry: at(13 * MIN),
          expiresAt: at(3 * DAY - 47 * MIN),
        }),
        rcpt("old.colleague@example.org", {
          status: "failed",
          kind: "rejected",
          problem: "mx.example.org refused the message: the mailbox doesn't exist.",
          remedy: "If many recipients refuse mail, check DNS (SPF, DKIM, DMARC), reverse DNS and blocklists under Deliverability.",
          response: "550 5.1.1 <old.colleague@example.org>: Recipient address rejected: User unknown",
          host: "mx.example.org",
          attempts: 1,
        }),
        rcpt("team@example.net", { status: "delivered", attempts: 1, response: "250 2.0.0 Ok: queued" }),
      ],
    },
    {
      id: "jn94aa2baaaa",
      sender: "ren@example.com",
      size: 3_520,
      createdAt: at(-2 * MIN),
      nextRetry: at(1 * MIN),
      expiresAt: at(3 * DAY - 2 * MIN),
      recipients: [rcpt("hello@new-client.example", { status: "waiting", nextRetry: at(1 * MIN), expiresAt: at(3 * DAY - 2 * MIN) })],
    },
  ];
  return s;
}

let cache: State | null = null;

function load(scenario: string): State {
  if (cache && cache.scenario === scenario) return cache;
  try {
    const raw = sessionStorage.getItem(KEY);
    if (raw) {
      const s = JSON.parse(raw) as State;
      if (s.scenario === scenario) return (cache = s);
    }
  } catch {}
  return (cache = seed(scenario));
}

function save(s: State) {
  cache = s;
  try {
    sessionStorage.setItem(KEY, JSON.stringify(s));
  } catch {}
}

function view(s: State): QueueResponse {
  return { paused: !!s.paused, pausedAt: s.paused, relayed: false, messages: s.messages };
}

/** Mirrors `ipaccess::normalize_network`. */
function normalizeNetwork(input: string, fail: Fail): string {
  const v = input.trim();
  const [ip, prefix, extra] = v.split("/");
  const v4 = /^(25[0-5]|2[0-4]\d|1?\d?\d)(\.(25[0-5]|2[0-4]\d|1?\d?\d)){3}$/.test(ip ?? "");
  const v6 = !v4 && /^[0-9a-f:]+$/i.test(ip ?? "") && (ip ?? "").includes(":");
  if (!v || extra !== undefined || (!v4 && !v6)) fail(400, "invalid_address", `${v || "That"} isn't an IP address or network (like 203.0.113.7 or 203.0.113.0/24).`, "address");
  const max = v4 ? 32 : 128;
  const p = prefix === undefined ? max : Number(prefix);
  if (!Number.isInteger(p) || p < 0 || p > max) fail(400, "invalid_address", `/${prefix} isn't a valid prefix length (0–${max}).`, "address");
  if (ip === "0.0.0.0" || ip === "::") fail(400, "invalid_address", "That would match every address.", "address");
  const min = v4 ? 8 : 16;
  if (p < min) fail(400, "invalid_address", `/${p} is too broad: use a network of /${min} or narrower.`, "address");
  if (v4 && p < 32) {
    const n = ip!.split(".").reduce((a, o) => (a << 8) + Number(o), 0) >>> 0;
    const masked = (n & (p === 0 ? 0 : (0xffffffff << (32 - p)) >>> 0)) >>> 0;
    return `${[24, 16, 8, 0].map((sh) => (masked >>> sh) & 255).join(".")}/${p}`;
  }
  return p === max ? ip!.toLowerCase() : `${ip!.toLowerCase()}/${p}`;
}

/** Handle /queue… and /security/… ; `null` when the path isn't ours. */
export function queueRoute(scenario: string, method: string, path: string, body: Record<string, unknown>, audit: Audit, fail: Fail): [number, unknown?] | null {
  const ours = path === "/queue" || path.startsWith("/queue/") || path.startsWith("/security/");
  if (!ours) return null;
  const s = load(scenario);
  const seg = path.split("/").filter(Boolean);
  const done = (r: [number, unknown?]): [number, unknown?] => {
    save(s);
    return r;
  };

  if (method === "GET" && path === "/queue") return done([200, view(s)]);
  if (method === "POST" && path === "/queue/retry") {
    const id = typeof body.id === "string" ? body.id : null;
    const targets = id ? s.messages.filter((m) => m.id === id) : s.messages;
    if (id && targets.length === 0) fail(404, "not_found", "That message was not found.");
    if (!s.paused) {
      for (const m of targets)
        for (const r of m.recipients)
          if (r.status === "retrying" || r.status === "waiting") {
            r.attempts += 1;
            r.nextRetry = at(r.attempts * 5 * MIN);
            if (r.status === "waiting") r.status = "retrying";
          }
    }
    audit("queue.retry", id ?? "all", `${targets.length} message${targets.length === 1 ? "" : "s"}`);
    return done([200, { retried: targets.length }]);
  }
  if (method === "POST" && path === "/queue/pause") {
    s.paused = s.paused ?? new Date().toISOString();
    audit("queue.pause", "outbound");
    return done([200, view(s)]);
  }
  if (method === "POST" && path === "/queue/resume") {
    s.paused = null;
    audit("queue.resume", "outbound");
    return done([200, view(s)]);
  }
  if (method === "DELETE" && seg[0] === "queue" && seg[1] && seg.length === 2) {
    const m = s.messages.find((x) => x.id === decodeURIComponent(seg[1]));
    if (!m) fail(404, "not_found", "That message was not found.");
    s.messages = s.messages.filter((x) => x !== m);
    audit("queue.cancel", m!.id, `${m!.sender || "<>"} → ${m!.recipients.map((r) => r.address).join(", ")}`);
    return done([204]);
  }

  if (method === "GET" && path === "/security/access") {
    const res: AccessResponse = {
      blocked: [...s.blocked].sort((a, b) => (b.createdAt ?? "").localeCompare(a.createdAt ?? "")),
      allowed: [...s.allowed].sort((a, b) => a.address.localeCompare(b.address)),
      policy: { enabled: true, failures: 20, windowMinutes: 60, banMinutes: 1440, recommended: true },
    };
    return done([200, res]);
  }
  if (method === "DELETE" && seg[1] === "blocked" && seg[2]) {
    const b = s.blocked.find((x) => x.id === decodeURIComponent(seg[2]));
    if (!b) fail(404, "not_found", "That blocked address was not found.");
    s.blocked = s.blocked.filter((x) => x !== b);
    audit("security.unblock", b!.address, b!.reasonText);
    return done([204]);
  }
  if (method === "POST" && path === "/security/allowed") {
    const net = normalizeNetwork(typeof body.address === "string" ? body.address : "", fail);
    if (s.allowed.some((a) => a.address === net)) fail(409, "conflict", `${net} is already on the allowlist.`, "address");
    const note = typeof body.note === "string" ? [...body.note].filter((c) => c >= " ").join("").trim().slice(0, 120) || null : null;
    const a: AllowedAddress = { id: `alw_${++s.seq}`, address: net, reason: note, createdAt: new Date().toISOString() };
    s.allowed.push(a);
    audit("security.allow", net, note);
    const blocked = s.blocked.find((b) => b.address === net);
    if (blocked) {
      s.blocked = s.blocked.filter((b) => b !== blocked);
      audit("security.unblock", net, "added to the allowlist");
    }
    return done([201, a]);
  }
  if (method === "DELETE" && seg[1] === "allowed" && seg[2]) {
    const a = s.allowed.find((x) => x.id === decodeURIComponent(seg[2]));
    if (!a) fail(404, "not_found", "That allowlist entry was not found.");
    s.allowed = s.allowed.filter((x) => x !== a);
    audit("security.allow_remove", a!.address);
    return done([204]);
  }
  return null;
}
