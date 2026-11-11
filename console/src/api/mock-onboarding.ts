import type { Check, InstallLogLine, InstallStep, OnboardingState, PanelKind } from "./types";

/**
 * The browser onboarding for `npm run dev:mock`: the same step machine as
 * `src/installer/mod.rs` (inputs → sending → host_record → certificate → dns →
 * ptr → first_mailbox → done), with the internet simulated by timers so the
 * whole wizard can be clicked through. The main mock (./mock.ts) owns the
 * domains, mailboxes, sending and certificate; it hands this module hooks.
 */

export interface MockOnboarding {
  /** Where the console was opened (chosen in the terminal). */
  panelHost: string;
  identity: { domain: string; hostname: string; adminEmail: string } | null;
  sendingConfirmed: boolean;
  ptrSkipped: boolean;
  /** When the mail hostname's A record "propagates" (set when the step first runs). */
  hostRecordAt: number | null;
  /** When Let's Encrypt "issues" the mail certificate. */
  certAt: number | null;
  /** When reverse DNS "appears" (the provider's panel takes a moment). */
  ptrAt: number | null;
}

export interface MockInstall {
  steps: InstallStep[];
  logs: InstallLogLine[];
  ptrStartedAt: number | null;
}

type Fail = (status: number, code: string, message: string, field?: string) => never;

export interface OnboardingHooks {
  install: () => MockInstall | null;
  complete: () => boolean;
  fail: Fail;
  audit: (action: string, target: string | null, detail?: string | null) => void;
  ownerEmail: () => string;
  ipv4: string;
  ipv6: string;
  sendingMode: () => "direct" | "relay";
  port25: () => Check;
  primary: () => { id: string; name: string } | null;
  /** Rename the placeholder domain and set the mail hostname (the engine's rename). */
  applyIdentity: (domain: string, hostname: string) => void;
  /** The A record of the mail hostname is published (e.g. through Cloudflare). */
  hostPublished: () => boolean;
  /** The host record now resolves (the mock marks it published). */
  hostPointsHere: () => void;
  /** Every required record of the primary domain resolves. */
  dnsReady: () => { ready: boolean; missing: string[] };
  /** The mail certificate exists; or a problem the person must fix (the "certificate" scenario). */
  cert: () => { valid: boolean; problem: string | null };
  issueCert: () => void;
  hasMailbox: () => boolean;
  finish: () => void;
}

export const TERMINAL_STEPS = ["preflight", "panel", "firewall", "install_engine", "bootstrap_engine", "edge_tls"];

const PTR_SKIPPED =
  "Skipped for now. Gmail and Outlook will likely reject mail sent directly from this server until reverse DNS is set (receiving is not affected).";

const iso = (msAgo = 0) => new Date(Date.now() - msAgo).toISOString();
const MIN = 60_000;

export function panelKind(host: string): PanelKind {
  if (/^[\d.]+$/.test(host) || host.includes(":")) return "ip";
  return host.endsWith(".sslip.io") ? "sslip" : "domain";
}

/** Mirrors `STEPS` in src/installer/state.rs: what the terminal leaves behind. */
export function freshSteps(panelHost: string): MockInstall {
  const t = (m: number) => iso(m * MIN);
  const step = (id: string, title: string, status: InstallStep["status"], detail: string | null, start?: number, end?: number): InstallStep => ({
    id,
    title,
    status,
    detail,
    startedAt: start === undefined ? null : t(start),
    finishedAt: end === undefined ? null : t(end),
  });
  return {
    ptrStartedAt: null,
    steps: [
      step("preflight", "Check this server", "done", "Ready, with 1 warning to review.", 9, 9),
      step("panel", "Choose where the console opens", "done", `The console opens at https://${panelHost}/console.`, 9, 9),
      step("firewall", "Open mail ports", "done", "Opened 25, 80, 443, 465, 587, 993, 4190 in ufw.", 8, 8),
      step("install_engine", "Install the mail engine", "done", "Stalwart 1.0.3 downloaded and verified; local resolver installed.", 8, 6),
      step("bootstrap_engine", "Configure the mail engine", "done", "Stalwart 1.0.3 is running.", 6, 5),
      step("edge_tls", "Start the console and HTTPS edge", "done", "The console and HTTPS edge are running on ports 80 and 443.", 5, 5),
      step("inputs", "Your mail domain", "waiting", "Choose your mail domain and the mail server's hostname in the console.", 5),
      step("sending", "How mail goes out", "pending", null),
      step("host_record", "Point the mail hostname here", "pending", null),
      step("certificate", "Get a trusted TLS certificate", "pending", null),
      step("dns", "Publish DNS records", "pending", null),
      step("ptr", "Set reverse DNS", "pending", null),
      step("first_mailbox", "Create your first mailbox", "pending", null),
      step("done", "Finish", "pending", null),
    ],
    logs: [
      { at: t(9), level: "info", step: "preflight", message: "Ready, with 1 warning to review." },
      { at: t(9), level: "info", step: "panel", message: `The console opens at https://${panelHost}/console.` },
      { at: t(6), level: "info", step: "bootstrap_engine", message: "Engine administrator admin@" + panelHost + " created" },
      { at: t(5), level: "info", step: "certificate", message: `Let's Encrypt issued a certificate for ${panelHost}.` },
      { at: t(5), level: "info", step: "edge_tls", message: "The console and HTTPS edge are running on ports 80 and 443." },
    ],
  };
}

function set(ins: MockInstall, id: string, status: InstallStep["status"], detail: string | null) {
  ins.steps = ins.steps.map((s) => {
    if (s.id !== id) return s;
    if (s.status === status && s.detail === detail) return s;
    const finished = status === "done" || status === "skipped";
    if (finished && s.status !== status) ins.logs = [...ins.logs, { at: iso(), level: status === "done" ? "info" : "warn", step: id, message: detail ?? "" }];
    return { ...s, status, detail, startedAt: s.startedAt ?? iso(), finishedAt: finished ? iso() : null };
  });
}

/** One pass of the server's gate machine (what `advance_gates` does every minute). */
export function advance(ob: MockOnboarding, h: OnboardingHooks) {
  const ins = h.install();
  if (!ins || h.complete()) return;
  const now = Date.now();
  const status = (id: string) => ins.steps.find((s) => s.id === id)?.status;
  const order = ["inputs", "sending", "host_record", "certificate", "dns", "ptr", "first_mailbox", "done"];
  for (const id of order) {
    const st = status(id);
    if (st === "done" || st === "skipped") continue;
    const host = ob.identity?.hostname ?? "";
    switch (id) {
      case "inputs":
        if (!ob.identity) return set(ins, id, "waiting", "Choose your mail domain and the mail server's hostname in the console.");
        set(ins, id, "done", `${ob.identity.domain} on ${host}; alerts go to ${ob.identity.adminEmail}.`);
        break;
      case "sending":
        if (!ob.sendingConfirmed) return set(ins, id, "waiting", "Choose how this server sends mail: directly, or through a relay provider.");
        set(ins, id, "done", h.sendingMode() === "relay" ? "Mail goes out through a relay provider." : "This server delivers mail itself.");
        break;
      case "host_record":
        ob.hostRecordAt ??= now + 12_000;
        if (now < ob.hostRecordAt && !h.hostPublished())
          return set(ins, id, "waiting", `${host} has no A record yet. At your DNS provider, add: ${host}  A  ${h.ipv4}`);
        h.hostPointsHere();
        set(ins, id, "done", `${host} → ${h.ipv4}.`);
        break;
      case "certificate": {
        const c = h.cert();
        if (c.problem) return set(ins, id, "waiting", c.problem);
        if (!c.valid) {
          ob.certAt ??= now + 9_000;
          if (now < ob.certAt) return set(ins, id, "waiting", `Requesting a certificate from Let's Encrypt for ${host}…`);
          h.issueCert();
        }
        set(ins, id, "done", `Let's Encrypt issued a certificate for ${host}, ${ob.panelHost}; it renews automatically.`);
        break;
      }
      case "dns": {
        const d = h.dnsReady();
        if (!d.ready)
          return set(
            ins,
            id,
            "waiting",
            `Publish ${d.missing.length} record${d.missing.length === 1 ? "" : "s"} at your DNS provider: ${d.missing.join(", ")}.`,
          );
        set(ins, id, "done", "All required DNS records are published.");
        break;
      }
      case "ptr":
        if (h.sendingMode() === "relay") {
          set(ins, id, "skipped", "Sending is relayed, so reverse DNS is recommended but not required.");
          break;
        }
        if (ob.ptrSkipped) {
          set(ins, id, "skipped", PTR_SKIPPED);
          break;
        }
        ob.ptrAt ??= now + 15_000;
        if (now < ob.ptrAt)
          return set(
            ins,
            id,
            "waiting",
            `Set reverse DNS for ${h.ipv4} to ${host} — In your VPS provider's control panel, under Networking → Reverse DNS. (Or switch to relay sending.)`,
          );
        set(ins, id, "done", `${h.ipv4} → ${host}`);
        break;
      case "first_mailbox":
        if (!h.hasMailbox()) return set(ins, id, "waiting", "Create your first mailbox in the console.");
        set(ins, id, "done", "The first mailbox exists.");
        break;
      case "done":
        h.finish();
        return;
    }
  }
}

function stateJson(ob: MockOnboarding, h: OnboardingHooks): OnboardingState {
  const ins = h.install();
  const kind = panelKind(ob.panelHost);
  const suggestedDomain = kind === "domain" ? ob.panelHost.split(".").slice(-2).join(".") : null;
  const p25 = h.port25();
  const primary = h.primary();
  return {
    complete: h.complete(),
    terminalDone: true,
    steps: ins?.steps ?? [],
    logs: ins?.logs ?? [],
    panel: { host: ob.panelHost, kind, url: `https://${ob.panelHost}/console` },
    identity: ob.identity,
    suggested: {
      domain: suggestedDomain,
      hostname: suggestedDomain ? `mail.${suggestedDomain}` : null,
      adminEmail: h.ownerEmail(),
    },
    sending: { confirmed: ob.sendingConfirmed, mode: h.sendingMode(), suggested: p25.status === "fail" ? "relay" : "direct", port25: p25 },
    server: { ipv4: h.ipv4, ipv6: h.ipv6 },
    mailHostname: ob.identity?.hostname ?? ob.panelHost,
    primaryDomain: primary?.name ?? "",
    primaryDomainId: primary?.id ?? null,
    ptrSkipped: ob.ptrSkipped,
  };
}

const DOMAIN_RE = /^(?=.{1,253}$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/i;
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** The onboarding endpoints. Returns null for any other path. */
export function onboardingRoute(
  ob: MockOnboarding,
  h: OnboardingHooks,
  method: string,
  path: string,
  body: Record<string, unknown>,
): [number, unknown] | null {
  const is = (m: string, p: string) => method === m && path === p;
  if (is("GET", "/onboarding")) {
    advance(ob, h);
    return [200, stateJson(ob, h)];
  }
  if (is("POST", "/onboarding/check")) {
    advance(ob, h);
    return [200, stateJson(ob, h)];
  }
  if (is("PUT", "/onboarding/ptr")) {
    if (h.complete()) h.fail(409, "conflict", "Setup is already complete; change this under Domains, Sending or System.");
    ob.ptrSkipped = body.skip === true;
    const ins = h.install();
    if (ins && !ob.ptrSkipped && ins.steps.find((s) => s.id === "ptr")?.status === "skipped") set(ins, "ptr", "pending", null);
    h.audit(ob.ptrSkipped ? "onboarding.ptr_skipped" : "onboarding.ptr_unskipped", null);
    advance(ob, h);
    return [200, stateJson(ob, h)];
  }
  if (is("PUT", "/onboarding/identity")) {
    if (h.complete()) h.fail(409, "conflict", "Setup is already complete; change this under Domains, Sending or System.");
    const s = (k: string) => (typeof body[k] === "string" ? (body[k] as string) : "").trim().toLowerCase().replace(/\.$/, "");
    const domain = s("domain");
    const hostname = s("hostname");
    const adminEmail = s("adminEmail");
    if (!DOMAIN_RE.test(domain)) h.fail(400, "invalid", `'${domain}' doesn't look like a domain. Try something like example.com.`, "domain");
    if (!DOMAIN_RE.test(hostname)) h.fail(400, "invalid", `'${hostname}' is not a valid hostname.`, "hostname");
    if (!EMAIL_RE.test(adminEmail)) h.fail(400, "invalid", `'${adminEmail}' is not an email address.`, "adminEmail");
    if (adminEmail.endsWith(`@${domain}`))
      h.fail(
        400,
        "invalid",
        `Use an address that does not depend on this server (not @${domain}): certificate and alert mail must reach you even if this server is down.`,
        "adminEmail",
      );
    const renamed = !!ob.identity && (ob.identity.domain !== domain || ob.identity.hostname !== hostname);
    h.applyIdentity(domain, hostname);
    ob.identity = { domain, hostname, adminEmail };
    const ins = h.install();
    if (ins) {
      set(ins, "inputs", "done", `${domain} on ${hostname}; alerts go to ${adminEmail}.`);
      if (renamed) {
        for (const id of ["host_record", "certificate", "dns", "ptr"]) set(ins, id, "pending", null);
        ob.hostRecordAt = null;
        ob.certAt = null;
        ob.ptrAt = null;
      }
    }
    h.audit("onboarding.identity", domain, `hostname ${hostname}`);
    advance(ob, h);
    return [200, stateJson(ob, h)];
  }
  return null;
}

/** PUT /sending during setup answers the sending step. */
export function sendingAnswered(ob: MockOnboarding | undefined, h: OnboardingHooks) {
  if (!ob || h.complete()) return;
  ob.sendingConfirmed = true;
  const ins = h.install();
  if (ins) set(ins, "sending", "pending", null);
  advance(ob, h);
}
