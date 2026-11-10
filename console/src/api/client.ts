import type {
  AdminUser,
  AlertsResponse,
  Alias,
  AppleProfileRequest,
  AppleProfileResponse,
  ApiError,
  AuditResponse,
  CreateAliasRequest,
  CreateAppPasswordResponse,
  CreateDomainRequest,
  CreateMailboxRequest,
  CreateMailboxResponse,
  DeliverabilityResponse,
  Domain,
  DomainDnsResponse,
  CertificateInfo,
  ConnectDnsProviderRequest,
  DnsPlan,
  DnsProviderStatus,
  DnsPublishResponse,
  PublishDnsRequest,
  FirstMailboxRequest,
  FirstMailboxResponse,
  ImportMailboxesRequest,
  ImportMailboxesResponse,
  InstallState,
  InviteAcceptRequest,
  InviteInfo,
  InviteLink,
  LoginRequest,
  Mailbox,
  MailRules,
  MigrateSource,
  MigrateStatusResponse,
  MigrateTestResponse,
  OverviewResponse,
  PortalLoginRequest,
  PortalMe,
  PortalPasswordRequest,
  PreflightResponse,
  ResetPasswordResponse,
  SendingResponse,
  SessionResponse,
  SetupRequest,
  SetupResponse,
  SetupTotpRequest,
  SystemResponse,
  TestAlertResponse,
  UpdateAlertSettingsRequest,
  UpdateDomainRequest,
  UpdateMailboxRequest,
  UpdateSendingRequest,
  UpdateStatus,
} from "./types";

/**
 * The one HTTP client for /api/yunori. Same posture as the Yunori client's
 * `src/lib/api.ts`: same-origin credentials, JSON in/out, a CSRF header on
 * every non-GET, and a hard redirect to the right sign-in page on 401.
 *
 * Mock mode (`npm run dev:mock`, VITE_MOCK=1) swaps `fetch` for the in-memory
 * mock in ./mock.ts. `__YUNORI_MOCK__` is a build-time literal, so the
 * production bundle contains neither the mock nor the dynamic import.
 */

export const API_BASE = "/api/yunori";

const MOCK = __YUNORI_MOCK__ && import.meta.env.VITE_MOCK === "1";

export class ApiRequestError extends Error {
  readonly status: number;
  readonly code: string;
  readonly field?: string;
  constructor(status: number, error: ApiError) {
    super(error.message);
    this.name = "ApiRequestError";
    this.status = status;
    this.code = error.code;
    this.field = error.field;
  }
}

/** A message that's always safe to show, whatever was thrown. */
export function errorMessage(err: unknown, fallback = "Something went wrong. Try again."): string {
  if (err instanceof ApiRequestError) return err.message;
  return fallback;
}

type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

interface RequestOptions {
  /** Don't redirect on 401 — the caller handles it (sign-in forms, session probes). */
  allow401?: boolean;
  signal?: AbortSignal;
}

let mockFetch: typeof fetch | null = null;
async function transport(input: string, init: RequestInit): Promise<Response> {
  if (MOCK) {
    if (!mockFetch) mockFetch = (await import("./mock")).mockFetch;
    return mockFetch(input, init);
  }
  return fetch(input, init);
}

function redirectToSignIn(path: string) {
  const portal = path.startsWith("/portal/");
  const target = portal ? "/account/login" : "/console/login";
  const here = window.location.pathname;
  if (here === target || here === "/console/setup") return;
  const next = `${here}${window.location.search}`;
  window.location.assign(`${target}?next=${encodeURIComponent(next)}`);
}

export async function request<T>(
  method: Method,
  path: string,
  body?: unknown,
  opts: RequestOptions = {},
): Promise<T> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (method !== "GET") headers["X-Yunori-Request"] = "1";
  if (body !== undefined) headers["Content-Type"] = "application/json";

  let res: Response;
  try {
    res = await transport(`${API_BASE}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: "same-origin",
      signal: opts.signal,
    });
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") throw err;
    throw new ApiRequestError(0, {
      code: "network",
      message: "Couldn't reach the server. Check your connection and try again.",
    });
  }

  if (res.status === 204) return undefined as T;

  const text = await res.text();
  let json: unknown = null;
  if (text) {
    try {
      json = JSON.parse(text);
    } catch {
      json = null;
    }
  }

  if (!res.ok) throw failure(res.status, json, path, opts);
  return json as T;
}

/** The error for a non-2xx response (redirecting to sign-in on 401 unless allowed). */
function failure(status: number, json: unknown, path: string, opts: RequestOptions): ApiRequestError {
  const apiError =
    json && typeof json === "object" && "error" in json && isApiError((json as { error: unknown }).error)
      ? (json as { error: ApiError }).error
      : {
          code: status === 401 ? "unauthenticated" : "http_error",
          message: status >= 500 ? "The server ran into a problem. Try again in a moment." : `The request failed (${status}).`,
        };
  if (status === 401 && !opts.allow401) redirectToSignIn(path);
  return new ApiRequestError(status, apiError);
}

/**
 * A GET that answers with a file (CSV export): the session cookie is enough,
 * no CSRF header. Returns the body and the server's suggested file name.
 */
export async function download(path: string, fallbackName: string): Promise<{ blob: Blob; filename: string }> {
  let res: Response;
  try {
    res = await transport(`${API_BASE}${path}`, { method: "GET", credentials: "same-origin" });
  } catch {
    throw new ApiRequestError(0, { code: "network", message: "Couldn't reach the server. Check your connection and try again." });
  }
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    let json: unknown;
    try {
      json = JSON.parse(text);
    } catch {
      json = null;
    }
    throw failure(res.status, json, path, {});
  }
  const name = /filename\*?=(?:UTF-8'')?"?([^";]+)"?/i.exec(res.headers.get("Content-Disposition") ?? "")?.[1];
  return { blob: await res.blob(), filename: name ? decodeURIComponent(name) : fallbackName };
}

function isApiError(v: unknown): v is ApiError {
  return !!v && typeof v === "object" && typeof (v as ApiError).code === "string" && typeof (v as ApiError).message === "string";
}

const q = (params: Record<string, string | undefined>) => {
  const s = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v) s.set(k, v);
  const str = s.toString();
  return str ? `?${str}` : "";
};
const id = (v: string) => encodeURIComponent(v);

// ── Console (admin) ─────────────────────────────────────────────────────────

export const api = {
  session: () => request<SessionResponse>("GET", "/session", undefined, { allow401: true }),
  setup: (body: SetupRequest) => request<SetupResponse>("POST", "/setup", body, { allow401: true }),
  setupTotp: (body: SetupTotpRequest) => request<unknown>("POST", "/setup/totp", body, { allow401: true }),
  login: (body: LoginRequest) => request<SessionResponse>("POST", "/login", body, { allow401: true }),
  logout: () => request<void>("POST", "/logout", undefined, { allow401: true }),

  overview: () => request<OverviewResponse>("GET", "/overview"),
  install: () => request<InstallState>("GET", "/install"),
  createFirstMailbox: (body: FirstMailboxRequest) => request<FirstMailboxResponse>("POST", "/install/first-mailbox", body),
  certificate: () => request<CertificateInfo>("GET", "/certificate"),
  /** "Try now": starts an attempt in the background; poll certificate() while inProgress. */
  renewCertificate: () => request<CertificateInfo>("POST", "/certificate/renew"),

  domains: () => request<Domain[]>("GET", "/domains"),
  createDomain: (body: CreateDomainRequest) => request<Domain>("POST", "/domains", body),
  updateDomain: (domainId: string, body: UpdateDomainRequest) =>
    request<Domain>("PATCH", `/domains/${id(domainId)}`, body),
  deleteDomain: (domainId: string) => request<void>("DELETE", `/domains/${id(domainId)}`),
  dns: (domainId: string) => request<DomainDnsResponse>("GET", `/domains/${id(domainId)}/dns`),
  checkDns: (domainId: string) => request<DomainDnsResponse>("POST", `/domains/${id(domainId)}/dns/check`),
  dnsProvider: (domainId: string) => request<DnsProviderStatus>("GET", `/domains/${id(domainId)}/dns/provider`),
  connectDnsProvider: (domainId: string, body: ConnectDnsProviderRequest) =>
    request<DnsProviderStatus>("PUT", `/domains/${id(domainId)}/dns/provider`, body),
  disconnectDnsProvider: (domainId: string) => request<DnsProviderStatus>("DELETE", `/domains/${id(domainId)}/dns/provider`),
  dnsPlan: (domainId: string) => request<DnsPlan>("GET", `/domains/${id(domainId)}/dns/plan`),
  publishDns: (domainId: string, body: PublishDnsRequest) =>
    request<DnsPublishResponse>("POST", `/domains/${id(domainId)}/dns/publish`, body),

  mailboxes: (domainId?: string) => request<Mailbox[]>("GET", `/mailboxes${q({ domainId })}`),
  createMailbox: (body: CreateMailboxRequest) => request<CreateMailboxResponse>("POST", "/mailboxes", body),
  updateMailbox: (mailboxId: string, body: UpdateMailboxRequest) =>
    request<Mailbox>("PATCH", `/mailboxes/${id(mailboxId)}`, body),
  resetMailboxPassword: (mailboxId: string, password?: string) =>
    request<ResetPasswordResponse>("POST", `/mailboxes/${id(mailboxId)}/password`, password ? { password } : {}),
  inviteMailbox: (mailboxId: string) => request<InviteLink>("POST", `/mailboxes/${id(mailboxId)}/invite`, {}),
  deleteMailbox: (mailboxId: string) => request<void>("DELETE", `/mailboxes/${id(mailboxId)}`),
  /** CSV of every mailbox (no passwords), as a file to save. */
  exportMailboxes: () => download("/mailboxes/export", "mailboxes.csv"),
  importMailboxes: (body: ImportMailboxesRequest) => request<ImportMailboxesResponse>("POST", "/mailboxes/import", body),

  aliases: () => request<Alias[]>("GET", "/aliases"),
  createAlias: (body: CreateAliasRequest) => request<Alias>("POST", "/aliases", body),
  deleteAlias: (address: string) => request<void>("DELETE", `/aliases${q({ address })}`),

  sending: () => request<SendingResponse>("GET", "/sending"),
  updateSending: (body: UpdateSendingRequest) => request<unknown>("PUT", "/sending", body),

  deliverability: () => request<DeliverabilityResponse>("GET", "/deliverability"),
  system: () => request<SystemResponse>("GET", "/system"),
  checkForUpdate: () => request<UpdateStatus>("POST", "/updates/check"),
  preflight: () => request<PreflightResponse>("GET", "/preflight"),
  audit: () => request<AuditResponse>("GET", "/audit"),
  admins: () => request<AdminUser[]>("GET", "/admins"),

  alerts: () => request<AlertsResponse>("GET", "/alerts"),
  updateAlertSettings: (body: UpdateAlertSettingsRequest) => request<AlertsResponse>("PUT", "/alerts/settings", body),
  sendTestAlert: () => request<TestAlertResponse>("POST", "/alerts/test"),
  /** Evaluate now instead of waiting for the next 10-minute check. */
  checkAlerts: () => request<AlertsResponse>("POST", "/alerts/check"),
};

// ── Self-service portal ─────────────────────────────────────────────────────

export const portalApi = {
  login: (body: PortalLoginRequest) => request<PortalMe>("POST", "/portal/login", body, { allow401: true }),
  me: () => request<PortalMe>("GET", "/portal/me"),
  /** Session probe for the sign-in page: 401 means "not signed in", not an error. */
  probe: () => request<PortalMe>("GET", "/portal/me", undefined, { allow401: true }),
  changePassword: (body: PortalPasswordRequest) => request<void>("POST", "/portal/password", body),
  createAppPassword: (description: string) =>
    request<CreateAppPasswordResponse>("POST", "/portal/app-passwords", { description }),
  revokeAppPassword: (appPasswordId: string) =>
    request<void>("DELETE", `/portal/app-passwords/${id(appPasswordId)}`),
  logout: () => request<void>("POST", "/portal/logout", undefined, { allow401: true }),
  inviteInfo: (token: string) => request<InviteInfo>("POST", "/portal/invite", { token }, { allow401: true }),
  acceptInvite: (body: InviteAcceptRequest) =>
    request<PortalMe>("POST", "/portal/invite/accept", body, { allow401: true }),
  appleProfile: (body: AppleProfileRequest) => request<AppleProfileResponse>("POST", "/portal/devices/apple", body),
  rules: () => request<MailRules>("GET", "/portal/rules"),
  saveRules: (body: MailRules) => request<MailRules>("PUT", "/portal/rules", body),
  migrateStatus: () => request<MigrateStatusResponse>("GET", "/portal/migrate"),
  migrateTest: (body: MigrateSource) => request<MigrateTestResponse>("POST", "/portal/migrate/test", body),
  migrateStart: (body: MigrateSource) => request<MigrateStatusResponse>("POST", "/portal/migrate", body),
  migrateCancel: () => request<void>("POST", "/portal/migrate/cancel"),
};
