/**
 * In-memory mock of the yunori-mail API (every endpoint in ./types.ts).
 *
 * Loaded ONLY when the console is built/served with VITE_MOCK=1 (`npm run
 * dev:mock`, `npm run build:mock`); client.ts imports it dynamically behind a
 * build-time literal, so production bundles never contain it.
 *
 * State is consistent across endpoints (a created mailbox appears in lists,
 * counts and the portal) and survives reloads via sessionStorage. Scenarios:
 *   ?mock=setup | signedout | certificate | installing | healthy | degraded
 * (or the dev panel, bottom-left) reseed the data.
 */
import type {
  AdminUser,
  Alias,
  ApiError,
  AppPassword,
  AuditEntry,
  CertificateInfo,
  CertProblem,
  CertState,
  Check,
  CheckStatus,
  DnsPlan,
  DnsPlanAction,
  DnsPlanItem,
  DnsProviderInfo,
  DnsProviderStatus,
  DnsPublishResponse,
  DnsPublishResult,
  DnsPublishSummary,
  DnsRecord,
  DomainDnsResponse,
  Domain,
  InstallLogLine,
  InstallState,
  InstallStep,
  ImportCreated,
  ImportSkipped,
  Mailbox,
  MailRules,
  OverviewResponse,
  PortalMe,
  QueueSummary,
  RelayProvider,
  SendingResponse,
  RelayIdentity,
  RelayRecord,
  SessionResponse,
  SystemResponse,
  UpdateStatus,
  VerdictLevel,
} from "./types";
import { routeAlerts, seedAlerts, type MockAlerts } from "./mock-alerts";

export type Scenario = "setup" | "signedout" | "certificate" | "installing" | "healthy" | "degraded";

export const SCENARIOS: { key: Scenario; label: string; hint: string; path: string }[] = [
  { key: "setup", label: "First run", hint: "setupRequired — any token ≥ 8 chars; code: any 6 digits but 000000", path: "/console/setup" },
  { key: "signedout", label: "Signed out", hint: "aiko@example.com · any 12+ char password · any 6 digits but 000000", path: "/console/login" },
  { key: "certificate", label: "Certificate pending", hint: "Port 80 blocked at the provider — press “Try now” to let it issue", path: "/console" },
  { key: "installing", label: "Installing", hint: "DNS step waiting — “Check now”, then create the first mailbox inline", path: "/console" },
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

const SES_REGIONS = ["us-east-1", "us-east-2", "us-west-2", "ca-central-1", "eu-west-1", "eu-west-2", "eu-central-1", "eu-north-1", "ap-south-1", "ap-southeast-1", "ap-southeast-2", "ap-northeast-1", "sa-east-1"];
const AMAZON_MAIL_FROM = { mxTemplate: "feedback-smtp.{region}.amazonses.com", spf: "v=spf1 include:amazonses.com ~all" };
const plain = { hostTemplate: null, regions: [], defaultRegion: null, username: null, verify: "none" as const, dkimSelector: null, mailFrom: null, domainsUrl: null };

// Mirrors `RELAY_PROVIDERS` in src/model.rs.
const PROVIDERS: RelayProvider[] = [
  {
    key: "ses", label: "Amazon SES", host: null, hostTemplate: "email-smtp.{region}.amazonaws.com", port: 587,
    regions: SES_REGIONS, defaultRegion: "us-east-1", username: null, secretLabel: "SMTP password", spfInclude: "amazonses.com",
    verify: "sesDkim", dkimSelector: null, mailFrom: { label: null, ...AMAZON_MAIL_FROM, required: false },
    docsUrl: "https://docs.aws.amazon.com/ses/latest/dg/smtp-credentials.html", domainsUrl: "https://console.aws.amazon.com/ses/home?region={region}#/identities",
  },
  {
    key: "resend", label: "Resend", host: "smtp.resend.com", hostTemplate: null, port: 587,
    regions: ["us-east-1", "eu-west-1", "sa-east-1", "ap-northeast-1"], defaultRegion: "us-east-1", username: "resend", secretLabel: "API key", spfInclude: null,
    verify: "dkimTxt", dkimSelector: "resend", mailFrom: { label: "send", ...AMAZON_MAIL_FROM, required: true },
    docsUrl: "https://resend.com/docs/send-with-smtp", domainsUrl: "https://resend.com/domains",
  },
  { ...plain, key: "postmark", label: "Postmark", host: "smtp.postmarkapp.com", port: 587, secretLabel: "Server API token", spfInclude: null, docsUrl: "https://postmarkapp.com/developer/user-guide/send-email-with-smtp" },
  { ...plain, key: "mailgun", label: "Mailgun", host: "smtp.mailgun.org", port: 587, secretLabel: "SMTP password", spfInclude: "mailgun.org", docsUrl: "https://documentation.mailgun.com/docs/mailgun/user-manual/sending-messages/send-smtp" },
  { ...plain, key: "smtp2go", label: "SMTP2GO", host: "mail.smtp2go.com", port: 587, secretLabel: "SMTP password", spfInclude: "spf.smtp2go.com", docsUrl: "https://support.smtp2go.com/" },
  { ...plain, key: "brevo", label: "Brevo", host: "smtp-relay.brevo.com", port: 587, secretLabel: "SMTP key", spfInclude: "spf.brevo.com", docsUrl: "https://help.brevo.com/" },
  { ...plain, key: "custom", label: "Other SMTP relay", host: null, port: 587, secretLabel: "Password", spfInclude: null, docsUrl: null },
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
interface MockCert {
  state: CertState;
  problem: CertProblem | null;
  /** A "Try now" attempt finishes at this time. */
  attemptUntil: number | null;
  lastAttempt: string | null;
  nextAttempt: string | null;
  failures: number;
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
  sending: {
    mode: "direct" | "relay";
    relay: { provider: string; host: string; port: number; username: string; password: string | null; region: string | null; spfInclude: string | null; identities: Record<string, RelayIdentity> } | null;
  };
  port25Open: boolean;
  queue: QueueSummary;
  install: InstallRow | null;
  installComplete: boolean;
  cert: MockCert;
  audit: AuditEntry[];
  portalAddress: string | null;
  appPasswords: Record<string, AppPassword[]>;
  /** Invite links by token (the real server keeps only a hash). */
  invites?: Record<string, { mailboxId: string; expiresAt: string }>;
  /** Auto-reply & forwarding per mailbox address (the real server keeps a Sieve script). */
  rules?: Record<string, MailRules>;
  /** Connected DNS providers by domain id (the real server seals the token; the mock keeps none). */
  dnsProviders?: Record<string, MockDnsProvider>;
  /** Alert settings, firing alerts and history (./mock-alerts.ts). */
  alerts?: MockAlerts;
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
    cert: { state: "valid", problem: null, attemptUntil: null, lastAttempt: iso(14 * DAY), nextAttempt: null, failures: 0 },
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
      mailboxes: [],
      install: installingState("dns"),
      audit: installerAudit(),
    };
  }

  if (scenario === "installing" || scenario === "certificate") {
    const cert = scenario === "certificate";
    return {
      ...base,
      installComplete: false,
      domains: [primaryDomain("installing")],
      mailboxes: [],
      install: installingState(cert ? "certificate" : "dns"),
      cert: cert
        ? { state: "self_signed", problem: port80Problem(), attemptUntil: null, lastAttempt: iso(2 * MIN), nextAttempt: new Date(now() + 4 * MIN).toISOString(), failures: 3 }
        : base.cert,
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

function port80Problem(): CertProblem {
  return {
    kind: "unreachable",
    name: HOST,
    summary: `Let's Encrypt could not connect to ${HOST} on port 80.`,
    remedy: "Allow incoming TCP 80 and 443 in your provider's firewall or security group (the server's own firewall was opened by the installer).",
    detail: `${IPV4}: Fetching http://${HOST}/.well-known/acme-challenge/yWq…: Timeout during connect (likely firewall problem)`,
  };
}

/** Mirrors `STEPS` in src/installer/state.rs. */
function installingState(stage: "certificate" | "dns"): InstallRow {
  const t = (m: number) => iso(m * MIN);
  const step = (id: string, title: string, status: InstallStep["status"], detail: string | null, start?: number, end?: number): InstallStep => ({
    id,
    title,
    status,
    detail,
    startedAt: start === undefined ? null : t(start),
    finishedAt: end === undefined ? null : t(end),
  });
  const cert = stage === "certificate";
  return {
    ptrStartedAt: null,
    steps: [
      step("preflight", "Check this server", "done", "Ready, with 1 warning to review.", 19, 19),
      step("inputs", "Your domain", "done", `example.com on ${HOST} (${IPV4}).`, 18, 18),
      step("host_record", "Point the mail hostname here", "done", `${HOST} → ${IPV4}.`, 18, 16),
      step("firewall", "Open mail ports", "done", "Opened 25, 80, 443, 465, 587, 993, 4190 in ufw.", 16, 16),
      step("install_engine", "Install the mail engine", "done", `Stalwart ${ENGINE} downloaded and verified; local resolver installed.`, 16, 14),
      step("bootstrap_engine", "Configure the mail engine", "done", `Stalwart ${ENGINE} is running.`, 14, 12),
      step("edge_tls", "Start the console and HTTPS edge", "done", "The console and HTTPS edge are running on ports 80 and 443.", 12, 12),
      cert
        ? step(
            "certificate",
            "Get a trusted TLS certificate",
            "waiting",
            `${port80Problem().summary} ${port80Problem().remedy} Retrying by itself in 4 minutes.`,
            12,
          )
        : step("certificate", "Get a trusted TLS certificate", "done", `Let's Encrypt issued a certificate for ${HOST}, valid until ${fmtIsoDay(now() + 89 * DAY)}; it renews automatically.`, 12, 11),
      cert
        ? step("dns", "Publish DNS records", "pending", null)
        : step("dns", "Publish DNS records", "waiting", "Publish 5 records at your DNS provider: MX example.com, TXT example.com, TXT v1-ed25519-" + SETUP_DAY + "._domainkey.example.com, TXT v1-rsa-" + SETUP_DAY + "._domainkey.example.com, TXT _dmarc.example.com.", 11),
      step("ptr", "Set reverse DNS", "pending", null),
      step("first_mailbox", "Create your first mailbox", "pending", null),
      step("done", "Finish", "pending", null),
    ],
    logs: [
      log(19, "info", "preflight", "Ready, with 1 warning to review."),
      log(18, "info", "inputs", `example.com on ${HOST} (${IPV4}).`),
      log(16, "info", "host_record", `${HOST} → ${IPV4}.`),
      log(14, "info", "install_engine", `Stalwart ${ENGINE} downloaded and verified; local resolver installed.`),
      log(12, "info", "bootstrap_engine", "Listeners and proxy trust configured"),
      log(12, "info", "certificate", "Requesting a certificate from Let's Encrypt: there is no certificate yet."),
      ...(cert
        ? [
            log(12, "warn", "certificate", `${port80Problem().summary} ${port80Problem().remedy}`),
            log(12, "warn", "certificate", `The CA said: ${port80Problem().detail}`),
            log(2, "info", "certificate", "Requesting a certificate from Let's Encrypt: a temporary self-signed certificate is in use."),
            log(2, "warn", "certificate", `${port80Problem().summary} ${port80Problem().remedy}`),
          ]
        : [log(11, "info", "certificate", `Let's Encrypt issued a certificate for ${HOST}.`), log(10, "info", "dns", "Waiting for MX, SPF, DKIM (2) and DMARC on example.com")]),
    ],
  };
}

function fmtIsoDay(ms: number) {
  return new Date(ms).toISOString().slice(0, 10);
}

function log(minAgo: number, level: InstallLogLine["level"], step: string | null, message: string): InstallLogLine {
  return { at: iso(minAgo * MIN), level, step, message };
}

function installerAudit(): AuditEntry[] {
  return [
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

// ── DNS provider automation ───────────────────────────────────────────────────

interface MockDnsProvider {
  provider: string;
  zoneName: string;
  connectedAt: string;
  connectedBy: string;
  lastPublish: DnsPublishSummary | null;
}

const DNS_PROVIDERS: DnsProviderInfo[] = [
  {
    id: "cloudflare",
    label: "Cloudflare",
    tokenHelp:
      'In Cloudflare, open My Profile → API Tokens → Create Token, start from "Edit zone DNS" and limit it to this domain\'s zone (Zone → DNS → Edit; Zone → Zone → Read).',
    tokenUrl: "https://dash.cloudflare.com/profile/api-tokens",
  },
];

function dnsProviderStatus(db: DB, d: DomainRow): DnsProviderStatus {
  const c = db.dnsProviders?.[d.id];
  return {
    domain: d.name,
    providers: DNS_PROVIDERS,
    connected: c ? { ...c, label: DNS_PROVIDERS.find((p) => p.id === c.provider)?.label ?? "DNS provider", hasSecret: true } : null,
  };
}

function dnsPlanFor(db: DB, d: DomainRow, c: MockDnsProvider): DnsPlan {
  const items: DnsPlanItem[] = dnsFor(db, d)
    .filter((r) => r.type !== "PTR")
    .map((r): DnsPlanItem => {
      const base = { id: r.id, type: r.type, name: r.name, required: r.required, value: r.expected };
      const state = d.published[r.id] ?? "missing";
      if (r.id === "aaaa") return { ...base, action: "skip", current: [], note: "Optional, and left to you: publish an IPv6 address only once reverse DNS for IPv6 is set too." };
      if (r.type === "CAA" && state !== "ok")
        return { ...base, action: "skip", current: [], note: "No CAA records exist, so every certificate authority may already issue for this domain. Nothing to add." };
      if (state === "ok") return { ...base, action: "unchanged", current: [r.expected] };
      if (state === "stale") {
        const current = r.actual.length ? r.actual : ["(an older value)"];
        if (r.id === "dmarc") return { ...base, action: "update", current, note: "Changes the policy to the DMARC step this server is on. Your report addresses stay." };
        return { ...base, action: "update", current };
      }
      if (r.id === "autodiscover" && d.isPrimary)
        return {
          ...base,
          action: "manual",
          current: ["A 198.51.100.7"],
          note: `${r.name} already has an A record, and a CNAME cannot sit beside other records. Remove it at Cloudflare if you no longer need it, then publish again.`,
        };
      return { ...base, action: "create", current: [] };
    });
  const counts: Record<DnsPlanAction, number> = { create: 0, update: 0, unchanged: 0, manual: 0, skip: 0 };
  for (const i of items) counts[i.action]++;
  const changes = items.filter((i) => i.action === "create" || i.action === "update");
  const planId = String(hashOf(changes.map((i) => `${i.id}:${i.action}`).join("|")));
  return { domain: d.name, provider: c.provider, providerLabel: "Cloudflare", zoneName: c.zoneName, items, counts, planId };
}

/** /domains/:id/dns/provider|plan|publish, or null when the path is something else. */
function dnsProviderRoute(db: DB, d: DomainRow, method: string, seg: string[], body: Body): [number, unknown?] | null {
  const sub = seg[3];
  if (sub === "provider") {
    if (method === "GET") return [200, dnsProviderStatus(db, d)];
    if (method === "DELETE") {
      if (db.dnsProviders?.[d.id]) {
        delete db.dnsProviders[d.id];
        audit(db, "dns.provider.disconnect", d.name);
      }
      return [200, dnsProviderStatus(db, d)];
    }
    if (method === "PUT") {
      const provider = str(body, "provider");
      const token = str(body, "token").trim();
      if (!DNS_PROVIDERS.some((p) => p.id === provider)) fail(400, "invalid_request", `Unknown DNS provider '${provider}'.`, "provider");
      const existing = db.dnsProviders?.[d.id];
      if (!token && !existing) fail(400, "invalid_request", "Enter the API token.", "token");
      if (token && (token.length < 20 || /\s/.test(token)))
        fail(422, "provider_auth", "That does not look like a Cloudflare API token (about 40 letters, digits, - and _). Copy it again from Cloudflare.", "token");
      if (token.toLowerCase().includes("bad"))
        fail(422, "provider_auth", "Cloudflare did not accept this token. Check that you copied all of it and that it has not expired or been revoked.", "token");
      if (token.toLowerCase().includes("nozone"))
        fail(
          422,
          "provider_zone",
          `This token cannot see a Cloudflare zone for ${d.name}. Check that ${d.name} is on this Cloudflare account and that the token's Zone Resources include it.`,
          "token",
        );
      const actor = db.admins.find((a) => a.id === db.adminId)?.email ?? "owner";
      db.dnsProviders = {
        ...(db.dnsProviders ?? {}),
        [d.id]: { provider, zoneName: d.name, connectedAt: iso(), connectedBy: actor, lastPublish: existing?.lastPublish ?? null },
      };
      audit(db, "dns.provider.connect", d.name, `Cloudflare zone ${d.name}`);
      return [200, dnsProviderStatus(db, d)];
    }
  }
  if (sub === "plan" || sub === "publish") {
    const c = db.dnsProviders?.[d.id];
    if (!c) return fail(409, "conflict", `No DNS provider is connected for ${d.name}.`);
    const plan = dnsPlanFor(db, d, c);
    if (sub === "plan" && method === "GET") return [200, plan];
    if (sub === "publish" && method === "POST") {
      if (typeof body.planId === "string" && body.planId !== plan.planId)
        fail(409, "plan_changed", "The records at the DNS provider changed since the preview. Review the new preview, then publish.");
      const only = Array.isArray(body.only) ? (body.only as string[]) : null;
      const dryRun = body.dryRun === true;
      const results: DnsPublishResult[] = [];
      if (!dryRun) {
        for (const i of plan.items) {
          if ((i.action !== "create" && i.action !== "update") || (only && !only.includes(i.id))) continue;
          d.published[i.id] = "ok";
          results.push({ id: i.id, ok: true });
        }
      }
      const created = results.filter((r) => plan.items.find((i) => i.id === r.id)?.action === "create").length;
      const updated = results.length - created;
      let dns: DomainDnsResponse | undefined;
      if (!dryRun) {
        if (results.length) {
          c.lastPublish = { at: iso(), by: c.connectedBy, created, updated, failed: 0 };
          audit(db, "dns.publish", d.name, `Cloudflare: created ${created}, updated ${updated}, failed 0 (${results.map((r) => r.id).join(", ")})`);
        }
        d.dnsCheckedAt = iso();
        const records = dnsFor(db, d);
        dns = { domain: d.name, records, checkedAt: d.dnsCheckedAt, zoneFile: zoneFile(d.name, records) };
      }
      const out: DnsPublishResponse = { ...plan, dryRun, results, created, updated, failed: 0, ...(dns ? { dns } : {}) };
      return [200, out];
    }
  }
  return null;
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
  const invite = Object.values(db.invites ?? {}).find((i) => i.mailboxId === m.id && Date.parse(i.expiresAt) > Date.now());
  return {
    ...rest,
    address,
    aliases: db.aliases.filter((a) => a.mailboxId === m.id).map((a) => a.address),
    inviteExpiresAt: invite?.expiresAt ?? null,
  };
}

/** A fresh invite for a mailbox, replacing an unused one (7 days, single use). */
function createInvite(db: DB, mailboxId: string) {
  const invites = Object.fromEntries(Object.entries(db.invites ?? {}).filter(([, i]) => i.mailboxId !== mailboxId));
  const token = randomFrom("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz0123456789", 43);
  const expiresAt = new Date(Date.now() + 7 * 864e5).toISOString().replace(/\.\d+Z$/, "Z");
  invites[token] = { mailboxId, expiresAt };
  db.invites = invites;
  return { url: `${window.location.origin}/account/welcome#invite=${token}`, expiresAt };
}

function liveInvite(db: DB, token: string) {
  const i = db.invites?.[token];
  const m = i && Date.parse(i.expiresAt) > Date.now() ? db.mailboxes.find((x) => x.id === i.mailboxId) : undefined;
  if (!i || !m)
    fail(410, "invite_invalid", "This invite link has expired or was already used. Ask the person who runs this mail server for a new one.");
  return { invite: i!, mailbox: m! };
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
    db.cert.state === "valid"
      ? { id: "tls", label: "TLS certificate", status: "pass", detail: `Valid for 64 more days (${HOST}); renews automatically.` }
      : {
          id: "tls",
          label: "TLS certificate",
          status: "warn",
          detail: "A temporary self-signed certificate is in use until Let's Encrypt issues the real one.",
          remedy: db.cert.problem ? `${db.cert.problem.summary} ${db.cert.problem.remedy}` : undefined,
        },
    port25Check(db),
    { id: "blocklists", label: "Blocklists", status: "pass", detail: `${IPV4} isn't listed on Spamhaus ZEN, Barracuda or SpamCop.` },
    { id: "engine", label: "Mail engine", status: "pass", detail: `Stalwart ${ENGINE} is running; SMTP, IMAP and JMAP answer.` },
    queue,
  ];
}

function verdict(db: DB): { level: VerdictLevel; sentence: string } {
  if (db.install && !db.install.steps.every((s) => s.status === "done" || s.status === "skipped")) {
    const st = (id: string) => db.install!.steps.find((s) => s.id === id)?.status;
    if (st("certificate") === "waiting")
      return { level: "attention", sentence: `Setup is waiting on a certificate: Let's Encrypt can't reach ${HOST} on port 80 yet.` };
    if (st("dns") === "waiting") return { level: "attention", sentence: "Setup is waiting on you: publish the DNS records for example.com so mail can find this server." };
    if (st("first_mailbox") === "waiting") return { level: "attention", sentence: "Almost there: create your first mailbox." };
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
    alerts: (db.alerts ??= seedAlerts(db.scenario)).active,
    setup: db.install ? { steps: db.install.steps, logs: db.install.logs } : null,
  };
}

function setStep(ins: InstallRow, id: string, patch: Partial<InstallStep>) {
  ins.steps = ins.steps.map((s) => (s.id === id ? { ...s, ...patch } : s));
}

function advanceInstall(db: DB) {
  advanceCert(db);
  const ins = db.install;
  if (!ins || ins.ptrStartedAt === null) return;
  if (now() - ins.ptrStartedAt < 6000) return;
  setStep(ins, "ptr", { status: "done", detail: `${IPV4} → ${HOST}`, finishedAt: iso() });
  setStep(ins, "first_mailbox", { status: "waiting", detail: "Create your first mailbox: right here, or with `sudo yunori-mail install --resume` on the server.", startedAt: iso() });
  ins.logs = [...ins.logs, log(0, "info", "ptr", `${IPV4} → ${HOST}`)];
  ins.ptrStartedAt = null;
}

/** A "Try now" attempt in flight finishes: the certificate is issued. */
function advanceCert(db: DB) {
  const c = db.cert;
  if (c.attemptUntil === null || now() < c.attemptUntil) return;
  c.attemptUntil = null;
  c.state = "valid";
  c.problem = null;
  c.failures = 0;
  c.nextAttempt = null;
  c.lastAttempt = iso();
  const ins = db.install;
  if (ins && ins.steps.find((s) => s.id === "certificate")?.status === "waiting") {
    setStep(ins, "certificate", {
      status: "done",
      detail: `Let's Encrypt issued a certificate for ${HOST}, valid until ${fmtIsoDay(now() + 90 * DAY)}; it renews automatically.`,
      finishedAt: iso(),
    });
    setStep(ins, "dns", {
      status: "waiting",
      detail: `Publish 5 records at your DNS provider: MX example.com, TXT example.com, TXT v1-ed25519-${SETUP_DAY}._domainkey.example.com, TXT v1-rsa-${SETUP_DAY}._domainkey.example.com, TXT _dmarc.example.com.`,
      startedAt: iso(),
    });
    ins.logs = [...ins.logs, log(0, "info", "certificate", `Let's Encrypt issued a certificate for ${HOST}.`)];
  }
  audit(db, "certificate.issue", HOST, "Let's Encrypt: a temporary self-signed certificate is in use");
}

function certStatus(db: DB): CertificateInfo {
  advanceCert(db);
  const c = db.cert;
  const valid = c.state === "valid";
  const notAfter = valid ? new Date(now() + 64 * DAY).toISOString() : new Date(now() + 3650 * DAY).toISOString();
  return {
    state: c.state,
    hostnames: valid ? [HOST, ...db.domains.flatMap((d) => [`autoconfig.${d.name}`, `mta-sts.${d.name}`])] : [HOST],
    issuer: valid ? "C=US, O=Let's Encrypt, CN=R11" : `CN=${HOST}`,
    notAfter,
    daysLeft: valid ? 64 : 3650,
    authority: "Let's Encrypt",
    renewsAt: valid ? new Date(now() + 34 * DAY).toISOString() : null,
    lastAttempt: c.lastAttempt,
    lastSuccess: valid ? iso(26 * DAY) : null,
    nextAttempt: c.nextAttempt,
    failures: c.failures,
    problem: c.problem,
    inProgress: c.attemptUntil !== null,
    servedMatches: valid ? true : null,
  };
}

function finishInstall(db: DB) {
  const ins = db.install;
  if (!ins) return;
  setStep(ins, "done", { status: "done", detail: "Yunori Mail Server is ready.", startedAt: iso(), finishedAt: iso() });
  ins.logs = [...ins.logs, log(0, "info", "done", "Yunori Mail Server is ready.")];
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
    certificate: certStatus(db),
    update: updateStatus(),
  };
}

/** The healthy box is one release behind, so the System page shows the update card. */
function updateStatus(): UpdateStatus {
  return {
    current: VERSION,
    latest: "0.2.0",
    available: true,
    publishedAt: iso(2 * 864e5),
    notesUrl: "https://github.com/FutureForge-Studios/yunori-mail/releases/tag/v0.2.0",
    checkedAt: iso(3 * 3600e3),
    problem: null,
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

/** A non-JSON body (the CSV export). */
class Raw {
  constructor(
    readonly body: string,
    readonly headers: Record<string, string>,
  ) {}
}

function json(status: number, body?: unknown): Response {
  if (body instanceof Raw) return new Response(body.body, { status, headers: body.headers });
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
    if (is("POST", "/portal/invite")) {
      const { invite, mailbox } = liveInvite(db, str(body, "token").trim());
      return [200, { address: addressOf(db, mailbox), displayName: mailbox.displayName, expiresAt: invite.expiresAt }];
    }
    if (is("POST", "/portal/invite/accept")) {
      const token = str(body, "token").trim();
      const password = str(body, "password");
      if (password.length < 12) fail(400, "invalid_request", "Use at least 12 characters.", "password");
      const { mailbox } = liveInvite(db, token);
      delete db.invites![token];
      mailbox.password = password;
      db.portalAddress = addressOf(db, mailbox);
      audit(db, "mailbox.invite_accept", db.portalAddress);
      return [200, portalMe(db)];
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
    if (is("POST", "/portal/devices/apple")) {
      const device = str(body, "device").trim();
      if (!device || device.length > 40) fail(400, "invalid_request", "Name the device, like “iPhone” (up to 40 characters).", "device");
      const ap: AppPassword = { id: `ap_${++db.seq}`, description: `${device} (profile)`, createdAt: iso() };
      db.appPasswords[address] = [...(db.appPasswords[address] ?? []), ap];
      const token = randomFrom("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz0123456789", 32);
      const expiresAt = new Date(Date.now() + 10 * 60e3).toISOString().replace(/\.\d+Z$/, "Z");
      return [201, { url: `https://${HOST}/account/profile/${token}`, expiresAt, appPassword: ap }];
    }
    if (is("GET", "/portal/rules")) return [200, db.rules?.[address] ?? defaultRules()];
    if (is("PUT", "/portal/rules")) {
      const rules = normaliseRules(body, address);
      db.rules = { ...db.rules, [address]: rules };
      return [200, rules];
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
  if (seg[0] === "alerts") {
    const r = routeAlerts((db.alerts ??= seedAlerts(db.scenario)), method, path, body, fail, (action, target, detail) =>
      audit(db, action, target, detail ?? null),
    );
    if (r) return r;
  }
  if (is("POST", "/updates/check")) return [200, { ...updateStatus(), checkedAt: iso() }];
  if (is("GET", "/install")) {
    advanceInstall(db);
    const st: InstallState = db.install ? { steps: db.install.steps, logs: db.install.logs } : { steps: [], logs: [] };
    return [200, st];
  }
  if (is("GET", "/certificate")) return [200, certStatus(db)];
  if (is("POST", "/certificate/renew")) {
    if (db.cert.attemptUntil === null) db.cert.attemptUntil = now() + 2500;
    audit(db, "certificate.renew_requested", HOST);
    return [202, certStatus(db)];
  }
  if (is("POST", "/install/first-mailbox")) {
    const ins = db.install;
    if (!ins) fail(409, "conflict", "Setup is already complete; create mailboxes under Mailboxes.");
    const existing = db.mailboxes.find((m) => m.role === "user");
    if (existing) fail(409, "conflict", `${addressOf(db, existing!)} already exists, so this step is done.`);
    const local = str(body, "local").trim().toLowerCase();
    if (!LOCAL_RE.test(local)) fail(400, "invalid", "Use letters, digits, dots, dashes or underscores.", "local");
    const primary = db.domains.find((d) => d.isPrimary)!;
    const password = genPassword();
    const row: MailboxRow = {
      id: `mbx_${++db.seq}`,
      local,
      domainId: primary.id,
      displayName: str(body, "displayName").trim() || null,
      role: "user",
      quotaBytes: null,
      usedBytes: 0,
      createdAt: iso(),
      password,
    };
    db.mailboxes.push(row);
    const address = addressOf(db, row);
    for (const a of ["postmaster", "abuse"]) {
      if (a !== local) db.aliases.push({ address: `${a}@${primary.name}`, mailboxId: row.id, mailboxAddress: address, enabled: true });
    }
    setStep(ins!, "first_mailbox", { status: "done", detail: `${address} exists.`, finishedAt: iso() });
    ins!.logs = [...ins!.logs, log(0, "info", "first_mailbox", `${address} created (postmaster@ and abuse@ deliver to it).`)];
    audit(db, "mailbox.create", address, "first mailbox; postmaster+abuse aliases");
    finishInstall(db);
    return [201, { mailbox: toMailbox(db, row), password }];
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
      const provided = dnsProviderRoute(db, dom, method, seg, body);
      if (provided) return provided;
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
    const inviteMode = body.invite === true;
    const pw = !inviteMode && typeof body.password === "string" && body.password ? body.password : null;
    if (pw !== null && pw.length < 12) fail(400, "weak_password", "Use at least 12 characters.", "password");
    const password = pw ?? genPassword();
    const quotaMb = typeof body.quotaMb === "number" ? body.quotaMb : null;
    const displayName = str(body, "displayName").trim() || null;
    const row: MailboxRow = { id: `mbx_${++db.seq}`, local, domainId: d!.id, displayName, role: "user", quotaBytes: quotaMb ? quotaMb * MB : null, usedBytes: 0, createdAt: iso(), password };
    db.mailboxes.push(row);
    audit(db, "mailbox.create", address, quotaMb ? `Quota ${quotaMb >= 1024 ? `${quotaMb / 1024} GB` : `${quotaMb} MB`}` : "Unlimited");
    if (inviteMode) {
      const invite = createInvite(db, row.id);
      audit(db, "mailbox.invite", address);
      return [201, { mailbox: toMailbox(db, row), invite }];
    }
    return [200, { mailbox: toMailbox(db, row), password }];
  }
  if (is("GET", "/mailboxes/export")) {
    const mb = (b: number | null) => (b === null ? "" : String(Math.floor(b / MB)));
    const lines = [CSV_HEADER, ...db.mailboxes.map((m) => {
      const x = toMailbox(db, m);
      return [x.address, x.displayName ?? "", mb(x.quotaBytes), mb(x.usedBytes), x.role, x.aliases.join(" "), x.createdAt ?? ""];
    })];
    audit(db, "mailbox.export", null, `${db.mailboxes.length} mailboxes`);
    const stamp = new Date().toISOString().slice(0, 10).replace(/-/g, "");
    return [
      200,
      new Raw(lines.map((l) => l.map(csvField).join(",") + "\r\n").join(""), {
        "Content-Type": "text/csv; charset=utf-8",
        "Content-Disposition": `attachment; filename="mailboxes-${stamp}.csv"`,
      }),
    ];
  }
  if (is("POST", "/mailboxes/import")) return [200, importMailboxes(db, body)];
  if (seg[0] === "mailboxes" && seg[1]) {
    const m = db.mailboxes.find((x) => x.id === seg[1]);
    if (!m) fail(404, "not_found", "That mailbox doesn't exist (any more).");
    const box = m!;
    const address = addressOf(db, box);
    if (method === "POST" && seg[2] === "invite") {
      if (box.role === "admin") fail(409, "conflict", "This is the mail engine's administrator account; it is not for reading mail.");
      const invite = createInvite(db, box.id);
      audit(db, "mailbox.invite", address);
      return [200, invite];
    }
    if (method === "POST" && seg[2] === "password") {
      const given = str(body, "password");
      if (given && given.length < 12) fail(400, "weak_password", "Use at least 12 characters.", "password");
      db.invites = Object.fromEntries(Object.entries(db.invites ?? {}).filter(([, i]) => i.mailboxId !== box.id));
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
      const p = provider!;
      const region = p.regions.length ? (str(r, "region") || p.defaultRegion) : null;
      if (p.regions.length && !/^[a-z]{2}(-[a-z]+)+-\d{1,2}$/.test(region ?? "")) fail(400, "invalid", "That isn't a region name, like us-east-1.", "region");
      const host = p.hostTemplate && region ? p.hostTemplate.replace("{region}", region) : str(r, "host").trim() || p.host || "";
      const port = Number(r.port);
      const username = str(r, "username").trim() || p.username || "";
      const password = str(r, "password");
      if (!host || !/^[a-z0-9.-]+$/i.test(host)) fail(400, "invalid", "Enter the relay's host name.", "host");
      if (!Number.isInteger(port) || port < 1 || port > 65535) fail(400, "invalid", "Enter a port.", "port");
      if (!username) fail(400, "invalid", "Enter the relay username.", "username");
      const keep = db.sending.relay?.password ?? null;
      if (!password && !keep) fail(400, "invalid", "Enter the relay password.", "password");
      const inputs = r.identities as Record<string, { dkim?: string; mailFrom?: string }> | undefined;
      const identities: Record<string, RelayIdentity> = inputs ? {} : (db.sending.relay?.identities ?? {});
      for (const [domain, input] of Object.entries(inputs ?? {})) {
        const dkim = (input.dkim ?? "").trim();
        const id: RelayIdentity = {};
        if (p.verify === "sesDkim") {
          if (dkim) {
            const tokens = [...new Set(dkim.toLowerCase().split(/[^a-z0-9]+/).filter((w) => w.length === 32))];
            if (tokens.length !== 3) fail(400, "invalid", tokens.length ? `Found ${tokens.length} DKIM token${tokens.length === 1 ? "" : "s"}; Amazon SES issues exactly three. Paste all three CNAME records.` : "Paste the three DKIM records Amazon SES shows for this domain.", `identities.${domain}.dkim`);
            id.dkimTokens = tokens;
          }
          const mf = (input.mailFrom ?? "").trim().toLowerCase().replace(/\.$/, "");
          if (mf) id.mailFrom = mf.endsWith(`.${domain}`) ? mf : `${mf}.${domain}`;
        } else if (p.verify === "dkimTxt" && dkim) {
          const v = dkim.replace(/["']/g, " ").replace(/\s+/g, " ").trim();
          const key = /p=([A-Za-z0-9+/= ]+)/.exec(v)?.[1]?.replace(/\s/g, "") ?? (/^[A-Za-z0-9+/=]+$/.test(v) ? v : "");
          if (key.length < 64) fail(400, "invalid", "That DKIM key looks incomplete. Copy the whole value from your provider.", `identities.${domain}.dkim`);
          id.dkimValue = v.includes("p=") ? v.replace(/p=[A-Za-z0-9+/= ]+/, `p=${key}`) : `p=${key}`;
        }
        if (Object.keys(id).length) identities[domain] = id;
      }
      db.sending = { mode, relay: { provider: p.key, host, port, username, password: password || keep, region, spfInclude: str(r, "spfInclude").trim().replace(/^include:/, "") || null, identities } };
      // The relay accepted the deferred mail on retry.
      db.queue = { total: 0, deferred: 0, groups: [] };
      audit(db, "sending.update", "relay", `${p.label} via ${host}:${port}`);
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

/** Mirrors `relay::domain_records` in src/relay.rs. */
function relayRecords(r: NonNullable<DB["sending"]["relay"]>, domain: string): RelayRecord[] {
  const p = PROVIDERS.find((x) => x.key === r.provider);
  if (!p) return [];
  const id = r.identities[domain];
  const out: RelayRecord[] = [];
  if (p.verify === "sesDkim")
    (id?.dkimTokens ?? []).forEach((t, i) =>
      out.push({ id: `relay:dkim:${i + 1}`, type: "CNAME", name: `${t}._domainkey.${domain}`, value: `${t}.dkim.amazonses.com`, required: true, purpose: `${p.label}: verifies this domain so SES will send for it (Easy DKIM).` }),
    );
  if (p.verify === "dkimTxt" && id?.dkimValue && p.dkimSelector)
    out.push({ id: "relay:dkim", type: "TXT", name: `${p.dkimSelector}._domainkey.${domain}`, value: id.dkimValue, required: true, purpose: `${p.label}: its DKIM key, which verifies this domain with ${p.label}.` });
  const mfName = p.mailFrom ? (p.mailFrom.label ? `${p.mailFrom.label}.${domain}` : id?.mailFrom) : undefined;
  const region = r.region ?? p.defaultRegion;
  if (p.mailFrom && mfName && region) {
    out.push({ id: "relay:mail-from-mx", type: "MX", name: mfName, value: `10 ${p.mailFrom.mxTemplate.replace("{region}", region)}`, required: p.mailFrom.required, purpose: `${p.label}: receives bounces for mail it sends as this domain.` });
    out.push({ id: "relay:mail-from-spf", type: "TXT", name: mfName, value: p.mailFrom.spf, required: p.mailFrom.required, purpose: `${p.label}: SPF for the bounce domain, so SPF aligns with your domain too.` });
  }
  return out;
}

function sendingView(db: DB): SendingResponse {
  const r = db.sending.relay;
  const p = r ? PROVIDERS.find((x) => x.key === r.provider) : undefined;
  return {
    mode: db.sending.mode,
    relay: r
      ? {
          provider: r.provider, host: r.host, port: r.port, username: r.username, region: r.region, spfInclude: r.spfInclude,
          identities: r.identities, hasPassword: !!r.password,
          domainsUrl: p?.domainsUrl ? p.domainsUrl.replace("{region}", r.region ?? p.defaultRegion ?? "us-east-1") : null,
        }
      : null,
    providers: PROVIDERS,
    domains: db.domains.map((d) => ({
      id: d.id,
      name: d.name,
      records: r ? relayRecords(r, d.name) : [],
      needsVerification: !!p && p.verify !== "none" && !(p.verify === "sesDkim" ? r?.identities[d.name]?.dkimTokens?.length : r?.identities[d.name]?.dkimValue),
    })),
    port25: port25Check(db),
    ptr: ptrCheck(db),
  };
}

// ── auto-reply & forwarding (mirrors src/rules.rs `normalised`) ─────────────

function defaultRules(): MailRules {
  return { vacation: { enabled: false, subject: "", body: "" }, forwarding: { enabled: false, address: "", keepCopy: true } };
}

function normaliseRules(body: Body, own: string): MailRules {
  const v = (body.vacation && typeof body.vacation === "object" ? body.vacation : {}) as Body;
  const f = (body.forwarding && typeof body.forwarding === "object" ? body.forwarding : {}) as Body;
  const clean = (t: string, newlines: boolean) =>
    [...t.replace(/\r\n/g, "\n")]
      .filter((c) => {
        const n = c.charCodeAt(0);
        return !(n < 32 || n === 127) || (newlines && (c === "\n" || c === "\t"));
      })
      .join("")
      .trim();
  const date = (k: string) => (typeof v[k] === "string" && (v[k] as string).trim() ? (v[k] as string).trim() : undefined);
  const vacation: MailRules["vacation"] = { enabled: v.enabled === true, subject: clean(str(v, "subject"), false), body: clean(str(v, "body"), true) };
  const startDate = date("startDate");
  const endDate = date("endDate");
  if (vacation.enabled) {
    if (!vacation.body) fail(400, "invalid_request", "Write the reply people will get.", "vacation.body");
    if (!vacation.subject) vacation.subject = "Out of office";
  }
  if ([...vacation.subject].length > 200) fail(400, "invalid_request", "Keep the subject under 200 characters.", "vacation.subject");
  if ([...vacation.body].length > 5000) fail(400, "invalid_request", "Keep the reply under 5000 characters.", "vacation.body");
  const validDate = (d: string) => /^\d{4}-\d{2}-\d{2}$/.test(d) && !Number.isNaN(Date.parse(`${d}T00:00:00Z`)) && new Date(`${d}T00:00:00Z`).toISOString().startsWith(d);
  if (startDate && !validDate(startDate)) fail(400, "invalid_request", "Use a date like 2026-12-24.", "vacation.startDate");
  if (endDate && !validDate(endDate)) fail(400, "invalid_request", "Use a date like 2026-12-24.", "vacation.endDate");
  if (startDate && endDate && endDate < startDate) fail(400, "invalid_request", "The last day comes before the first day.", "vacation.endDate");
  if (startDate) vacation.startDate = startDate;
  if (endDate) vacation.endDate = endDate;
  const forwarding: MailRules["forwarding"] = {
    enabled: f.enabled === true,
    address: str(f, "address").trim().toLowerCase(),
    keepCopy: f.keepCopy !== false,
  };
  if (forwarding.enabled) {
    if (!EMAIL_RE.test(forwarding.address)) fail(400, "invalid_request", "Enter a valid email address.", "forwarding.address");
    if (forwarding.address === own.toLowerCase()) fail(400, "invalid_request", "That is this mailbox; forward to another address.", "forwarding.address");
  }
  return { vacation, forwarding };
}

// ── CSV export / import (mirrors src/csv.rs and admin::import_mailboxes) ─────

const CSV_HEADER = ["address", "display_name", "quota_mb", "used_mb", "role", "aliases", "created_at"];
const IMPORT_MAX_ROWS = 1000;

function csvField(v: string): string {
  const x = /^[=+\-@\t\r]/.test(v) ? `'${v}` : v;
  return /[,"\r\n]/.test(x) ? `"${x.replace(/"/g, '""')}"` : x;
}

function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;
  let line = 1;
  const src = text.replace(/^\ufeff/, "");
  const pushRow = () => {
    row.push(field);
    field = "";
    if (!(row.length === 1 && !row[0].trim())) rows.push(row);
    row = [];
  };
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    if (inQuotes) {
      if (c === '"' && src[i + 1] === '"') {
        field += '"';
        i++;
      } else if (c === '"') inQuotes = false;
      else {
        if (c === "\n") line++;
        field += c;
      }
      continue;
    }
    if (c === '"' && field === "") inQuotes = true;
    else if (c === ",") {
      row.push(field);
      field = "";
    } else if (c === "\r") continue;
    else if (c === "\n") {
      line++;
      pushRow();
    } else field += c;
  }
  if (inQuotes) fail(400, "invalid_request", `a quoted field is not closed (line ${line})`, "csv");
  if (field || row.length) pushRow();
  return rows;
}

function importMailboxes(db: DB, body: Body) {
  const rows = parseCsv(str(body, "csv"));
  if (rows[0]?.[0]?.trim().toLowerCase() === "address") rows.shift();
  if (rows.length === 0) fail(400, "invalid_request", "The file has no mailboxes in it.", "csv");
  if (rows.length > IMPORT_MAX_ROWS) fail(400, "invalid_request", `Import at most ${IMPORT_MAX_ROWS} mailboxes at a time.`, "csv");
  const invite = body.invite !== false;
  const dryRun = body.dryRun === true;
  const taken = new Set([...db.mailboxes.map((m) => addressOf(db, m)), ...db.aliases.map((a) => a.address)]);
  const ready: { address: string; local: string; domainId: string; displayName: string | null; quotaMb: number | null }[] = [];
  const skipped: ImportSkipped[] = [];
  rows.forEach((row, i) => {
    const address = (row[0] ?? "").trim().toLowerCase();
    const skip = (reason: string) => skipped.push({ line: i + 1, address, reason });
    const at = address.indexOf("@");
    if (at < 0) return skip("Not an email address.");
    const local = address.slice(0, at);
    const domainName = address.slice(at + 1);
    if (!LOCAL_RE.test(local) || !EMAIL_RE.test(address)) return skip("Use letters, digits, dots, dashes or underscores before the @.");
    const d = db.domains.find((x) => x.name === domainName);
    if (!d) return skip("That domain is not on this server; add it first.");
    if (taken.has(address)) return skip("Already in use (or listed twice).");
    const q = (row[2] ?? "").trim();
    let quotaMb: number | null = null;
    if (q) {
      if (!/^\d+$/.test(q)) return skip("The quota is not a whole number of MB.");
      quotaMb = Number(q) || null;
      if (quotaMb !== null && quotaMb > 16 * 1024 * 1024) return skip("A quota can be at most 16 TB.");
    }
    taken.add(address);
    ready.push({ address, local, domainId: d.id, displayName: (row[1] ?? "").trim() || null, quotaMb });
  });
  const created: ImportCreated[] = [];
  if (!dryRun) {
    for (const r of ready) {
      const password = genPassword();
      const mrow: MailboxRow = {
        id: `mbx_${++db.seq}`, local: r.local, domainId: r.domainId, displayName: r.displayName, role: "user",
        quotaBytes: r.quotaMb ? r.quotaMb * MB : null, usedBytes: 0, createdAt: iso(), password,
      };
      db.mailboxes.push(mrow);
      audit(db, "mailbox.create", r.address, "CSV import");
      if (invite) {
        const link = createInvite(db, mrow.id);
        audit(db, "mailbox.invite", r.address);
        created.push({ address: r.address, inviteUrl: link.url, inviteExpiresAt: link.expiresAt });
      } else created.push({ address: r.address, password });
    }
  }
  return { dryRun, ready: ready.map((r) => r.address), created, skipped };
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
    settings: {
      imap: { host: HOST, port: 993, security: "SSL/TLS" },
      smtp: { host: HOST, port: 465, security: "SSL/TLS" },
      dav: { host: HOST, url: `https://${HOST}/` },
    },
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
