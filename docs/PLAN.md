<!--
  Copied from the Yunori client repository (aka-nahal/Tori, docs/YUNORI-MAIL-SERVER-PLAN.md at fce15a2)
  when Yunori Mail Server moved into its own repository. Paths starting with
  `yunori-mail/` now live at the root of this repository; other paths
  (`src/…`, `server/…`, `shared/…`) refer to the client repository.
-->

# Yunori Mail Server — Research & Product Plan

*Prepared 2026-11-05. Research deliverable for a new, separate product in the Yunori family: an installable, self-hosted mail server modeled on (and improving on) the mail feature of [Openship](https://github.com/oblien/openship). This repository (codename **Tori**) is the Yunori webmail **client**; the mail server is a **sibling product**, not a feature of the client.*

---

## 0. Decisions this document locks in

| # | Decision | Why |
|---|---|---|
| D1 | **Yunori Mail Server is a separate product** with its own repo, installer, release train and admin console. | The client's first principle is *"client, not provider"* (`PRODUCT.md`). Folding a mail server into the client would blur that and couple two very different release/ops cadences. |
| D2 | **The customer is the provider.** The server runs on the customer's own VPS; FutureForge never stores mailboxes in v1. | Same trust story as the client; avoids becoming an email host (abuse desk, IP reputation, lawful-intercept, data-residency obligations). A managed "Yunori Cloud Mail" is an explicit later decision (§11 P4, §13), not a default. |
| D3 | **The Yunori client gets a first-class, but not privileged, connection** to a Yunori Mail Server: auto-discovery + a provider badge + (later) a native JMAP transport. It still connects with per-mailbox credentials like any other account. | Keeps the client's identity model and credential handling unchanged; a Yunori server is "the best-supported mail server", not a back door. |
| D4 | **Rust on the server, Yunori design on the screen.** Everything that runs on the customer's VPS is Rust: one small static binary (`yunori-mail`) that is the installer, the control plane and the web-console server. The console UI is a React SPA in the live yunori.email design (tokens, Geist/Fraunces type, sun mark) built on the Yunori client's primitives (`src/components/ui/*`), compiled once and embedded into the binary (§9.1). We **do not write an MTA from scratch**: we pair with a proven engine (§5). | User direction (2026-11-05): lightweight and efficient, same design as Yunori. A Rust control plane idles at a few MB of RAM, ships as one file, and has no Node/npm runtime on the mail host. The embedded SPA makes the console look exactly like Yunori at zero extra runtime cost. Mail transport is a 30-year-old minefield; we orchestrate it, we don't re-implement it. |

| D5 | **Engine: Stalwart 1.x Community (AGPL-3.0), unmodified, as a separate pinned process** driven only through its JMAP management API. Never embedded as a library, never forked. | The only all-in-one mail server written in Rust (SMTP, IMAP4rev2, **JMAP with push**, POP3, ManageSieve, CalDAV/CardDAV, DKIM/SPF/DMARC/ARC, MTA-STS, TLS-RPT, DANE, ACME, built-in spam filter). Idles at ~100 MB RAM vs 6–8 GB for mailcow. Fully API-driven since 1.0. Has no webmail — exactly the gap the Yunori client fills. §4–§5. |
| D6 | **Native systemd services, no Docker requirement.** Two binaries (`yunori-mail`, `stalwart`) plus the distro's `unbound` resolver. | Lightest possible footprint; avoids Docker's ufw-bypass and bridge-network IP-masking pitfalls (both hit Openship). A Compose variant is a later convenience, not the default. |
| D7 | **`yunori-mail` is the HTTPS edge and the certificate authority on the box.** It owns :80/:443, runs ACME, serves the console, and reverse-proxies JMAP/DAV/autoconfig/MTA-STS to Stalwart on loopback; it pushes the same certificate into Stalwart for SMTP/IMAP. | One cert lifecycle, one public HTTP surface, Stalwart's management listener never exposed. §6.3. |
| D8 | **Every install gate is verified, never self-attested.** DNS records and PTR/FCrDNS are checked against public resolvers before the installer moves on; "continue anyway" exists only for optional records and always says what breaks. | Openship's DNS/PTR gates are button clicks; that is where self-hosted mail silently fails. `PRODUCT.md` principle 1: never hide state. |

*(Sections: 1 framing · 2 client relationship · 3 Openship teardown · 4 engine landscape · 5 engine decision & licensing · 6 architecture · 7 installer · 8 DNS/TLS/deliverability · 9 console & design · 10 client integration · 11 phasing · 12 risks · 13 open questions · 14 execution log · 15 sources.)*

---

## 1. Product framing

### 1.1 The family

```
Yunori (brand, yunori.email)
├── Yunori            — the webmail client (this repo, codename Tori): unified inbox over
│                       any number of external mailboxes. "Client, not provider."
├── Yunori Desktop    — desktop shell of the client (FutureForge-Studios/yunori-desktop)
└── Yunori Mail Server — NEW: install a real mail server on your own VPS in minutes,
                        managed from a calm web console, connected to the client in one click.
```

### 1.2 Who it is for

- **Founders / small teams / agencies** who want `you@yourcompany.com` mailboxes without paying per-seat Workspace/M365 prices, and who already rent a VPS.
- **Privacy-minded individuals and families** who want their mail on hardware they control.
- **Existing Yunori client users** with a custom domain — the server is the natural "own the whole stack" upgrade.

### 1.3 Job to be done

*"Give my domain real, deliverable mailboxes on a server I own — without becoming a mail-server admin."* Success = a user goes from a fresh VPS to a mailbox that sends to Gmail's inbox (not spam) and is readable in the Yunori client in **under 20 minutes**, with every DNS record and health signal explained in plain language.

### 1.4 Principles (inherit the client's, adapted)

1. **Honest about deliverability.** Self-hosted mail lives or dies on port 25, PTR, IP reputation and DNS alignment. Preflight tells the truth *before* installing; health tells the truth *after*. Never market "one click".
2. **Calm is the feature.** A mail server console should read like a status page, not a cockpit. Same torii-red restraint (≈5% per view).
3. **Reversible and recoverable by default.** Every install step is idempotent and resumable; backups are on from day one; repair beats reinstall.
4. **Plain-language DNS.** Every record shows *what*, *why*, *current value vs expected*, and a copy button — or is written automatically through a DNS-provider API.
5. **Least privilege everywhere.** The console never sees mailbox contents; admin actions are audited; secrets are write-only (same rules as the client's Admin Console).

---

## 2. Relationship to the Yunori client (this repo)

### 2.1 What changes in the client — and what must not

**Must not change:** the identity model (Yunori login ≠ mailbox), credential encryption (`server/crypto.ts`, HKDF `"credentials"`), the system-mail invariant (`server/services/system-mail.ts` never sends user mail), the SSRF guard on every user-supplied mail host (`net-guard.ts`), and the single sync writer (`services/sync/index.ts`). A Yunori server is reached through the same `providerFor(account)` factory as Gmail.

**Small, additive changes:**

| Change | Where | Notes |
|---|---|---|
| `yunori` provider preset (label, help URL, no fixed domains) | `shared/providers.ts` | Badge + copy only; hosts come from discovery. |
| Discovery recognizes a Yunori server | `server/services/discovery.ts` | Yunori servers publish standard Thunderbird **autoconfig** + **RFC 6186 SRV** records (they already rank #2 and #4 in our discovery order) plus a `<displayName>`/`<documentation>` marker the client maps to the `yunori` preset. No new discovery source needed. |
| "Connect from your server" deep link | onboarding connect step + `mailbox-setup.tsx` | The server console shows *Open in Yunori* → `https://<client>/welcome?connect=<email>`; the wizard pre-fills the address and runs discovery. Password still typed by the user (or an app password issued by the server console). |
| Provider-level status hints | `accounts-section.tsx` | If the account is a Yunori server, link "Server health" to the server console URL from autoconfig. Never fetch server internals with the user's mailbox credentials. |
| **Later:** JMAP push, then a native JMAP transport | `server/services/providers/jmap-provider.ts` | Stalwart speaks JMAP (D5). Step 1 uses JMAP push only as a "something changed" signal that triggers the existing IMAP sync; step 2 is a full provider. Both slot in behind the existing capability interface — `providerFor` branches, nothing above it changes. §10. |

### 2.2 What the client must never become

- Not the server's admin console (no domain/mailbox management inside the client's `/admin`; that console administers *the Yunori client platform*, not customers' mail servers).
- Not a relay: user mail still goes out through the mailbox's own SMTP submission on the customer's server.

---

## 3. Openship teardown — what they built, what to copy, what to avoid

Source: a read-only review of `oblien/openship` at `main` (2026-11), mail engine + provisioning layer and the full product surface (API, dashboard, CLI, webmail). Openship is an Apache-2.0 self-hosted PaaS; its mail feature ("Openship Mail") is one module of it. Its bundled engine is GPL-3 (iRedMail) and its webmail is a fork of Zero (0.email).

### 3.1 Architecture in one picture

```
Openship control plane (API + dashboard + CLI, Postgres)          Customer VPS
  ├─ mail_servers row (cache only)        ── SSH (ssh2, sudo -n) ──►  docker: openship-mail  (--network host, NET_ADMIN)
  ├─ SSE install wizard (9 steps)                                       Debian 12 + supervisord: postfix, dovecot,
  ├─ admin services → `psql -c` over SSH                                amavis(+SpamAssassin), clamav, iredapd, fail2ban
  └─ webmail = separate app (Zero fork)                              docker: postgres:16-alpine (127.0.0.1:5432)
                                                                     bind mounts: /var/lib/openship/mail/{vmail,spool,dkim,config,pgdata}
                                                                     state: /root/.openship/mail-state.json
```

- **No agent and no API on the mail host.** Every admin action is a shell command or a `psql -c '<sql>'` string sent over SSH. Consequence (their own `TODO.md`): *the mail server cannot be administered without the Openship control plane.*
- **The engine is iRedMail run at image-build time**, slimmed by a script that deletes iRedAdmin/Roundcube/SOGo/nginx/PHP/LDAP/MySQL, then patched at boot by replacing `build-placeholder` strings with real secrets. Two topologies coexist (container and legacy host install) with no migration between them.
- **State is a JSON file on the box** (versioned, mutex-guarded, survives a lost control plane via *Scan → Adopt*). It holds DB secrets and postmaster passwords **in plaintext**; all five DB roles share one password.

### 3.2 The install flow (the part worth studying)

Preflight runs **before** the progress stream opens (SSH works? one install per server via an advisory lock), so failures are a readable error, not a half-open stream. Then nine resumable steps, each with a timeout, state persisted at every boundary:

| # | Step | Notes |
|---|---|---|
| 1 | Ensure components | Docker + their edge proxy (OpenResty + certbot) |
| 2 | Check port 25 | Outbound probe to `portquiz.net:25`; **warning only** → suggests a relay |
| 3 | Ensure reverse proxy | Owns 80/443 for ACME; refuses to take over a foreign proxy |
| 4 | Open firewall | ufw/firewalld rules; refuses non-persistent raw iptables rules and prints them instead |
| 5 | Deploy engine | Generate secrets (reused on retry), start Postgres sidecar, start engine, wait for :25 (180 s) and :993 |
| 6 | DKIM keys | Build record set (A/AAAA, MX, SPF, DKIM, DMARC) → **DNS hold** |
| — | PTR hold | Explains "PTR is set in your VPS panel, not at your DNS provider" → **PTR hold** |
| 7–8 | Request + configure TLS | HTTP-01 via their edge; verify cert/key match; link into Postfix/Dovecot; reload |
| 9 | Verify reachability | Listener table on the box + TCP probes from outside on 25/465/587/993 |

Retry-from-step-N, cancel, reset (state only), forget (row only), and *Scan/Adopt* round it out. **Weak spot:** the DNS and PTR gates are self-attested button clicks, not verified lookups, and there is no RAM / OS / blocklist / PTR preflight at all.

### 3.3 Product surface

- **Admin:** domains (auto DKIM + record set + `postmaster@`), mailboxes (SSHA512 via `doveadm`, quota, soft/hard delete), aliases + catch-all, quotas/stats, a protected system mailbox with "repair and rotate", certificates (disk vs **served** fingerprint, auto-renew).
- **Sending:** direct, or relay via a **data-driven provider registry** (SES, SendGrid, Mailgun, Postmark, Resend, Oracle, custom) with scope *all senders* or *selected domains/addresses*, per-destination TLS policy, DKIM still signed locally so DMARC aligns.
- **DNS:** live scan against **public resolvers** (pass/warn/fail with expected vs actual); auto-apply only via Cloudflare; no MTA-STS, TLS-RPT, autoconfig or SRV.
- **Health:** 9-daemon catalog graded required/advisory/informational; **delivery health from the mail queue** with deferrals classified (auth/tls/network/rejected) and relay-aware remedies; outside-in port probes; a one-line verdict that names the consequence ("delivered WITHOUT virus scanning").
- **Backups:** account tables + DKIM/config + optional maildirs → `tar | zstd`; restore to the same or **another** server (migration).
- **Extras:** CLI with full parity + JSON-lines mode; typed SDK; MCP tool descriptions on routes; "inbound capture" (domain-wide BCC to a hidden collector → Slack/Telegram/Discord/webhook notifications, with loop guard and spam gate); test-email modal with deep links to Gmail/Outlook/Yahoo/iCloud/Proton.
- **Webmail:** Zero fork on Bun/Hono/tRPC over IMAP+SMTP. Every AI feature its marketing advertises is a stub; no password change, Sieve, vacation or send-as for end users; no JMAP.

### 3.4 Copy / avoid

| Copy (Yunori will do this) | Avoid (Yunori will do better) |
|---|---|
| Resumable, step-numbered installer with persisted logs, per-step timeouts and explicit DNS/PTR holds | Self-attested gates → Yunori **verifies** DNS against public resolvers and PTR/FCrDNS before continuing |
| Preflight that fails *before* streaming | No RAM/OS/blocklist/PTR preflight → Yunori checks all of them, with plain-language verdicts |
| Engine image + state on host volumes; upgrade = pull, recreate, roll back | Running a distro installer at build time and `sed`-patching placeholders → Yunori ships a single engine binary with typed config |
| Outside-in reachability probes; served-vs-disk cert fingerprint | Plaintext secrets in a state file; one shared DB password → Yunori encrypts secrets at rest, per-component credentials |
| Queue-based delivery health with classified deferrals | Admin plane only inside the remote control plane → Yunori's admin API lives **on the mail host** |
| Data-driven relay registry with per-sender scope; DKIM signed locally | Relay applied by the browser after install → Yunori applies it server-side inside the install transaction |
| Write-only secrets (`hasPassword`), "leave blank to keep" | Amavis + ClamAV + SpamAssassin + iRedAPD + fail2ban + Postgres (nine daemons; ClamAV alone typically wants 1 GB+ of RAM) → one engine process |
| Scan/Adopt/Forget (disposable control plane) | No MTA-STS, TLS-RPT, autoconfig/SRV → Yunori publishes all of them (the Yunori client's discovery depends on autoconfig + SRV) |
| Protected system mailbox; soft vs hard delete; dependents preview | No end-user self-service → Yunori has password change, app passwords, Sieve/vacation, aliases |
| Health verdict that names the consequence | Marketing features the product stubs → Yunori markets only what ships (`PRODUCT.md` principle 5) |

---

## 4. Engine landscape (researched 2026-11-05)

We orchestrate an engine; we don't write one. Writing our own SMTP + IMAP + JMAP server from Rust crates is a multi-year project and the wrong place to spend a small team.

| Engine | Language / license | Install | Idle RAM | Automation surface | JMAP | Notes (2026) |
|---|---|---|---|---|---|---|
| **Stalwart 1.x** | **Rust** / AGPL-3.0 (+ optional Enterprise licence) | Single binary + systemd, or container | **~86–103 MiB measured**; 1 GB VPS fine for 5–10 users | JMAP management API (`/jmap/`, typed `x:*` objects), scoped API keys, `stalwart-cli apply` idempotent plans, recovery-mode bootstrap | ✅ incl. EventSource / WebSocket / Web Push | 1.0.0 on 2026-10-26, 1.0.3 on 2026-11-04. ~70 DNS-provider APIs, DKIM rotation, DMARC/TLS-RPT report parsing, MTA-STS, DANE, ACME. No webmail. No built-in antivirus (MTA Hooks / milter). |
| mox | Go / **MIT** | Single binary, `quickstart` | ~512 MB OK | Web admin + webapi/webhooks (sending side) | ❌ | Excellent standards coverage (MTA-STS, DANE, TLS-RPT, autoconfig, ACME); one maintainer; no CalDAV/ARC yet. **Our fallback engine.** |
| Maddy | Go / GPL-3.0 | Binary / container | Low | CLI | ❌ | IMAP storage still "beta". |
| docker-mailserver | MIT (bundles GPL parts) | Compose | Not stated | `setup.sh` CLI, no UI | ❌ | Postfix + Dovecot + Rspamd + ClamAV; containers only. |
| mailcow | GPL-3.0 | Compose + `update.sh` | **6 GiB min**, 8 recommended | Full UI + REST API | ❌ | Heavy; SOGo groupware. |
| Mail-in-a-Box | CC0 | One script, dedicated Ubuntu box | ~1 GB | Panel + status checks + REST | ❌ | Runs its own DNS; great status-page UX to borrow. |
| Mailu | MIT | Compose generator | 1 GB (~3 GB with ClamAV) | UI | ❌ | Patch-only release line. |
| iRedMail (Openship's engine) | GPL installer; iRedAdmin-Pro proprietary | Script | Multi-GB with ClamAV | Pro panel ($499/yr) | ❌ | Nine daemons; Openship had to slim and patch it heavily (§3). |
| Poste.io / Postal / chasquid | Proprietary / MIT (outbound only) / Apache-2.0 (SMTP only) | — | — | — | ❌ | Not full mail servers for our purpose. |

**Rust crates we *do* use in our own code** (all Apache-2.0/MIT unless noted): `mail-auth` (SPF/DKIM/DMARC/ARC verify + report parsing), `mail-parser`, `mail-builder`, `mail-send` (test sends), `smtp-proto`, `hickory-resolver` (DNS + DNSSEC/TLSA checks via public resolvers), `rustls`/`tokio-rustls`, `instant-acme` (ACME), `axum` (API), `rust-embed` (console SPA), `sqlx` + SQLite (state), `imap-codec` (IMAP probes), `jmap-client`, optionally `dns-update` (the same 70-provider DNS library Stalwart uses). **Avoid** `sieve-rs` (AGPL), and the unofficial `stalwart` / `stalwart-lite` crates on crates.io.

## 5. Engine decision & licensing

### 5.1 Why Stalwart

1. **Rust, light, one process.** Matches the user's direction (D4) and keeps a 1–2 GB VPS viable. The total budget for the box (§6.5) is under 256 MB idle.
2. **API-first since 1.0.** Every setting is a typed object behind one JMAP management API. Bootstrap is documented for unattended installs: start once with `STALWART_RECOVERY_MODE=1` (management API only, no mail services), apply a plan, restart. A third-party control plane can run it fully headless — which is precisely what Openship lacked (its admin plane was `psql` over SSH).
3. **It already does the hard standards work** — DKIM (RSA + Ed25519, 90-day rotation), DMARC/TLS-RPT/ARF report ingestion, MTA-STS, DANE, ARC, ACME, ~70 DNS-provider APIs, spam classifier, greylisting, DNSBLs, auth-failure IP blocking — so our job is product, not protocol.
4. **JMAP with push** gives the Yunori client a real-time path that IMAP polling never will (§10).
5. **No webmail** — the Yunori client is the webmail. Clean product split.

### 5.2 How we run it

- **Separate process, never embedded.** Running an unmodified AGPL program and talking to it over an API is aggregation, not a derivative work; embedding its crates would make our control plane AGPL *and* leave us maintaining a fork of internals that are not published or stable.
- **Pinned, never `latest`.** Each `yunori-mail` release ships a manifest naming the exact Stalwart version + SHA-256 it was tested against. Upgrades go through our upgrader (§7.6), which also runs the per-release admin steps Stalwart's notes require (e.g. 1.0.2 "re-enable encryption at rest" + new role permissions; 1.0.3 one restart with `STALWART_UPDATE_ROLE_PERMISSIONS=true`). Users never read Stalwart release notes.
- **Behind an `Engine` seam.** `yunori-mail` talks to the engine through one internal trait (domains, accounts, aliases, DKIM, queue, reports, settings, reload, health). Only the Stalwart adapter is built; the seam exists so **mox (MIT)** can become a second adapter if Stalwart's licensing or stability ever blocks us.

### 5.3 Licensing — what is settled and what is not

| Topic | Position |
|---|---|
| Our code (`yunori-mail`, console, probe service) | Ours to license. **Recommendation: AGPL-3.0**, same as the Yunori client — consistent brand promise, and harmless because nothing links Stalwart. (Owner decision — §13.) |
| Stalwart binary | Official builds compile Enterprise-gated code in by default (`AGPL-3.0-only OR LicenseRef-SEL`); the Enterprise licence (SELv2) forbids redistribution. **Default plan: the installer downloads the official artifact from Stalwart's own release channel** and verifies our pinned checksum — we never re-host it. **Before launch: get written confirmation from Stalwart Labs**; fallback is building and shipping an AGPL-only build (`--no-default-features --features rocks`) with source offered. Note: Stalwart ties the database schema version to the edition, so pick the edition once, at install. |
| Enterprise features | Not needed for v1 (single organisation, self-hosted). Multi-tenancy, branding, undelete, LLM spam and alerts are Enterprise (€0.89–2 per mailbox per year). A future hosted offering would need Enterprise or Stalwart's Reseller Agreement. |
| Vendor risk | Single company; since 1.0 the test suites and new features are developed privately (release source stays public). Mitigations: pin + staged rollout, our own end-to-end test suite against each pinned version, the mox seam. |

---

## 6. Architecture

### 6.1 What lands on the customer's VPS

```
Customer VPS (Debian 12/13 or Ubuntu 22.04/24.04/26.04 · x86-64-v2 or aarch64 · ≥1 GB RAM)
│
├─ yunori-mail.service      (Rust, our code, user `yunori`, CAP_NET_BIND_SERVICE)
│    ├─ edge     :80  ACME HTTP-01 + redirect          :443 TLS (rustls), routes:
│    │            /console/*  /account/*  /api/yunori/*      → yunori-mail itself
│    │            /jmap* /dav/* /.well-known/* /mail/config-v1.1.xml
│    │            /autodiscover/* /api/* (Stalwart's own)     → Stalwart HTTP on 127.0.0.1:8080
│    ├─ api      console REST API (axum) + embedded console SPA (rust-embed)
│    ├─ jobs     cert renewal · DNS/PTR re-checks · blocklist monitor · queue health ·
│    │           report digests · backups · update checks
│    └─ store    /var/lib/yunori-mail/yunori.db (SQLite: install state, admins, audit chain,
│                encrypted secrets) · key in /etc/yunori-mail/master.key (0600) or systemd-creds
│
├─ stalwart.service         (official pinned binary, user `stalwart`)
│    ├─ public   :25 SMTP · :465 submissions · :587 submission · :993 IMAPS · :4190 ManageSieve
│    │           (:110/:995 POP3 off by default)
│    ├─ private  127.0.0.1:8080 HTTP (JMAP, DAV, autoconfig, MTA-STS, management API)
│    └─ data     /var/lib/stalwart (RocksDB + filesystem blobs) · /etc/stalwart/config.json
│
├─ unbound.service          (distro package; local validating resolver on 127.0.0.1:53 —
│                            Spamhaus refuses queries arriving via public resolvers)
│
└─ yunori-mail-root helper  (root-only systemd one-shots for the few privileged ops:
                             firewall rules, package installs, binary swaps)
```

### 6.2 Control flow

- **Console → API → engine.** The console never talks to Stalwart. `yunori-mail` holds a Stalwart API key with *Replace* permissions (only what it needs), `allowedIps: [127.0.0.1]`, rotated on a schedule and stored encrypted.
- **No SSH control plane.** Unlike Openship, the mail server is administrable on its own — console, CLI (`yunori-mail domain|mailbox|alias …` with JSON output) and API. A future remote helper (Yunori Desktop installing over SSH, §11 P4) only bootstraps; it never becomes the admin plane.
- **Disposable control plane.** All truth lives in Stalwart's store + `yunori.db`; `yunori-mail adopt` rebuilds `yunori.db` from a running Stalwart (Openship's Scan/Adopt idea, without plaintext secrets).

### 6.3 Edge & certificates (D7)

- `yunori-mail` runs ACME (`instant-acme`, HTTP-01 on :80; DNS-01 when a DNS provider is connected or when installed behind an existing proxy). Certificates: one for the mail host (`mail.<domain>`), plus one per mail domain covering `autoconfig.<d>`, `autodiscover.<d>`, `mta-sts.<d>` once their DNS resolves.
- After issue/renewal it writes the certificate into Stalwart (manual certificate management) and calls `x:Action ReloadTlsCertificates`, then **verifies the served certificate** on 465, 587 (STARTTLS) and 993 matches disk (Openship's best health check).
- Stalwart's HTTP listener trusts `X-Forwarded-For` only from 127.0.0.1 so its rate-limits see real client IPs.
- **"Behind my proxy" mode** (Coolify/Caddy/Traefik users): the edge binds loopback, the user's proxy forwards; ACME switches to DNS-01. Later phase.

### 6.4 Security model

- **Console admins**: argon2id passwords + **mandatory second factor** (TOTP at launch, passkeys in P3); sessions are AES-GCM cookies like the client's `tori_session`; CSRF tokens; login throttling; optional IP allow-list. Admins can be invited with roles (owner / admin / read-only) — a small subset of the client's RBAC model.
- **Mail-content invariant** (same as the client's Admin Console): the console never displays message contents; queue views show envelope + status only.
- **Secrets are write-only and encrypted at rest** (relay credentials, DNS-provider tokens, Stalwart API key, backup keys): API responses carry `hasSecret` only; "leave blank to keep". No plaintext passwords in any state file — the opposite of Openship's `mail-state.json`.
- **Audit ledger**: every admin mutation is logged twice — searchable activity log + append-only hash-chained ledger — porting the client's `server/admin/audit.ts` design.
- **Bootstrap hygiene**: the recovery-mode admin is single-use and its environment variables are removed before the normal start (a leftover recovery variable is a permanent backdoor); `yunori-mail doctor` checks for it forever after.
- **Supply chain**: signed releases (minisign + cosign), `cargo build --locked`, SBOM and provenance attestations; Stalwart artifacts verified against the pinned SHA-256 in our manifest.
- **Host hardening**: systemd sandboxing on both services (`ProtectSystem=strict`, `NoNewPrivileges`, `PrivateTmp`, explicit `ReadWritePaths`), persistent firewall rules, SSH left untouched.

### 6.5 Resource budget (acceptance targets)

| Component | Idle RAM target | Notes |
|---|---|---|
| Stalwart | ~100 MB | Measured 86–103 MiB on 1.0.0 with three accounts |
| yunori-mail | < 30 MB | Async Rust; SPA served from memory-mapped embedded assets |
| unbound | ~10–20 MB | Distro defaults, cache-only |
| **Total** | **< 256 MB** | Leaves a 1 GB VPS comfortable; ClamAV (optional, §7.7) adds ~1 GB and is off by default |

Binary target: `yunori-mail` < 25 MB (console assets pre-compressed with brotli inside the binary).

### 6.6 Repository layout (new repo `FutureForge-Studios/yunori-mail`)

```
yunori-mail/
├── crates/
│   ├── yunori-mail/       # the binary (clap): install · serve · doctor · upgrade · backup · restore · adopt · domain|mailbox|alias …
│   ├── engine/            # Engine trait + Stalwart adapter (typed JMAP management client; types generated from /api/schema)
│   ├── checks/            # preflight, DNS, PTR/FCrDNS, blocklist, port, TLS, auth-results probes (hickory, rustls, mail-auth)
│   ├── edge/              # TLS edge, ACME, reverse proxy, autoconfig responder
│   ├── installer/         # resumable step machine + OS adapters (apt/dnf, ufw/firewalld/nftables, systemd)
│   ├── store/             # SQLite (sqlx) state, secrets vault, hash-chained audit
│   └── api/               # console REST API + OpenAPI document
├── console/               # React 19 + Vite + Tailwind SPA in the Yunori design system (§9), embedded at build
├── probe/                 # probe.yunori.email: what's-my-IP, connect-back port checks, auth-results reflector (§7.3)
├── packaging/             # one-line install.sh, systemd units, cloud-init snippet, marketplace images
├── e2e/                   # full installs on throwaway VPSs per pinned Stalwart version (nightly)
└── docs/
```

---

## 7. Installer

### 7.1 Entry points

| Phase | Entry point | Who it is for |
|---|---|---|
| P1 | **Built.** `curl -fsSL https://github.com/FutureForge-Studios/yunori-mail/releases/latest/download/install.sh \| sudo sh` on the VPS (short alias `get.yunori.email/mail` → that URL) → verifies and installs the static `yunori-mail` binary → runs `yunori-mail install` (guided terminal wizard; every answer also a flag after `sh -s --`). Re-running it resumes a paused install or updates a finished one | Anyone with SSH to a fresh VPS |
| P2 | `.deb` / `.rpm` packages + apt/dnf repo; cloud-init snippet to paste into the provider's "user data" box | Repeatable installs |
| P4 | **Install from Yunori Desktop**: the desktop app connects over SSH and runs the same script, streaming progress in Yunori's UI; marketplace one-click images (Hetzner, Vultr) | Non-terminal users |

The script is short, readable, and pins a version; it never pipes further remote code into a shell.

### 7.2 One honest prerequisite before anything installs

The only thing the user must do first: **point `mail.<domain>` (A, and AAAA if they have IPv6) at the server.** The installer checks it against public resolvers and waits with a live countdown. This makes ACME work on the first try and means the console opens at a real HTTPS URL — no self-signed certificates, no setup page on a bare IP.

### 7.3 Preflight (fails before installing anything; every result in plain language)

| Check | Blocking? | How |
|---|---|---|
| OS + version, systemd present | Block on unsupported | `/etc/os-release` |
| CPU: x86-64-v2 or aarch64 | Block | `/proc/cpuinfo` flags (Stalwart 1.0 crash-loops on Proxmox `kvm64`) |
| RAM ≥ 1 GB, disk ≥ 10 GB free, fd limit | Block below minimum, warn below 2 GB | `/proc/meminfo`, `statvfs` |
| Ports 25/80/443/465/587/993/4190 free | Block, naming the process holding each | `ss -ltnp` |
| Existing MTA (postfix/exim/sendmail) | Block, with "disable it" instructions | package + unit detection |
| Clock synchronised | Warn | `timedatectl` (DKIM signatures and TLS need it) |
| Public IPv4/IPv6 | Info | `probe.yunori.email`, falling back to the local interface |
| **Outbound port 25** | Warn → recommends relay mode | TCP connect to the probe service on :25 and to a major provider's MX (connect only, never send) |
| **Inbound reachability** | Warn | The probe connects *back* to :25/:443 (opt-in; explains it sees only IP + port) |
| **Hosting provider** | Info | IP → ASN → provider; shows *that provider's* port-25 unblock and PTR instructions (§8.4) |
| **PTR / FCrDNS** | Warn now, verified gate later | Reverse lookup; forward-confirm |
| **IP blocklists** | Warn | Spamhaus ZEN via the local resolver (or a DQS key), plus other major lists |
| `mail.<domain>` → this IP | Gate (§7.2) | 1.1.1.1, 8.8.8.8, 9.9.9.9 |

`probe.yunori.email` is a tiny Rust service FutureForge runs: what's-my-IP, connect-back port checks, and an **auth-results reflector** (send a message to `check@probe.yunori.email`, it replies with SPF/DKIM/DMARC/TLS results and a spam score). It stores nothing beyond rate-limit counters, is documented on the privacy page, and every check degrades gracefully when it is unreachable. It replaces Openship's reliance on third-party `portquiz.net` and `ipify`.

### 7.4 Steps (resumable state machine; per-step timeouts; logs persisted; every step idempotent)

| # | Step | Detail | Verify before moving on |
|---|---|---|---|
| 1 | Preflight | §7.3 | Blocking checks pass |
| 2 | Inputs | Primary domain, mail hostname (default `mail.<domain>`), an **external** admin email (ACME + alerts), sending mode (direct / relay — pre-selected from the port-25 and provider results), optional DNS-provider token | Domain syntax; DNS provider token works (read-only test call) |
| 3 | Host record gate | §7.2 | A/AAAA resolve to this box on public resolvers |
| 4 | Install engine | Create users; download pinned Stalwart + verify checksum; write `config.json` (RocksDB + filesystem blobs); install `unbound`; hardened systemd units | Binary checksum; units load |
| 5 | Bootstrap engine | Start in recovery mode bound to 127.0.0.1 → apply plan: listeners, domain, DKIM (Ed25519 + RSA-2048), admin role, scoped API key, resolver → 127.0.0.1, spam-filter defaults, **DMARC `p=none`** (Stalwart defaults to `reject`), relay route if chosen → remove recovery variables → normal start | JMAP session OK; SMTP banner, IMAP CAPABILITY and submission AUTH on loopback |
| 6 | Edge + TLS | Start edge on :80/:443, issue certificate, push into Stalwart, reload | Served certificate on 465/587/993/443 matches disk |
| 7 | Firewall | ufw / firewalld, or a persistent `/etc/nftables.d` file; cloud firewall reminder with provider-specific steps | Local listeners + probe connect-back |
| 8 | **DNS gate** | Full record set (§8.1) from Stalwart's `dnsZoneFile` + our additions. With a provider token → applied automatically (plan → apply, showing a diff); otherwise copy-cards + a zone file | Required records pass on public resolvers (polled); optional ones may be skipped with a stated consequence |
| 9 | **PTR gate** | Exact per-provider instructions ("this is in your VPS panel, not your DNS provider") | PTR → mail host and FCrDNS match. If the provider can't or won't: "continue — receiving works, sending will be relayed" |
| 10 | First mailbox + outside-in test | Create the admin's mailbox; send a message to the reflector; show the auth-results report | SPF/DKIM/DMARC pass, TLS used |
| 11 | Done | Print the console URL + **one-time setup link** (set console password + second factor) + **recovery kit** (encryption key for backups, shown once) + "Open in Yunori" for the new mailbox + warm-up guidance | — |

Resume with `yunori-mail install --resume` (or reopen the console mid-install, which shows the same step list). Retry-from-step, cancel and `yunori-mail uninstall` (stops services, keeps data unless `--purge`) all exist — Openship had no uninstall.

### 7.5 Day-2: domains, mailboxes, aliases

- **Add domain** → Stalwart domain + DKIM → record set → the same verified DNS gate, per domain → per-domain autoconfig/MTA-STS certificates once their records resolve.
- **Mailboxes**: create (generated password shown once, copyable), quota, app passwords, aliases (refuse ones that would shadow a mailbox), catch-all, disable vs delete with a dependents preview, and a protected system mailbox (`postmaster@`, the report mailbox) with "repair".
- **Self-service portal** (`/account`) for every mailbox user: change password, app passwords, two-factor, vacation responder, filters (Sieve generated from a simple rules UI; raw Sieve for power users), and "Open in Yunori" — all things Openship's stack lacked.

### 7.6 Upgrades

- `yunori-mail upgrade` (and a console button): self-update the signed binary → if the release pins a new Stalwart version: **snapshot** → stop → swap binary → run the scripted per-release admin steps → start → **health gate** (JMAP session, SMTP banner, IMAP, submission AUTH, served certificate) → on failure, automatic rollback to the previous binary + snapshot.
- Patch releases can auto-apply in a nightly maintenance window (opt-in); minor releases notify; Stalwart **major** versions (data-format changes, `stalwart-migrate`) are always explicit, guided, and backed up first.
- Our nightly `e2e/` suite installs each pinned version on throwaway VPSs and runs the full send/receive/JMAP/upgrade/rollback path before a release manifest moves.

### 7.7 Backups, restore, migration

- **On by default** at install, locally, and to off-box S3-compatible storage (Backblaze B2, Cloudflare R2, Hetzner Object Storage, AWS S3) when configured. Encrypted with AES-256-GCM under a key derived from the recovery kit, then chunked and gzipped — the same shape as the client's Neon backup design (`server/services/backup/`).
- **Contents**: Stalwart data + blobs, `config.json`, `yunori.db`, DKIM material, certificates.
- **Consistency**: Stalwart 1.0's native export/backup path is **unverified** (P0 spike). Fallback: filesystem snapshot (btrfs/LVM if present) or a brief stop-copy-start in the maintenance window.
- **Restore** to the same box, or **migrate** to a new one: install with `--restore <snapshot>`, then a guided "move MX / A / PTR" checklist that verifies each change.
- **Optional antivirus**: ClamAV via a Stalwart MTA Hook served by `yunori-mail` (`/hooks/av` → `clamd` INSTREAM). Off by default (about 1 GB of RAM); when clamd is down the console says plainly that mail is being delivered unscanned.

---

## 8. DNS, TLS & deliverability

### 8.1 Records Yunori publishes (per domain)

| Record | Name | Value / purpose | Required? |
|---|---|---|---|
| A / AAAA | `mail.<domain>` | Server IPv4 / IPv6 | ✅ (AAAA only if the IPv6 PTR can be set too) |
| MX | `<domain>` | `10 mail.<primary>` | ✅ |
| SPF | `<domain>` TXT | `v=spf1 mx -all` (+ the relay's `include:` in relay mode); stay under 10 lookups | ✅ |
| DKIM | `<selector>._domainkey.<domain>` | Two keys: Ed25519 + RSA-2048 | ✅ |
| DMARC | `_dmarc.<domain>` | Ladder: `p=none` → `quarantine` → `reject`, each step proposed by the console after clean aggregate reports; `rua` to a local report mailbox Stalwart parses | ✅ |
| MTA-STS | `_mta-sts.<domain>` TXT + `https://mta-sts.<domain>/.well-known/mta-sts.txt` | Policy starts in `testing`, moves to `enforce` after clean TLS reports | Recommended |
| TLS-RPT | `_smtp._tls.<domain>` | `rua` to the local report mailbox | Recommended |
| Client discovery | `autoconfig.<d>` / `autodiscover.<d>` CNAME → mail host; SRV `_submissions._tcp` (465), `_imaps._tcp` (993), `_jmap._tcp` | Thunderbird/Outlook/Apple auto-setup — **and the Yunori client's discovery** (`server/services/discovery.ts` ranks autoconfig #2 and SRV #4) | Recommended |
| CAA | `<domain>` | Allow only our ACME CA (Stalwart pins `accounturi`) | Optional |
| TLSA (DANE) | `_25._tcp.mail.<domain>` | `3 1 1` — **only if the zone is DNSSEC-signed**; requires key reuse across ACME renewals | Optional, advanced (P3) |
| PTR | set at the VPS provider | `<ip>` → `mail.<domain>`, forward-confirmed | ✅ for direct sending |
| HELO | — | `mail.<domain>`, matching the PTR | ✅ (configured, not a DNS record) |

**DNS automation**: when the user connects a DNS provider, Yunori hands it to Stalwart's own DNS management, so **automatic DKIM rotation** (90-day, overlapping selectors) works — Stalwart only rotates when it manages DNS. Yunori still verifies everything independently against public resolvers and shows *expected vs actual* per record. Manual-DNS users get a guided rotation: new selector → publish → verify → switch → retire. Provider gotchas to handle in the UI: Hetzner's old DNS API is retired (Cloud API tokens only); Namecheap's API needs account eligibility plus an IP allow-list; long DKIM TXT records must be split into strings correctly.

### 8.2 Sending modes

| Mode | When the installer suggests it | How |
|---|---|---|
| **Direct** | Outbound :25 open, PTR settable, IP not listed | Stalwart delivers directly; warm-up guidance shown for the first weeks |
| **Relay (smarthost)** | Port 25 blocked, no PTR control, listed IP, or the user prefers it | A data-driven provider registry (SES, Postmark, Mailgun, SMTP2GO, Brevo, Resend, custom); scope = all senders, or selected domains/addresses; per-destination TLS required; **DKIM still signed locally** so DMARC aligns; SPF `include:` and provider DNS records added to the record set automatically |

Receiving always stays on the box. The relay is configured **server-side within the install** (Openship applied it from the browser after completion, and lost it if the tab closed).

**Amazon SES and Resend are first-class** (built, `yunori-mail/src/relay.rs`; same facts as Openship's provider table):

| | Amazon SES | Resend |
|---|---|---|
| SMTP host | Derived from the region: `email-smtp.<region>.amazonaws.com` (never typed) | `smtp.resend.com`, username `resend`, an API key as the password |
| Domain verification | The operator pastes the three Easy DKIM CNAMEs from the SES console (rows, CSV or bare tokens); Yunori publishes `<token>._domainkey` → `<token>.dkim.amazonses.com` | The operator pastes the DKIM value for `resend._domainkey` |
| Bounce domain | Optional custom MAIL FROM (e.g. `bounce.<domain>`): MX `feedback-smtp.<region>.amazonses.com` + `v=spf1 include:amazonses.com ~all` | Always `send.<domain>`, same two records for the chosen region (required by Resend) |
| Root SPF | `include:amazonses.com` | Unchanged (bounces use `send.`, so SPF aligns there) |

These rows join each domain's DNS checklist and are verified against public resolvers like every other record (a provider CNAME must be that exact CNAME). Yunori never calls the provider's API, so no AWS or Resend admin key is stored: only the SMTP credential, sealed in the vault. The custom provider takes an optional SPF include. *Not built yet:* per-sender scope (relay only selected domains or addresses).

### 8.3 Ongoing deliverability monitoring (console → Deliverability)

- DNS drift (hourly) and certificate expiry plus served-certificate match (daily).
- Blocklists every 6 h via the local resolver; on a listing: which list, a delisting link, and "switch to relay" as one click.
- **Queue health** every 5 min: deferrals classified as auth / TLS / network / rejected-by-recipient, with relay-aware remedies (Openship's "nine green daemons ≠ mail leaving the box").
- **DMARC and TLS-RPT digests** from Stalwart's parsed report objects: who sends as your domain, pass rates, and a "ready for `quarantine`?" nudge.
- **Test send**: to the reflector, or with deep links to check Gmail, Outlook, Yahoo, iCloud and Proton inboxes.
- Pointers to Google Postmaster Tools and Microsoft SNDS/JMRP for warm-up.
- Diagnostics using Stalwart's `TroubleshootDmarc` and `ClassifySpam` actions.

### 8.4 Hosting-provider reality (researched 2026-11-05 — show these in-product, keep them current)

| Provider | Outbound :25 by default | What the user must do |
|---|---|---|
| Hetzner Cloud | Blocked (25, 465) | After the first paid invoice / 1 month: limit request (case by case); PTR in the console |
| Vultr | Blocked | Support ticket describing use, SPF/DKIM, volumes |
| Linode / Akamai | Blocked (25/465/587) | Set rDNS, then ticket; full account review |
| DigitalOcean | Blocked (25/465/587); officially discourages self-hosted mail | Use **relay mode** |
| AWS EC2 | Blocked | Per-region removal form (≤48 h, can be rejected); PTR via Elastic IP |
| GCP | Blocked to external destinations | Use **relay mode** |
| Azure | Blocked except Enterprise agreements | Use **relay mode** |
| Oracle Cloud | Blocked (tenancies after June 2021) | Service-limit request per region; PTR by ticket |
| Scaleway | Closed (25/465/587) | Identity verification, then open in the security group |
| OVHcloud, Contabo | Reportedly open / unclear — community sources conflict | Preflight measures it rather than trusting a table |

Mailbox-provider rules that matter for a personal or SMB server (rarely "bulk"): Gmail requires SPF *or* DKIM, valid forward + reverse DNS, TLS and a spam rate under 0.3% for all senders. Bulk senders (>5k/day to Gmail; Outlook.com since May 2025) need SPF + DKIM + DMARC with alignment. Yunori's defaults meet the bulk bar anyway.

---

## 9. Console & design ("same designs as Yunori")

### 9.1 Design source of truth

**The live Yunori product, [yunori.email](https://yunori.email), is the source of truth for how the console looks** (owner direction 2026-11-05: "use yunori.mail design"). The console takes from it:

- **Tokens**: the light `:root{…}` and `.dark{…}` blocks: paper `hsl(39 44% 96.5%)` and navy ink `hsl(225 30% 10%)` in light mode, night `#060913` in dark mode, the torii vermilion `hsl(14 84% 44%)`, plus surface/raised/overlay, success/warning/info inks, shadows and motion durations. `console/scripts/sync-yunori-design.mjs` extracts them verbatim from the site's inlined CSS into the generated `src/design/yunori/tokens.css`. `npm run design:yunori:check` reports drift; CI runs it as an advisory step, because the file is a pinned snapshot that is refreshed on purpose, not on every site deploy.
- **Type**: Geist (UI) and Geist Mono, with Fraunces at weight 340 and tight tracking for display headings, matching the site. All three are **self-hosted** woff2 (`@fontsource-variable/*`), bundled into the binary. A mail-server console must not call Google Fonts.
- **Mark**: the sun resting on its horizon (`src/components/yunori-logo.tsx`), drawn in `currentColor`. Only the favicon carries the accent: `#060913` ground with the mark in `#f88f62`.

**Structure still comes from the Yunori client** (this repo): the Tailwind preset and the `ui/*` primitives (button, card, dialog, input, table…) are vendored by `console/scripts/sync-design.mjs` with a drift check, and the admin shell patterns follow `src/features/admin/` (collapsible sidebar ⌘\, ⌘K palette, breadcrumbs, data tables, confirm dialogs). Rules carried over unchanged: the torii accent stays at about 5% of a view, WCAG 2.1 AA, `prefers-reduced-motion`, full keyboard operation. yunori.email's tokens load *after* the vendored `globals.css`, so the site's palette and type win wherever both define a value.

**Later (P2):** extract `@yunori/ui` (tokens + Tailwind preset + primitives + admin shell kit) as a workspace package consumed by yunori.email, the client and the console, so the sync scripts go away.

The console is a static SPA embedded with `rust-embed`, served with the client's strict CSP and security-header posture (`server/security-headers.ts`). It makes no external requests.

### 9.2 Information architecture

| Section | What it shows |
|---|---|
| **Overview** | One-sentence verdict naming the consequence ("Receiving works; sending is relayed because port 25 is blocked"), key signals, setup checklist until complete |
| **Domains** | Per-domain DNS table (record, expected, actual, status, copy), auto-apply, DKIM status/rotation, add domain |
| **Mailboxes** | Users, quotas, aliases, catch-all, app passwords, disable/delete, "Open in Yunori" |
| **Sending** | Direct vs relay, provider registry, scope, test send |
| **Deliverability** | PTR/FCrDNS, blocklists, queue health, DMARC/TLS-RPT digests, DMARC ladder, MTA-STS mode |
| **Security** | Certificates, admin users & 2FA, API keys, audit ledger, sessions |
| **Backups** | Schedule, destinations, runs, restore / migrate, recovery kit |
| **System** | Services (Stalwart, edge, resolver), versions, updates, resources, logs (no mail contents) |

Every page is keyboard-operable, and every status pairs color with text — never color alone (same as the client's accessibility floor).

---

## 10. Yunori client integration roadmap (changes in *this* repo)

All additive; the invariants in §2.1 hold throughout.

| Step | Change | Files |
|---|---|---|
| C1 (with server P1) | `yunori` preset (label, badge, help link); discovery maps a Yunori server's autoconfig marker (`<displayName>`/`<documentation>` served by the Yunori edge) to it | `shared/providers.ts`, `server/services/discovery.ts` |
| C2 (with server P1) | `?connect=<email>` deep link pre-fills the connect wizard; accounts list links "Server health" to the console URL taken from autoconfig | `src/features/onboarding/onboarding-flow.tsx`, `src/features/accounts/mailbox-setup.tsx`, `accounts-section.tsx` |
| C3 (server P3) | **JMAP push as a wake-up signal**: for accounts whose discovery advertises JMAP, hold one EventSource per account server-side; a `StateChange` triggers the existing `syncAccountFolder` immediately. Mail still syncs over IMAP — latency drops from 60 s polling to near-instant with zero change to the sync writer or `messageKey()` identity | `server/jobs/mailbox-watcher.ts`, new `server/services/providers/jmap-push.ts` |
| C4 (later) | Full `JmapProvider` (RFC 8620/8621) behind the capability interface. **Hard part:** the sync engine and `messageKey()` assume IMAP `uidValidity`/`uid`; JMAP ids are opaque strings with state tokens. Needs a design note on identity mapping before any code | `server/services/providers/*`, `server/services/sync/*` |
| C5 (optional) | Calendar/contacts from CalDAV/CardDAV for the Meeting Context and Relationship features | Future |

---

## 11. Phasing

Each phase ships on its own. Sizes are indicative for a two-engineer team.

### P0 — Spikes & decisions (≈2 weeks)

- Headless Stalwart 1.0.x bootstrap end-to-end in Rust (recovery mode → apply → restart), and manual certificate push + `ReloadTlsCertificates`.
- Stalwart 1.0 backup/export path (§7.7).
- Licensing: written confirmation from Stalwart Labs on download-from-upstream (§5.3); our own licence decision.
- Formal trademark search for "Yunori" (USPTO/EUIPO/JPO classes 9/38/42 — no conflict found in a web search, but a web search is not a clearance).
- Minimal `probe.yunori.email`.
- Design-sync script.
- **Exit:** a scripted install on one Hetzner VPS sends to and receives from Gmail.

### P1 — "It installs" MVP (≈8–10 weeks)

- Terminal wizard (§7.4) for Debian/Ubuntu on x86-64.
- Verified DNS + PTR gates with manual DNS.
- Edge + ACME.
- Console v1: Overview, Domains, Mailboxes, Sending (direct), System.
- Self-service portal: password, app passwords, 2FA.
- autoconfig/SRV/MTA-STS/TLS-RPT published.
- CLI parity for domain/mailbox/alias.
- Upgrades with rollback.
- Local backups.
- Client C1 + C2.
- **Acceptance:** fresh VPS → mailbox that receives from Gmail and lands in Gmail's *inbox* (not spam) → connected in the Yunori client via discovery, in **under 20 minutes**. Idle RAM < 256 MB. Kill-and-resume at every step works. Every check's copy reviewed against `PRODUCT.md`.

### P2 — Deliverability & operations (≈6–8 weeks)

- Relay mode + provider registry.
- DNS-provider automation + automatic DKIM rotation.
- Blocklist monitor, queue health, DMARC/TLS-RPT digests + the DMARC ladder.
- Reflector test.
- Alerts (email to the external admin address, webhook/ntfy).
- Encrypted off-box backups + restore/migrate.
- aarch64 builds.
- `.deb`/`.rpm` packages.
- Extract `@yunori/ui`.
- **Acceptance:** a VPS with port 25 blocked reaches a working setup in relay mode with no manual Postfix-style config. A restore onto a second VPS passes the migration checklist.

### P3 — Client-native & depth (≈6 weeks)

- Client C3 (JMAP push wake-up).
- Vacation + filters UI in the portal.
- Optional ClamAV hook.
- Passkeys for console + portal.
- DANE/TLSA for DNSSEC zones.
- Multi-admin roles.
- C4 design note.

### P4 — Reach (as demand shows)

- Install from Yunori Desktop over SSH.
- Marketplace images.
- "Behind my proxy" mode.
- Compose variant.
- RHEL-family support.
- **Decision gate** on a managed **Yunori Cloud Mail** (requires Stalwart Enterprise or its Reseller Agreement, an abuse desk, IP-reputation operations, and a revisit of D2).

```
P0 spikes ─► P1 MVP (+ client C1/C2) ─┬─► P2 deliverability & ops ─► P4 reach
                                      └─► P3 client-native (C3) ───┘
```

---

## 12. Risks

| Risk | Likelihood / impact | Mitigation |
|---|---|---|
| Port 25 blocked on most clouds; new IPs have poor reputation | High / High | Measure in preflight; provider-specific instructions; relay mode as a first-class path; warm-up guidance; blocklist monitor |
| Stalwart churn (1.0.x patch releases already needed admin action) | High / Medium | Pin + checksum; scripted per-release steps; nightly e2e per version; staged rollout; automatic rollback |
| Single-vendor dependency; private upstream tests | Medium / High | `Engine` seam + mox fallback; our own e2e suite; keep to documented APIs only |
| Licensing ambiguity of official binaries (SEL code compiled in) | Medium / High | Download from upstream rather than redistribute; written confirmation; AGPL-only build fallback |
| Data loss on customer boxes | Medium / Very high | Backups on by default; recovery kit; restore tested in e2e; refuse to start Stalwart silently on an empty store when data existed (an upstream pitfall) |
| Console exposed on the public internet | Medium / High | Mandatory 2FA; throttling; optional IP allow-list; signed auto-updates; audit ledger |
| Abuse / brand risk (spam from Yunori-installed servers) | Low–Medium / Medium | Sane outbound rate limits by default; terms of use; we operate no sending infrastructure |
| `probe.yunori.email` becomes a dependency | Low / Low | Every check degrades gracefully; can be disabled; stores nothing |
| Blurring the client's "client, not provider" principle | Low / High | D1–D3; no server admin inside the client; no privileged connection |
| Name conflict ("Yunori") | Unknown | Formal clearance in P0 |

---

## 13. Open questions for the owner

1. **Licence for `yunori-mail`**: AGPL-3.0 (recommended, matches the client) or source-available/proprietary?
2. **Business model**: free and open with paid support/hosting later, or a paid "Pro" tier (e.g. off-box backups, multi-admin, priority updates)?
3. **Blessed providers for the "recommended VPS" list** (Hetzner? OVH? Netcup?) — and do we want referral partnerships?
4. **Console location**: `https://mail.<domain>/console` (proposed) or a separate admin host?
5. **Install from Yunori Desktop** (P4): wanted early as the main non-technical path, or later?
6. **Hosted Yunori Cloud Mail**: rule it out for 2027, or keep the P4 decision gate?

---

## 14. Execution log (2026-11-05): P0 + P1 core built

Built in this branch:
- **The `yunori-mail/` product.** It lives in this repo until its own repo exists (D1). This session could not create
  repositories, and the directory is self-contained so it can be moved out unchanged.
- **Client steps C1 and C2** (§10).

### Shipped

| Area | Where | Notes |
|---|---|---|
| Stalwart integration spike | `yunori-mail/docs/stalwart-integration.md` | Bootstrap mode, recovery mode, the default-listener trap and the JMAP wire format, read from the Stalwart v1.0.3 source and schema |
| Engine seam + Stalwart adapter + mock | `src/engine/` | Domains, accounts, aliases, zone file, queue, relay route, certificate push, self-service (as the user) |
| Verified DNS / PTR / blocklists / provider guidance | `src/dns/` | Public resolvers, FCrDNS, DMARC ladder, relay SPF include |
| Preflight | `src/checks.rs` | OS, x86-64-v2, RAM, disk, port owners, existing MTA, clock, outbound :25, public IP |
| Resumable installer (11 steps) | `src/installer/` | Pinned + SHA-256-verified engine, hardened systemd units, firewall, bootstrap |
| HTTPS edge | `src/edge/` | rustls, ACME HTTP-01, hot reload, served-cert probe on 465/587/993, allowlisted JMAP/DAV proxy, Yunori autoconfig |
| Console + portal API | `src/api/` | Argon2id + mandatory TOTP, revocable DB sessions, CSRF header, throttling, hash-chained audit |
| Encrypted backups, CLI, bootstrap script, CI, VPS smoke test | `src/backup.rs`, `src/cli.rs`, `packaging/install.sh`, `.github/workflows/yunori-mail.yml`, `e2e/vps-smoke.sh` | |
| Console SPA | `yunori-mail/console/` | Embedded at build; tokens, type and mark from yunori.email, primitives vendored from the client, each with a drift check |
| Client C1/C2 | `shared/providers.ts`, `server/services/discovery.ts`, onboarding, login `next`, accounts | `yunori` preset + autoconfig marker, `?connect=` deep link, account-portal link |

**Verification:**
- `cargo clippy -D warnings` is clean, and 64 Rust tests pass, including API integration tests against the mock
  engine.
- The real binary was smoke-tested over HTTP in `--dev --mock-engine` mode: first-run setup with an independently
  computed TOTP, overview, the DNS check, autoconfig and the security headers.
- On the client side, typecheck, lint and 48 vitest tests pass.

### Deviations from the plan, and why

1. **Single crate with modules, not a multi-crate workspace.** It is faster to iterate on. Split it when the repo
   moves.
2. **The engine admin authenticates with HTTP Basic, as the bootstrap admin, over loopback only.** The plan called
   for a scoped API key (§6.2). That is deferred to P2: `x:ApiKey` creation needs live verification first.
3. **The setup token is issued after the HTTPS step, not at the end.** The DNS and PTR gates then continue in the
   browser, and `serve` advances those gates every minute without root. The plan had the console only after the
   install.
4. **Revocable DB-backed sessions** instead of the client's encrypted-cookie pattern.
5. **`probe.yunori.email` is not built yet.**
   - Public IP comes from OpenDNS, falling back to the interface address.
   - Outbound :25 is a connect-only probe to major MX hosts.
   - The inbound connect-back check and the auth-results reflector remain P2.
6. **Backups use `stalwart --export`** (stop → export → start, a few seconds of downtime) plus `yunori.db` and the
   master key, encrypted in 4 MB AES-GCM chunks.

**Console integration:** Playwright drove the **real release binary** (9.4 MB, console embedded; dev mode, mock
engine) under its production CSP:
- first-run setup with TOTP;
- all nine console pages;
- mailbox creation (HTTP 201);
- the `/account` portal, in light and dark themes.

There were no CSP violations, page errors or external requests.

**CI:** `.github/workflows/yunori-mail.yml` is valid YAML, but every run on this repository ends in
`startup_failure` before any job starts, even with only GitHub-owned actions. GitHub Actions appears to be disabled
or restricted for `aka-nahal/Tori` (repository or account setting). The workflow is kept for the dedicated repo;
until then, run the same commands locally.

### Verified against the real Stalwart 1.0.3 binary (follow-up, same day)

**`src/engine/live_tests.rs` passes end to end.** It drives the installer's real code paths (bootstrap mode, then recovery mode, then normal mode), then every engine operation:
- domains, catch-all and enable;
- the live zone file with both DKIM keys;
- accounts, quotas, aliases, admin password reset;
- user login, app passwords (which also authenticate) and self-service password change;
- queue, reload, and relay route on/update/off;
- domain deletion;
- a certificate push and replacement, with the served certificate verified on 993, 465 and 587 (STARTTLS).

**Full-stack browser run.** `yunori-mail serve --dev --engine-url` against that engine, via Playwright:
- setup with TOTP and a real DNS check;
- a mailbox created in the UI, aliases and the shadow guard;
- deliverability with the real queue, and an intact audit chain;
- the portal signed in as the new mailbox, with an app password;
- JMAP through the edge returns 307, then a 200 session;
- autoconfig carries the marker, and Stalwart's `/admin` returns 404.

There were no CSP violations, page errors or external requests.

**Seven bugs only a live engine could reveal, all fixed** (details in `yunori-mail/docs/stalwart-integration.md`):
1. singleton addressing;
2. `proxyTrustedNetworks` is PROXY-protocol trust;
3. loopback `X-Forwarded-For` is rejected with 400;
4. multi-line DKIM records and extra zone records;
5. DKIM objects block domain deletion;
6. the old certificate kept being served after a swap;
7. TLSA records are hidden until ACME key reuse exists, because publishing them under DNSSEC would break mail at the next renewal.

### Still not verified

- **Real-world DNS assertions.** The sandbox intercepts DNS; run `cargo test -- --ignored` on a normal host.
- **ACME issuance, the firewall step, the hardened systemd units and upgrade/rollback on a real VPS.** These need a public IP and DNS. `e2e/vps-smoke.sh` remains the gate before any release.

### Console adopts the yunori.email design (follow-up, same day)

Owner direction: "use yunori.mail design". The console and portal now take their palette, type and mark from the live product (§9.1):
- `console/scripts/sync-yunori-design.mjs` copies yunori.email's light and dark token blocks verbatim into `src/design/yunori/tokens.css`. CI runs `--check` as an advisory step.
- Geist, Geist Mono and Fraunces are self-hosted, replacing Inter and Instrument Serif.
- The sun-on-horizon mark and the site's favicon replace the client's logo, which is no longer vendored.
- Tailwind's emerald/amber status colours map onto the site's success/warning inks.

**Verified on the release binary** (9.4 MB, console embedded, mock engine, production CSP):
- all nine console pages, mailbox creation and the portal, in light and dark;
- all three fonts load from the binary;
- body `#faf7f2` on `#121621` (light), `#060913` (dark); headings in Fraunces 340.

There were no CSP violations, page errors or external requests.

---

## 15. Sources

- Openship: <https://github.com/oblien/openship> (read-only code review of `main`, 2026-11). DeepWiki: <https://deepwiki.com/oblien/openship/10-email-server>.
- Stalwart:
  - releases: <https://github.com/stalwartlabs/stalwart/releases>
  - management: <https://stalw.art/docs/management/>
  - declarative deployments: <https://stalw.art/docs/configuration/declarative-deployments/>
  - requirements: <https://stalw.art/docs/install/requirements>
  - upgrades: <https://stalw.art/docs/install/upgrade/>
  - Enterprise: <https://stalw.art/docs/server/enterprise/>
  - pricing: <https://stalw.art/pricing/>
  - licence: <https://stalw.art/legal/license-world>
  - ACME: <https://stalw.art/docs/server/tls/acme/challenges>
  - DNS providers: <https://stalw.art/docs/server/dns/provider>
  - JMAP push: <https://www.stalw.art/docs/http/jmap/push>
  - filters/hooks: <https://stalw.art/docs/mta/filter/overview>
  - 1.0 footprint write-up: <https://computingforgeeks.com/install-stalwart-mail-server-docker/>
- Other engines:
  - mox: <https://www.xmox.nl/>
  - Maddy: <https://github.com/foxcpp/maddy>
  - docker-mailserver: <https://github.com/docker-mailserver/docker-mailserver>
  - mailcow: <https://docs.mailcow.email/getstarted/prerequisite-system/>
  - Mail-in-a-Box: <https://mailinabox.email/>
  - iRedMail: <https://www.iredmail.org/pricing.html>
  - Poste.io: <https://poste.io/>
- Providers:
  - Hetzner: <https://docs.hetzner.com/cloud/servers/faq/>
  - Vultr: <https://docs.vultr.com/support/products/compute/why-is-smtp-blocked>
  - Linode: <https://www.linode.com/community/questions/19082/>
  - DigitalOcean: <https://docs.digitalocean.com/support/why-is-smtp-blocked/>
  - AWS: <https://repost.aws/knowledge-center/ec2-port-25-throttle>
  - GCP: <https://docs.cloud.google.com/compute/docs/tutorials/sending-mail>
  - Azure: <https://learn.microsoft.com/en-us/azure/virtual-network/troubleshoot-outbound-smtp-connectivity>
  - Oracle: <https://docs.oracle.com/en-us/iaas/Content/Network/Concepts/reverse_dns.htm>
  - Scaleway: <https://scaleway.com/en/docs/instances/how-to/send-emails-from-your-instance>
- Sender rules:
  - Gmail: <https://support.google.com/a/answer/81126>
  - Spamhaus public-resolver behaviour: <https://www.spamhaus.org/news/article/807/using-our-public-mirrors-check-your-return-codes-now>
- Relay pricing (verify before quoting to users):
  - SES: <https://aws.amazon.com/ses/pricing/>
  - Mailgun: <https://www.mailgun.com/pricing/>
  - SMTP2GO: <https://www.smtp2go.com/pricing/>
