/**
 * In-memory mock of the yunori-mail API (every endpoint in ./types.ts).
 *
 * Loaded ONLY when the console is built/served with VITE_MOCK=1 (`npm run
 * dev:mock`, `npm run build:mock`); client.ts imports it dynamically behind a
 * build-time literal, so production bundles never contain it.
 *
 * State is consistent across endpoints (a created mailbox appears in lists,
 * counts and the portal) and survives reloads via sessionStorage. Scenarios:
 *   ?mock=setup | signedout | installing | healthy | degraded
 * (or the dev panel, bottom-left) reseed the data.
 */
import type {
  AdminUser,
  Alias,
  ApiError,
  AppPassword,
  AuditEntry,
  Check,
  CheckStatus,
  DnsRecord,
  Domain,
  InstallLogLine,
  InstallState,
  InstallStep,
  Mailbox,
  OverviewResponse,
  PortalMe,
  QueueSummary,
  RelayProvider,
  SendingResponse,
  SessionResponse,
  SystemResponse,
  VerdictLevel,
} from "./types";

export type Scenario = "setup" | "signedout" | "installing" | "healthy" | "degraded";

export const SCENARIOS: { key: Scenario; label: string; hint: string; path: string }[] = [
  { key: "setup", label: "First run", hint: "setupRequired — any token ≥ 8 chars; code: any 6 digits but 000000", path: "/console/setup" },
  { key: "signedout", label: "Signed out", hint: "aiko@example.com · any 12+ char password · any 6 digits but 000000", path: "/console/login" },
  { key: "installing", label: "Installing", hint: "DNS step waiting — press “Check now” to let it finish", path: "/console" },
  { key: "healthy", label: "Healthy", hint: "Everything passes · portal signed in as aiko@example.com", path: "/console" },
  { key: "degraded", label: "Port 25 blocked", hint: "Direct sending fails → relay recommended", path: "/console" },
];

// ── constants ────────────────────────────────────────────────────────────────

const HOST = "mail.example.com";
const IPV4 = "203.0.113.24";
const IPV6 = "2001:db8:5::24";
const CLIENT_ORIGIN = "https://app.yunori.email";
const VERSION = "0.1.0";
const ENGINE = "1.0.3";
const SETUP_DAY = "20261021";

const PROVIDERS: RelayProvider[] = [
  { key: "ses", label: "Amazon SES", host: null, port: 587, spfInclude: "amazonses.com", docsUrl: "https://docs.aws.amazon.com/ses/latest/dg/smtp-credentials.html" },
  { key: "postmark", label: "Postmark", host: "smtp.postmarkapp.com", port: 587, spfInclude: "spf.mtasv.net", docsUrl: "https://postmarkapp.com/developer/user-guide/send-email-with-smtp" },
  { key: "mailgun", label: "Mailgun", host: "smtp.mailgun.org", port: 587, spfInclude: "mailgun.org", docsUrl: "https://documentation.mailgun.com/docs/mailgun/user-manual/sending-messages/" },
  { key: "smtp2go", label: "SMTP2GO", host: "mail.smtp2go.com", port: 587, spfInclude: "spf.smtp2go.com", docsUrl: "https://support.smtp2go.com/" },
  { key: "brevo", label: "Brevo", host: "smtp-relay.brevo.com", port: 587, spfInclude: "spf.brevo.com", docsUrl: "https://help.brevo.com/" },
  { key: "resend", label: "Resend", host: "smtp.resend.com", port: 465, spfInclude: "amazonses.com", docsUrl: "https://resend.com/docs/send-with-smtp" },
  { key: "custom", label: "Another SMTP relay", host: null, port: 587, spfInclude: null, docsUrl: null },
];

// ── state ────────────────────────────────────────────────────────────────────

interface DomainRow extends Omit<Domain, "mailboxCount" | "aliasCount" | "dnsStatus"> {
  /** Which DNS record ids currently resolve correctly. */
  published: Record<string, "ok" | "missing" | "stale">;
}
interface MailboxRow extends Omit<Mailbox, "aliases" | "address"> {
  password: string | null; // known only for mailboxes created/reset in this mock session
}
interface InstallRow {
  steps: InstallStep[];
  logs: InstallLogLine[];
  ptrStartedAt: number | null;
}
interface DB {
  scenario: Scenario;
  setupRequired: boolean;
  authenticated: boolean;
  pendingSetup: { email: string; secret: string } | null;
  admins: AdminUser[];
  adminId: string | null;
  domains: DomainRow[];
  mailboxes: MailboxRow[];
  aliases: Alias[];
  sending: { mode: "direct" | "relay"; relay: { provider: string; host: string; port: number; username: string; password: string | null } | null };
  port25Open: boolean;
  queue: QueueSummary;
  install: InstallRow | null;
  installComplete: boolean;
  audit: AuditEntry[];
  portalAddress: string | null;
  appPasswords: Record<string, AppPassword[]>;
  seq: number;
}

const DB_KEY = "yunori:mock:db";
const SCENARIO_KEY = "yunori:mock:scenario";

const now = () => Date.now();
const iso = (msAgo = 0) => new Date(now() - msAgo).toISOString();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const GB = 1024 ** 3;
const MB = 1024 ** 2;

// Deterministic pseudo-random for keys; Math.random for one-time secrets.
function prng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 2 ** 32;
  };
}
function pseudoBase64(len: number, seed: number) {
  const abc = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const r = prng(seed);
  let out = "";
  for (let i = 0; i < len; i++) out += abc[Math.floor(r() * 64)];
  return out;
}
function randomFrom(alphabet: string, n: number) {
  const buf = new Uint32Array(n);
  crypto.getRandomValues(buf);
  return Array.from(buf, (v) => alphabet[v % alphabet.length]).join("");
}
function genPassword() {
  const a = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
  return [randomFrom(a, 4), randomFrom(a, 4), randomFrom(a, 4), randomFrom(a, 4)].join("-");
}
function hashOf(s: string) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

// ── seeds ────────────────────────────────────────────────────────────────────

const OWNER: AdminUser = {
  id: "adm_01",
  email: "aiko@example.com",
  role: "owner",
  totpEnabled: true,
  createdAt: iso(15 * DAY),
};

function allPublished(ids: string[]): Record<string, "ok"> {
  return Object.fromEntries(ids.map((i) => [i, "ok" as const]));
}

function seed(scenario: Scenario): DB {
  const base: DB = {
    scenario,
    setupRequired: false,
    authenticated: true,
    pendingSetup: null,
    admins: [OWNER],
    adminId: OWNER.id,
    domains: [],
    mailboxes: [],
    aliases: [],
    sending: { mode: "direct", relay: null },
    port25Open: true,
    queue: { total: 0, deferred: 0, groups: [] },
    install: null,
    installComplete: true,
    audit: [],
    portalAddress: null,
    appPasswords: {},
    seq: 100,
  };

  if (scenario === "setup") {
    return {
      ...base,
      setupRequired: true,
      authenticated: false,
      admins: [],
      adminId: null,
      installComplete: false,
      domains: [primaryDomain("installing")],
      mailboxes: [postmaster()],
      install: installingState(),
      audit: installerAudit(),
    };
  }

  if (scenario === "installing") {
    return {
      ...base,
      installComplete: false,
      domains: [primaryDomain("installing")],
      mailboxes: [postmaster()],
      install: installingState(),
      audit: [...adminAudit().slice(-1), ...installerAudit()],
    };
  }

  const healthy: DB = {
    ...base,
    domains: [primaryDomain("healthy"), secondaryDomain()],
    mailboxes: [
      { id: "mbx_aiko", local: "aiko", domainId: "dom_com", displayName: "Aiko Tanaka", role: "admin", quotaBytes: 10 * GB, usedBytes: 3.4 * GB, createdAt: iso(15 * DAY), password: null },
      { id: "mbx_ren", local: "ren", domainId: "dom_com", displayName: "Ren Ishikawa", role: "user", quotaBytes: 5 * GB, usedBytes: 1.1 * GB, createdAt: iso(14 * DAY), password: null },
      { id: "mbx_mina", local: "mina", domainId: "dom_com", displayName: "Mina Sato", role: "user", quotaBytes: 5 * GB, usedBytes: 4.62 * GB, createdAt: iso(9 * DAY), password: null },
      postmaster(),
      { id: "mbx_sam", local: "sam", domainId: "dom_net", displayName: "Sam Okafor", role: "user", quotaBytes: 2 * GB, usedBytes: 240 * MB, createdAt: iso(4 * DAY), password: null },
    ],
    aliases: [
      { address: "info@example.com", mailboxId: "mbx_aiko", mailboxAddress: "aiko@example.com", enabled: true },
      { address: "hello@example.com", mailboxId: "mbx_aiko", mailboxAddress: "aiko@example.com", enabled: true },
      { address: "billing@example.com", mailboxId: "mbx_ren", mailboxAddress: "ren@example.com", enabled: true },
      { address: "abuse@example.com", mailboxId: "mbx_postmaster", mailboxAddress: "postmaster@example.com", enabled: true },
      { address: "support@example.net", mailboxId: "mbx_sam", mailboxAddress: "sam@example.net", enabled: false },
    ],
    queue: {
      total: 2,
      deferred: 1,
      groups: [
        {
          reason: "rejected",
          count: 1,
          example: "550 5.1.1 <old.colleague@example.org>: Recipient address rejected: User unknown",
          remedy: "The recipient's server says the address doesn't exist. Nothing to fix here — confirm the address with the sender.",
        },
      ],
    },
    audit: [...adminAudit(), ...installerAudit()],
    portalAddress: "aiko@example.com",
    appPasswords: {
      "aiko@example.com": [
        { id: "ap_1", description: "iPhone Mail", createdAt: iso(12 * DAY) },
        { id: "ap_2", description: "Thunderbird on the studio desktop", createdAt: iso(6 * DAY) },
      ],
    },
  };

  if (scenario === "degraded") {
    return {
      ...healthy,
      port25Open: false,
      queue: {
        total: 14,
        deferred: 14,
        groups: [
          {
            reason: "network",
            count: 12,
            example: "connect to gmail-smtp-in.l.google.com[142.250.102.26]:25: Connection timed out",
            remedy: "Your provider blocks outbound port 25, so these can't leave the server. Switch sending to a relay — they'll go out on the next retry.",
          },
          {
            reason: "tls",
            count: 2,
            example: "TLS handshake with mx.example.org[198.51.100.7]:25 timed out",
            remedy: "Same cause: the connection never gets through port 25. A relay fixes this too.",
          },
        ],
      },
    };
  }
  if (scenario === "signedout") return { ...healthy, authenticated: false, portalAddress: null };
  return healthy;
}

function postmaster(): MailboxRow {
  return { id: "mbx_postmaster", local: "postmaster", domainId: "dom_com", displayName: "Postmaster", role: "user", quotaBytes: null, usedBytes: 18 * MB, createdAt: iso(15 * DAY), password: null };
}

function primaryDomain(mode: "installing" | "healthy"): DomainRow {
  const ids = recordTemplates("example.com", true, null).map((r) => r.id);
  const published: DomainRow["published"] =
    mode === "healthy"
      ? allPublished(ids)
      : Object.fromEntries(ids.map((i) => [i, i === "a" || i === "aaaa" ? "ok" : "missing"]));
  return { id: "dom_com", name: "example.com", enabled: true, isPrimary: true, catchAll: null, dnsCheckedAt: iso(mode === "healthy" ? 6 * MIN : 40 * 1000), createdAt: iso(15 * DAY), published };
}

function secondaryDomain(): DomainRow {
  const ids = recordTemplates("example.net", false, null).map((r) => r.id);
  const published: DomainRow["published"] = allPublished(ids);
  published.dmarc = "stale"; // still p=none
  published["tls-rpt"] = "missing";
  published.caa = "missing";
  return { id: "dom_net", name: "example.net", enabled: true, isPrimary: false, catchAll: "sam@example.net", dnsCheckedAt: iso(22 * MIN), createdAt: iso(4 * DAY), published };
}

function installingState(): InstallRow {
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
      step("preflight", "Preflight checks", "done", "Ubuntu 24.04, 4 GB RAM, ports 25, 80 and 443 reachable.", 19, 19),
      step("inputs", "Server details", "done", `${HOST} will serve mail for example.com.`, 18, 18),
      step("host_record", "Host record", "done", `${HOST} resolves to ${IPV4}.`, 18, 16),
      step("install_engine", "Install the mail engine", "done", `Stalwart ${ENGINE} downloaded, verified and installed.`, 16, 14),
      step("bootstrap_engine", "Configure the mail engine", "done", "Listeners, DKIM keys and the postmaster mailbox are ready.", 14, 12),
      step("edge_tls", "TLS certificate", "done", `Let's Encrypt issued a certificate for ${HOST}.`, 12, 11),
      step("firewall", "Firewall", "done", "Opened 25, 465, 587, 993, 80 and 443; everything else stays closed.", 11, 11),
      step(
        "dns",
        "DNS records",
        "waiting",
        "Publish the 5 required records for example.com at your DNS provider — MX, SPF, two DKIM keys and DMARC. Yunori checks every minute and continues on its own once they resolve.",
        11,
      ),
      step("ptr", "Reverse DNS (PTR)", "pending", null),
      step("first_mailbox", "First mailbox", "pending", null),
      step("done", "Finish", "pending", null),
    ],
    logs: [
      log(19, "info", "preflight", "Ubuntu 24.04.1 LTS, x86_64, 4 vCPU, 4 GB RAM, 78 GB free"),
      log(19, "info", "preflight", "Outbound :25 to the reflector succeeded"),
      log(19, "warn", "preflight", "IPv6 present but no PTR for 2001:db8:5::24 — AAAA stays optional"),
      log(18, "info", "inputs", `Hostname ${HOST}, primary domain example.com`),
      log(16, "info", "host_record", `${HOST} A ${IPV4} (via 3 public resolvers)`),
      log(14, "info", "install_engine", `stalwart ${ENGINE} sha256 verified, installed to /usr/local/bin`),
      log(12, "info", "bootstrap_engine", "Bootstrap complete; recovery admin removed"),
      log(12, "info", "bootstrap_engine", `DKIM keys generated: v1-ed25519-${SETUP_DAY}, v1-rsa-${SETUP_DAY}`),
      log(11, "info", "edge_tls", "ACME order finalised (HTTP-01)"),
      log(11, "info", "firewall", "nftables ruleset applied"),
      log(10, "info", "dns", "Waiting for MX, SPF, DKIM (2) and DMARC on example.com"),
      log(5, "info", "dns", "Checked 3 resolvers: 0 of 5 required records found"),
      log(1, "info", "dns", "Checked 3 resolvers: 0 of 5 required records found"),
    ],
  };
}

function log(minAgo: number, level: InstallLogLine["level"], step: string | null, message: string): InstallLogLine {
  return { at: iso(minAgo * MIN), level, step, message };
}

function installerAudit(): AuditEntry[] {
  return [
    { id: 4, at: iso(12 * MIN + 14 * DAY), actor: "installer", action: "mailbox.create", target: "postmaster@example.com", detail: "System mailbox" },
    { id: 3, at: iso(12 * MIN + 14 * DAY + MIN), actor: "installer", action: "domain.create", target: "example.com", detail: "Primary domain; DKIM ed25519 + rsa" },
    { id: 2, at: iso(14 * MIN + 14 * DAY), actor: "installer", action: "engine.install", target: `stalwart ${ENGINE}`, detail: null },
    { id: 1, at: iso(19 * MIN + 14 * DAY), actor: "installer", action: "install.start", target: HOST, detail: `yunori-mail ${VERSION}` },
  ];
}

function adminAudit(): AuditEntry[] {
  return [
    { id: 16, at: iso(3 * HOUR), actor: OWNER.email, action: "session.login", target: null, detail: "TOTP verified" },
    { id: 15, at: iso(22 * HOUR), actor: OWNER.email, action: "domain.update", target: "example.net", detail: "Catch-all → sam@example.net" },
    { id: 14, at: iso(1 * DAY + 2 * HOUR), actor: OWNER.email, action: "alias.disable", target: "support@example.net", detail: null },
    { id: 13, at: iso(4 * DAY), actor: OWNER.email, action: "mailbox.create", target: "sam@example.net", detail: "Quota 2 GB" },
    { id: 12, at: iso(4 * DAY + 5 * MIN), actor: OWNER.email, action: "domain.create", target: "example.net", detail: null },
    { id: 11, at: iso(6 * DAY), actor: "system", action: "dns.drift", target: "example.com", detail: "SPF changed outside Yunori; restored by owner" },
    { id: 10, at: iso(9 * DAY), actor: OWNER.email, action: "mailbox.create", target: "mina@example.com", detail: "Quota 5 GB" },
    { id: 9, at: iso(12 * DAY), actor: OWNER.email, action: "alias.create", target: "billing@example.com", detail: "→ ren@example.com" },
    { id: 8, at: iso(13 * DAY), actor: OWNER.email, action: "alias.create", target: "info@example.com", detail: "→ aiko@example.com" },
    { id: 7, at: iso(14 * DAY), actor: OWNER.email, action: "mailbox.create", target: "ren@example.com", detail: "Quota 5 GB" },
    { id: 6, at: iso(14 * DAY + 5 * MIN), actor: OWNER.email, action: "mailbox.create", target: "aiko@example.com", detail: "Quota 10 GB" },
    { id: 5, at: iso(14 * DAY + 10 * MIN), actor: OWNER.email, action: "admin.totp_enable", target: OWNER.email, detail: null },
  ];
}

// ── DNS ──────────────────────────────────────────────────────────────────────

type RecordTemplate = Omit<DnsRecord, "actual" | "status" | "remedy">;

function recordTemplates(domain: string, primary: boolean, relayInclude: string | null): RecordTemplate[] {
  const seedN = hashOf(domain);
  const spf = `v=spf1 mx${relayInclude ? ` include:${relayInclude}` : ""} -all`;
  const list: RecordTemplate[] = [];
  if (primary) {
    list.push(
      { id: "a", type: "A", name: HOST, expected: IPV4, required: true, purpose: "Points the mail host name at this server." },
      { id: "aaaa", type: "AAAA", name: HOST, expected: IPV6, required: false, purpose: "IPv6 address of the mail host. Publish it only if reverse DNS for IPv6 is set too." },
    );
  }
  list.push(
    { id: "mx", type: "MX", name: domain, expected: `10 ${HOST}`, required: true, purpose: `Tells other servers to deliver mail for ${domain} to this server.` },
    { id: "spf", type: "TXT", name: domain, expected: spf, required: true, purpose: `Lists the servers allowed to send mail as ${domain}.` },
    {
      id: `dkim:v1-ed25519-${SETUP_DAY}`,
      type: "TXT",
      name: `v1-ed25519-${SETUP_DAY}._domainkey.${domain}`,
      expected: `v=DKIM1; k=ed25519; p=${pseudoBase64(43, seedN)}=`,
      required: true,
      purpose: "Public key receivers use to verify that mail from this server wasn't altered (Ed25519).",
    },
    {
      id: `dkim:v1-rsa-${SETUP_DAY}`,
      type: "TXT",
      name: `v1-rsa-${SETUP_DAY}._domainkey.${domain}`,
      expected: `v=DKIM1; k=rsa; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA${pseudoBase64(330, seedN + 7)}IDAQAB`,
      required: true,
      purpose: "The same signature as an RSA key, for receivers that don't support Ed25519 yet.",
    },
    {
      id: "dmarc",
      type: "TXT",
      name: `_dmarc.${domain}`,
      expected: `v=DMARC1; p=quarantine; rua=mailto:dmarc-reports@${domain}; adkim=s; aspf=s`,
      required: true,
      purpose: "Tells receivers what to do with mail that fails SPF and DKIM, and where to send reports.",
    },
    { id: "mta-sts", type: "TXT", name: `_mta-sts.${domain}`, expected: `v=STSv1; id=${SETUP_DAY}T0930`, required: false, purpose: "Announces that mail to this domain must be delivered over TLS (MTA-STS)." },
    { id: "mta-sts-host", type: "CNAME", name: `mta-sts.${domain}`, expected: HOST, required: false, purpose: "Serves the MTA-STS policy file over HTTPS." },
    { id: "tls-rpt", type: "TXT", name: `_smtp._tls.${domain}`, expected: `v=TLSRPTv1; rua=mailto:tls-reports@${domain}`, required: false, purpose: "Where other servers report problems delivering to you over TLS." },
    { id: "autoconfig", type: "CNAME", name: `autoconfig.${domain}`, expected: HOST, required: false, purpose: "Lets Yunori, Thunderbird and other apps set up a mailbox automatically." },
    { id: "autodiscover", type: "CNAME", name: `autodiscover.${domain}`, expected: HOST, required: false, purpose: "Automatic setup for Outlook and Apple Mail." },
    { id: "srv:_submissions._tcp", type: "SRV", name: `_submissions._tcp.${domain}`, expected: `0 1 465 ${HOST}`, required: false, purpose: "Advertises the secure sending port (465) to mail apps." },
    { id: "srv:_imaps._tcp", type: "SRV", name: `_imaps._tcp.${domain}`, expected: `0 1 993 ${HOST}`, required: false, purpose: "Advertises the secure IMAP port (993) to mail apps." },
    { id: "srv:_jmap._tcp", type: "SRV", name: `_jmap._tcp.${domain}`, expected: `0 1 443 ${HOST}`, required: false, purpose: "Advertises JMAP, the modern protocol the Yunori client prefers." },
    { id: "caa", type: "CAA", name: domain, expected: `0 issue "letsencrypt.org"`, required: false, purpose: "Allows only Let's Encrypt to issue certificates for this domain." },
  );
  if (primary) {
    list.push({
      id: "ptr",
      type: "PTR",
      name: IPV4.split(".").reverse().join(".") + ".in-addr.arpa",
      expected: HOST,
      required: true,
      purpose: `Reverse DNS: lets receivers confirm ${IPV4} really is ${HOST}. It's set at your VPS provider, not your DNS host.`,
    });
  }
  return list;
}

function dnsFor(db: DB, d: DomainRow): DnsRecord[] {
  const relay = db.sending.mode === "relay" && db.sending.relay ? PROVIDERS.find((p) => p.key === db.sending.relay!.provider)?.spfInclude ?? null : null;
  const installing = !!db.install && d.isPrimary;
  return recordTemplates(d.name, d.isPrimary, relay).map((t): DnsRecord => {
    const state = d.published[t.id] ?? "missing";
    if (state === "ok") return { ...t, actual: [t.expected], status: "pass" };
    if (t.id === "ptr" && installing) {
      return { ...t, actual: [], status: "unknown", remedy: "Checked once the records above resolve — the installer walks you through it." };
    }
    if (state === "stale") {
      if (t.id === "dmarc") {
        return {
          ...t,
          actual: [`v=DMARC1; p=none; rua=mailto:dmarc-reports@${d.name}`],
          status: "warn",
          remedy: "Reports have been clean for 21 days. Raise the policy to p=quarantine by publishing the expected value.",
        };
      }
      return { ...t, actual: ["(an older value)"], status: t.required ? "fail" : "warn", remedy: "Replace the published value with the expected one." };
    }
    if (!t.required) {
      return {
        ...t,
        actual: [],
        status: installing ? "info" : "warn",
        remedy: installing ? "Optional — add it after the required records." : "Recommended: add this record at your DNS provider.",
      };
    }
    return {
      ...t,
      actual: [],
      status: "fail",
      remedy: t.id === "ptr" ? "Set reverse DNS in your VPS provider's control panel." : "Add this record at your DNS provider, exactly as shown.",
    };
  });
}

function zoneFile(domain: string, records: DnsRecord[]): string {
  const lines = [`; Yunori Mail — records for ${domain}`, `; Generated ${new Date().toISOString()}. PTR is set at your VPS provider, not in this zone.`, ""];
  for (const r of records) {
    if (r.type === "PTR") continue;
    const name = `${r.name}.`;
    let value = r.expected;
    if (r.type === "TXT") {
      const chunks = value.match(/.{1,255}/g) ?? [value];
      value = chunks.map((c) => `"${c.replace(/"/g, '\\"')}"`).join(" ");
    } else if (r.type === "MX" || r.type === "CNAME" || r.type === "SRV") {
      value = `${value}.`;
    }
    lines.push(`${name.padEnd(48)} 3600 IN ${r.type.padEnd(5)} ${value}`);
  }
  return lines.join("\n") + "\n";
}

const ORDER: Record<CheckStatus, number> = { fail: 0, warn: 1, unknown: 2, info: 3, pass: 4 };
const worst = (s: CheckStatus[]) => s.reduce<CheckStatus>((w, x) => (ORDER[x] < ORDER[w] ? x : w), "pass");

function toDomain(db: DB, d: DomainRow): Domain {
  const { published: _p, ...rest } = d;
  const records = dnsFor(db, d);
  return {
    ...rest,
    mailboxCount: db.mailboxes.filter((m) => m.domainId === d.id).length,
    aliasCount: db.aliases.filter((a) => a.address.endsWith(`@${d.name}`)).length,
    dnsStatus: worst(records.filter((r) => r.required).map((r) => r.status)),
  };
}

function addressOf(db: DB, m: MailboxRow) {
  const d = db.domains.find((x) => x.id === m.domainId);
  return `${m.local}@${d?.name ?? "unknown"}`;
}

function toMailbox(db: DB, m: MailboxRow): Mailbox {
  const { password: _pw, ...rest } = m;
  const address = addressOf(db, m);
  return { ...rest, address, aliases: db.aliases.filter((a) => a.mailboxId === m.id).map((a) => a.address) };
}

// ── derived views ────────────────────────────────────────────────────────────

function session(db: DB): SessionResponse {
  return {
    authenticated: db.authenticated,
    setupRequired: db.setupRequired,
    admin: db.authenticated ? db.admins.find((a) => a.id === db.adminId) ?? null : null,
    server: {
      hostname: HOST,
      primaryDomain: "example.com",
      version: VERSION,
      engineVersion: ENGINE,
      installComplete: db.installComplete,
      clientOrigin: CLIENT_ORIGIN,
    },
  };
}

function port25Check(db: DB): Check {
  return db.port25Open
    ? { id: "port25", label: "Outbound port 25", status: "pass", detail: "Connections to other mail servers on port 25 succeed." }
    : db.sending.mode === "relay"
      ? { id: "port25", label: "Outbound port 25", status: "info", detail: "Your provider blocks outbound port 25. That's fine: mail leaves through your relay instead." }
      : {
          id: "port25",
          label: "Outbound port 25",
          status: "fail",
          detail: "Your hosting provider blocks outbound port 25, so this server can't deliver mail directly.",
          remedy: "Switch sending to a relay (Sending → Relay), or ask your provider to open port 25.",
        };
}

function ptrCheck(db: DB): Check {
  if (db.install) return { id: "ptr", label: "Reverse DNS (PTR)", status: "unknown", detail: "Checked after the DNS step — the installer will ask you to set it." };
  return { id: "ptr", label: "Reverse DNS (PTR)", status: "pass", detail: `${IPV4} → ${HOST}, and ${HOST} resolves back to ${IPV4}.` };
}

function signals(db: DB): Check[] {
  const domains = db.domains.map((d) => toDomain(db, d));
  const failing = domains.filter((d) => d.dnsStatus === "fail");
  const warning = domains.filter((d) => d.dnsStatus === "warn");
  const dns: Check = failing.length
    ? {
        id: "dns",
        label: "DNS records",
        status: "fail",
        detail: `${failing.map((d) => d.name).join(", ")} ${failing.length === 1 ? "is" : "are"} missing required records, so mail can't reach ${failing.length === 1 ? "it" : "them"} yet.`,
        remedy: "Publish the records shown on the domain's page, then press “Check now”.",
      }
    : warning.length
      ? {
          id: "dns",
          label: "DNS records",
          status: "warn",
          detail: `Required records resolve for every domain. ${warning.map((d) => d.name).join(", ")} still has a DMARC policy of p=none.`,
          remedy: "Open the domain and publish the stricter DMARC value when you're ready.",
        }
      : { id: "dns", label: "DNS records", status: "pass", detail: `Every required record resolves for ${domains.length} domain${domains.length === 1 ? "" : "s"}.` };
  const deferred = db.queue.deferred;
  const queue: Check =
    deferred === 0
      ? { id: "queue", label: "Outgoing queue", status: "pass", detail: "Nothing is waiting to be delivered." }
      : !db.port25Open && db.sending.mode === "direct"
        ? { id: "queue", label: "Outgoing queue", status: "fail", detail: `${deferred} messages are stuck: every attempt times out on port 25.`, remedy: "Switch sending to a relay; queued mail is retried automatically." }
        : { id: "queue", label: "Outgoing queue", status: "info", detail: `${db.queue.total} queued, ${deferred} deferred by the recipient's server — see Deliverability for the reasons.` };
  return [
    dns,
    ptrCheck(db),
    { id: "tls", label: "TLS certificate", status: "pass", detail: `Valid for ${HOST} until ${fmtDay(now() + 64 * DAY)} (Let's Encrypt). The served certificate matches.` },
    port25Check(db),
    { id: "blocklists", label: "Blocklists", status: "pass", detail: `${IPV4} isn't listed on Spamhaus ZEN, Barracuda or SpamCop.` },
    { id: "engine", label: "Mail engine", status: "pass", detail: `Stalwart ${ENGINE} is running; SMTP, IMAP and JMAP answer.` },
    queue,
  ];
}

function fmtDay(ms: number) {
  return new Intl.DateTimeFormat("en-US", { dateStyle: "medium" }).format(new Date(ms));
}

function verdict(db: DB): { level: VerdictLevel; sentence: string } {
  if (db.install) {
    const dns = db.install.steps.find((s) => s.id === "dns");
    if (dns?.status === "waiting") return { level: "attention", sentence: "Setup is waiting on you: publish the DNS records for example.com so mail can find this server." };
    return { level: "attention", sentence: `Almost there — confirming reverse DNS for ${HOST}.` };
  }
  if (!db.port25Open && db.sending.mode === "direct") {
    return { level: "degraded", sentence: "Receiving works, but outgoing mail is stuck: your provider blocks port 25. Switch sending to a relay." };
  }
  const domains = db.domains.map((d) => toDomain(db, d));
  const failing = domains.find((d) => d.dnsStatus === "fail");
  if (failing) return { level: "attention", sentence: `Mail for ${failing.name} can't arrive until its DNS records are published.` };
  if (!db.port25Open) return { level: "ok", sentence: "Receiving works; sending is relayed because port 25 is blocked." };
  return { level: "ok", sentence: `Everything's working — mail flows in and out of ${HOST}.` };
}

function overview(db: DB): OverviewResponse {
  return {
    verdict: verdict(db),
    stats: { domains: db.domains.length, mailboxes: db.mailboxes.length, aliases: db.aliases.length, queued: db.queue.total },
    sendingMode: db.sending.mode,
    signals: signals(db),
    setup: db.install ? { steps: db.install.steps, logs: db.install.logs } : null,
  };
}

function advanceInstall(db: DB) {
  const ins = db.install;
  if (!ins || ins.ptrStartedAt === null) return;
  if (now() - ins.ptrStartedAt < 6000) return;
  const set = (id: string, patch: Partial<InstallStep>) => {
    ins.steps = ins.steps.map((s) => (s.id === id ? { ...s, ...patch } : s));
  };
  set("ptr", { status: "done", detail: `${IPV4} → ${HOST}, confirmed both ways.`, finishedAt: iso() });
  set("first_mailbox", { status: "done", detail: "aiko@example.com is ready — its password was shown in the terminal.", startedAt: iso(), finishedAt: iso() });
  set("done", { status: "done", detail: "Your server is ready.", startedAt: iso(), finishedAt: iso() });
  ins.logs = [...ins.logs, log(0, "info", "ptr", `PTR ${IPV4} → ${HOST} (forward-confirmed)`), log(0, "info", "first_mailbox", "Created aiko@example.com"), log(0, "info", "done", "Install complete")];
  ins.ptrStartedAt = null;
  db.mailboxes = [
    { id: "mbx_aiko", local: "aiko", domainId: "dom_com", displayName: "Aiko Tanaka", role: "admin", quotaBytes: 10 * GB, usedBytes: 0, createdAt: iso(), password: null },
    ...db.mailboxes,
  ];
  // Keep the finished checklist visible for one more poll, then drop it.
  window.setTimeout(() => {
    const cur = load();
    if (cur.install && cur.install.steps.every((s) => s.status === "done" || s.status === "skipped")) {
      cur.install = null;
      cur.installComplete = true;
      cur.scenario = "healthy";
      save(cur);
    }
  }, 4000);
}

function system(db: DB): SystemResponse {
  return {
    versions: { yunori: VERSION, engine: ENGINE },
    services: [
      { name: "stalwart", status: "running", detail: `Up 14 days · SMTP, IMAP and JMAP answering` },
      { name: "yunori-mail", status: "running", detail: "Up 14 days · edge on :443, console, DNS monitor" },
      { name: "unbound", status: "running", detail: "Local resolver · DNSSEC validating" },
    ],
    resources: { memTotalBytes: 4 * GB, memUsedBytes: 1.38 * GB, diskTotalBytes: 80 * GB, diskFreeBytes: (db.install ? 77.6 : 61.2) * GB, load1: 0.18 },
    certificate: {
      hostnames: [HOST, ...db.domains.flatMap((d) => [`autoconfig.${d.name}`, `mta-sts.${d.name}`])],
      issuer: "Let's Encrypt (R11)",
      notAfter: new Date(now() + 64 * DAY).toISOString(),
      servedMatches: true,
    },
  };
}

function preflight(db: DB): Check[] {
  return [
    { id: "os", label: "Operating system", status: "pass", detail: "Ubuntu 24.04.1 LTS (supported)." },
    { id: "memory", label: "Memory", status: "pass", detail: "4 GB RAM — comfortably above the 2 GB minimum." },
    { id: "disk", label: "Disk space", status: "pass", detail: `${db.install ? "77.6" : "61.2"} GB free on /var/lib.` },
    { id: "ports-in", label: "Inbound ports", status: "pass", detail: "25, 465, 587, 993, 80 and 443 are reachable from the internet." },
    db.port25Open
      ? { id: "port25-out", label: "Outbound port 25", status: "pass", detail: "Reached the test reflector on port 25." }
      : {
          id: "port25-out",
          label: "Outbound port 25",
          status: db.sending.mode === "relay" ? "warn" : "fail",
          detail: "Connection to the reflector on port 25 timed out — your provider blocks it.",
          remedy: db.sending.mode === "relay" ? "Nothing to do while you send through a relay." : "Use relay mode, or ask your provider to open port 25.",
        },
    { id: "ptr", label: "Reverse DNS", status: db.install ? "unknown" : "pass", detail: db.install ? "Not set yet — expected at this stage." : `${IPV4} → ${HOST}.` },
    { id: "ipv6", label: "IPv6", status: "info", detail: `${IPV6} is configured, but has no reverse DNS — IPv6 sending stays off.`, remedy: "Optional: set an IPv6 PTR at your provider to enable it." },
    { id: "time", label: "Clock", status: "pass", detail: "In sync (chrony, offset 3 ms)." },
    { id: "blocklists", label: "Blocklists", status: "pass", detail: `${IPV4} isn't listed.` },
  ];
}

// ── persistence ──────────────────────────────────────────────────────────────

let cache: DB | null = null;

function load(): DB {
  if (cache) return cache;
  const param = new URLSearchParams(window.location.search).get("mock");
  if (param && SCENARIOS.some((s) => s.key === param)) {
    cache = seed(param as Scenario);
    try {
      localStorage.setItem(SCENARIO_KEY, param);
    } catch {}
    save(cache);
    return cache;
  }
  try {
    const raw = sessionStorage.getItem(DB_KEY);
    if (raw) {
      cache = JSON.parse(raw) as DB;
      return cache;
    }
  } catch {}
  let sc: Scenario = "healthy";
  try {
    const v = localStorage.getItem(SCENARIO_KEY);
    if (v && SCENARIOS.some((s) => s.key === v)) sc = v as Scenario;
  } catch {}
  cache = seed(sc);
  save(cache);
  return cache;
}

function save(db: DB) {
  cache = db;
  try {
    sessionStorage.setItem(DB_KEY, JSON.stringify(db));
  } catch {}
}

export function currentScenario(): Scenario {
  return load().scenario;
}

/** Reseed for a scenario (the dev panel reloads the page afterwards). */
export function setScenario(s: Scenario) {
  try {
    localStorage.setItem(SCENARIO_KEY, s);
  } catch {}
  save(seed(s));
}

// ── request handling ─────────────────────────────────────────────────────────

class HttpError extends Error {
  constructor(
    readonly status: number,
    readonly error: ApiError,
  ) {
    super(error.message);
  }
}
const fail = (status: number, code: string, message: string, field?: string): never => {
  throw new HttpError(status, { code, message, ...(field ? { field } : {}) });
};

function json(status: number, body?: unknown): Response {
  if (status === 204 || body === undefined) return new Response(null, { status: 204 });
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function audit(db: DB, action: string, target: string | null, detail: string | null = null) {
  const actor = db.admins.find((a) => a.id === db.adminId)?.email ?? "system";
  const id = Math.max(0, ...db.audit.map((e) => e.id)) + 1;
  db.audit = [{ id, at: iso(), actor, action, target, detail }, ...db.audit];
}

const LOCAL_RE = /^[a-z0-9](?:[a-z0-9._+-]{0,62}[a-z0-9])?$/i;
const DOMAIN_RE = /^(?=.{1,253}$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/i;
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

type Body = Record<string, unknown>;
const str = (b: Body, k: string) => (typeof b[k] === "string" ? (b[k] as string) : "");

function requireAdmin(db: DB) {
  if (!db.authenticated) fail(401, "unauthenticated", "Sign in to continue.");
}
function requirePortal(db: DB): string {
  if (!db.portalAddress) fail(401, "unauthenticated", "Sign in to continue.");
  return db.portalAddress!;
}

function findMailboxByAddress(db: DB, address: string) {
  return db.mailboxes.find((m) => addressOf(db, m).toLowerCase() === address.toLowerCase());
}

function route(db: DB, method: string, path: string, query: URLSearchParams, body: Body): [number, unknown?] {
  const seg = path.split("/").filter(Boolean);
  const is = (m: string, p: string) => method === m && new RegExp(`^${p}$`).test(path);

  // ── session & auth
  if (is("GET", "/session")) return [200, session(db)];
  if (is("POST", "/setup")) {
    if (!db.setupRequired) fail(409, "conflict", "Setup is already complete. Sign in instead.");
    const token = str(body, "token").trim();
    const email = str(body, "email").trim().toLowerCase();
    const password = str(body, "password");
    if (token.length < 8 || token.toLowerCase().includes("bad"))
      fail(403, "invalid_token", "That setup token isn't valid. Use the one printed at the end of the install — it works once.", "token");
    if (!EMAIL_RE.test(email)) fail(400, "invalid_email", "Enter a valid email address.", "email");
    if (password.length < 12) fail(400, "weak_password", "Use at least 12 characters.", "password");
    const secret = randomFrom("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567", 32);
    db.pendingSetup = { email, secret };
    return [
      200,
      {
        totpSecret: secret,
        otpauthUrl: `otpauth://totp/Yunori%20Mail:${encodeURIComponent(email)}?secret=${secret}&issuer=Yunori%20Mail&algorithm=SHA1&digits=6&period=30`,
      },
    ];
  }
  if (is("POST", "/setup/totp")) {
    if (!db.pendingSetup) fail(400, "setup_not_started", "Start setup again — the previous step expired.");
    const code = str(body, "code");
    if (!/^\d{6}$/.test(code) || code === "000000")
      fail(400, "invalid_code", "That code didn't match. Codes change every 30 seconds — enter the newest one.", "code");
    const admin: AdminUser = { id: "adm_01", email: db.pendingSetup!.email, role: "owner", totpEnabled: true, createdAt: iso() };
    db.admins = [admin];
    db.adminId = admin.id;
    db.setupRequired = false;
    db.authenticated = true;
    db.pendingSetup = null;
    db.scenario = "installing";
    audit(db, "admin.create", admin.email, "Owner; TOTP enabled");
    return [200, session(db)];
  }
  if (is("POST", "/login")) {
    if (db.setupRequired) fail(409, "setup_required", "This server hasn't been set up yet.");
    const email = str(body, "email").trim().toLowerCase();
    const password = str(body, "password");
    const totp = str(body, "totp");
    if (!/^\d{6}$/.test(totp)) fail(400, "totp_required", "Enter the 6-digit code from your authenticator app.", "totp");
    const admin = db.admins.find((a) => a.email === email);
    if (!admin || password.length < 12 || totp === "000000")
      fail(401, "invalid_credentials", "That email, password and code didn't match. Check all three and try again.");
    db.authenticated = true;
    db.adminId = admin!.id;
    audit(db, "session.login", null, "TOTP verified");
    return [200, session(db)];
  }
  if (is("POST", "/logout")) {
    db.authenticated = false;
    return [204];
  }

  // ── portal
  if (seg[0] === "portal") {
    if (is("POST", "/portal/login")) {
      const email = str(body, "email").trim().toLowerCase();
      const password = str(body, "password");
      const m = findMailboxByAddress(db, email);
      const ok = m && password.length >= 8 && (m.password === null || m.password === password);
      if (!ok) fail(401, "invalid_credentials", "That address and password didn't match.");
      db.portalAddress = addressOf(db, m!);
      return [200, portalMe(db)];
    }
    if (is("POST", "/portal/logout")) {
      db.portalAddress = null;
      return [204];
    }
    const address = requirePortal(db);
    if (is("GET", "/portal/me")) return [200, portalMe(db)];
    if (is("POST", "/portal/password")) {
      const m = findMailboxByAddress(db, address)!;
      const current = str(body, "current");
      const next = str(body, "next");
      if (!current || (m.password !== null && m.password !== current)) fail(400, "invalid_credentials", "Your current password isn't right.", "current");
      if (next.length < 12) fail(400, "weak_password", "Use at least 12 characters.", "next");
      if (next === current) fail(400, "same_password", "Choose a password you haven't used here before.", "next");
      m.password = next;
      return [204];
    }
    if (is("POST", "/portal/app-passwords")) {
      const description = str(body, "description").trim();
      if (!description) fail(400, "invalid", "Name it after the app or device that will use it.", "description");
      const ap: AppPassword = { id: `ap_${++db.seq}`, description: description.slice(0, 80), createdAt: iso() };
      db.appPasswords[address] = [...(db.appPasswords[address] ?? []), ap];
      const secret = [0, 1, 2, 3].map(() => randomFrom("abcdefghijkmnopqrstuvwxyz", 4)).join(" ");
      return [200, { appPassword: ap, secret }];
    }
    if (method === "DELETE" && seg[1] === "app-passwords" && seg[2]) {
      const list = db.appPasswords[address] ?? [];
      if (!list.some((a) => a.id === seg[2])) fail(404, "not_found", "That app password no longer exists.");
      db.appPasswords[address] = list.filter((a) => a.id !== seg[2]);
      return [204];
    }
    fail(404, "not_found", "Unknown portal endpoint.");
  }

  // ── everything below needs an admin session
  requireAdmin(db);

  if (is("GET", "/overview")) return [200, overview(db)];
  if (is("GET", "/install")) {
    advanceInstall(db);
    const st: InstallState = db.install ? { steps: db.install.steps, logs: db.install.logs } : { steps: [], logs: [] };
    return [200, st];
  }

  // ── domains
  if (is("GET", "/domains")) return [200, db.domains.map((d) => toDomain(db, d))];
  if (is("POST", "/domains")) {
    const name = str(body, "name").trim().toLowerCase().replace(/\.$/, "");
    if (!DOMAIN_RE.test(name)) fail(400, "invalid_domain", "Enter a domain like example.org — no “http://”, no “@”.", "name");
    if (db.domains.some((d) => d.name === name)) fail(409, "conflict", `${name} is already on this server.`, "name");
    const ids = recordTemplates(name, false, null).map((r) => r.id);
    const row: DomainRow = { id: `dom_${++db.seq}`, name, enabled: true, isPrimary: false, catchAll: null, dnsCheckedAt: iso(), createdAt: iso(), published: Object.fromEntries(ids.map((i) => [i, "missing" as const])) };
    db.domains.push(row);
    audit(db, "domain.create", name, "DKIM ed25519 + rsa generated");
    return [200, toDomain(db, row)];
  }
  if (seg[0] === "domains" && seg[1]) {
    const d = db.domains.find((x) => x.id === seg[1]);
    if (!d) fail(404, "not_found", "That domain doesn't exist (any more).");
    const dom = d!;
    if (seg[2] === "dns") {
      if (method === "GET" && !seg[3]) {
        const records = dnsFor(db, dom);
        return [200, { domain: dom.name, records, checkedAt: dom.dnsCheckedAt, zoneFile: zoneFile(dom.name, records) }];
      }
      if (method === "POST" && seg[3] === "check") {
        dom.dnsCheckedAt = iso();
        const ins = db.install;
        if (ins && dom.isPrimary && ins.steps.find((s) => s.id === "dns")?.status === "waiting") {
          for (const k of Object.keys(dom.published)) dom.published[k] = "ok";
          ins.steps = ins.steps.map((s) =>
            s.id === "dns"
              ? { ...s, status: "done", detail: "All 5 required records resolve from 3 public resolvers.", finishedAt: iso() }
              : s.id === "ptr"
                ? { ...s, status: "running", detail: `Checking that ${IPV4} points back to ${HOST}…`, startedAt: iso() }
                : s,
          );
          ins.logs = [...ins.logs, log(0, "info", "dns", "5 of 5 required records found"), log(0, "info", "ptr", `Looking up PTR for ${IPV4}`)];
          ins.ptrStartedAt = now();
        }
        audit(db, "dns.check", dom.name);
        const records = dnsFor(db, dom);
        return [200, { domain: dom.name, records, checkedAt: dom.dnsCheckedAt, zoneFile: zoneFile(dom.name, records) }];
      }
    }
    if (method === "PATCH" && !seg[2]) {
      if ("catchAll" in body) {
        const v = body.catchAll;
        if (v === null || v === "") dom.catchAll = null;
        else if (typeof v === "string" && findMailboxByAddress(db, v)) dom.catchAll = v.toLowerCase();
        else fail(400, "invalid", "Catch-all must deliver to an existing mailbox.", "catchAll");
        audit(db, "domain.update", dom.name, `Catch-all → ${dom.catchAll ?? "off"}`);
      }
      if (typeof body.enabled === "boolean") {
        if (dom.isPrimary && !body.enabled) fail(409, "conflict", "The primary domain can't be disabled — it names this server.");
        dom.enabled = body.enabled;
        audit(db, dom.enabled ? "domain.enable" : "domain.disable", dom.name);
      }
      return [200, toDomain(db, dom)];
    }
    if (method === "DELETE" && !seg[2]) {
      if (dom.isPrimary) fail(409, "conflict", "The primary domain can't be deleted — it names this server.");
      const boxes = db.mailboxes.filter((m) => m.domainId === dom.id).length;
      const aliases = db.aliases.filter((a) => a.address.endsWith(`@${dom.name}`)).length;
      if (boxes > 0)
        fail(
          409,
          "conflict",
          `${dom.name} still has ${boxes} mailbox${boxes === 1 ? "" : "es"}${aliases ? ` and ${aliases} alias${aliases === 1 ? "" : "es"}` : ""}. Delete them first — deleting a domain never removes mail silently.`,
        );
      db.aliases = db.aliases.filter((a) => !a.address.endsWith(`@${dom.name}`));
      db.domains = db.domains.filter((x) => x.id !== dom.id);
      audit(db, "domain.delete", dom.name);
      return [204];
    }
  }

  // ── mailboxes
  if (is("GET", "/mailboxes")) {
    const domainId = query.get("domainId");
    return [200, db.mailboxes.filter((m) => !domainId || m.domainId === domainId).map((m) => toMailbox(db, m))];
  }
  if (is("POST", "/mailboxes")) {
    const local = str(body, "local").trim().toLowerCase();
    const d = db.domains.find((x) => x.id === body.domainId);
    if (!d) fail(400, "invalid", "Choose a domain.", "domainId");
    if (!LOCAL_RE.test(local)) fail(400, "invalid_local", "Use letters, numbers, dots, dashes or underscores.", "local");
    const address = `${local}@${d!.name}`;
    if (findMailboxByAddress(db, address)) fail(409, "conflict", `${address} already exists.`, "local");
    if (db.aliases.some((a) => a.address === address))
      fail(409, "conflict", `${address} is an alias right now. Remove the alias first, or pick another name.`, "local");
    const pw = typeof body.password === "string" && body.password ? body.password : null;
    if (pw !== null && pw.length < 12) fail(400, "weak_password", "Use at least 12 characters.", "password");
    const password = pw ?? genPassword();
    const quotaMb = typeof body.quotaMb === "number" ? body.quotaMb : null;
    const displayName = str(body, "displayName").trim() || null;
    const row: MailboxRow = { id: `mbx_${++db.seq}`, local, domainId: d!.id, displayName, role: "user", quotaBytes: quotaMb ? quotaMb * MB : null, usedBytes: 0, createdAt: iso(), password };
    db.mailboxes.push(row);
    audit(db, "mailbox.create", address, quotaMb ? `Quota ${quotaMb >= 1024 ? `${quotaMb / 1024} GB` : `${quotaMb} MB`}` : "Unlimited");
    return [200, { mailbox: toMailbox(db, row), password }];
  }
  if (seg[0] === "mailboxes" && seg[1]) {
    const m = db.mailboxes.find((x) => x.id === seg[1]);
    if (!m) fail(404, "not_found", "That mailbox doesn't exist (any more).");
    const box = m!;
    const address = addressOf(db, box);
    if (method === "POST" && seg[2] === "password") {
      const given = str(body, "password");
      if (given && given.length < 12) fail(400, "weak_password", "Use at least 12 characters.", "password");
      box.password = given || genPassword();
      audit(db, "mailbox.password_reset", address);
      return [200, { password: box.password }];
    }
    if (method === "PATCH" && !seg[2]) {
      if ("displayName" in body) box.displayName = (typeof body.displayName === "string" && body.displayName.trim()) || null;
      if ("quotaMb" in body) {
        const q = body.quotaMb;
        if (q !== null && (typeof q !== "number" || q <= 0)) fail(400, "invalid", "Quota must be a positive number of MB, or unlimited.", "quotaMb");
        box.quotaBytes = q === null ? null : (q as number) * MB;
      }
      audit(db, "mailbox.update", address);
      return [200, toMailbox(db, box)];
    }
    if (method === "DELETE" && !seg[2]) {
      if (box.local === "postmaster") fail(409, "conflict", "postmaster@ is a protected system mailbox — mail servers are required to have one.");
      db.aliases = db.aliases.filter((a) => a.mailboxId !== box.id);
      db.domains.forEach((d) => {
        if (d.catchAll === address) d.catchAll = null;
      });
      db.mailboxes = db.mailboxes.filter((x) => x.id !== box.id);
      audit(db, "mailbox.delete", address);
      return [204];
    }
  }

  // ── aliases
  if (is("GET", "/aliases")) return [200, db.aliases];
  if (is("POST", "/aliases")) {
    const local = str(body, "local").trim().toLowerCase();
    const d = db.domains.find((x) => x.id === body.domainId);
    const target = db.mailboxes.find((x) => x.id === body.mailboxId);
    if (!d) fail(400, "invalid", "Choose a domain.", "domainId");
    if (!target) fail(400, "invalid", "Choose the mailbox that should receive this mail.", "mailboxId");
    if (!LOCAL_RE.test(local)) fail(400, "invalid_local", "Use letters, numbers, dots, dashes or underscores.", "local");
    const address = `${local}@${d!.name}`;
    if (findMailboxByAddress(db, address))
      fail(409, "conflict", `${address} is already a mailbox. An alias with the same address would hide it, so it isn't allowed.`, "local");
    if (db.aliases.some((a) => a.address === address)) fail(409, "conflict", `${address} is already an alias.`, "local");
    const alias: Alias = { address, mailboxId: target!.id, mailboxAddress: addressOf(db, target!), enabled: true };
    db.aliases.push(alias);
    audit(db, "alias.create", address, `→ ${alias.mailboxAddress}`);
    return [200, alias];
  }
  if (is("DELETE", "/aliases")) {
    const address = (query.get("address") ?? "").toLowerCase();
    if (!db.aliases.some((a) => a.address === address)) fail(404, "not_found", "That alias doesn't exist (any more).");
    db.aliases = db.aliases.filter((a) => a.address !== address);
    audit(db, "alias.delete", address);
    return [204];
  }

  // ── sending
  if (is("GET", "/sending")) return [200, sendingView(db)];
  if (is("PUT", "/sending")) {
    const mode = body.mode === "relay" ? "relay" : "direct";
    if (mode === "relay") {
      const r = (body.relay ?? {}) as Body;
      const provider = PROVIDERS.find((p) => p.key === r.provider);
      if (!provider) fail(400, "invalid", "Choose a relay provider.", "provider");
      const host = str(r, "host").trim();
      const port = Number(r.port);
      const username = str(r, "username").trim();
      const password = str(r, "password");
      if (!host || !/^[a-z0-9.-]+$/i.test(host)) fail(400, "invalid", "Enter the relay's SMTP host name.", "host");
      if (!Number.isInteger(port) || port < 1 || port > 65535) fail(400, "invalid", "Port must be between 1 and 65535.", "port");
      if (!username) fail(400, "invalid", "Enter the relay username (often an API key name).", "username");
      const keep = db.sending.relay?.password ?? null;
      if (!password && !keep) fail(400, "invalid", "Enter the relay password.", "password");
      db.sending = { mode, relay: { provider: provider!.key, host, port, username, password: password || keep } };
      // The relay accepted the deferred mail on retry.
      db.queue = { total: 0, deferred: 0, groups: [] };
      audit(db, "sending.update", "relay", `${provider!.label} via ${host}:${port}`);
    } else {
      db.sending = { ...db.sending, mode: "direct" };
      audit(db, "sending.update", "direct", null);
    }
    return [200, sendingView(db)];
  }

  if (is("GET", "/deliverability")) {
    const primary = db.domains.find((d) => d.isPrimary);
    const dmarc = primary ? dnsFor(db, primary).find((r) => r.id === "dmarc") : undefined;
    return [
      200,
      {
        checks: [
          { ...ptrCheck(db), label: "Reverse DNS (PTR / FCrDNS)" },
          { id: "helo", label: "HELO name", status: db.install ? "unknown" : "pass", detail: `The server introduces itself as ${HOST}, matching its PTR.` },
          { id: "blocklists", label: "Blocklists", status: "pass", detail: `${IPV4} isn't listed on Spamhaus ZEN, Barracuda or SpamCop. Checked every 6 hours.` },
          port25Check(db),
          { id: "tls", label: "TLS", status: "pass", detail: "STARTTLS on 25 and implicit TLS on 465/993 use a valid certificate." },
          {
            id: "mta-sts",
            label: "MTA-STS",
            status: "info",
            detail: "Policy is in testing mode: senders report TLS problems but still deliver.",
            remedy: "Move to enforce after two weeks of clean TLS reports.",
          },
        ],
        queue: db.queue,
        dmarcPolicy: !dmarc || dmarc.status === "fail" || dmarc.status === "unknown" ? "unknown" : dmarc.status === "warn" ? "none" : "quarantine",
        checkedAt: iso(9 * MIN),
      },
    ];
  }

  if (is("GET", "/system")) return [200, system(db)];
  if (is("GET", "/preflight")) return [200, preflight(db)];
  if (is("GET", "/audit")) return [200, { entries: db.audit, chainOk: true }];
  if (is("GET", "/admins")) return [200, db.admins];

  fail(404, "not_found", `No mock for ${method} ${path}`);
  return [500];
}

function sendingView(db: DB): SendingResponse {
  const r = db.sending.relay;
  return {
    mode: db.sending.mode,
    relay: r ? { provider: r.provider, host: r.host, port: r.port, username: r.username, hasPassword: !!r.password } : null,
    providers: PROVIDERS,
    port25: port25Check(db),
    ptr: ptrCheck(db),
  };
}

function portalMe(db: DB): PortalMe {
  const m = findMailboxByAddress(db, db.portalAddress!)!;
  const address = addressOf(db, m);
  return {
    address,
    displayName: m.displayName,
    quotaBytes: m.quotaBytes,
    usedBytes: m.usedBytes,
    appPasswords: db.appPasswords[address] ?? [],
    settings: { imap: { host: HOST, port: 993, security: "SSL/TLS" }, smtp: { host: HOST, port: 465, security: "SSL/TLS" } },
    clientOrigin: CLIENT_ORIGIN,
  };
}

/** Drop-in `fetch` for /api/yunori/* — same status codes and error shapes as the server. */
export async function mockFetch(input: RequestInfo | URL, init: RequestInit = {}): Promise<Response> {
  const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url, window.location.origin);
  const method = (init.method ?? "GET").toUpperCase();
  const path = url.pathname.replace(/^\/api\/yunori/, "") || "/";
  const slow = path === "/preflight" || path.endsWith("/dns/check");
  await sleep(slow ? 900 + Math.random() * 500 : 90 + Math.random() * 160);
  if (init.signal?.aborted) throw new DOMException("Aborted", "AbortError");

  const headers = new Headers(init.headers);
  if (method !== "GET" && headers.get("X-Yunori-Request") !== "1") {
    return json(403, { error: { code: "csrf", message: "Missing X-Yunori-Request header." } });
  }
  let body: Body = {};
  if (typeof init.body === "string" && init.body) {
    try {
      body = JSON.parse(init.body) as Body;
    } catch {
      return json(400, { error: { code: "bad_json", message: "The request body isn't valid JSON." } });
    }
  }

  const db = load();
  try {
    const [status, payload] = route(db, method, path, url.searchParams, body);
    save(db);
    return json(status, payload);
  } catch (err) {
    save(db);
    if (err instanceof HttpError) return json(err.status, { error: err.error });
    console.error("[mock]", err);
    return json(500, { error: { code: "mock_error", message: "The mock threw — see the console." } });
  }
}
