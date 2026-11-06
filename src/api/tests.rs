//! API integration tests: the real router against the in-memory engine.

use crate::app::App;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Client {
    router: axum::Router,
    cookies: Vec<String>,
}

impl Client {
    fn new() -> (Self, std::sync::Arc<App>) {
        Self::with(App::for_tests())
    }

    fn with(app: std::sync::Arc<App>) -> (Self, std::sync::Arc<App>) {
        (
            Self {
                router: super::router(app.clone()),
                cookies: vec![],
            },
            app,
        )
    }

    async fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        csrf: bool,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if !self.cookies.is_empty() {
            req = req.header(header::COOKIE, self.cookies.join("; "));
        }
        if csrf {
            req = req.header("x-yunori-request", "1");
        }
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        for c in res.headers().get_all(header::SET_COOKIE) {
            let kv = c.to_str().unwrap().split(';').next().unwrap().to_string();
            let name = kv.split('=').next().unwrap().to_string();
            self.cookies.retain(|x| !x.starts_with(&format!("{name}=")));
            if !kv.ends_with('=') {
                self.cookies.push(kv);
            }
        }
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn get(&mut self, path: &str) -> (StatusCode, Value) {
        self.send("GET", path, None, false).await
    }
    async fn post(&mut self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", path, Some(body), true).await
    }

    /// Complete first-run setup and end up signed in.
    async fn sign_in(&mut self, app: &App) {
        let token = app.db.issue_setup_token().unwrap();
        let (s, b) = self.post("/setup", json!({"token": token, "email": "owner@elsewhere.example", "password": "a-long-passphrase-1"})).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let secret = b["totpSecret"].as_str().unwrap().to_string();
        let code = crate::auth::totp_for(&secret, "owner@elsewhere.example")
            .unwrap()
            .generate_current()
            .unwrap();
        let (s, b) = self.post("/setup/totp", json!({"code": code})).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(b["authenticated"], true);
    }
}

#[tokio::test]
async fn mutations_require_the_csrf_header() {
    let (mut c, _) = Client::new();
    let (s, b) = c
        .send(
            "POST",
            "/login",
            Some(json!({"email": "a@b.co", "password": "x", "totp": "1"})),
            false,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(b["error"]["code"], "csrf");
}

#[tokio::test]
async fn console_requires_a_session() {
    let (mut c, _) = Client::new();
    let (s, b) = c.get("/overview").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(b["error"]["code"], "unauthenticated");
    let (s, b) = c.get("/session").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["setupRequired"], true);
    assert_eq!(b["authenticated"], false);
    assert_eq!(b["server"]["hostname"], "mail.example.test");
}

#[tokio::test]
async fn setup_rejects_bad_token_and_requires_totp() {
    let (mut c, app) = Client::new();
    app.db.issue_setup_token().unwrap();
    let (s, b) = c
        .post(
            "/setup",
            json!({"token": "wrong", "email": "o@x.example", "password": "a-long-passphrase-1"}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(b["error"]["field"], "token");

    let token = app.db.issue_setup_token().unwrap();
    let (s, _) = c
        .post(
            "/setup",
            json!({"token": token, "email": "o@x.example", "password": "a-long-passphrase-1"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    // A setup session is not an admin session until TOTP is confirmed.
    let (s, _) = c.get("/overview").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = c.post("/setup/totp", json!({"code": "000000"})).await;
    assert!(s == StatusCode::UNAUTHORIZED || s == StatusCode::OK);
}

#[tokio::test]
async fn login_is_non_enumerating() {
    let (mut c, app) = Client::new();
    c.sign_in(&app).await;
    c.post("/logout", json!({})).await;
    let (s1, b1) = c
        .post(
            "/login",
            json!({"email": "nobody@x.example", "password": "whatever-123456", "totp": "123456"}),
        )
        .await;
    let (s2, b2) = c.post("/login", json!({"email": "owner@elsewhere.example", "password": "wrong-password-1", "totp": "123456"})).await;
    assert_eq!(s1, StatusCode::UNAUTHORIZED);
    assert_eq!(s2, StatusCode::UNAUTHORIZED);
    assert_eq!(b1["error"]["message"], b2["error"]["message"]);
}

#[tokio::test]
async fn domains_mailboxes_aliases_flow() {
    let (mut c, app) = Client::new();
    c.sign_in(&app).await;

    let (s, b) = c.get("/domains").await;
    assert_eq!(s, StatusCode::OK);
    let primary = b
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["isPrimary"] == true)
        .unwrap()
        .clone();
    assert_eq!(primary["name"], "example.test");
    let domain_id = primary["id"].as_str().unwrap().to_string();

    // Primary domain cannot be deleted.
    let (s, _) = c
        .send("DELETE", &format!("/domains/{domain_id}"), None, true)
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Create a mailbox with a generated password.
    let (s, b) = c
        .post(
            "/mailboxes",
            json!({"local": "Sora", "domainId": domain_id, "displayName": "Sora", "quotaMb": 1024}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["mailbox"]["address"], "sora@example.test");
    assert_eq!(b["mailbox"]["quotaBytes"], 1024 * 1024 * 1024);
    assert!(b["password"].as_str().unwrap().len() >= 20);
    let mailbox_id = b["mailbox"]["id"].as_str().unwrap().to_string();

    // Duplicate address → conflict.
    let (s, _) = c
        .post(
            "/mailboxes",
            json!({"local": "sora", "domainId": domain_id}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    // Short explicit password → field error.
    let (s, b) = c
        .post(
            "/mailboxes",
            json!({"local": "kai", "domainId": domain_id, "password": "short"}),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(b["error"]["field"], "password");

    // Alias that would shadow a mailbox → conflict; a fresh alias works.
    let (s, _) = c
        .post(
            "/aliases",
            json!({"local": "ana", "domainId": domain_id, "mailboxId": mailbox_id}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, b) = c
        .post(
            "/aliases",
            json!({"local": "team", "domainId": domain_id, "mailboxId": mailbox_id}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (_, b) = c.get("/aliases").await;
    assert!(
        b.as_array()
            .unwrap()
            .iter()
            .any(|a| a["address"] == "team@example.test"
                && a["mailboxAddress"] == "sora@example.test")
    );
    let (s, _) = c
        .send("DELETE", "/aliases?address=team@example.test", None, true)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Reset password, then delete.
    let (s, b) = c
        .post(&format!("/mailboxes/{mailbox_id}/password"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(b["password"].is_string());
    let (s, _) = c
        .send("DELETE", &format!("/mailboxes/{mailbox_id}"), None, true)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Everything above was audited and the chain verifies.
    let (_, b) = c.get("/audit").await;
    assert_eq!(b["chainOk"], true);
    let actions: Vec<&str> = b["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for a in [
        "mailbox.create",
        "alias.create",
        "alias.delete",
        "mailbox.password_reset",
        "mailbox.delete",
    ] {
        assert!(actions.contains(&a), "missing audit {a}");
    }
}

#[tokio::test]
async fn overview_and_sending_relay() {
    let (mut c, app) = Client::new();
    c.sign_in(&app).await;
    let (s, b) = c.get("/overview").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["verdict"]["sentence"].as_str().unwrap().len() > 10);
    assert_eq!(b["stats"]["mailboxes"], 3);

    // Switch to relay; the password is write-only.
    let (s, b) = c
        .send("PUT", "/sending", Some(json!({"mode": "relay", "relay": {"provider": "postmark", "host": "smtp.postmarkapp.com", "port": 587, "username": "token", "password": "relay-secret"}})), true)
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["mode"], "relay");
    assert_eq!(b["relay"]["hasPassword"], true);
    assert!(!b.to_string().contains("relay-secret"));
    assert_eq!(
        app.mock().unwrap().relay().unwrap().host,
        "smtp.postmarkapp.com"
    );

    let (s, b) = c.get("/deliverability").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["queue"]["deferred"], 1);
    assert_eq!(b["dmarcPolicy"], "none");
}

#[tokio::test]
async fn sending_through_amazon_ses_and_resend() {
    let (mut c, app) = Client::new();
    c.sign_in(&app).await;
    let (s, b) = c.get("/sending").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let keys: Vec<&str> = b["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        &keys[..2],
        ["ses", "resend"],
        "SES and Resend lead the picker"
    );
    assert!(
        b["domains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "example.test")
    );

    // Amazon SES: region → host; pasted Easy DKIM rows → three CNAMEs + MAIL FROM.
    let t = [
        "abcdefghijklmnopqrstuvwxyz012345",
        "bcdefghijklmnopqrstuvwxyz0123456",
        "cdefghijklmnopqrstuvwxyz01234567",
    ];
    let pasted = t
        .iter()
        .map(|t| format!("{t}._domainkey.example.test CNAME {t}.dkim.amazonses.com"))
        .collect::<Vec<_>>()
        .join("\n");
    let (s, b) = c
        .send(
            "PUT",
            "/sending",
            Some(json!({"mode": "relay", "relay": {
                "provider": "ses", "region": "eu-west-1", "port": 587,
                "username": "AKIAEXAMPLESMTP", "password": "ses-smtp-secret",
                "identities": {"example.test": {"dkim": pasted, "mailFrom": "bounce"}}
            }})),
            true,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["relay"]["host"], "email-smtp.eu-west-1.amazonaws.com");
    assert_eq!(
        app.mock().unwrap().relay().unwrap().host,
        "email-smtp.eu-west-1.amazonaws.com"
    );
    assert!(!b.to_string().contains("ses-smtp-secret"));
    let dom = b["domains"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "example.test")
        .unwrap()
        .clone();
    assert_eq!(dom["needsVerification"], false);
    let recs = dom["records"].as_array().unwrap();
    assert_eq!(recs.iter().filter(|r| r["type"] == "CNAME").count(), 3);
    assert!(recs.iter().any(|r| r["type"] == "MX"
        && r["name"] == "bounce.example.test"
        && r["value"] == "10 feedback-smtp.eu-west-1.amazonses.com"));
    assert!(
        b["relay"]["domainsUrl"]
            .as_str()
            .unwrap()
            .contains("region=eu-west-1")
    );

    // A bad paste is reported on that domain's field; nothing is saved.
    let (s, b) = c
        .send("PUT", "/sending", Some(json!({"mode": "relay", "relay": {
            "provider": "ses", "region": "eu-west-1", "port": 587, "username": "AKIAEXAMPLESMTP",
            "identities": {"example.test": {"dkim": t[0]}}
        }})), true)
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(b["error"]["field"], "identities.example.test.dkim");

    // Resend: fixed host and username, the API key as the password, send.<domain> records.
    let (s, b) = c
        .send("PUT", "/sending", Some(json!({"mode": "relay", "relay": {
            "provider": "resend", "region": "eu-west-1", "port": 587, "password": "re_live_key",
            "identities": {"example.test": {"dkim": "p=MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC7xYwHmS4fXh5qk9Jh2bU0Lw8mN3pQ1rT6vY"}}
        }})), true)
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["relay"]["host"], "smtp.resend.com");
    assert_eq!(b["relay"]["username"], "resend");
    let relay = app.mock().unwrap().relay().unwrap();
    assert_eq!(
        (relay.host.as_str(), relay.username.as_str()),
        ("smtp.resend.com", "resend")
    );
    let dom = b["domains"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "example.test")
        .unwrap()
        .clone();
    let recs = dom["records"].as_array().unwrap();
    assert!(
        recs.iter()
            .any(|r| r["name"] == "resend._domainkey.example.test" && r["type"] == "TXT")
    );
    assert!(recs.iter().any(|r| r["name"] == "send.example.test"
        && r["type"] == "MX"
        && r["value"] == "10 feedback-smtp.eu-west-1.amazonses.com"));
    assert!(
        recs.iter().all(|r| r["type"] != "CNAME"),
        "SES rows do not leak into Resend"
    );

    let (_, b) = c.get("/audit").await;
    assert!(
        b.to_string().contains("provider records for example.test"),
        "{b}"
    );
}

#[tokio::test]
async fn portal_acts_as_the_user() {
    let (mut c, _) = Client::new();
    let (s, _) = c
        .post(
            "/portal/login",
            json!({"email": "ana@example.test", "password": "nope"}),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, b) = c
        .post(
            "/portal/login",
            json!({"email": "ana@example.test", "password": "demo-password-1234"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["address"], "ana@example.test");
    assert_eq!(b["settings"]["imap"]["port"], 993);

    let (s, b) = c
        .post("/portal/app-passwords", json!({"description": "Phone"}))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["appPassword"]["id"].as_str().unwrap().to_string();
    let (_, b) = c.get("/portal/me").await;
    assert_eq!(b["appPasswords"].as_array().unwrap().len(), 1);
    let (s, _) = c
        .send("DELETE", &format!("/portal/app-passwords/{id}"), None, true)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Wrong current password is rejected; correct one changes it.
    let (s, _) = c
        .post(
            "/portal/password",
            json!({"current": "wrong", "next": "a-brand-new-password"}),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = c
        .post(
            "/portal/password",
            json!({"current": "demo-password-1234", "next": "a-brand-new-password"}),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // The portal session does not grant console access.
    let (s, _) = c.get("/overview").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn certificate_status_and_try_now() {
    let (mut c, app) = Client::with(App::for_tests_isolated());
    let (s, _) = c.get("/certificate").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    c.sign_in(&app).await;
    let (s, b) = c.get("/certificate").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "missing");
    assert_eq!(b["authority"], "Let's Encrypt");
    assert_eq!(b["inProgress"], false);
    // A self-signed certificate is not reported as healthy.
    let (cert, key) = crate::edge::tls::self_signed(&["mail.example.test".into()]).unwrap();
    crate::edge::tls::write_cert_files(&app.paths, &cert, &key).unwrap();
    let (_, b) = c.get("/certificate").await;
    assert_eq!(b["state"], "self_signed");
    let (_, b) = c.get("/overview").await;
    let tls = b["signals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "tls")
        .unwrap()
        .clone();
    assert_ne!(tls["status"], "pass", "{tls}");
    // Development mode never talks to a CA.
    let (s, b) = c.post("/certificate/renew", json!({})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
}

#[tokio::test]
async fn onboarding_creates_the_first_mailbox_once() {
    let (mut c, app) = Client::with(App::for_tests_isolated());
    c.sign_in(&app).await;
    // Complete installs send people to Mailboxes instead.
    let (s, _) = c
        .post("/install/first-mailbox", json!({"local": "hello"}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    let mut cfg = app.config().unwrap();
    cfg.install_complete = false;
    app.save_config(&cfg).unwrap();
    let engine = app.engine().unwrap();
    for a in engine.list_accounts().await.unwrap() {
        if !a.is_admin {
            engine.delete_account(&a.id).await.unwrap();
        }
    }
    let (s, b) = c
        .post("/install/first-mailbox", json!({"local": "Bad Name"}))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(b["error"]["field"], "local");

    let (s, b) = c
        .post(
            "/install/first-mailbox",
            json!({"local": "Hello", "displayName": "Front desk"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["mailbox"]["address"], "hello@example.test");
    assert!(b["password"].as_str().unwrap().len() >= 16);
    let aliases: Vec<String> = serde_json::from_value(b["mailbox"]["aliases"].clone()).unwrap();
    assert!(
        aliases.contains(&"postmaster@example.test".to_string()),
        "{aliases:?}"
    );
    assert!(
        aliases.contains(&"abuse@example.test".to_string()),
        "{aliases:?}"
    );

    let (s, b) = c
        .post("/install/first-mailbox", json!({"local": "other"}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(
        b["error"]["message"]
            .as_str()
            .unwrap()
            .contains("hello@example.test")
    );
}
