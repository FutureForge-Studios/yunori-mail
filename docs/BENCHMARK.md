# Yunori Mail Server vs Openship Mail vs BillionMail — hands-on, 2026-11-10

Three fresh, identical VMs (InstaBox, Debian 13, Firecracker, 4 vCPU / 8 GB, public IPv6, NATed IPv4 egress, no
inbound from the internet, outbound TCP 25 blocked). Each product was installed the way its README tells a user to,
then exercised locally. Nothing below is from documentation alone; every number was measured.

| | **Yunori Mail Server 0.2.0** | **Openship Mail 0.9.11** | **BillionMail main (4.9.3)** |
|---|---|---|---|
| Install | one-line script → 1 static binary (9.3 MB) + pinned, checksummed Stalwart 1.0.3 | CLI + `openship up` + 9-step wizard over root SSH | `git clone main` + `install.sh` |
| Downloaded | ≈ 57 MB (binary + engine) | 3.57 GB control plane + 1.74 GB mail image | 511 MB compressed, 1.86 GB on disk |
| Disk after install | ≈ 0.2 GB | ≈ 5.4 GB | ≈ 2.2 GB |
| Idle RAM (RSS) | 150–185 MB: Stalwart 139–174 MB + yunori-mail 10–12 MB | +2.2 GB (ClamAV alone 965 MB) | +430 MB (`free`), 290 MiB in containers |
| Processes / containers | 2 services (+ unbound) | 7 containers, 2 Postgres | 7 containers, 76 PIDs, 20 `docker-proxy` |
| Preflight before installing | yes: OS, systemd, CPU, RAM, disk, ports, other MTAs, clock, outbound 25, IPv4/IPv6, hostname → here | port-25 warning only | none (hostname silently `example.com`) |
| DNS / PTR gates | verified against public resolvers, IPv6-aware | self-attested button (`dns-ack` accepted a `.test` domain with no records) | real lookups, system resolver, nothing gates; DMARC check passes any record |
| Let's Encrypt | staging rehearsal, backoff, cross-process lock, plain-language failures | production directly | production directly, no DNS check, gives up after one failure |
| TLS before ACME | per-install self-signed | one private key shared by every install (baked in the image) | per-install self-signed; Docker method ships a key in the public repo |
| Console auth | owner + TOTP (replay-protected), revocable sessions, CSRF header, hash-chained audit | password | password + captcha, no 2FA, console also on plain HTTP |
| Secrets at rest | AES-256-GCM vault, API returns `hasPassword` only | plaintext state file (stale after a retry) | world-readable `.env`; mailbox list API returns every mailbox's plaintext password |
| Host privilege of the control plane | `yunori` user, hardened systemd unit, `CAP_NET_BIND_SERVICE` only | root over SSH from a container holding the Docker socket | root container with `docker.sock` |
| Plaintext IMAP (143) / POP3 | not listening | listening (LOGINDISABLED) | listening, plaintext LOGIN accepted |
| IPv6 | first-class (works IPv6-only) | tells you to publish AAAA, daemons are IPv4-only | IPv4-only guidance, wrong behind NAT |
| Submission → IMAP, DKIM | ✓ (587, 465, aliases, JMAP) | ✓ | ✓ (two signatures) |
| No open relay / no spoofing | ✓ / ✓ | ✓ / – | ✓ / ✓ |
| Backup | encrypted (`yunori-mail backup`), restore round-trip verified | optional, excludes mail and DKIM by default, unencrypted 0666 file | none |
| Upgrade | pinned engine, snapshot, health gate, rollback | rollback-guarded image swap | `git merge -Xtheirs` from `main`, conflicting files deleted |
| Uninstall | `yunori-mail uninstall` (keeps data unless `--purge`) | leaves the mail server running with no control plane | none |
| Calendars/contacts, autoconfig, MTA-STS, phone profiles | ✓ | – | – |

## What the others do that Yunori doesn't (yet)

- **BillionMail:** an email-marketing suite (campaigns, lists, templates, warm-up, open/click analytics), bundled
  Roundcube webmail, multi-IP sending. Webmail is deliberately the Yunori client's job; marketing is out of scope.
  *CSV mailbox import/export: added in 0.2.0 (with invite links instead of BillionMail's plaintext passwords).*
- **Openship:** `--schema`/JSON-stdin on every CLI write; preconfigured fail2ban and postscreen.
  *`install --dry-run` and `--json` list output: added in 0.2.0.*
- **Neither** offers an end-user out-of-office reply or forwarding without webmail; Yunori 0.2.0 does, in the
  portal.

## Where Yunori now goes further (0.2.0, all exercised on a real install)

| | Yunori | Openship | BillionMail |
|---|---|---|---|
| DNS automation | Cloudflare: preview, write only missing/wrong records (never deletes), then verified on public resolvers | Cloudflare auto-apply, gates self-attested | none |
| Alerts | certificate, engine down, stuck queue, blocklist, DNS drift, disk, stale backups → email (own sender) + webhook/ntfy, one message per change, daily reminder at most | none | none |
| Queue | list with plain-language reasons, retry, cancel, pause/resume (re-applied after engine restarts) | health only | none |
| Sign-in bans | 20 failures/hour → 1-day ban (Stalwart's stock: 100/day, forever); blocked list, unblock, allowlist | fail2ban (no UI) | fail2ban on console only |
| Bring your old mail | Gmail, Microsoft 365, iCloud, Fastmail, Yahoo, any IMAP; resumable, no duplicates, flags and dates kept | none | none |
| Out-of-office & forwarding | portal, no webmail needed | webmail only | webmail only |
| Mailbox CSV import/export | dry run, invite links | none | plaintext passwords in export |

## Yunori defects this exercise found, all fixed in 0.2.0

1. **Critical:** the edge's proxy allowlist checked the raw path, so `/dav/.%2E/login` reached Stalwart's web admin
   (verified live). Paths are now decoded segment by segment and re-checked after URL normalisation; e2e asserts eleven
   encoded variants return 404.
2. **Backup could not run** on a real install: the engine (user `stalwart`) could not enter Yunori's state directory.
   It now exports into a private directory of its own.
3. **Restore could never succeed:** Stalwart imports only into an empty store and restore imported over the live one,
   then left both services stopped. The current store is now moved aside, put back automatically on failure, and the
   services always restart. `--import` also leaves the search index empty (mail is there, IMAP SEARCH finds nothing),
   so restore schedules Stalwart's `StoreMaintenance`/`reindexAccounts` task; search returns within minutes.
4. The rate-limit table grew without bound on attacker-chosen keys (memory exhaustion); bounded and swept.
5. Anyone could lock the owner out of the console with 8 failed logins; limits are now per address *and* IP.
6. TOTP codes could be replayed within their window; each time step is accepted once.
7. The console could reset the engine administrator's password and disconnect itself; refused (API and CLI).
8. The saved relay password would follow a changed relay host; a new destination needs it typed again.
9. The edge bound only `[::]` (failed on kernels without IPv6, IPv4-blind with `bindv6only=1`); dual-stack explicit,
   IPv4 fallback.
10. Stalwart's bootstrap management listener was reachable from outside during install; a transient nftables table
    now limits it to loopback.
11. An interrupted first engine run could never be resumed; it is now restarted cleanly.
12. Portal sessions were sliding (password kept in memory indefinitely); hard 30-minute cap, as documented.
13. Concurrent audit writes from the CLI and the service could fail after the change was made; `BEGIN IMMEDIATE`.
14. Quota arithmetic overflowed on absurd input; capped at 16 TB.
15. No graceful shutdown and a lightly hardened unit; SIGTERM drains requests, unit gains the full systemd sandbox.
16. The public install script still served v0.1.0, whose `preflight` advertised in the README did not exist.

Reproduce: `sudo python3 e2e/vps-e2e.py` on an installed throwaway server (and `e2e/vps-smoke.sh` where port 80 is
reachable for Let's Encrypt).
