# Yunori Mail Server

Install a real, deliverable mail server on your own VPS in minutes, and run it from a calm web console that looks
and feels like the [Yunori](https://yunori.email) client.

```bash
curl -fsSL https://yunori.email/server/install.sh | sudo sh
```

That one line downloads a single static binary for your CPU (x86-64 or ARM64), checks its SHA-256, and starts the
guided installer. Run the same line again at any time: a paused install resumes, a finished one is updated in place.

Not sure your server is suitable? Check it first. This installs only the binary and changes nothing else:

```bash
curl -fsSL https://yunori.email/server/install.sh | sudo sh -s -- preflight
```

> **Status: pre-release.** The plan, research and decisions are in [`docs/PLAN.md`](docs/PLAN.md); the engine
> integration notes are in [`docs/stalwart-integration.md`](docs/stalwart-integration.md). The product was
> incubated inside the [Yunori client repository](https://github.com/aka-nahal/Tori), whose history up to the move
> is preserved here.

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
- **Invite links instead of handed-over passwords.** Create a mailbox (or pick an existing one) and send its owner
  a link: they choose their own password and land on their account page. The token travels in the URL fragment,
  only its hash is stored, it works once and expires after 7 days; a password reset withdraws it.
- **A self-service portal** at `/account` for every mailbox: password, app passwords, all the settings, and
  **Connect your devices**:
  - **Yunori:** a QR code that opens the client on a phone with the address filled in.
  - **iPhone, iPad & Mac:** a configuration profile (mail, and optionally calendars and contacts) built around an
    app password made for that device. It downloads once, within 10 minutes, from a QR code or on the device itself,
    and is never written to disk. iOS Mail ignores autoconfig, so this is how a phone is set up without typing
    server names.
  - **Android & other apps:** autoconfig covers mail; DAVx⁵ covers calendars and contacts.
- **Calendars and contacts** (Stalwart's CalDAV/CardDAV) are reachable through the edge
  (`/.well-known/caldav`, `/.well-known/carddav`) and announced in autoconfig.
- **Works with the Yunori client out of the box.**
  - The edge serves Thunderbird autoconfig carrying the Yunori marker, so the client recognises the server.
  - Consoles and the portal link users straight into the client's connect flow (`/welcome?connect=you@domain`);
    the client opens its mailbox step with the address filled in and the server's settings detected.

Idle footprint target: under 256 MB RAM for the whole box (Stalwart ~100 MB, `yunori-mail` < 30 MB, unbound).

## Requirements

| | |
|---|---|
| OS | Debian 12/13 or Ubuntu 22.04/24.04/26.04, with systemd, on a fresh server |
| CPU | x86-64-v2 or ARM64 (on Proxmox, use CPU type `host`, not `kvm64`) |
| RAM / disk | 1 GB+ / 10 GB+ |
| Network | A static public IPv4, and the A record `mail.<your-domain>` pointed at it **before** installing. IPv6-only servers install too (AAAA instead of A), but servers that speak only IPv4 cannot reach them; preflight says so |
| Sending directly | Outbound port 25 open and reverse DNS (PTR) settable at your VPS provider; otherwise use relay mode |

## Install

```bash
# One line, interactive
curl -fsSL https://yunori.email/server/install.sh | sudo sh

# One line, unattended: every answer as a flag after `-s --` (--yes never asks)
curl -fsSL https://yunori.email/server/install.sh \
  | sudo sh -s -- --domain example.com --admin-email you@gmail.com --first-mailbox hello --yes

# Read it first, or pin a version
curl -fsSLO https://yunori.email/server/install.sh
less install.sh
sudo YUNORI_VERSION=0.1.0 sh install.sh

# The binary is already here: run the installer directly
sudo yunori-mail install

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

What the one-line script does, and nothing else (`packaging/install.sh`):
1. It checks for 64-bit Linux with systemd, and that you are root.
2. It downloads one release asset over HTTPS with curl or wget, then verifies its SHA-256 (and a minisign signature
   once releases are signed).
3. It swaps in `/usr/local/bin/yunori-mail` atomically and hands over to `yunori-mail install` on your terminal
   (or to `yunori-mail preflight` when the first argument is `preflight`).

Nothing runs until the whole script has arrived, so a cut-off download can't run half an installer. Environment
overrides: `YUNORI_VERSION`, `YUNORI_SKIP_INSTALL=1` (download and verify only), `YUNORI_BASE_URL` (an `https://` or
`file://` mirror), `YUNORI_FEED` (the release feed) and `YUNORI_PREFIX`.

**Where the bytes come from.** [yunori.email/server](https://yunori.email/server) serves this repository's releases
(the Yunori website, `server/mail-server-feed.ts` in the client repository):

| Address | What |
|---|---|
| `https://yunori.email/server/install.sh` | The latest release's installer |
| `https://yunori.email/server/v<version>/<file>` | A release file; the installer downloads the binary and its `.sha256` from here |
| `https://yunori.email/server/latest.json` | `{version, tag, publishedAt, notesUrl, install}`; servers check it daily |

Files are redirects to the GitHub release, so the SHA-256 check is unchanged. If yunori.email can't be reached, the
installer downloads from GitHub instead, and the GitHub URL works as a one-liner too:
`curl -fsSL https://github.com/FutureForge-Studios/yunori-mail/releases/latest/download/install.sh | sudo sh`.

Releases are cut by pushing a `v*` tag, by pushing a `release/v<version>` branch, or from **Actions → release → Run
workflow** with the version (`.github/workflows/release.yml`). Each one publishes static musl binaries
for x86-64 and ARM64, `SHA256SUMS`, and an `install.sh` pinned to that version, so the line above always installs the
newest release.

### What the installer does

The terminal wizard checks the server **before asking anything**: OS, CPU, memory, disk, free ports, other mail
software, clock, outbound port 25, public address, blocklists and hosting provider. A server that can't work stops
here, with the fix for each problem. Then it asks a few questions with sensible suggestions:

- the domain;
- the mail hostname (`mail.<domain>`);
- an alerts address on another provider;
- the first mailbox;
- direct or relay sending. Relay is suggested when port 25 is blocked, and the provider, region, username and
  hidden password are asked next.

A wrong answer is asked again with a hint, not a crash. It shows a summary to confirm, then runs 12 resumable steps:

1. Check this server
2. Your domain
3. **Point the mail hostname here** (verified on public DNS; it also catches a Cloudflare proxy, a stale IPv6 record
   or a CAA record that would block the certificate)
4. Open mail ports
5. Install the mail engine (pinned, checksum-verified)
6. Configure the mail engine
7. Start the console and HTTPS edge
8. **Get a trusted TLS certificate** (automatic; see below)
9. **Publish DNS records** (verified on public DNS)
10. **Set reverse DNS** (verified, forward-confirmed)
11. Create your first mailbox
12. Finish

As soon as the console is up (step 7), the wizard prints a **one-time setup link**:
`https://mail.<domain>/console/setup#token=…`. The token is in the URL fragment, so it never reaches a server log.
Open it to create the console owner (two-factor authentication is mandatory) and carry on in the browser.

From there the console shows the same checklist live:

- what to fix for the certificate, with a **Try now** button;
- the DNS records to publish, with copy buttons and a zone file;
- reverse-DNS instructions for your provider;
- a form for the first mailbox. It shows the password once, with "Open in Yunori" and the mail-app settings.

The server re-checks every minute and moves on by itself. The terminal can also wait at each gate with a countdown.
When you stop it or pass `--no-wait`, it prints a summary of what you have and what is left. To pick up from the
terminal again, run `sudo yunori-mail install --resume`. Lost the link? `sudo yunori-mail setup-token` prints a new
one.

### Certificates (automatic HTTPS)

Certificates come from Let's Encrypt over HTTP-01 and need no configuration.

- **Before asking the CA,** Yunori checks the things that usually make it fail and says which one in plain words:
  - the hostname has no A record, or points elsewhere;
  - it is behind Cloudflare's proxy;
  - a stale AAAA record (CAs try IPv6 first);
  - a CAA record that excludes Let's Encrypt;
  - port 80 isn't served by Yunori.
- **When the CA says no,** its answer is translated too. For example, "Let's Encrypt could not connect to
  mail.example.com on port 80. Allow incoming TCP 80 and 443 in your provider's firewall or security group." The CA's
  own words stay one click away.
- **Retries are automatic,** with backoff from 30 seconds up to 12 hours. After a failure, each retry is rehearsed on
  Let's Encrypt's staging CA first, so fixing a firewall never burns the production rate limits. Until the first
  certificate arrives, the console runs on a temporary self-signed one; it switches without a restart.
- **Renewal** starts 30 days before expiry. When `autoconfig.`, `autodiscover.` or `mta-sts.` names of a hosted
  domain start pointing here, the certificate is extended to cover them. A helper name that fails is left out for a
  day rather than blocking the mail hostname.
- **The same certificate is pushed into the mail engine,** and the server verifies what is actually served on
  465/587/993.

`yunori-mail certificate status` explains the current state and the last attempt. `sudo yunori-mail certificate
renew`, or **Try now** in the console, asks immediately. Another ACME CA can be used with `--acme-directory <url>`
(or `YUNORI_ACME_DIRECTORY`), and `YUNORI_ACME_STAGING=1` uses Let's Encrypt staging for test installs. The choice is
saved, so the service uses it too.

## Day two

```bash
yunori-mail preflight [--hostname mail.example.com]   # can this server run mail? (changes nothing)
yunori-mail status                       # install progress
yunori-mail doctor                       # end-to-end health, with fixes
yunori-mail domain add example.org       # then: yunori-mail domain dns example.org
yunori-mail mailbox add ana@example.com --name "Ana" --quota-mb 5120
yunori-mail alias add sales@example.com ana@example.com
yunori-mail certificate status          # automatic HTTPS: state, last attempt, next retry
sudo yunori-mail certificate renew       # ask the CA now
sudo yunori-mail setup-token             # a new one-time console setup link
sudo yunori-mail backup                  # encrypted; prints a recovery key once
yunori-mail update --check               # is there a newer release? (the console's System page says so too)
sudo yunori-mail update                  # take it: verified download, binary swap, then the engine upgrade below
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
cargo run -- serve --dev --mock-engine                           # prints a one-time setup link for http://127.0.0.1:8787/console
npm --prefix console run dev:mock                                # console only, in-memory API; scenarios incl. "Certificate pending"
cargo test                                                       # unit + API integration tests (mock engine)
cargo test -- --ignored                                          # live DNS tests (needs real outbound DNS)
YUNORI_STALWART_BIN=/path/to/stalwart cargo test live_engine -- --ignored --nocapture --test-threads=1
                                                                 # end-to-end against a REAL Stalwart (binds :25/465/587/993/4190)
YUNORI_ENGINE_ADMIN=user:pass cargo run -- serve --dev --engine-url http://127.0.0.1:18180   # console against a local Stalwart
cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

- `YUNORI_ROOT=/some/dir` relocates every path (state, certs, units) for safe local runs.
- `YUNORI_ACME_STAGING=1` uses Let's Encrypt staging; `YUNORI_ACME_DIRECTORY=<url>` uses another ACME CA.
- `YUNORI_LIVE_ACME=1 cargo test live_acme -- --ignored` talks to Let's Encrypt staging (account + a refused order;
  no validation) to check the ACME conversation and how CA errors are explained.
- `e2e/vps-smoke.sh` exercises the real engine on a throwaway VPS.

Source layout:

| Path | What |
|---|---|
| `src/engine/` | Engine trait, Stalwart JMAP adapter, mock |
| `src/dns/` | Record set, public-resolver checks, PTR/FCrDNS, DNSBLs |
| `src/installer/` | Step machine + OS operations |
| `src/edge/` | TLS, automatic HTTPS (`certs.rs` decides when, checks first, retries, explains; `acme.rs` talks to the CA), proxy, autoconfig, embedded console |
| `src/api/` | Console + portal REST API |
| `src/store/` | SQLite, vault, audit |
| `console/` | React SPA in the yunori.email design: tokens synced from the live site (`scripts/sync-yunori-design.mjs`), self-hosted Geist/Fraunces, primitives vendored from the Yunori client (`scripts/sync-design.mjs`) |

## License

AGPL-3.0, the same as the Yunori client. Stalwart is a separate program, licensed AGPL-3.0 by Stalwart Labs. The
installer downloads the official release from Stalwart's GitHub releases, and Yunori never re-hosts it. See plan
§5.3 for the open licensing questions.
