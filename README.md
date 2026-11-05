# Yunori Mail Server

Install a real, deliverable mail server on your own VPS in minutes, and run it from a calm web console that looks
and feels like the [Yunori](https://yunori.email) client.

```bash
curl -fsSL https://get.yunori.email/mail | sudo sh
```

> **Status: pre-release.** This directory incubates the product inside the Yunori client repo until its own
> repository (`FutureForge-Studios/yunori-mail`) exists. The plan, research and decisions are in
> [`docs/YUNORI-MAIL-SERVER-PLAN.md`](../docs/YUNORI-MAIL-SERVER-PLAN.md).

## What you get

- **One small Rust binary, `yunori-mail`.** It is the installer, the control plane, the HTTPS edge (ACME
  certificates) and the console, which is embedded in the binary.
- **[Stalwart](https://stalw.art) 1.x as the mail engine.** SMTP, IMAP, JMAP with push, CalDAV/CardDAV,
  DKIM/SPF/DMARC/ARC, MTA-STS, TLS-RPT and spam filtering.
  - It runs **unmodified, as a separate, pinned process**.
  - `yunori-mail` drives it through Stalwart's JMAP management API.
- **Honest deliverability.**
  - Preflight measures outbound port 25, reverse DNS, blocklists and your hosting provider *before* installing
    anything.
  - The DNS and reverse-DNS steps are **verified against public resolvers**, never "I've done it" buttons.
- **Relay mode** for providers that block port 25: Amazon SES, Resend, Postmark, Mailgun, SMTP2GO, Brevo or any
  SMTP relay. DKIM is still signed locally.
  - **Amazon SES:** pick the region and the SMTP host follows. Paste the three Easy DKIM records from SES, and
    optionally set a custom MAIL FROM domain.
  - **Resend:** paste the DKIM value; the `send.` bounce records are filled in for your region.
  - The provider's records join each domain's DNS checklist and are checked like every other record.
- **A self-service portal** at `/account` for every mailbox: change password, app passwords, mail-app settings,
  and "Open in Yunori".
- **Works with the Yunori client out of the box.**
  - The edge serves Thunderbird autoconfig carrying the Yunori marker, so the client recognises the server.
  - Consoles link users straight into the client's connect flow (`/welcome?connect=you@domain`).

Idle footprint target: under 256 MB RAM for the whole box (Stalwart ~100 MB, `yunori-mail` < 30 MB, unbound).

## Requirements

| | |
|---|---|
| OS | Debian 12/13 or Ubuntu 22.04/24.04/26.04, with systemd, on a fresh server |
| CPU | x86-64-v2 or ARM64 (on Proxmox, use CPU type `host`, not `kvm64`) |
| RAM / disk | 1 GB+ / 10 GB+ |
| Network | A static public IPv4, and the A record `mail.<your-domain>` pointed at it **before** installing |
| Sending directly | Outbound port 25 open and reverse DNS (PTR) settable at your VPS provider; otherwise use relay mode |

## Install

```bash
# Interactive
sudo yunori-mail install

# Unattended
sudo yunori-mail install --domain example.com --admin-email you@gmail.com --first-mailbox hello

# Relay mode (the password is read from an environment variable, never from argv)
POSTMARK_TOKEN=… sudo -E yunori-mail install --domain example.com --admin-email you@gmail.com \
  --sending relay --relay-provider postmark --relay-user "$POSTMARK_TOKEN" --relay-password-env POSTMARK_TOKEN

# Amazon SES: the SMTP host comes from the region (SMTP credentials, not AWS access keys)
SES_SMTP_PASSWORD=… sudo -E yunori-mail install --domain example.com --admin-email you@gmail.com \
  --sending relay --relay-provider ses --relay-region eu-west-1 --relay-user AKIA… --relay-password-env SES_SMTP_PASSWORD

# Resend: host and username are fixed; the password is an API key
RESEND_API_KEY=re_… sudo -E yunori-mail install --domain example.com --admin-email you@gmail.com \
  --sending relay --relay-provider resend --relay-password-env RESEND_API_KEY
```

After the install, add each domain's SES or Resend records under **Sending** in the console.

The installer has 11 resumable steps:
1. preflight
2. inputs
3. host record
4. firewall
5. engine (downloaded and checksum-verified)
6. engine bootstrap
7. HTTPS + certificate
8. **DNS gate**
9. **reverse-DNS gate**
10. first mailbox
11. done

After step 7 it prints a one-time **setup token**. Open `https://mail.<domain>/console/setup` to create the console
owner (two-factor authentication is mandatory) and follow the remaining gates in the browser. They advance on
their own once your records propagate. Interrupted? Run `sudo yunori-mail install --resume`.

## Day two

```bash
yunori-mail status                       # install progress
yunori-mail doctor                       # end-to-end health, with fixes
yunori-mail domain add example.org       # then: yunori-mail domain dns example.org
yunori-mail mailbox add ana@example.com --name "Ana" --quota-mb 5120
yunori-mail alias add sales@example.com ana@example.com
yunori-mail certificate status
sudo yunori-mail backup                  # encrypted; prints a recovery key once
sudo yunori-mail upgrade                 # pinned engine upgrade: snapshot → swap → health gate → rollback on failure
sudo yunori-mail restore <file.ybk>
sudo yunori-mail uninstall [--purge]
```

## Layout on the server

| Path | What |
|---|---|
| `/usr/local/bin/{yunori-mail,stalwart}` | Binaries. Stalwart is pinned to an exact version and SHA-256 per `yunori-mail` release |
| `/etc/systemd/system/{yunori-mail,stalwart}.service` | Hardened units (`ProtectSystem=strict`, `NoNewPrivileges`, …) |
| `/etc/stalwart/config.json`, `/var/lib/stalwart` | Engine data-store pointer + RocksDB/blob store |
| `/etc/yunori-mail/master.key` | Vault key: every secret is sealed with it using AES-256-GCM. No plaintext secrets anywhere |
| `/var/lib/yunori-mail/yunori.db` | Install state, console admins/sessions, sealed secrets, check caches, hash-chained audit ledger |

Ports:
- Stalwart owns 25, 465, 587, 993 and 4190.
- `yunori-mail` owns 80 and 443: ACME, console, API, autoconfig, and a proxy for JMAP/DAV/MTA-STS only.
- Stalwart's own web admin is never exposed.

## Development

```bash
npm --prefix console install && npm --prefix console run build   # the SPA gets embedded at compile time
cargo run -- serve --dev --mock-engine                           # http://127.0.0.1:8787/console (prints a setup token)
cargo test                                                       # unit + API integration tests (mock engine)
cargo test -- --ignored                                          # live DNS tests (needs real outbound DNS)
YUNORI_STALWART_BIN=/path/to/stalwart cargo test live_engine -- --ignored --nocapture --test-threads=1
                                                                 # end-to-end against a REAL Stalwart (binds :25/465/587/993/4190)
YUNORI_ENGINE_ADMIN=user:pass cargo run -- serve --dev --engine-url http://127.0.0.1:18180   # console against a local Stalwart
cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

- `YUNORI_ROOT=/some/dir` relocates every path (state, certs, units) for safe local runs.
- `YUNORI_ACME_STAGING=1` uses Let's Encrypt staging.
- `e2e/vps-smoke.sh` exercises the real engine on a throwaway VPS.

Source layout:

| Path | What |
|---|---|
| `src/engine/` | Engine trait, Stalwart JMAP adapter, mock |
| `src/dns/` | Record set, public-resolver checks, PTR/FCrDNS, DNSBLs |
| `src/installer/` | Step machine + OS operations |
| `src/edge/` | TLS, ACME, proxy, autoconfig, embedded console |
| `src/api/` | Console + portal REST API |
| `src/store/` | SQLite, vault, audit |
| `console/` | React SPA in the yunori.email design: tokens synced from the live site (`scripts/sync-yunori-design.mjs`), self-hosted Geist/Fraunces, primitives vendored from the Yunori client (`scripts/sync-design.mjs`) |

## License

AGPL-3.0, the same as the Yunori client. Stalwart is a separate program, licensed AGPL-3.0 by Stalwart Labs. The
installer downloads the official release from Stalwart's GitHub releases, and Yunori never re-hosts it. See plan
§5.3 for the open licensing questions.
