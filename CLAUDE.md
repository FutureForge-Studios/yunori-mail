# CLAUDE.md

Guidance for Claude Code (claude.ai/code) in this repository.

## What this is

**Yunori Mail Server**: an installable, self-hosted mail server in the Yunori family (FutureForge Studios). One Rust
binary, `yunori-mail`, is the installer, the control plane, the HTTPS edge and the web console. The mail engine is
[Stalwart](https://stalw.art) 1.0.x, run **unmodified as a separate, pinned process** and driven only through its JMAP
management API. The Yunori webmail **client** lives in a different repository ([aka-nahal/Tori](https://github.com/aka-nahal/Tori)).

- Plan, research and decisions: `docs/PLAN.md` (copied from the client repo when this product moved out).
- Stalwart wire format and what live testing revealed: `docs/stalwart-integration.md`.

## Commands

```bash
npm --prefix console ci && npm --prefix console run build   # console SPA → console/dist (embedded at compile time)
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cargo build --release                                        # target/release/yunori-mail (~9 MB, console embedded)
yunori-mail serve --dev --mock-engine                        # http://127.0.0.1:8787/console, no root, in-memory engine (prints a setup link)
yunori-mail preflight                                        # read-only server check (what the wizard runs first)
yunori-mail update --check                                   # newest release on yunori.email/server/latest.json
YUNORI_LIVE_ACME=1 cargo test live_acme -- --ignored         # ACME conversation against Let's Encrypt staging
YUNORI_STALWART_BIN=/path/to/stalwart cargo test -- --ignored live_   # end-to-end against a real Stalwart
sudo python3 e2e/vps-e2e.py                                  # on an installed throwaway server: every route and command
node e2e/ui-smoke.mjs                                        # then the real console/portal in Chromium (see its header)
```

Console checks: `npm --prefix console run typecheck`, `npm --prefix console run lint`. Design drift:
`node console/scripts/sync-design.mjs --check --from ../../Tori` (client primitives; needs a client checkout) and
`node console/scripts/sync-yunori-design.mjs --check` (yunori.email tokens; network).

## Layout

| Path | What |
|---|---|
| `src/engine/` | `Engine` trait, Stalwart JMAP adapter (`stalwart.rs`), in-memory mock, live tests |
| `src/dns/` | Expected record set (`records.rs`), zone parser, public-resolver checks, PTR/FCrDNS, DNSBLs; `provider/` DNS-provider API (Cloudflare) + publish plan, driven by `src/dns_publish.rs` |
| `src/rules.rs` | Portal auto-reply + forwarding → one user-owned Sieve script (settings embedded, strings escaped) |
| `src/csv.rs` | RFC 4180 CSV for mailbox import/export (formula-safe export) |
| `src/relay.rs` | Relay provider rules (Amazon SES region→host, Easy DKIM, MAIL FROM; Resend DKIM + `send.`) |
| `src/installer/` | 12 resumable steps + OS operations (pinned engine download, systemd, firewall) |
| `src/cli.rs` | Terminal wizard (preflight first, re-asks bad answers, summary card) and day-two commands |
| `src/edge/` | rustls, automatic HTTPS (`certs.rs`: when/why/backoff/staging rehearsal; `acme.rs`: the CA), allowlisted JMAP/DAV proxy, autoconfig (mail + DAV), Apple profiles (`mobileconfig.rs`), embedded console |
| `src/api/` | Console + `/account` portal REST API |
| `src/alerts/` | Proactive alerts: conditions + de-dup state machine (`mod.rs`), email/webhook/ntfy channels, minimal SMTP submission client |
| `src/updates.rs` | Daily check against `yunori.email/server/latest.json`; `yunori-mail update` runs the hosted installer |
| `src/store/` | SQLite, AES-256-GCM vault, hash-chained audit log, mailbox invites |
| `console/` | React 19 + Vite + Tailwind SPA in the yunori.email design |

## Invariants (don't regress)

- **Stalwart is never patched or re-hosted.** Its admin UI is never exposed (`/admin` → 404 through the edge). The
  proxy allowlist judges the path as the engine will see it (percent-decoded, dot segments refused, re-checked after URL
  parsing); `e2e/vps-e2e.py` probes encoded `..` variants.
- **Secrets are write-only.** Relay credentials, engine admin, backup keys: sealed in the vault, API responses carry
  `hasPassword`/`hasSecret` only.
- **DNS and PTR are verified against public resolvers**, never self-attested. New records join
  `dns::records::expected_records` and get an evaluation branch.
- **DNS-provider writes go through `dns_publish`** (preview plan first; `DnsProvider` has no delete, so records are
  only created or rewritten in place) and are followed by the public-resolver check. The provider token is sealed
  (`dns_provider.<domain>.token`) and never returned, logged or audited.
- **Certificates only go through `edge::certs::ensure`** (never `acme::order` directly): it holds the cross-process
  lock, the backoff and the staging rehearsal that keep Let's Encrypt's production rate limits safe. Every failure
  becomes a `CertProblem` with a summary and a remedy a person can act on.
- **The setup token travels in the URL fragment** (`/console/setup#token=…`) and is shown once; never log it. Mailbox
  invites follow the same rule (`/account/welcome#invite=…`, hash stored, single use, 7 days).
- **Device profiles carry an app password**: built in memory, served once (`/account/profile/<token>`, 10 minutes),
  never written to disk or logged.
- **Console admins need TOTP**; sessions are revocable DB rows; every mutation is audit-logged (hash chain).
- **The portal acts as the mailbox user** with the user's own credentials, held in memory for at most 30 minutes.
- **Design:** colours/type/mark come from yunori.email (`console/src/design/yunori/tokens.css`, generated); primitives
  are vendored from the client (`console/src/design/vendor/`, generated). Never hand-edit generated files.
