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
| `x:Action` (create-only) | `ReloadSettings`, `ReloadTlsCertificates`, `TroubleshootDmarc`, `ClassifySpam`, `PauseMtaQueue`, `ResumeMtaQueue` |
| `x:QueuedMessage` | Queue health (`recipients` map with per-recipient status) |
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
- JMAP over WebSocket (`/jmap/ws`) is not proxied through the edge (the HTTP proxy cannot carry an upgrade); clients
  use EventSource push, which works.
