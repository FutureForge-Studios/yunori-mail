#!/bin/sh
# End-to-end smoke test on a THROWAWAY VPS (never a production box).
#
# Prereqs: fresh Debian 12/13 or Ubuntu 24.04 VM, root, a test domain whose DNS
# you control, and an A record  mail.<domain> → this VM already published.
#
#   E2E_DOMAIN=yunori-e2e.example E2E_ADMIN=you@gmail.com sh e2e/vps-smoke.sh
#
# Exercises the real Stalwart engine: install (pinned + checksummed), bootstrap,
# listeners, ACME, served-certificate probe, domain/mailbox/alias CRUD through
# the CLI (same engine adapter as the console), backup, doctor.

set -eu
: "${E2E_DOMAIN:?set E2E_DOMAIN}"
: "${E2E_ADMIN:?set E2E_ADMIN (an address on another provider)}"
BIN="${BIN:-/usr/local/bin/yunori-mail}"

step() { printf '\n== %s\n' "$*"; }

step "install (stops at the DNS gate on a fresh domain; that is expected)"
"$BIN" install --domain "$E2E_DOMAIN" --admin-email "$E2E_ADMIN" --first-mailbox e2e --no-wait || true
"$BIN" status

step "engine reachable through the adapter"
"$BIN" domain list
"$BIN" mailbox list

step "mailbox + alias lifecycle"
"$BIN" mailbox add "probe@$E2E_DOMAIN" --name "E2E Probe" --quota-mb 100
"$BIN" alias add "probe-alias@$E2E_DOMAIN" "probe@$E2E_DOMAIN"
"$BIN" alias list | grep -q "probe-alias@$E2E_DOMAIN"
"$BIN" alias remove "probe-alias@$E2E_DOMAIN"
"$BIN" mailbox passwd "probe@$E2E_DOMAIN"
"$BIN" mailbox remove "probe@$E2E_DOMAIN" --yes

step "listeners: 25/465/587/993/4190 owned by stalwart, 80/443 by yunori-mail"
ss -ltnp | grep -E ':(25|465|587|993|4190)\b' | grep -q stalwart
ss -ltnp | grep -E ':(80|443)\b' | grep -q yunori-mail

step "autoconfig carries the Yunori marker"
curl -fsk "https://127.0.0.1/mail/config-v1.1.xml?emailaddress=e2e@$E2E_DOMAIN" -H "Host: mail.$E2E_DOMAIN" \
  | grep -q '<displayShortName>Yunori</displayShortName>'

step "JMAP is proxied, Stalwart's own admin is not"
code=$(curl -sk -o /dev/null -w '%{http_code}' https://127.0.0.1/.well-known/jmap)
[ "$code" != "404" ] || { echo "JMAP not proxied"; exit 1; }
code=$(curl -sk -o /dev/null -w '%{http_code}' https://127.0.0.1/admin)
[ "$code" = "404" ] || { echo "Stalwart admin exposed ($code)"; exit 1; }

step "certificate + doctor"
"$BIN" certificate status
"$BIN" doctor || echo "(doctor reported problems — expected until DNS/PTR are published)"

step "encrypted backup"
"$BIN" backup --out /root/yunori-e2e-backups
ls -l /root/yunori-e2e-backups

printf "\nE2E smoke passed.\n"
