# Stalwart 1.0.x integration notes (P0 spike)

Findings from reading the Stalwart **v1.0.3** source (`stalwartlabs/stalwart@v1.0.3`, AGPL-3.0) and its
published schema (`resources/schema/schema_oss.json.gz`), then **verified against the real v1.0.3 binary**
(2026-11-05). The live test is `src/engine/live_tests.rs`:

```sh
YUNORI_STALWART_BIN=/path/to/stalwart cargo test live_engine -- --ignored --nocapture --test-threads=1
```

## Verified on a live engine (and fixed in the adapter)

| Finding | Consequence for Yunori |
|---|---|
| Singletons (`x:SystemSettings`, `x:Http`, `x:MtaOutboundStrategy`, `x:AccountPassword`) answer only to `ids: ["singleton"]`. `ids: null` returns an empty list (only `x:Bootstrap` is special). | Address singletons by the constant id `singleton`. |
| `SystemSettings.proxyTrustedNetworks` means **PROXY-protocol** trust. If it includes loopback, every local connection must send a PROXY header (`network.proxy-error`). | Never set it. The edge's client IP travels via `x:Http.clientIpSource = xForwardedFor`. |
| An `X-Forwarded-For` that claims a loopback address is rejected with HTTP 400 (`http.x-forwarded-invalid`, a deliberate check). | The edge forwards only real client IPs, and omits the header otherwise. |
| The zone file writes the RSA DKIM key as a multi-line `( "…" "…" )` TXT record. It also publishes host SPF (`mail.<d> TXT v=spf1 a -all`), `_pop3s` SRV, `ua-auto-config` CNAME + `_ua-auto-config` TXT, and, after a certificate exists, TLSA for 25/443/465/993/995. | The parser joins parenthesised records. Host SPF is an optional row. POP3 SRV is dropped (Yunori opens no POP3). TLSA is hidden until ACME key reuse exists. |
| Auto-generated `x:DkimSignature` objects link to their domain, so destroying the domain fails with `objectIsLinked`. | `delete_domain` destroys the domain's DKIM signatures first (`filter: {domainId}`). |
| TLS certificates are chosen by SNI among **all** stored certificates. If a replaced certificate is still present at reload, it can keep being served. | Destroy the old certificate first, then `ReloadTlsCertificates`. 993/465/587 (STARTTLS) were verified to serve the pushed certificate. |
| Bootstrap's tracer defaults to `/var/log/stalwart/`. On hosts without IPv6, `[::]` listeners fall back to `0.0.0.0` (a warning only). | Bootstrap sets the tracer path explicitly. |
| JSON-pointer patches (`aliases/<k>`, `credentials/<k>/secret`) work. Plaintext secrets are hashed server-side, and password strength is enforced. App-password create returns `{id, secret}`, and app passwords authenticate JMAP Basic auth. | As designed. |

## Naming the server after its first run (verified live, 2026-11-11)

The terminal bootstraps the engine before anyone has chosen a mail domain. The browser names it later, and nothing is
restarted. Live test: `src/engine/live_onboarding_tests.rs`, run with
`cargo test live_engine_late_identity -- --ignored --nocapture`.

| Finding | Consequence for Yunori |
|---|---|
| `x:Bootstrap` requires `defaultHostname` and `defaultDomain`. It refuses `setup.yunori.invalid` (`Invalid default domain (defaultDomain)`), and `x:Domain` refuses `*.example`; `*.test`, `localhost` and `203-0-113-10.sslip.io` are accepted. | The first run uses placeholder names derived from the console's address: its parent domain, or its sslip.io name (`installer::panel::placeholders`). |
| `x:Domain/set update {"name": …}` renames the domain in place. DKIM signatures stay linked by `domainId`, and the zone file re-derives every record (`v1-ed25519-…._domainkey.<new>`, MX, SRV, MTA-STS, TLS-RPT). | `Engine::rename_domain`; `installer::apply_identity` renames the placeholder, but only while no mailbox lives on it. |
| The administrator is `admin@<domain>` and follows the rename. The old name keeps authenticating until the engine restarts (cached), then only `admin@<new>` works. A bare `admin` never does. | The sealed `engine.admin` credential is rewritten as soon as the rename succeeds. |
| `x:SystemSettings/set {"defaultHostname"}` is stored at once, but the SMTP greeting and EHLO change only after `x:Action ReloadSettings`. It survives a restart. With `mailExchangers[].hostname = null` and service hostnames `null`, the zone file's MX and SRV targets follow it. | `Engine::set_hostname` = update + `ReloadSettings`. |
| Listeners and TLS don't depend on the hostname. 465/587/993 keep serving the certificate pushed through `x:Certificate` + `defaultCertificateId`, before and after the change. | The mail certificate is pushed as before, once it covers the new name. |

## Process & files

| Thing | Value |
|---|---|
| Binary | `stalwart` (single static binary; `--config <path>`, `--export <dir>`, `--import <dir>`, `--console`) |
| Config file | `/etc/stalwart/config.json` — **only** the data-store settings, e.g. `{"@type":"RocksDb","path":"/var/lib/stalwart/"}`. Everything else lives in the store. |
| Upstream unit | `Type=notify`, `LimitNOFILE=65536`, `KillSignal=SIGINT`, `User=stalwart`, `AmbientCapabilities=CAP_NET_BIND_SERVICE`, `Conflicts=postfix sendmail exim4` |
| Env | `STALWART_RECOVERY_MODE`, `STALWART_RECOVERY_ADMIN=user:pass`, `STALWART_RECOVERY_MODE_PORT` (default 8080), `STALWART_HOSTNAME`, `STALWART_PUBLIC_URL`, `STALWART_HTTPS_PORT`, `STALWART_UPDATE_ROLE_PERMISSIONS` |

## Modes

- **Bootstrap mode** = `config.json` does not exist. Stalwart opens a plain-HTTP management listener on
  `[::]:$STALWART_RECOVERY_MODE_PORT` (all interfaces → while it runs the installer adds a transient nftables table,
  `inet yunori_bootstrap`, that drops the port except on loopback, and keeps the window short),
  uses an ephemeral store, and only the `x:Bootstrap` object is accessible. If `STALWART_RECOVERY_ADMIN` is
  unset it prints a temporary admin password to stderr — **we always pin it** so nothing secret hits logs.
- **Recovery mode** = `STALWART_RECOVERY_MODE=1` with a real store: management API only, no mail listeners.
- **Normal mode**: listeners from the store. On first normal start, if **no** `NetworkListener` exists Stalwart
  inserts defaults (`smtp :25`, `submissions :465`, `imaps :993`, `pop3s :995`, `sieve :4190`, `https :443`,
  `http :8080`, all on `[::]`). Yunori creates its own listeners in recovery mode first, so the defaults are
  never inserted and `:80/:443` stay free for the Yunori edge.

## Headless install sequence (what `installer::bootstrap_engine` does)

1. Start `stalwart --config /etc/stalwart/config.json` (file absent) with
   `STALWART_RECOVERY_ADMIN=admin:<random>` and `STALWART_RECOVERY_MODE_PORT=18081`.
2. `x:Bootstrap/get` (ids: null) → the singleton id + defaults.
3. `x:Bootstrap/set` update the singleton with
   `defaultHostname`, `defaultDomain`, `dataStore {"@type":"RocksDb","path":…}`,
   `generateDkimKeys: true`, `requestTlsCertificate: false` (Yunori owns ACME), `dnsServer {"@type":"Manual"}`.
   The response's `updated[<id>]` carries `{username: "admin@<domain>", secret: "<16 chars>"}` — the permanent
   admin. Stalwart writes `config.json`, creates the domain (DKIM automatic, DNS manual), `SystemSettings`,
   and schedules the DKIM key-generation task.
4. Stop. Start in **recovery mode** (`STALWART_RECOVERY_MODE=1`, same pinned recovery admin) and create the
   Yunori listener set (below), the `Http` trust settings and `SystemSettings` service hostnames.
5. Stop. Start normally under systemd **without** any recovery variables (a leftover recovery admin is a
   permanent backdoor — `yunori-mail doctor` checks for it).

## Yunori listener set

| name | protocol | bind | useTls | implicitTls |
|---|---|---|---|---|
| `smtp` | smtp | `[::]:25` | true (STARTTLS) | false |
| `submissions` | smtp | `[::]:465` | true | true |
| `submission` | smtp | `[::]:587` | true (STARTTLS) | false |
| `imaps` | imap | `[::]:993` | true | true |
| `sieve` | manageSieve | `[::]:4190` | true (STARTTLS) | false |
| `http-internal` | http | `127.0.0.1:18080` | false | false |

POP3 is off by default. The edge proxies JMAP/DAV/well-known paths to `127.0.0.1:18080`.

## JMAP management API

- `POST /jmap/` with `Authorization: Basic …` (admin) or `Bearer …`;
  `using: ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"]`; methods `x:<Object>/get|set|query`;
  `accountId` may be omitted (defaults to the caller).
- **Wire encodings:** `Set<T>` → `{"value": true, …}`; `List<T>`/`objectList` → `{"0": {…}, "1": {…}}`
  (string integer keys); tagged unions → `{"@type": "Variant", …fields}`; maps → plain JSON objects.
- Errors: method-level `["error", {"type", "description"}, callId]`; per-object `notCreated` / `notUpdated` /
  `notDestroyed` maps with `{type, description, properties?}`.

## Objects Yunori uses

| Object | Use |
|---|---|
| `x:Bootstrap` (singleton) | First-run provisioning (above) |
| `x:Domain` | `name`, `aliases` (set), `enable`, `catchAllAddress`, `dkimManagement`, `dnsManagement`, `certificateManagement`, **`dnsZoneFile` (server-set text — the DNS records Stalwart expects)**, `reportAddressUri` (default `mailto:postmaster`) |
| `x:Account` (`@type: User`) | `name` (local part), `domainId`, `emailAddress` (server-set), `credentials` (objectList of `{"@type":"Password","secret":…}` / `AppPassword` / `ApiKey`), `quotas` (map `StorageQuota`→n, e.g. `maxDiskQuota`), `usedDiskQuota`, `aliases` (objectList of `{name, domainId, enable}`), `roles` (`{"@type":"User"}`/`Admin`), `description` |
| `x:NetworkListener` | Listener set |
| `x:SystemSettings` | `defaultHostname`, `defaultDomainId`, `services` (per-protocol `hostname`/`cleartext`), `proxyTrustedNetworks`, `providerInfo` |
| `x:Http` (singleton) | `clientIpSource`, `forwardedHops` (edge → Stalwart trust) |
| `x:Certificate` | `certificate` / `privateKey` as `{"@type":"Text","value":…}`; `SystemSettings.defaultCertificateId` — how Yunori pushes its ACME certificate in |
| `x:Action` (create-only) | `ReloadSettings`, `ReloadTlsCertificates`, `ReloadBlockedIps`, `TroubleshootDmarc`, `ClassifySpam`, `PauseMtaQueue`, `ResumeMtaQueue` (also `ReloadLookupStores`, `UpdateApps`, `InvalidateCaches`, `InvalidateNegativeCaches`) |
| `x:QueuedMessage` | Queue health and the queue manager (below) |
| `x:BlockedIp` / `x:AllowedIp` | Blocked addresses and the allowlist (below) |
| `x:Security` (singleton) | Automatic IP banning thresholds (below) |
| `x:MtaRoute` (`Relay` variant) | Relay / smarthost mode (P2) |

## DNS management (`x:DnsServer`, `Domain.dnsManagement`) — not used

Stalwart 1.0.3 can publish DNS itself: an `x:DnsServer` object (tagged union, ~70 providers: `Cloudflare`, `Route53`,
`Hetzner`, `Tsig` (RFC 2136), …; `secret` is an `x:SecretKey` such as `{"@type": "Value", "secret": "…"}`) and
`Domain.dnsManagement = {"@type": "Automatic", "dnsServerId", "origin", "publishRecords": {"mx": true, …}}` with
`DnsRecordType` values `dkim tlsa spf mx dmarc srv mtaSts tlsRpt caa autoConfig autoConfigLegacy autoDiscover`.
Verified live (2026-11-10, `src/dns/provider/live.rs`, `cargo test live_dns_management -- --ignored --nocapture`):

| Finding | Consequence for Yunori |
|---|---|
| `x:DnsServer/get` masks the secret (`"secret": "****"`). | Write-only, as Yunori's own vault. |
| Switching a domain to `Automatic` immediately queues an `x:Task` `{"@type": "DnsManagement", "updateRecords": …}` that calls the provider's fixed public endpoint. | It cannot be pointed at a test double; nothing to preview first. |
| It **sets whole RRsets** ("Failed to set DNS RRSet for live.test./MX"), and with only `mx`+`spf` selected it also touched the host-name SPF (`mail.<d>/TXT`). | Replacing an RRset deletes other records of that name and type (e.g. a second MX): against Yunori's "never delete" rule. |
| It publishes Stalwart's own values: DMARC `p=reject`, no relay `include:`, no relay CNAMEs, no host A/AAAA. | Differs from `dns::records::expected_records` (DMARC ladder, relay rows, host rows). |
| Failures surface only as the task's `status: {"@type": "Retry", "failureReason": "<provider JSON>"}`, retried later. | No per-record result a person can act on. |

So Yunori calls the provider's API itself (`src/dns/provider/`, Cloudflare first): preview diff, writes only what is
missing or wrong, never deletes, then the public-resolver check. Domains stay `dnsManagement: Manual`. The cost:
Stalwart's automatic DKIM rotation needs Automatic DNS, so rotation remains a guided/manual flow for now (a later
option is handing Stalwart a provider limited to `publishRecords: {dkim}`, once its RRset behaviour for selector
names is checked).

## Autoconfig

Stalwart's own autoconfig (`legacy_autoconfig.rs`) uses the bare domain as `displayShortName` and offers
IMAP 993 + SMTP 465/587. Yunori's edge therefore answers `/mail/config-v1.1.xml` itself with the
`displayShortName` **Yunori** marker the Yunori client recognises (see `src/edge/autoconfig.rs`).

## Backups

`stalwart --export <dir>` / `--import <dir>` exist in 1.0.3 (whole-store logical export). Yunori's backup job
stops Stalwart, exports, restarts, then encrypts + uploads; consistency without downtime is a P2 follow-up.

Live-tested facts (2026-11-10):
- Both run as the `stalwart` user, which cannot enter `/var/lib/yunori-mail` (root:yunori 2770). Export and import go
  through `/var/lib/yunori-mail-engine-io/<job>` (stalwart, 0700), removed afterwards.
- `--import` refuses a store that already holds data ("Cannot import: the target database already contains data in
  the key ranges being imported"). `yunori-mail restore` moves the live store to `<data>.pre-restore-<stamp>`,
  imports into an empty one, puts the old one back if anything fails, and restarts both services either way.
- `--import` restores every message but not the full-text index: IMAP SEARCH and JMAP filters find nothing until a
  reindex. Reindexing is **not** an `x:Action`; it is a task: `x:Task/set` create
  `{"@type": "StoreMaintenance", "maintenanceType": "reindexAccounts"}` (value camelCase; `"ReindexAccounts"` is
  rejected). Stalwart fans it out into per-account `AccountMaintenance`/`reindex` tasks due a few minutes apart.
- **Mailbox rules (portal)** use standard JMAP Sieve (`urn:ietf:params:jmap:sieve`) *as the user*: upload the
  script to `/jmap/upload/<accountId>/` (`application/sieve`; the session's `uploadUrl` names the public host, so
  only its path is used against loopback), then `SieveScript/set` create `{name, blobId}` with
  `onSuccessActivateScript: "#s"` or update `{blobId}`. Stalwart compiles on set: an invalid script is refused, not
  stored. It refuses to delete the *active* script and `onSuccessDeactivateScript` runs after the destroy, so
  deactivation is a separate `SieveScript/set` first. The script carries its own settings on line 1
  (`# yunori-rules v1 <base64 json>`), so the portal reads back what the engine runs and Yunori stores nothing.
  Verified live: `vacation :days 1` replies once per sender per day with `Auto-Submitted: auto-replied`;
  `redirect :copy` keeps a copy, plain `redirect` does not.
- **Alert email** (`src/alerts/`) is submitted to `127.0.0.1:465` (implicit TLS, `AUTH PLAIN`) as a dedicated
  `yunori-alerts@<primary domain>` mailbox whose random password is sealed in the vault; the engine admin is never
  used for mail. Verified live: the message is queued and delivered (the From must be the authenticated address).
  A wrong password gets `535 5.7.8 Authentication credentials invalid.`; Yunori then gives the sender a new password
  and retries once. The default `MtaInboundThrottle` allows **5 connections per second per IP**, so a sixth
  connection from loopback (Yunori's certificate probes count) is closed before the greeting, as a TLS EOF without
  `close_notify`. The client treats a close before the greeting as throttling: it waits 1.2 s and retries, up to
  three times.
- **Mail import ("Move your mail here")** writes as the user with standard JMAP Mail
  (`urn:ietf:params:jmap:mail`): upload the raw message to `/jmap/upload/<accountId>/` (`message/rfc822`), then
  `Email/import` with `mailboxIds`, `keywords` (`$seen`, `$flagged`, `$answered`, `$draft`, …) and `receivedAt`;
  IMAP then shows the same flags and INTERNALDATE (verified by `src/migrate/live_tests.rs`, which uses the engine
  as both the old server over IMAP and the new mailbox). JMAP was chosen over IMAP APPEND to the local engine: it
  needs no TLS hop to our own listener, and it is the API Yunori already speaks. Facts: the session's
  `maxSizeUpload` is 50 000 000 bytes; a new account's mailboxes are Inbox, Drafts, `Sent Items`,
  `Deleted Items` and `Junk Mail` (roles inbox/drafts/sent/trash/junk; there is **no Archive** until one is
  created); IMAP LIST carries SPECIAL-USE attributes; `Mailbox/set` creates **unsubscribed** mailboxes unless
  `isSubscribed: true` is sent (IMAP apps listing subscriptions would not show them); the engine adds a
  `$hasnoattachment` keyword of its own.
- **Acting for a user as the administrator**: signing in as `<user>%<admin>` with the admin's *password* gives a
  session that is the user's (`username` and `primaryAccounts` are the user's). App passwords cannot be used this
  way ("App passwords cannot be used for impersonation"). `yunori-mail mailbox migrate` uses it, so an
  administrator does not need each person's password.
- An app password can revoke itself (`x:AppPassword/set destroy` authenticated with that same app password). The
  portal's import job runs on a temporary app password made with the user's credentials and destroys it when the
  job ends, so it never holds the user's real password past the portal's 30-minute window.
- JMAP over WebSocket (`/jmap/ws`) is not proxied through the edge (the HTTP proxy cannot carry an upgrade); clients
  use EventSource push, which works.

## Outgoing queue (verified live, 2026-11-10)

`src/engine/stalwart_ops.rs`, live test `live_engine_queue_and_login_abuse` (`src/engine/live_queue_tests.rs`).

- `x:QueuedMessage/query` + `/get`. Properties: `returnPath` (envelope sender, `""` for bounces), `recipients`,
  `nextRetry`, `nextNotify`, `createdAt`, `size`, `priority`, `envId`, `flags` (set: `authenticated`, `dsn`, …),
  `receivedFromIp`, `receivedViaPort`, `blobId`. Yunori never requests `blobId` (envelope and status only).
- `recipients` is a **map keyed by address** of `x:QueuedRecipient`: `retryCount`, `retryDue`, `notifyCount`,
  `notifyDue`, `queueName` (`remote`, …), `flags` (`dsnSent`, `spamPayload`), `orcpt`, `expires` (either
  `{"@type": "Ttl", "expiresAt"}` or `{"@type": "Attempts", "expiresAttempts"}`; default TTL is 3 days) and
  `status`: `{"@type": "Scheduled" | "Completed" | "TemporaryFailure" | "PermanentFailure", errorType,
  errorMessage, errorCommand, responseCode, responseEnhanced, responseMessage, responseHostname}`.
  `errorType` ∈ `dnsError`, `unexpectedResponse`, `connectionError`, `tlsError`, `daneError`, `mtaStsError`,
  `rateLimited`, `concurrencyLimited`, `io`, `encryptionError`.
- What real failures look like: a relay on a closed port → `TemporaryFailure`, `connectionError`,
  `"I/O error: Connection refused (os error 111)"`, `responseHostname` = the relay host. A domain without MX
  (`x@nowhere.invalid`) → `PermanentFailure`, `dnsError`, `"no MX record found."`, a DSN to the sender at once; the
  message stays queued while another recipient is pending. No network → `dnsError` `"…Network is unreachable"`.
- **Retry now** = `x:QueuedMessage/set` update `{"nextRetry": "2000-01-01T00:00:00Z"}` (what Stalwart's own web
  admin sends). The attempt follows within a second or two (`retryCount` +1, new `retryDue`). An unknown id →
  `notUpdated` `notFound`.
- **Cancel** = `x:QueuedMessage/set` destroy. The message is gone; no DSN is sent. Unknown id → `notFound`.
- **Pause/resume** = `x:Action` `PauseMtaQueue` / `ResumeMtaQueue`. While paused, a retry-now only moves
  `nextRetry`; no attempt is made. The state is **in memory only and not readable**: an engine restart resumes
  the queue. Yunori records the pause (`kv queue.paused`) and re-applies it every minute (`jobs.rs`).

## Blocked and allowed IPs, automatic banning (verified live, 2026-11-10)

- `x:Security` (singleton) holds the banning rules, each a rate `{count, period}` (period in ms) plus a ban
  duration (ms, `null` = permanent): `authBanRate`/`authBanPeriod` (failed sign-ins), `abuseBanRate`/`…Period`
  (relaying or failed `RCPT TO`), `loiterBanRate`/`…Period`, `scanBanRate`/`…Period` with `scanBanPaths` (HTTP
  globs such as `*.php*`, `*/wp-*`). **1.0.3 defaults:** auth 100/day, abuse 35/day, loiter 150/day, scan 30/day,
  every ban permanent. Changes apply after `x:Action ReloadSettings`.
- **Yunori sets auth to 20 failures per hour → blocked for 1 day** (`ipaccess::YUNORI_AUTH_BAN`): at install
  (`bootstrap_engine`) and, for existing servers, on the first start of the service after an upgrade (`jobs.rs`).
  It is applied only when the engine still has the stock value (or none), and only once (`kv
  engine.auth_ban_policy = v1`), so a policy chosen by hand is kept. The other three rules keep Stalwart's defaults.
- Failed IMAP `LOGIN`, SMTP `AUTH` (465) and JMAP/HTTP Basic sign-ins all count. After the threshold the address
  gets an `x:BlockedIp` `{address, reason: "authFailure", createdAt, expiresAt}` and new connections are dropped
  during the TLS handshake (HTTP: 403). Over HTTP the address is the one in `X-Forwarded-For` (the edge's real
  client), so a guessing client is banned, not the edge.
- **127.0.0.1 is never banned** (it can fail forever), but the rest of 127/8 is: 127.0.0.2 and 127.0.0.3 were
  banned in the live test. That is how the test produces "non-loopback" sources locally. Yunori's own engine
  calls come from 127.0.0.1 and are therefore safe from bans.
- `reason` ∈ `authFailure`, `rcptToFailure`, `loitering`, `portScanning`, `manual`, `other`.
- **Unblock** = `x:BlockedIp/set` destroy **then `x:Action ReloadBlockedIps`**: without the reload the ban stays in
  force. `ReloadSettings` alone does not lift it.
- **Allowlist** = `x:AllowedIp/set` create `{address, reason?}` (`address` is an IP or CIDR; anything else →
  `invalidPatch` "Failed to parse IpAddrOrMask"). It takes effect after **`ReloadSettings`** (`ReloadBlockedIps`
  alone does not reload it); removal likewise. An allowlisted address can fail sign-ins without ever being banned.
  Yunori refuses networks broader than /8 (IPv4) or /16 (IPv6) and canonicalises (`10.0.0.7/24` → `10.0.0.0/24`).
