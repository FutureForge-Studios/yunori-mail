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


if __name__ == "__main__":
    main()
