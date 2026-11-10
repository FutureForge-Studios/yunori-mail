/**
 * Yunori Mail Server — console ⇄ yunori-mail API contract.
 *
 * Source of truth for the JSON the Rust server (`src/api/`) emits.
 * All endpoints live under `/api/yunori`. All bodies are JSON (camelCase).
 *
 * Auth: admin session cookie `yunori_session` (HttpOnly, Secure, SameSite=Strict).
 * Every non-GET request must send header `X-Yunori-Request: 1` (CSRF guard) and
 * `Content-Type: application/json`. Errors are `{ error: ApiError }` with a 4xx/5xx status.
 * 401 → not signed in; 403 → signed in but not allowed (or CSRF header missing).
 *
 * The end-user self-service portal uses a separate cookie `yunori_portal` and the
 * `/api/yunori/portal/*` endpoints at the bottom of this file.
 */

// ── Shared ────────────────────────────────────────────────────────────────

export interface ApiError {
  /** Stable machine code, e.g. "invalid_credentials", "totp_required", "not_found", "conflict", "engine_unavailable". */
  code: string;
  /** Plain-language sentence safe to show the user. */
  message: string;
  /** Optional field the error refers to (form validation). */
  field?: string;
}

export type CheckStatus = "pass" | "warn" | "fail" | "info" | "unknown";

/** One diagnostic result. Every status is paired with text — never color alone. */
export interface Check {
  id: string;
  label: string;
  status: CheckStatus;
  /** What we observed, in plain language. */
  detail: string;
  /** What to do about it (absent when nothing to do). */
  remedy?: string;
  /** Optional link for more help. */
  helpUrl?: string;
}

// ── Session / auth (GET /session, POST /setup, POST /setup/totp, POST /login, POST /logout) ──

export interface ServerInfo {
  /** e.g. "mail.example.com" */
  hostname: string;
  primaryDomain: string | null;
  version: string;
  engineVersion: string | null;
  /** True once the installer finished every required step. */
  installComplete: boolean;
  /** Origin of the Yunori client used for "Open in Yunori" deep links. */
  clientOrigin: string;
}

export interface AdminUser {
  id: string;
  email: string;
  role: "owner" | "admin" | "viewer";
  totpEnabled: boolean;
  createdAt: string; // RFC 3339
}

/** GET /session — always 200. */
export interface SessionResponse {
  authenticated: boolean;
  /** No admin exists yet: the console shows the first-run setup screen (needs the setup token from the installer). */
  setupRequired: boolean;
  admin: AdminUser | null;
  server: ServerInfo;
}

/** POST /setup — create the first owner. The token is printed once by `yunori-mail install`. */
export interface SetupRequest {
  token: string;
  email: string;
  password: string; // ≥ 12 chars
}
export interface SetupResponse {
  /** Base32 TOTP secret + otpauth:// URL for the QR code. Second factor is mandatory. */
  totpSecret: string;
  otpauthUrl: string;
}
/** POST /setup/totp — confirm the first code; on success sets the session cookie. */
export interface SetupTotpRequest {
  code: string;
}

/** POST /login — on success sets the session cookie and returns the SessionResponse. */
export interface LoginRequest {
  email: string;
  password: string;
  totp: string;
}

// ── Overview (GET /overview) ──────────────────────────────────────────────

export type VerdictLevel = "ok" | "attention" | "degraded" | "down";

export interface OverviewResponse {
  verdict: {
    level: VerdictLevel;
    /** One sentence that names the consequence, e.g. "Receiving works; sending is relayed because port 25 is blocked." */
    sentence: string;
  };
  stats: {
    domains: number;
    mailboxes: number;
    aliases: number;
    queued: number;
  };
  sendingMode: "direct" | "relay";
  /** Key signals (DNS, PTR, TLS, port 25, blocklists, engine). */
  signals: Check[];
  /** Present until setup is complete. */
  setup: InstallState | null;
}

// ── Install progress (GET /install) ──────────────────────────────────────

export type StepStatus = "pending" | "running" | "done" | "failed" | "waiting" | "skipped";

export interface InstallStep {
  id: string; // "preflight" | "inputs" | "host_record" | "firewall" | "install_engine" | "bootstrap_engine" | "edge_tls" | "certificate" | "dns" | "ptr" | "first_mailbox" | "done"
  title: string;
  status: StepStatus;
  /** Plain-language status line; for "waiting" it says exactly what the user must do. */
  detail: string | null;
  startedAt: string | null;
  finishedAt: string | null;
}

export interface InstallLogLine {
  at: string;
  level: "info" | "warn" | "error";
  step: string | null;
  message: string;
}

export interface InstallState {
  steps: InstallStep[];
  logs: InstallLogLine[];
}

// ── Domains ──────────────────────────────────────────────────────────────

export interface Domain {
  id: string;
  name: string;
  enabled: boolean;
  isPrimary: boolean;
  catchAll: string | null;
  mailboxCount: number;
  /** Aliases on this domain (dependents shown before a delete). */
  aliasCount: number;
  /** Worst status across required DNS records at last check. */
  dnsStatus: CheckStatus;
  dnsCheckedAt: string | null;
  createdAt: string | null;
}

/** GET /domains → Domain[] ; GET /domains/:id → Domain ; POST /domains {name} → Domain ; PATCH /domains/:id {catchAll?: string|null, enabled?: boolean} → Domain ; DELETE /domains/:id → 204 (409 "conflict" while mailboxes exist). */
export interface CreateDomainRequest {
  name: string;
}
export interface UpdateDomainRequest {
  catchAll?: string | null;
  enabled?: boolean;
}

export type DnsRecordType = "A" | "AAAA" | "MX" | "TXT" | "CNAME" | "SRV" | "CAA" | "TLSA" | "PTR";

export interface DnsRecord {
  /** Stable id, e.g. "mx", "spf", "dkim:v1-ed25519-20261105", "dmarc", "mta-sts", "tls-rpt", "autoconfig", "srv:_imaps._tcp", "ptr". */
  id: string;
  type: DnsRecordType;
  /** Fully-qualified name without trailing dot, e.g. "_dmarc.example.com". */
  name: string;
  /** Exact value to publish (copy button copies this). */
  expected: string;
  /** What public resolvers currently return (empty when nothing). */
  actual: string[];
  status: CheckStatus;
  required: boolean;
  /** One sentence: why this record exists. */
  purpose: string;
  remedy?: string;
}

/** GET /domains/:id/dns (cached) and POST /domains/:id/dns/check (fresh). */
export interface DomainDnsResponse {
  domain: string;
  records: DnsRecord[];
  checkedAt: string | null;
  /** BIND-style zone file for "copy all". */
  zoneFile: string;
}

// ── DNS provider automation ───────────────────────────────────────────────
// GET/PUT/DELETE /domains/:id/dns/provider → DnsProviderStatus (PUT {provider, token?}: token is
// write-only, blank re-checks the saved one); GET /domains/:id/dns/plan → DnsPlan (read-only at the
// provider); POST /domains/:id/dns/publish {only?, planId?, dryRun?} → DnsPublishResponse
// (409 "plan_changed" when the provider changed since the preview).

export interface DnsProviderInfo {
  id: string;
  label: string;
  /** How to create a least-privilege token, in one paragraph. */
  tokenHelp: string;
  tokenUrl: string;
}

export interface DnsPublishSummary {
  at: string;
  by: string;
  created: number;
  updated: number;
  failed: number;
}

export interface DnsProviderConnection {
  provider: string;
  label: string;
  zoneName: string;
  /** The token itself is never returned. */
  hasSecret: boolean;
  connectedAt: string;
  connectedBy: string;
  lastPublish: DnsPublishSummary | null;
}

export interface DnsProviderStatus {
  domain: string;
  connected: DnsProviderConnection | null;
  providers: DnsProviderInfo[];
}

export interface ConnectDnsProviderRequest {
  provider: string;
  token?: string;
}

/** create/update: Yunori writes it · unchanged: already right · manual: needs a person (Yunori never deletes) · skip: left alone on purpose. */
export type DnsPlanAction = "create" | "update" | "unchanged" | "manual" | "skip";

export interface DnsPlanItem {
  id: string;
  type: DnsRecordType;
  name: string;
  required: boolean;
  action: DnsPlanAction;
  /** What Yunori writes (create/update) or expects. */
  value: string;
  /** What the provider holds now at this name (same kind of record). */
  current: string[];
  note?: string;
}

export interface DnsPlan {
  domain: string;
  provider: string;
  providerLabel: string;
  zoneName: string;
  items: DnsPlanItem[];
  counts: Record<DnsPlanAction, number>;
  /** Send back with publish so it refuses a stale preview. */
  planId: string;
}

export interface PublishDnsRequest {
  /** Item ids to write (default: every create/update). */
  only?: string[];
  planId?: string;
  dryRun?: boolean;
}

export interface DnsPublishResult {
  id: string;
  ok: boolean;
  error?: string;
}

export interface DnsPublishResponse extends DnsPlan {
  dryRun: boolean;
  results: DnsPublishResult[];
  created: number;
  updated: number;
  failed: number;
  /** The public-resolver check run right after publishing. */
  dns?: DomainDnsResponse;
}

// ── Mailboxes ────────────────────────────────────────────────────────────

export interface Mailbox {
  id: string;
  address: string; // user@example.com
  local: string;
  domainId: string;
  displayName: string | null;
  role: "user" | "admin";
  quotaBytes: number | null; // null = unlimited
  usedBytes: number | null;
  aliases: string[]; // full addresses
  createdAt: string | null;
  /** Expiry of an unused invite link, while one is out. */
  inviteExpiresAt?: string | null;
}

/** GET /mailboxes?domainId= → Mailbox[] */
export interface CreateMailboxRequest {
  local: string;
  domainId: string;
  displayName?: string;
  quotaMb?: number | null;
  /** Omit to have the server generate a strong password. */
  password?: string;
  /** Let the person choose their own password through an invite link (no password is returned). */
  invite?: boolean;
}
/** A one-time link to /account/welcome#invite=…, valid for 7 days. */
export interface InviteLink {
  url: string;
  expiresAt: string;
}
/** POST /install/first-mailbox — the onboarding checklist's first mailbox (with postmaster@ and abuse@). */
export interface FirstMailboxRequest {
  local: string;
  displayName?: string;
}
/** POST /mailboxes → CreateMailboxResponse (the password is shown once, never again).
 *  With `invite: true` the response carries `invite` instead of `password`. */
export interface CreateMailboxResponse {
  mailbox: Mailbox;
  password?: string;
  invite?: InviteLink;
}
/** POST /install/first-mailbox → the first mailbox and its generated password (shown once). */
export interface FirstMailboxResponse {
  mailbox: Mailbox;
  password: string;
}
/** POST /mailboxes/:id/invite → InviteLink (replaces an unused one; a password reset withdraws it). */
/** PATCH /mailboxes/:id → Mailbox */
export interface UpdateMailboxRequest {
  displayName?: string | null;
  quotaMb?: number | null;
}
/** POST /mailboxes/:id/password {password?} → {password} ; DELETE /mailboxes/:id → 204 */
export interface ResetPasswordResponse {
  password: string;
}

/** GET /mailboxes/export → text/csv attachment ("mailboxes-YYYYMMDD.csv"):
 *  address, display_name, quota_mb, used_mb, role, aliases (space-separated), created_at. No passwords. */
/** POST /mailboxes/import — CSV columns `address[,display_name[,quota_mb]]`, header row optional
 *  (the export's own file is accepted). At most 1000 rows; 400 field "csv" for an unreadable/empty/too-large file. */
export interface ImportMailboxesRequest {
  csv: string;
  /** Send each person an invite link (default true) instead of generating a password. */
  invite: boolean;
  /** Check every row and report, creating nothing. */
  dryRun: boolean;
}
export interface ImportCreated {
  address: string;
  inviteUrl?: string;
  inviteExpiresAt?: string;
  /** Only when `invite` was false — shown once. */
  password?: string;
}
export interface ImportSkipped {
  /** 1-based data row (after any header); null when the engine refused it during creation. */
  line: number | null;
  address: string;
  reason: string;
}
export interface ImportMailboxesResponse {
  dryRun: boolean;
  /** Addresses that passed the checks (would be / were attempted). */
  ready: string[];
  created: ImportCreated[];
  skipped: ImportSkipped[];
}

// ── Aliases ──────────────────────────────────────────────────────────────

export interface Alias {
  address: string;
  mailboxId: string;
  mailboxAddress: string;
  enabled: boolean;
}
/** GET /aliases → Alias[] ; POST /aliases → Alias (409 "conflict" if it would shadow a mailbox) ; DELETE /aliases?address=… → 204 */
export interface CreateAliasRequest {
  local: string;
  domainId: string;
  mailboxId: string;
}

// ── Sending (GET /sending, PUT /sending) ─────────────────────────────────

export type RelayVerify = "none" | "sesDkim" | "dkimTxt";

/** Bounce (MAIL FROM) domain records a provider asks for. */
export interface MailFromSpec {
  /** Fixed label under the domain (Resend: "send"); null = operator's choice (SES). */
  label: string | null;
  /** MX target; `{region}` is substituted. */
  mxTemplate: string;
  spf: string;
  required: boolean;
}

export interface RelayProvider {
  key: string; // "ses" | "resend" | "postmark" | "mailgun" | "smtp2go" | "brevo" | "custom"
  label: string;
  /** Fixed SMTP host (pre-filled, editable); null = see hostTemplate, or user supplies (custom). */
  host: string | null;
  /** Host derived from the region (`{region}`); the host is then not editable. */
  hostTemplate: string | null;
  port: number;
  regions: string[];
  defaultRegion: string | null;
  /** SMTP username the provider mandates (pre-filled). */
  username: string | null;
  /** What the provider calls the SMTP password ("API key", "SMTP password"…). */
  secretLabel: string;
  spfInclude: string | null;
  verify: RelayVerify;
  dkimSelector: string | null;
  mailFrom: MailFromSpec | null;
  docsUrl: string | null;
  /** Where to add a domain at the provider (`{region}` substituted). */
  domainsUrl: string | null;
}

/** What a provider issued for one domain (public DNS values). */
export interface RelayIdentity {
  dkimTokens?: string[];
  dkimValue?: string;
  mailFrom?: string;
}

export interface RelayRecord {
  id: string;
  type: string;
  name: string;
  value: string;
  required: boolean;
  purpose: string;
}

export interface SendingDomain {
  id: string;
  name: string;
  /** The saved relay provider's records for this domain (checked on the domain's DNS page). */
  records: RelayRecord[];
  /** The provider needs records for this domain that haven't been pasted yet. */
  needsVerification: boolean;
}

export interface SendingResponse {
  mode: "direct" | "relay";
  relay: {
    provider: string;
    host: string;
    port: number;
    username: string;
    region: string | null;
    spfInclude: string | null;
    identities: Record<string, RelayIdentity>;
    hasPassword: boolean;
    domainsUrl: string | null;
  } | null;
  providers: RelayProvider[];
  domains: SendingDomain[];
  /** Outbound port-25 reachability result from the last check. */
  port25: Check;
  ptr: Check;
}

export interface UpdateSendingRequest {
  mode: "direct" | "relay";
  relay?: {
    provider: string;
    /** Ignored for providers with a hostTemplate (derived from the region). */
    host: string;
    port: number;
    /** Empty = the provider's fixed username. */
    username: string;
    /** Omit / empty to keep the stored password. */
    password?: string;
    region?: string;
    spfInclude?: string;
    /** Per-domain provider records as pasted; omit to keep the saved ones. */
    identities?: Record<string, { dkim?: string; mailFrom?: string }>;
  };
}

// ── Deliverability (GET /deliverability) ─────────────────────────────────

export interface QueueSummary {
  total: number;
  deferred: number;
  /** Grouped deferral reasons, classified. */
  groups: Array<{
    reason: "auth" | "tls" | "network" | "rejected" | "other";
    count: number;
    example: string; // sample server response, envelope only — never message content
    remedy: string;
  }>;
}

export interface DeliverabilityResponse {
  checks: Check[]; // PTR/FCrDNS, blocklists, port 25, TLS
  queue: QueueSummary;
  dmarcPolicy: "none" | "quarantine" | "reject" | "unknown";
  checkedAt: string | null;
}

// ── System (GET /system, GET /preflight) ─────────────────────────────────

export interface ServiceStatus {
  name: string; // "stalwart" | "yunori-mail" | "unbound"
  status: "running" | "stopped" | "failed" | "unknown";
  detail: string | null;
}

/** Why the last certificate attempt failed, in words a person can act on. */
export interface CertProblem {
  kind:
    | "dns_missing"
    | "dns_elsewhere"
    | "cloudflare_proxy"
    | "ipv6_elsewhere"
    | "caa"
    | "edge_down"
    | "unreachable"
    | "wrong_response"
    | "dns"
    | "rate_limited"
    | "ca_unreachable"
    | "other";
  name: string | null;
  summary: string;
  remedy: string;
  /** The certificate authority's own words. */
  detail?: string;
  retryAfter?: string;
}

export type CertState = "valid" | "expiring" | "expired" | "self_signed" | "test" | "missing";

/** GET /certificate (also `certificate` in GET /system). Automatic HTTPS status. */
export interface CertificateInfo {
  state: CertState;
  hostnames: string[];
  issuer: string | null;
  notAfter: string | null;
  daysLeft: number | null;
  /** "Let's Encrypt", or the configured CA. */
  authority: string;
  /** When automatic renewal starts (30 days before expiry). */
  renewsAt: string | null;
  lastAttempt: string | null;
  lastSuccess: string | null;
  nextAttempt: string | null;
  failures: number;
  problem: CertProblem | null;
  /** A certificate is being requested right now. */
  inProgress: boolean;
  /** The certificate actually served on 465/993 matches the one on disk. */
  servedMatches: boolean | null;
}

export interface SystemResponse {
  versions: { yunori: string; engine: string | null };
  services: ServiceStatus[];
  resources: {
    memTotalBytes: number;
    memUsedBytes: number;
    diskTotalBytes: number;
    diskFreeBytes: number;
    load1: number;
  };
  certificate: CertificateInfo | null;
  update: UpdateStatus;
}

/**
 * The daily check against yunori.email/server/latest.json (src/updates.rs).
 * POST /updates/check → UpdateStatus asks again now. Nothing is installed by
 * either; `sudo yunori-mail update` does that.
 */
export interface UpdateStatus {
  current: string;
  /** Newest published release, once a check has got an answer. */
  latest: string | null;
  available: boolean;
  publishedAt: string | null;
  /** Release notes on github.com. */
  notesUrl: string | null;
  checkedAt: string | null;
  /** Why the last check got no answer; the last answer is kept. */
  problem: string | null;
}

/** GET /preflight → Check[] (fresh run of the installer's preflight). */
export type PreflightResponse = Check[];

// ── Security (GET /audit, GET /admins) ───────────────────────────────────

export interface AuditEntry {
  id: number;
  at: string;
  actor: string; // admin email or "installer" / "system"
  action: string; // e.g. "mailbox.create"
  target: string | null;
  detail: string | null; // never secrets, never mail content
}

export interface AuditResponse {
  entries: AuditEntry[];
  /** Result of walking the hash chain end to end. */
  chainOk: boolean;
}

// ── Self-service portal (/api/yunori/portal/*, cookie `yunori_portal`) ──

/** POST /portal/login {email, password} → PortalMe */
export interface PortalLoginRequest {
  email: string;
  password: string;
}

export interface AppPassword {
  id: string;
  description: string;
  createdAt: string | null;
}

/** GET /portal/me */
export interface PortalMe {
  address: string;
  displayName: string | null;
  quotaBytes: number | null;
  usedBytes: number | null;
  appPasswords: AppPassword[];
  /** Mail settings to show ("use these in any mail app"). */
  settings: {
    imap: { host: string; port: number; security: "SSL/TLS" };
    smtp: { host: string; port: number; security: "SSL/TLS" };
    /** Calendars and contacts (CalDAV/CardDAV); apps find the paths from the host. */
    dav: { host: string; url: string };
  };
  clientOrigin: string;
}

/** POST /portal/invite {token} → InviteInfo, or 410 { code: "invite_invalid" } */
export interface InviteInfo {
  address: string;
  displayName: string | null;
  expiresAt: string;
}
/** POST /portal/invite/accept {token, password} → PortalMe (and a portal session) */
export interface InviteAcceptRequest {
  token: string;
  password: string;
}

/** POST /portal/devices/apple {device, calendars} → a one-time profile link (10 minutes) */
export interface AppleProfileRequest {
  device: string;
  calendars: boolean;
}
export interface AppleProfileResponse {
  url: string;
  expiresAt: string;
  appPassword: AppPassword;
}

/** GET /portal/rules → MailRules ; PUT /portal/rules MailRules → normalised MailRules.
 *  400 field: vacation.subject | vacation.body | vacation.startDate | vacation.endDate | forwarding.address */
export interface VacationRule {
  enabled: boolean;
  /** Defaults to "Out of office" when on and empty; ≤ 200 characters. */
  subject: string;
  /** Required when on; ≤ 5000 characters. */
  body: string;
  /** YYYY-MM-DD; absent = from now. */
  startDate?: string;
  /** YYYY-MM-DD (≥ startDate); absent = until switched off. */
  endDate?: string;
}
export interface ForwardingRule {
  enabled: boolean;
  address: string;
  keepCopy: boolean;
}
export interface MailRules {
  vacation: VacationRule;
  forwarding: ForwardingRule;
}

/** POST /portal/password {current, next} → 204 */
export interface PortalPasswordRequest {
  current: string;
  next: string;
}
/** POST /portal/app-passwords {description} → {appPassword, secret} (secret shown once) ; DELETE /portal/app-passwords/:id → 204 ; POST /portal/logout → 204 */
export interface CreateAppPasswordResponse {
  appPassword: AppPassword;
  secret: string;
}

// ── Adopted 2026-11-05 (were console proposals) ──
// POST /setup/totp and POST /login → SessionResponse; PUT /sending → SendingResponse;
// GET /admins → AdminUser[]; GET /domains/:id → Domain; Domain.aliasCount;
// throttled login/setup/portal endpoints → 429 { code: "rate_limited" }.
