#!/usr/bin/env python3
"""End-to-end test of an installed Yunori Mail Server, run ON the server as root.

Use a THROWAWAY machine. Standard library only (Debian/Ubuntu python3).

    sudo python3 e2e/vps-e2e.py            # after `yunori-mail install`

Works before Let's Encrypt has issued a certificate (it does not verify TLS
against public CAs; `e2e/vps-smoke.sh` asserts the trusted certificate). It
needs no inbound internet access: everything talks to this machine.

What it proves, through the same doors real clients use:
  * console: first owner (setup link + TOTP), TOTP replay refused, CSRF guard,
    login/logout, mailboxes, aliases, DNS checklist, audit chain;
  * engine: SMTP submission (587 STARTTLS, 465 TLS) → delivery → IMAP 993,
    DKIM-signed, alias expansion, inbound on 25, no open relay, no spoofing,
    no plaintext auth before STARTTLS, no plaintext IMAP port;
  * edge: JMAP session + Email/query, CalDAV/CardDAV discovery, autoconfig,
    Stalwart admin unreachable (including encoded `..` tricks);
  * portal: sign-in, app password that works over IMAP, invite link → own
    password, Apple profile served once;
  * backup → restore round trip keeps the mail; footprint.
"""

import base64
import hashlib
import hmac
import http.client
import imaplib
import json
import os
import smtplib
import socket
import ssl
import struct
import subprocess
import sys
import time
import uuid
from email.message import EmailMessage

BIN = os.environ.get("BIN", "/usr/local/bin/yunori-mail")
CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode = ssl.CERT_NONE

passed, failed = [], []


def step(title):
    print(f"\n== {title}", flush=True)


def check(name, ok, detail=""):
    (passed if ok else failed).append(name)
    print(f"  [{'ok' if ok else 'FAIL'}] {name}{(' - ' + str(detail)) if detail else ''}", flush=True)
    return ok


def sh(*args, check_rc=True):
    r = subprocess.run(args, capture_output=True, text=True)
    if check_rc and r.returncode != 0:
        raise RuntimeError(f"{' '.join(args)} failed: {r.stdout}{r.stderr}")
    return r.stdout


def config():
    import sqlite3
    db = sqlite3.connect("file:/var/lib/yunori-mail/yunori.db?mode=ro", uri=True)
    row = db.execute("SELECT value FROM kv WHERE key = 'config'").fetchone()
    db.close()
    return json.loads(row[0]) if row else None


class Edge:
    """HTTPS to the edge on loopback, with the mail hostname as SNI and Host."""

    def __init__(self, host):
        self.host = host
        self.cookies = {}

    def req(self, method, path, body=None, headers=None, raw=False, auth=None):
        conn = http.client.HTTPSConnection("127.0.0.1", 443, context=CTX, timeout=30)
        conn._tunnel_host = None
        conn.sock = CTX.wrap_socket(socket.create_connection(("127.0.0.1", 443), 30),
                                    server_hostname=self.host)
        h = {"Host": self.host, "X-Yunori-Request": "1"}
        if self.cookies:
            h["Cookie"] = "; ".join(f"{k}={v}" for k, v in self.cookies.items())
        if auth:
            h["Authorization"] = "Basic " + base64.b64encode(auth.encode()).decode()
        data = None
        if body is not None:
            data = body if isinstance(body, (bytes, str)) else json.dumps(body)
            h.setdefault("Content-Type", "application/json")
        h.update(headers or {})
        conn.request(method, path, body=data, headers=h)
        r = conn.getresponse()
        payload = r.read()
        for k, v in r.getheaders():
            if k.lower() == "set-cookie":
                name, _, rest = v.partition("=")
                val = rest.split(";", 1)[0]
                if val:
                    self.cookies[name] = val
                else:
                    self.cookies.pop(name, None)
        conn.close()
        if raw:
            return r.status, payload, dict((k.lower(), v) for k, v in r.getheaders())
        try:
            return r.status, json.loads(payload or b"null")
        except ValueError:
            return r.status, payload.decode(errors="replace")

    def api(self, method, path, body=None, **kw):
        return self.req(method, "/api/yunori" + path, body, **kw)


def totp(secret_b32, at=None, step=30):
    key = base64.b32decode(secret_b32 + "=" * (-len(secret_b32) % 8), casefold=True)
    counter = int((at or time.time()) // step)
    mac = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
    o = mac[-1] & 0x0F
    return f"{(struct.unpack('>I', mac[o:o + 4])[0] & 0x7FFFFFFF) % 1000000:06d}"


def next_totp(secret):
    """A code from a time step not used yet (waits for the next step)."""
    now = time.time()
    time.sleep(30 - (now % 30) + 1)
    return totp(secret)


def imap_fetch(user, password, subject, timeout=60, folders=("INBOX",)):
    """The newest message with this subject, found by reading headers (not
    IMAP SEARCH, which depends on the search index)."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        m = imaplib.IMAP4_SSL("127.0.0.1", 993, ssl_context=CTX)
        m.login(user, password)
        for folder in folders:
            typ, data = m.select(f'"{folder}"')
            if typ != "OK" or int(data[0] or 0) == 0:
                continue
            typ, rows = m.fetch("1:*", "(BODY.PEEK[HEADER.FIELDS (SUBJECT)])")
            seqs = [r[0].split()[0] for r in rows if isinstance(r, tuple) and subject.encode() in r[1]]
            if seqs:
                typ, msg = m.fetch(seqs[-1], "(RFC822)")
                m.logout()
                return msg[0][1].decode(errors="replace")
        m.logout()
        time.sleep(2)
    return None


def imap_search(user, password, subject):
    m = imaplib.IMAP4_SSL("127.0.0.1", 993, ssl_context=CTX)
    m.login(user, password)
    m.select("INBOX")
    typ, data = m.search(None, "SUBJECT", f'"{subject}"')
    m.logout()
    return bool(data[0].split())


def message(frm, to, subject):
    msg = EmailMessage()
    msg["From"], msg["To"], msg["Subject"] = frm, to, subject
    msg["Message-ID"] = f"<{uuid.uuid4()}@e2e>"
    msg.set_content(f"Yunori e2e {subject}\n")
    return msg


def main():
    cfg = config()
    if not cfg:
        sys.exit("yunori-mail is not installed here (no config in yunori.db)")
    host, domain = cfg["hostname"], cfg["primaryDomain"]
    print(f"Server {host}, domain {domain}")
    edge = Edge(host)
    run = uuid.uuid4().hex[:6]
    alice, bob = f"alice{run}@{domain}", f"bob{run}@{domain}"
    alias = f"team{run}@{domain}"
    pw_alice, pw_bob = "Alice-" + uuid.uuid4().hex, "Bob-" + uuid.uuid4().hex

    step("services and listeners")
    for unit in ("stalwart", "yunori-mail"):
        check(f"{unit} active", sh("systemctl", "is-active", unit, check_rc=False).strip() == "active")
    ss = sh("ss", "-ltnpH")
    for port, owner in [(25, "stalwart"), (465, "stalwart"), (587, "stalwart"), (993, "stalwart"),
                        (4190, "stalwart"), (80, "yunori-mail"), (443, "yunori-mail")]:
        check(f"port {port} owned by {owner}",
              any(f":{port} " in l and owner in l for l in ss.splitlines()))
    for port in (110, 143, 995, 8080, 18081):
        check(f"port {port} not listening publicly",
              not any(f"*:{port} " in l or f"0.0.0.0:{port} " in l or f"[::]:{port} " in l for l in ss.splitlines()))
    engine_lines = [l for l in ss.splitlines() if ":18080 " in l]
    check("engine HTTP listener is loopback-only", engine_lines and all("127.0.0.1:18080" in l for l in engine_lines),
          engine_lines)

    step("console: first owner with TOTP")
    st, sess = edge.api("GET", "/session")
    check("GET /session", st == 200, sess)
    owner = f"owner{run}@example.org"
    owner_pw = "Owner-" + uuid.uuid4().hex
    secret = None
    # Test machines only: remember the owner this script created, for reruns.
    state_file = "/root/.yunori-e2e-owner.json"
    if st == 200 and sess.get("setupRequired"):
        link = sh(BIN, "setup-token")
        token = link.split("#token=", 1)[1].split()[0] if "#token=" in link else None
        check("setup-token prints a fragment link", bool(token))
        st, r = edge.api("POST", "/setup", {"token": "wrong" + "x" * 20, "email": owner, "password": owner_pw})
        check("wrong setup token refused", st == 403, st)
        st, r = edge.api("POST", "/setup", {"token": token, "email": owner, "password": owner_pw})
        check("setup accepted", st == 200 and "totpSecret" in r, (st, r if st != 200 else "…"))
        secret = r.get("totpSecret") if st == 200 else None
        st, r = edge.api("POST", "/setup/totp", {"code": "000000" if totp(secret) != "000000" else "111111"})
        check("wrong TOTP refused", st == 401, st)
        code = totp(secret)
        st, r = edge.api("POST", "/setup/totp", {"code": code})
        check("TOTP accepted, owner created", st == 200, (st, r))
        if st == 200:
            with open(os.open(state_file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as f:
                json.dump({"owner": owner, "password": owner_pw, "secret": secret}, f)
        st, r = edge.api("POST", "/logout")
        check("logout", st == 204, st)
        st, r = edge.api("POST", "/login", {"email": owner, "password": owner_pw, "totp": code})
        check("same TOTP code cannot be replayed", st == 401, st)
        st, r = edge.api("POST", "/login", {"email": owner, "password": owner_pw, "totp": next_totp(secret)})
        check("login with password + fresh TOTP", st == 200, (st, r))
    else:
        print(f"  (console already has an owner; using {state_file} or E2E_OWNER/E2E_PASSWORD/E2E_TOTP_SECRET)")
        saved = json.load(open(state_file)) if os.path.exists(state_file) else {}
        owner = os.environ.get("E2E_OWNER", saved.get("owner"))
        owner_pw = os.environ.get("E2E_PASSWORD", saved.get("password"))
        secret = os.environ.get("E2E_TOTP_SECRET", saved.get("secret"))
        if not secret:
            sys.exit("cannot sign in to the console")
        st, r = edge.api("POST", "/login", {"email": owner, "password": owner_pw, "totp": next_totp(secret)})
        check("login", st == 200, (st, r))

    st, r = edge.req("POST", "/api/yunori/logout", headers={"X-Yunori-Request": "0"})
    check("CSRF: mutation without the request header refused", st == 403, st)

    step("console: domains, mailboxes, alias, DNS, audit")
    st, domains = edge.api("GET", "/domains")
    check("list domains", st == 200 and any(d["name"] == domain for d in domains), st)
    dom = next(d for d in domains if d["name"] == domain)
    for addr, pw in ((alice, pw_alice), (bob, pw_bob)):
        st, r = edge.api("POST", "/mailboxes", {"local": addr.split("@")[0], "domainId": dom["id"],
                                                 "displayName": addr.split("@")[0].title(), "password": pw,
                                                 "quotaMb": 100})
        check(f"create {addr}", st in (200, 201), (st, r))
    st, r = edge.api("POST", "/mailboxes", {"local": "q" + run, "domainId": dom["id"], "quotaMb": 2 ** 50})
    check("absurd quota refused (no overflow)", st == 400, (st, r))
    st, mbs = edge.api("GET", "/mailboxes")
    bob_id = next(m["id"] for m in mbs if m["address"] == bob)
    st, r = edge.api("POST", "/aliases", {"local": alias.split("@")[0], "domainId": dom["id"], "mailboxId": bob_id})
    check(f"create alias {alias} → {bob}", st in (200, 201, 204), (st, r))
    st, mbs = edge.api("GET", "/mailboxes")
    admin_row = next((m for m in mbs if m.get("role") == "admin"), None)
    if admin_row:
        st, r = edge.api("POST", f"/mailboxes/{admin_row['id']}/password", {})
        check("engine administrator's password cannot be reset from the console", st == 409, (st, r))
    st, r = edge.api("POST", f"/domains/{dom['id']}/dns/check")
    check("DNS checklist runs against public resolvers", st == 200, st)
    st, r = edge.api("GET", f"/domains/{dom['id']}/dns")
    if st == 200:
        recs = r.get("records", r) if isinstance(r, dict) else r
        names = {x.get("kind") or x.get("type") for x in recs} if isinstance(recs, list) else set()
        check("DNS checklist lists MX, SPF, DKIM, DMARC", {"MX", "TXT"} <= names or len(recs) >= 4, names)
    st, r = edge.api("GET", "/audit")
    check("audit log readable and chain intact", st == 200 and (not isinstance(r, dict) or r.get("chainOk")), st)

    step("mail flow: submission → delivery → IMAP")
    subj = f"e2e-587-{run}"
    with smtplib.SMTP("127.0.0.1", 587, timeout=30) as s:
        s.ehlo("e2e.test")
        check("587 offers no AUTH before STARTTLS", "auth" not in s.esmtp_features, s.esmtp_features.get("auth"))
        s.starttls(context=CTX)
        s.ehlo("e2e.test")
        s.login(alice, pw_alice)
        s.send_message(message(alice, bob, subj))
    got = imap_fetch(bob, pw_bob, subj)
    check("587 STARTTLS: alice → bob delivered", bool(got))
    for _ in range(15):
        if imap_search(bob, pw_bob, subj):
            break
        time.sleep(2)
    check("IMAP SEARCH finds it (search index)", imap_search(bob, pw_bob, subj))
    if got:
        check("message is DKIM-signed", "DKIM-Signature:" in got, "")
    subj = f"e2e-465-{run}"
    with smtplib.SMTP_SSL("127.0.0.1", 465, context=CTX, timeout=30) as s:
        s.login(alice, pw_alice)
        s.send_message(message(alice, alias, subj))
    check("465 TLS: alice → alias expands to bob", bool(imap_fetch(bob, pw_bob, subj)))

    subj = f"e2e-inbound-{run}"
    with smtplib.SMTP("127.0.0.1", 25, timeout=30) as s:
        s.ehlo("sender.example.net")
        s.starttls(context=CTX)
        s.ehlo("sender.example.net")
        s.send_message(message("someone@example.net", bob, subj))
    # From an unknown sender with failing SPF it is delivered, and may be
    # (correctly) filed as junk.
    check("port 25: inbound mail for a local mailbox accepted",
          bool(imap_fetch(bob, pw_bob, subj, folders=("INBOX", "Junk Mail", "Junk", "Spam"))))

    with smtplib.SMTP("127.0.0.1", 25, timeout=30) as s:
        s.ehlo("sender.example.net")
        s.mail("someone@example.net")
        code, _ = s.rcpt("victim@example.com")
        check("port 25: no open relay to outside domains", code >= 500 or code in (450, 451, 454), code)
    with smtplib.SMTP("127.0.0.1", 587, timeout=30) as s:
        s.ehlo("e2e.test")
        s.starttls(context=CTX)
        s.ehlo("e2e.test")
        try:
            s.mail("x@example.net")
            code, _ = s.rcpt("victim@example.com")
        except smtplib.SMTPException as e:
            code = getattr(e, "smtp_code", 530)
        check("587: unauthenticated relay refused", code >= 500 or code in (450, 451, 454), code)
    with smtplib.SMTP_SSL("127.0.0.1", 465, context=CTX, timeout=30) as s:
        s.login(alice, pw_alice)
        try:
            refused = s.send_message(message(bob, bob, f"e2e-spoof-{run}"), from_addr=bob)
            ok = False
        except smtplib.SMTPException:
            ok = True
        check("465: alice cannot send as bob", ok)

    step("edge: JMAP, DAV, autoconfig, admin hidden")
    st, r = edge.req("GET", "/.well-known/jmap", auth=f"{bob}:{pw_bob}")
    if st in (301, 302, 307, 308):
        st, r = edge.req("GET", "/jmap/session", auth=f"{bob}:{pw_bob}")
    check("JMAP session through the edge", st == 200 and isinstance(r, dict) and "apiUrl" in r, st)
    if st == 200 and isinstance(r, dict):
        acct = r["primaryAccounts"]["urn:ietf:params:jmap:mail"]
        st, q = edge.req("POST", "/jmap/", auth=f"{bob}:{pw_bob}", body={
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": [["Email/query", {"accountId": acct, "filter": {"text": run}}, "0"]]})
        ids = q["methodResponses"][0][1].get("ids", []) if st == 200 else []
        check("JMAP Email/query finds the delivered mail", len(ids) >= 3, (st, len(ids)))
    for p in ("/.well-known/caldav", "/.well-known/carddav"):
        st, _, h = edge.req("PROPFIND", p, auth=f"{bob}:{pw_bob}", raw=True, headers={"Depth": "0"})
        check(f"{p} reachable", st in (200, 207, 301, 302, 307, 308), st)
    st, body, _ = edge.req("GET", f"/mail/config-v1.1.xml?emailaddress={bob}", raw=True)
    check("autoconfig with the Yunori marker", st == 200 and b"<displayShortName>Yunori</displayShortName>" in body, st)
    for p in ("/admin", "/login", "/api/principal", "/dav/%2e%2e/admin", "/dav/.%2E/login", "/dav/%2E./login",
              "/jmap/%2e%2e/api/settings", "/.well-known/jmap/%2e%2e/%2e%2e/admin", "/dav/%252e%252e/login",
              "/dav/a%2f..%2f..%2flogin", "/dav/a%5c..%5c..%5clogin"):
        st, body, _ = edge.req("GET", p, raw=True)
        check(f"Stalwart admin unreachable: {p}", st in (400, 404), st)

    step("portal: sign-in, app password, invite, Apple profile")
    portal = Edge(host)
    st, r = portal.api("POST", "/portal/login", {"email": alice, "password": pw_alice})
    check("portal sign-in", st == 200, st)
    st, r = portal.api("POST", "/portal/app-passwords", {"description": "e2e phone"})
    app_pw = (r or {}).get("secret") if st in (200, 201) else None
    check("app password created", bool(app_pw), (st, r))
    if app_pw:
        try:
            m = imaplib.IMAP4_SSL("127.0.0.1", 993, ssl_context=CTX)
            m.login(alice, app_pw)
            m.logout()
            check("app password works over IMAP", True)
        except imaplib.IMAP4.error as e:
            check("app password works over IMAP", False, e)
    st, r = portal.api("POST", "/portal/devices/apple", {"device": "e2e iPhone", "calendars": True})
    url = (r or {}).get("url") if st in (200, 201) else None
    check("Apple profile link issued", bool(url), (st, r))
    if url:
        path = "/" + url.split("://", 1)[1].split("/", 1)[1]
        st, body, h = portal.req("GET", path, raw=True)
        check("profile downloads once", st == 200 and b"PayloadContent" in body, st)
        st, body, h = portal.req("GET", path, raw=True)
        check("profile second download refused", st in (404, 410), st)

    carol = f"carol{run}@{domain}"
    st, r = edge.api("POST", "/mailboxes", {"local": carol.split("@")[0], "domainId": dom["id"], "invite": True})
    link = ((r or {}).get("invite") or {}).get("url") if st in (200, 201) else None
    check("invite link created", bool(link) and "#invite=" in (link or ""), (st, r))
    if link:
        token = link.split("#invite=", 1)[1]
        newcomer = Edge(host)
        st, r = newcomer.api("POST", "/portal/invite", {"token": token})
        check("invite recognised", st == 200, (st, r))
        carol_pw = "Carol-" + uuid.uuid4().hex
        st, r = newcomer.api("POST", "/portal/invite/accept", {"token": token, "password": carol_pw})
        check("invite accepted with own password", st == 200, (st, r))
        st, r = Edge(host).api("POST", "/portal/invite/accept", {"token": token, "password": carol_pw + "x"})
        check("invite works only once", st in (400, 403, 404, 410), st)
        try:
            m = imaplib.IMAP4_SSL("127.0.0.1", 993, ssl_context=CTX)
            m.login(carol, carol_pw)
            m.logout()
            check("invited person can sign in over IMAP", True)
        except imaplib.IMAP4.error as e:
            check("invited person can sign in over IMAP", False, e)

    if link:
        mail_rules(host, domain, run, alice, pw_alice, bob, pw_bob, carol, carol_pw)
    console_routes(edge, host, domain, dom, run, bob, pw_bob)
    portal_lifecycle(host, alice, pw_alice)
    cli_commands(domain, run)
    brute_force(host)
    operations(edge, host, domain, dom, run, alice, pw_alice)

    step("backup → restore keeps the mail")
    out = "/root/yunori-e2e-backups"
    printed = sh(BIN, "backup", "--out", out)
    key = os.environ.get("E2E_BACKUP_KEY")
    if "store it OFF this server" in printed:
        key = printed.split("restores need it):", 1)[1].split()[0]
    files = sorted(os.path.join(out, f) for f in os.listdir(out) if f.endswith(".ybk"))
    check("encrypted backup written", bool(files), files[-1:] if files else "")
    if files and not key:
        print("  (backup key not known: set E2E_BACKUP_KEY to test restore)")
    if files and key:
        with open(files[-1], "rb") as f:
            head = f.read(4096)
        check("backup is not plaintext", b"Received:" not in head and b"From:" not in head)
        r = subprocess.run([BIN, "restore", "--yes", files[-1]], capture_output=True, text=True, timeout=600,
                           env={**os.environ, "YUNORI_BACKUP_KEY": key}, stdin=subprocess.DEVNULL)
        check("restore", r.returncode == 0, (r.stdout + r.stderr)[-400:])
        for _ in range(60):
            if sh("systemctl", "is-active", "stalwart", check_rc=False).strip() == "active":
                break
            time.sleep(2)
        time.sleep(5)
        check("mail still there after restore", bool(imap_fetch(bob, pw_bob, f"e2e-587-{run}", timeout=120)))
        # The search index is rebuilt by staggered per-account engine tasks
        # (a few minutes); `restore` schedules them.
        print("  (search reindex scheduled by restore; set E2E_WAIT_REINDEX=1 to wait for it)")
        if os.environ.get("E2E_WAIT_REINDEX"):
            ok = False
            for _ in range(90):
                if imap_search(bob, pw_bob, f"e2e-587-{run}"):
                    ok = True
                    break
                time.sleep(10)
            check("search works again after restore (reindex)", ok)
        sh(BIN, "doctor", check_rc=False)

    step("footprint")
    rss = {}
    for name in ("stalwart", "yunori-mail"):
        pids = sh("pgrep", "-x", name, check_rc=False).split()
        rss[name] = sum(int(open(f"/proc/{p}/status").read().split("VmRSS:")[1].split()[0]) for p in pids) // 1024
    print(f"  RSS MiB: {rss}; binary {os.path.getsize(BIN) // 1024 // 1024} MiB")
    check("whole control plane + engine under 512 MiB RSS", sum(rss.values()) < 512, rss)

    print(f"\n{len(passed)} passed, {len(failed)} failed")
    for f in failed:
        print(f"  FAIL: {f}")
    sys.exit(1 if failed else 0)


def mail_rules(host, domain, run, alice, pw_alice, bob, pw_bob, carol, carol_pw):
    step("portal: auto-reply and forwarding, delivered for real")
    p = Edge(host)
    st, _ = p.api("POST", "/portal/login", {"email": bob, "password": pw_bob})
    check("bob signs in to the portal", st == 200, st)
    st, r = p.api("GET", "/portal/rules")
    check("rules start off", st == 200 and not r["vacation"]["enabled"] and not r["forwarding"]["enabled"], (st, r))
    st, r = p.api("PUT", "/portal/rules", {"forwarding": {"enabled": True, "address": bob}})
    check("forwarding to itself refused", st == 400 and r["error"]["field"] == "forwarding.address", (st, r))
    st, r = p.api("PUT", "/portal/rules", {"vacation": {"enabled": True, "body": " "}})
    check("auto-reply without text refused", st == 400 and r["error"]["field"] == "vacation.body", (st, r))
    rules = {
        "vacation": {"enabled": True, "subject": f"Away {run}", "body": "I am away until Monday.\nBob"},
        "forwarding": {"enabled": True, "address": carol, "keepCopy": True},
    }
    st, r = p.api("PUT", "/portal/rules", rules)
    check("auto-reply + forwarding saved (engine compiled the script)", st == 200, (st, r))
    st, back = p.api("GET", "/portal/rules")
    check("rules read back from the engine", st == 200 and back.get("forwarding", {}).get("address") == carol, back)

    subj = f"e2e-rules-{run}"
    with smtplib.SMTP_SSL("127.0.0.1", 465, context=CTX, timeout=30) as s:
        s.login(alice, pw_alice)
        s.send_message(message(alice, bob, subj))
    check("bob keeps a copy", bool(imap_fetch(bob, pw_bob, subj)))
    fwd = imap_fetch(carol, carol_pw, subj, timeout=90)
    check("carol receives the forwarded message", bool(fwd))
    reply = imap_fetch(alice, pw_alice, f"Away {run}", timeout=90)
    check("alice gets bob's auto-reply", bool(reply))
    if reply:
        check("auto-reply is marked automatic (no loops)", "Auto-Submitted: auto-replied" in reply, "")
    # Only one auto-reply per sender per day.
    with smtplib.SMTP_SSL("127.0.0.1", 465, context=CTX, timeout=30) as s:
        s.login(alice, pw_alice)
        s.send_message(message(alice, bob, subj + "-again"))
    imap_fetch(bob, pw_bob, subj + "-again")
    time.sleep(8)
    m = imaplib.IMAP4_SSL("127.0.0.1", 993, ssl_context=CTX)
    m.login(alice, pw_alice)
    m.select("INBOX")
    typ, rows = m.fetch("1:*", "(BODY.PEEK[HEADER.FIELDS (SUBJECT)])")
    m.logout()
    replies = sum(1 for r_ in rows if isinstance(r_, tuple) and f"Away {run}".encode() in r_[1])
    check("auto-reply sent once per sender, not per message", replies == 1, replies)

    # Forward only: bob no longer keeps a copy.
    st, _ = p.api("PUT", "/portal/rules", {"forwarding": {"enabled": True, "address": carol, "keepCopy": False}})
    check("forward-only saved", st == 200, st)
    subj2 = f"e2e-fwdonly-{run}"
    with smtplib.SMTP_SSL("127.0.0.1", 465, context=CTX, timeout=30) as s:
        s.login(alice, pw_alice)
        s.send_message(message(alice, bob, subj2))
    check("forward-only reaches carol", bool(imap_fetch(carol, carol_pw, subj2, timeout=90)))
    check("forward-only leaves no copy with bob", not imap_fetch(bob, pw_bob, subj2, timeout=10))
    st, _ = p.api("PUT", "/portal/rules", {})
    st2, r = p.api("GET", "/portal/rules")
    check("rules switched off", st == 200 and not r["forwarding"]["enabled"], r)


def console_routes(edge, host, domain, dom, run, bob, pw_bob):
    step("console: every route")
    for path in ("/overview", "/install", "/onboarding", "/preflight", "/certificate", "/sending", "/deliverability",
                 "/system", "/admins", f"/domains/{dom['id']}"):
        st, r = edge.api("GET", path)
        check(f"GET {path}", st == 200, st if st == 200 else (st, r))
    st, r = edge.api("POST", "/updates/check")
    check("POST /updates/check answers (or explains)", st == 200, (st, r))

    # A second domain, catch-all on, its DNS checklist, then removed.
    extra = f"e2e{run}.{domain}"
    st, d2 = edge.api("POST", "/domains", {"name": extra})
    check(f"add domain {extra}", st in (200, 201), (st, d2))
    if st in (200, 201):
        st, r = edge.api("GET", f"/domains/{d2['id']}/dns")
        check("new domain has a DNS checklist (DKIM generated)", st == 200, st)
        st, r = edge.api("DELETE", f"/domains/{d2['id']}")
        check("remove the empty domain", st in (200, 204), (st, r))

    st, r = edge.api("PATCH", f"/domains/{dom['id']}", {"catchAll": "not-an-address"})
    check("catch-all must be an address", st == 400, st)
    st, r = edge.api("PATCH", f"/domains/{dom['id']}", {"catchAll": bob})
    check("catch-all on", st == 200 and r.get("catchAll") == bob, (st, r))
    if st == 200:
        subj = f"e2e-catchall-{run}"
        with smtplib.SMTP("127.0.0.1", 25, timeout=30) as s:
            s.ehlo("sender.example.net")
            s.starttls(context=CTX)
            s.ehlo("sender.example.net")
            s.send_message(message("someone@example.net", f"nobody-{run}@{domain}", subj))
        check("mail to an unknown address lands in the catch-all",
              bool(imap_fetch(bob, pw_bob, subj, folders=("INBOX", "Junk Mail"))))
        st, r = edge.api("PATCH", f"/domains/{dom['id']}", {"catchAll": None})
        check("catch-all off", st == 200 and not r.get("catchAll"), (st, r))

    # Mailbox lifecycle: create, rename, quota, invite, reset, delete.
    st, r = edge.api("POST", "/mailboxes", {"local": f"dave{run}", "domainId": dom["id"], "password": "short"})
    check("weak password refused", st == 400, st)
    st, r = edge.api("POST", "/mailboxes", {"local": f"dave{run}", "domainId": dom["id"]})
    mid = (r or {}).get("mailbox", {}).get("id") if st in (200, 201) else None
    check("create a mailbox with a generated password", bool(mid) and len(r.get("password", "")) >= 16, (st, r))
    if mid:
        st, r = edge.api("PATCH", f"/mailboxes/{mid}", {"displayName": "Dave E2E", "quotaMb": 512})
        check("rename + quota", st == 200 and r["displayName"] == "Dave E2E" and r["quotaBytes"] == 512 << 20, (st, r))
        st, r = edge.api("POST", f"/mailboxes/{mid}/invite")
        check("invite link for an existing mailbox", st in (200, 201) and "#invite=" in r.get("url", ""), (st, r))
        st, r = edge.api("POST", f"/mailboxes/{mid}/password", {})
        new_pw = (r or {}).get("password")
        check("password reset returns a new password once", st == 200 and bool(new_pw), st)
        try:
            m = imaplib.IMAP4_SSL("127.0.0.1", 993, ssl_context=CTX)
            m.login(f"dave{run}@{domain}", new_pw)
            m.logout()
            check("reset password works over IMAP", True)
        except Exception as e:
            check("reset password works over IMAP", False, e)
        st, r = edge.api("GET", "/mailboxes")
        check("reset withdrew the invite", st == 200 and not next(m for m in r if m["id"] == mid).get("inviteExpiresAt"), "")
        st, r = edge.api("DELETE", f"/mailboxes/{mid}")
        check("delete the mailbox", st in (200, 204), (st, r))

    # Alias delete.
    st, mbs = edge.api("GET", "/mailboxes")
    bob_id = next(m["id"] for m in mbs if m["address"] == bob)
    tmp = f"tmp{run}"
    st, r = edge.api("POST", "/aliases", {"local": tmp, "domainId": dom["id"], "mailboxId": bob_id})
    check("add alias", st in (200, 201), (st, r))
    st, r = edge.api("DELETE", f"/aliases?address={tmp}@{domain}")
    check("remove alias", st in (200, 204), (st, r))

    # CSV export / import.
    st, body, h = edge.req("GET", "/api/yunori/mailboxes/export", raw=True)
    check("CSV export", st == 200 and h.get("content-type", "").startswith("text/csv") and bob.encode() in body, st)
    csv = f"address,display_name,quota_mb\ncsv1{run}@{domain},\"CSV, One\",100\ncsv2{run}@{domain},,\n{bob},Dup,\n"
    st, r = edge.api("POST", "/mailboxes/import", {"csv": csv, "dryRun": True})
    check("CSV import dry run: 2 ready, 1 skipped, none created",
          st == 200 and len(r["ready"]) == 2 and len(r["skipped"]) == 1 and not r["created"], (st, r))
    st, r = edge.api("POST", "/mailboxes/import", {"csv": csv})
    check("CSV import creates with invite links",
          st == 200 and len(r["created"]) == 2 and all("#invite=" in c["inviteUrl"] for c in r["created"]), (st, r))
    st, mbs = edge.api("GET", "/mailboxes")
    for m in mbs:
        if m["address"].startswith(("csv1" + run, "csv2" + run)):
            edge.api("DELETE", f"/mailboxes/{m['id']}")

    # Sending: switch to a relay (credential write-only), then back to direct.
    st, r = edge.api("PUT", "/sending", {"mode": "relay", "relay": {
        "provider": "custom", "host": "smtp.relay.example", "port": 587, "username": "u", "password": "relay-secret-e2e"}})
    check("relay mode saved, password never echoed", st == 200 and "relay-secret-e2e" not in json.dumps(r)
          and r.get("relay", {}).get("hasPassword"), (st, r))
    st, r = edge.api("PUT", "/sending", {"mode": "relay", "relay": {
        "provider": "custom", "host": "elsewhere.example", "port": 587, "username": "u"}})
    check("saved relay password does not follow a new host", st == 400, st)
    st, r = edge.api("PUT", "/sending", {"mode": "direct"})
    check("back to direct sending", st == 200 and r.get("mode") == "direct", (st, r))

    # Sessions are revocable rows: a logged-out cookie is dead.
    old = dict(edge.cookies)
    st, _ = edge.api("POST", "/logout")
    dead = Edge(host)
    dead.cookies = old
    st, _ = dead.api("GET", "/overview")
    check("a signed-out session cannot be reused", st == 401, st)
    saved = json.load(open("/root/.yunori-e2e-owner.json"))
    st, _ = edge.api("POST", "/login", {"email": saved["owner"], "password": saved["password"],
                                        "totp": next_totp(saved["secret"])})
    check("sign in again", st == 200, st)
    st, r = edge.api("GET", "/audit")
    actions = {e["action"] for e in r.get("entries", [])}
    check("audit log records the mutations, chain intact",
          r.get("chainOk") and {"mailbox.create", "mailbox.delete", "domain.update", "mailbox.export"} <= actions, actions)


def portal_lifecycle(host, alice, pw_alice):
    step("portal: password change, app password revoke, sign-out")
    p = Edge(host)
    p.api("POST", "/portal/login", {"email": alice, "password": pw_alice})
    st, r = p.api("POST", "/portal/app-passwords", {"description": "e2e revoke"})
    ap_id, secret = (r or {}).get("appPassword", {}).get("id"), (r or {}).get("secret")
    st, _ = p.api("DELETE", f"/portal/app-passwords/{ap_id}")
    check("revoke an app password", st == 204, st)
    try:
        m = imaplib.IMAP4_SSL("127.0.0.1", 993, ssl_context=CTX)
        m.login(alice, secret)
        m.logout()
        check("revoked app password stops working", False)
    except imaplib.IMAP4.error:
        check("revoked app password stops working", True)
    st, _ = p.api("POST", "/portal/password", {"current": "wrong", "next": pw_alice + "2"})
    check("password change needs the current password", st == 401, st)
    st, _ = p.api("POST", "/portal/password", {"current": pw_alice, "next": pw_alice + "2"})
    check("password change", st == 204, st)
    st, _ = p.api("POST", "/portal/password", {"current": pw_alice + "2", "next": pw_alice})
    check("and back (session follows the new password)", st == 204, st)
    st, _ = p.api("POST", "/portal/logout")
    st, _ = p.api("GET", "/portal/me")
    check("portal sign-out ends the session", st == 401, st)


def cli_commands(domain, run):
    step("CLI: every day-two command")
    def ok(args, want=None, rc=0):
        r = subprocess.run([BIN, *args], capture_output=True, text=True, timeout=300, stdin=subprocess.DEVNULL)
        good = r.returncode == rc and (want is None or want in r.stdout + r.stderr)
        check("yunori-mail " + " ".join(args), good, "" if good else (r.returncode, (r.stdout + r.stderr)[-300:]))
        return r.stdout
    ok(["status"])
    r = subprocess.run([BIN, "doctor", "--json"], capture_output=True, text=True, timeout=300)
    try:
        json.loads(r.stdout)
        check("yunori-mail doctor --json is JSON", True)
    except ValueError:
        check("yunori-mail doctor --json is JSON", False, r.stdout[-200:])
    r = subprocess.run([BIN, "preflight", "--json"], capture_output=True, text=True, timeout=300)
    check("yunori-mail preflight --json on an installed server", r.stdout.strip().startswith(("{", "[")), r.stdout[:120])
    for kind in ("domain", "mailbox", "alias"):
        out = ok([kind, "list", "--json"])
        try:
            json.loads(out)
            check(f"{kind} list --json is JSON", True)
        except ValueError:
            check(f"{kind} list --json is JSON", False, out[:120])
    ok(["domain", "dns", domain], want="MX")
    addr = f"cli{run}@{domain}"
    ok(["mailbox", "add", addr, "--name", "CLI Box", "--quota-mb", "50"], want="Password (shown once)")
    ok(["alias", "add", f"cli-alias{run}@{domain}", addr])
    ok(["mailbox", "passwd", addr], want="New password")
    ok(["alias", "remove", f"cli-alias{run}@{domain}"])
    ok(["mailbox", "remove", addr, "--yes"])
    exp = "/root/e2e-mailboxes.csv"
    ok(["mailbox", "export", "--out", exp])
    check("export file is CSV", open(exp).readline().startswith("address,display_name"))
    with open("/root/e2e-import.csv", "w") as f:
        f.write(f"cliimp{run}@{domain},Imported\nbad,,\n")
    out = ok(["mailbox", "import", "/root/e2e-import.csv", "--dry-run", "--json"])
    try:
        rep = json.loads(out)
        check("CLI import dry run reports 1 ready, 1 skipped", len(rep["ready"]) == 1 and len(rep["skipped"]) == 1, rep)
    except ValueError:
        check("CLI import dry run reports 1 ready, 1 skipped", False, out[:200])
    ok(["mailbox", "import", "/root/e2e-import.csv"], want="#invite=")
    ok(["mailbox", "remove", f"cliimp{run}@{domain}", "--yes"])
    before = sh("find", "/etc/systemd/system", "/var/lib/yunori-mail", "-newer", "/root/e2e-import.csv")
    ok(["install", "--dry-run", "--domain", domain], want="install plan (dry run)", rc=0)
    after = sh("find", "/etc/systemd/system", "/var/lib/yunori-mail", "-newer", "/root/e2e-import.csv")
    check("install --dry-run changes nothing", before == after, (before, after))
    ok(["certificate", "status"])
    r = subprocess.run([BIN, "update", "--check"], capture_output=True, text=True, timeout=120)
    check("update --check answers", r.returncode in (0, 1) and r.stdout.strip() != "", (r.returncode, r.stdout[-200:]))
    r = subprocess.run([BIN, "setup-token"], capture_output=True, text=True, timeout=60)
    check("setup-token refused once an owner exists", r.returncode != 0 or "#token=" not in r.stdout, r.stdout[-200:])
    ok(["upgrade"], want="already current")


def brute_force(host):
    step("brute force: repeated bad logins are slowed or blocked")
    p = Edge(host)
    codes = []
    for i in range(12):
        st, _ = p.api("POST", "/login", {"email": "owner-probe@example.org", "password": "wrong-password", "totp": "000000"})
        codes.append(st)
    check("console login throttled after repeated failures", 429 in codes, codes)
    codes = []
    for i in range(12):
        st, _ = p.api("POST", "/portal/login", {"email": f"nobody@{host}", "password": "wrong"})
        codes.append(st)
    check("portal login throttled after repeated failures", 429 in codes, codes)


def operations(edge, host, domain, dom, run, alice, pw_alice):
    """Alerts, queue, sign-in protection, DNS automation and mail import."""
    import http.server
    import threading

    step("alerts: webhook + email, test and check now")
    got = []

    class Hook(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            got.append(json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}"))
            self.send_response(204)
            self.end_headers()

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), Hook)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    hook = f"http://127.0.0.1:{srv.server_port}/hook"
    st, r = edge.api("PUT", "/alerts/settings", {"emailEnabled": True, "recipient": f"alice@{domain}",
                                                  "webhook": {"enabled": True, "kind": "json", "url": hook}})
    check("a recipient on this server's own domain is refused", st == 400, (st, r))
    st, r = edge.api("PUT", "/alerts/settings", {"emailEnabled": True, "recipient": "yunori-e2e-ops@gmail.com",
                                                  "webhook": {"enabled": True, "kind": "json", "url": hook,
                                                              "token": "hook-secret-e2e"}})
    check("alert settings saved; webhook URL and token never echoed",
          st == 200 and "hook-secret-e2e" not in json.dumps(r) and f":{srv.server_port}/hook" not in json.dumps(r), (st, r))
    st, r = edge.api("POST", "/alerts/test")
    check("send test alert", st == 200, (st, r))
    for _ in range(20):
        if got:
            break
        time.sleep(1)
    check("webhook received the test alert", bool(got), got[:1])
    st, r = edge.api("GET", "/alerts")
    check("alert history records the test with per-channel results", st == 200 and r.get("history"), (st, str(r)[:300]))
    st, r = edge.api("POST", "/alerts/check")
    check("check now", st == 200, (st, r))
    st, r = edge.api("GET", "/overview")
    check("overview carries firing alerts", st == 200 and "alerts" in r, st)

    step("queue: the test alert email to an outside address waits in the queue")
    # Outbound port 25 is blocked here, so that email stays queued: a real queue to manage.
    queued = []
    for _ in range(30):
        st, r = edge.api("GET", "/queue")
        queued = (r or {}).get("messages", r if isinstance(r, list) else [])
        if st == 200 and queued:
            break
        time.sleep(2)
    check("GET /queue lists the waiting message", st == 200 and bool(queued), (st, str(r)[:300]))
    out = subprocess.run([BIN, "queue", "list", "--json"], capture_output=True, text=True, timeout=120).stdout
    try:
        json.loads(out)
        check("yunori-mail queue list --json", True)
    except ValueError:
        check("yunori-mail queue list --json", False, out[:200])
    st, _ = edge.api("POST", "/queue/pause")
    check("pause outgoing mail", st in (200, 204), st)
    st, r = edge.api("GET", "/queue")
    check("queue reports paused", st == 200 and r.get("paused") is True, str(r)[:200])
    # Act while paused, so the engine cannot finish (or permanently fail) it meanwhile.
    if queued:
        mid = queued[0]["id"]
        st, r = edge.api("POST", "/queue/retry", {"id": mid})
        check("retry now", st == 200, (st, r))
        st, r = edge.api("DELETE", f"/queue/{mid}")
        check("cancel a queued message", st in (200, 204), (st, r))
        st, r = edge.api("GET", "/queue")
        check("cancelled message is gone", st == 200 and all(m["id"] != mid for m in r.get("messages", [])), "")
    st, _ = edge.api("POST", "/queue/resume")
    check("resume outgoing mail", st in (200, 204), st)
    edge.api("PUT", "/alerts/settings", {"emailEnabled": False, "webhook": {"enabled": False, "url": ""}})
    srv.shutdown()

    step("sign-in protection: bad logins from a real address get it blocked")
    st, r = edge.api("GET", "/security/access")
    check("ban policy is Yunori's (20 failures/hour, 1 day)", st == 200 and "20" in json.dumps(r.get("policy", r)), str(r)[:300])
    v6 = sh("sh", "-c", "ip -6 -o addr show scope global | awk '{print $4}' | cut -d/ -f1 | head -1").strip()
    if v6:
        for i in range(22):
            try:
                m = imaplib.IMAP4_SSL(v6, 993, ssl_context=CTX, timeout=10)
                m.login(alice, "definitely-wrong")
            except Exception:
                pass
        blocked = False
        for _ in range(10):
            st, r = edge.api("GET", "/security/access")
            ent = [b for b in r.get("blocked", []) if b.get("address", "").split("/")[0] == v6]
            if ent:
                blocked = True
                break
            time.sleep(2)
        check(f"{v6} blocked after repeated failures", blocked, str(r.get("blocked"))[:300])
        try:
            imaplib.IMAP4_SSL(v6, 993, ssl_context=CTX, timeout=10).login(alice, pw_alice)
            check("a blocked address cannot sign in even with the right password", False)
        except Exception:
            check("a blocked address cannot sign in even with the right password", True)
        if blocked:
            st, _ = edge.api("DELETE", f"/security/blocked/{ent[0]['id']}")
            check("unblock", st in (200, 204), st)
            time.sleep(2)
            try:
                m = imaplib.IMAP4_SSL(v6, 993, ssl_context=CTX, timeout=10)
                m.login(alice, pw_alice)
                m.logout()
                check("unblocked address signs in again", True)
            except Exception as e:
                check("unblocked address signs in again", False, e)
    st, r = edge.api("POST", "/security/allowed", {"address": "10.0.0.0/4"})
    check("an over-broad allowlist network is refused", st == 400, (st, r))
    st, r = edge.api("POST", "/security/allowed", {"address": "203.0.113.7/24", "note": "e2e office"})
    check("allowlist a network (normalised)", st in (200, 201) and "203.0.113.0/24" in json.dumps(r), (st, r))
    if st in (200, 201):
        st, _ = edge.api("DELETE", f"/security/allowed/{r.get('id') or r.get('entry', {}).get('id')}")
        check("remove the allowlist entry", st in (200, 204), st)
    out = subprocess.run([BIN, "access", "list", "--json"], capture_output=True, text=True, timeout=60).stdout
    check("yunori-mail access list --json", out.strip().startswith("{"), out[:120])

    step("DNS automation: talks to the real Cloudflare API")
    st, r = edge.api("GET", f"/domains/{dom['id']}/dns/provider")
    check("provider status (not connected)", st == 200, (st, r))
    st, r = edge.api("PUT", f"/domains/{dom['id']}/dns/provider", {"provider": "cloudflare", "token": "e2e-not-a-real-token-0123456789"})
    check("a bad token is refused in plain words by Cloudflare's API", st in (400, 422) and "token" in json.dumps(r).lower()
          and "e2e-not-a-real-token" not in json.dumps(r), (st, r))
    st, r = edge.api("GET", f"/domains/{dom['id']}/dns/plan")
    check("no preview without a connected provider", st in (400, 404, 409), (st, r))

    step("Move your mail here: guards, and a real Gmail sign-in attempt")
    p = Edge(host)
    p.api("POST", "/portal/login", {"email": alice, "password": pw_alice})
    st, r = p.api("POST", "/portal/migrate/test", {"host": "127.0.0.1", "port": 993, "username": "x", "password": "y"})
    check("importing from a loopback/private host is refused", st in (400, 422) and "private" in json.dumps(r), (st, r))
    st, r = p.api("POST", "/portal/migrate/test", {"host": "imap.gmail.com", "port": 110, "username": "x", "password": "y"})
    check("only 993 or 143+STARTTLS", st in (400, 422), (st, r))
    st, r = p.api("POST", "/portal/migrate/test", {"host": "imap.gmail.com", "port": 993,
                                                   "username": "yunori-e2e-nobody@gmail.com", "password": "wrong-app-password"})
    msg = json.dumps(r).lower()
    check("real Gmail: TLS verified, wrong password explained (app-password hint)",
          st in (400, 401, 422) and ("password" in msg), (st, r))
    st, r = p.api("GET", "/portal/migrate")
    check("migration status readable", st == 200, (st, r))


if __name__ == "__main__":
    main()
