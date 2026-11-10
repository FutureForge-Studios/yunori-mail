//! API tests for the queue manager and the blocked/allowed addresses, against
//! the in-memory engine (same style as `tests.rs`).

use crate::app::App;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Client {
    router: axum::Router,
    cookies: Vec<String>,
}

impl Client {
    async fn signed_in() -> (Self, Arc<App>) {
        let app = App::for_tests_isolated();
        let mut c = Client {
            router: super::router(app.clone()),
            cookies: vec![],
        };
        let token = app.db.issue_setup_token().unwrap();
        let (s, b) = c
            .send(
                "POST",
                "/setup",
                Some(json!({"token": token, "email": "owner@elsewhere.example", "password": "a-long-passphrase-1"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let code =
            crate::auth::totp_for(b["totpSecret"].as_str().unwrap(), "owner@elsewhere.example")
                .unwrap()
                .generate_current()
                .unwrap();
        let (s, b) = c
            .send("POST", "/setup/totp", Some(json!({"code": code})))
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        (c, app)
    }

    async fn send(&mut self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if !self.cookies.is_empty() {
            req = req.header(header::COOKIE, self.cookies.join("; "));
        }
        if method != "GET" {
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
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn audit_actions(app: &App) -> Vec<(String, Option<String>)> {
    app.db
        .audit_list(200)
        .unwrap()
        .into_iter()
        .map(|e| (e.action, e.target))
        .collect()
}

#[tokio::test]
async fn queue_and_access_need_a_session() {
    let app = App::for_tests_isolated();
    let mut c = Client {
        router: super::router(app),
        cookies: vec![],
    };
    for (m, p) in [
        ("GET", "/queue"),
        ("POST", "/queue/pause"),
        ("DELETE", "/queue/q1"),
        ("GET", "/security/access"),
        ("DELETE", "/security/blocked/b1"),
    ] {
        let (s, _) = c.send(m, p, None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{m} {p}");
    }
}

#[tokio::test]
async fn queue_lists_explains_retries_cancels_and_pauses() {
    let (mut c, app) = Client::signed_in().await;
    let (s, q) = c.send("GET", "/queue", None).await;
    assert_eq!(s, StatusCode::OK, "{q}");
    assert_eq!(q["paused"], false);
    let m = &q["messages"][0];
    assert_eq!(m["id"], "q1");
    assert_eq!(m["sender"], "ana@example.test");
    let r = &m["recipients"][0];
    assert_eq!(r["status"], "retrying");
    assert_eq!(r["attempts"], 2);
    assert_eq!(r["kind"], "other");
    assert!(
        r["problem"].as_str().unwrap().contains("try again later"),
        "{r}"
    );
    assert!(r["response"].as_str().unwrap().starts_with("451 4.7.650"));
    assert_eq!(m["expiresAt"], "2026-11-10T09:30:00Z");

    // Retry one, then all.
    let (s, b) = c
        .send("POST", "/queue/retry", Some(json!({"id": "q1"})))
        .await;
    assert_eq!((s, b["retried"].as_u64()), (StatusCode::OK, Some(1)));
    let (_, q) = c.send("GET", "/queue", None).await;
    assert_eq!(q["messages"][0]["recipients"][0]["attempts"], 3);
    let (s, b) = c.send("POST", "/queue/retry", Some(json!({}))).await;
    assert_eq!((s, b["retried"].as_u64()), (StatusCode::OK, Some(1)));
    let (s, b) = c
        .send("POST", "/queue/retry", Some(json!({"id": "nope"})))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");
    assert!(b["error"]["message"].as_str().unwrap().contains("message"));

    // Pause and resume: recorded here, applied on the engine.
    let (s, q) = c.send("POST", "/queue/pause", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(q["paused"], true);
    assert!(q["pausedAt"].is_string());
    assert!(app.mock().unwrap().queue_paused());
    let (_, q) = c.send("POST", "/queue/resume", None).await;
    assert_eq!(q["paused"], false);
    assert!(!app.mock().unwrap().queue_paused());

    // Cancel.
    let (s, _) = c.send("DELETE", "/queue/q1", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, q) = c.send("GET", "/queue", None).await;
    assert_eq!(q["messages"].as_array().unwrap().len(), 0);
    let (s, _) = c.send("DELETE", "/queue/q1", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = c.send("DELETE", "/queue/..%2Fx", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let actions = audit_actions(&app);
    for want in ["queue.retry", "queue.pause", "queue.resume", "queue.cancel"] {
        assert!(actions.iter().any(|(a, _)| a == want), "{want} audited");
    }
    assert!(
        actions
            .iter()
            .any(|(a, t)| a == "queue.retry" && t.as_deref() == Some("all"))
    );
    assert!(app.db.audit_verify().unwrap());
}

#[tokio::test]
async fn a_pause_survives_an_engine_restart() {
    let (mut c, app) = Client::signed_in().await;
    c.send("POST", "/queue/pause", None).await;
    // The engine forgets (Stalwart keeps the pause in memory only) …
    app.engine().unwrap().queue_set_paused(false).await.unwrap();
    // … and the minute job puts it back.
    crate::mailqueue::reassert_pause(&app).await;
    assert!(app.mock().unwrap().queue_paused());
}

#[tokio::test]
async fn blocked_addresses_and_the_allowlist() {
    let (mut c, app) = Client::signed_in().await;
    let (s, v) = c.send("GET", "/security/access", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let blocked = v["blocked"].as_array().unwrap();
    assert_eq!(blocked.len(), 2);
    assert_eq!(blocked[0]["address"], "198.51.100.23", "newest first");
    assert_eq!(blocked[0]["reasonText"], "Too many failed sign-ins");
    assert!(blocked[1]["expiresAt"].is_null(), "permanent block");
    assert_eq!(v["policy"]["enabled"], true);

    // Unblock by id.
    let (s, _) = c.send("DELETE", "/security/blocked/b1", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = c.send("DELETE", "/security/blocked/b1", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Allowlist: validated, canonical, no duplicates.
    for bad in ["nope", "10.0.0.0/33", "0.0.0.0/0", "10.0.0.0/4"] {
        let (s, b) = c
            .send("POST", "/security/allowed", Some(json!({"address": bad})))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {b}");
        assert_eq!(b["error"]["field"], "address");
    }
    let (s, a) = c
        .send(
            "POST",
            "/security/allowed",
            Some(json!({"address": "203.0.113.77/24", "note": "office\nVPN"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    assert_eq!(a["address"], "203.0.113.0/24");
    assert_eq!(a["reason"], "officeVPN");
    let (s, _) = c
        .send(
            "POST",
            "/security/allowed",
            Some(json!({"address": "203.0.113.0/24"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Allowing a blocked address also lifts its block.
    let (s, _) = c
        .send(
            "POST",
            "/security/allowed",
            Some(json!({"address": "203.0.113.200"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, v) = c.send("GET", "/security/access", None).await;
    assert_eq!(v["blocked"].as_array().unwrap().len(), 0);
    assert_eq!(v["allowed"].as_array().unwrap().len(), 2);

    let id = a["id"].as_str().unwrap();
    let (s, _) = c
        .send("DELETE", &format!("/security/allowed/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, v) = c.send("GET", "/security/access", None).await;
    assert_eq!(v["allowed"].as_array().unwrap().len(), 1);

    let actions = audit_actions(&app);
    for want in [
        "security.unblock",
        "security.allow",
        "security.allow_remove",
    ] {
        assert!(actions.iter().any(|(a, _)| a == want), "{want} audited");
    }
}
