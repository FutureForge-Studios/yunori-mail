//! Alerts API against the in-memory engine: write-only webhook secrets,
//! the test send, and "Check now" firing once (not on every evaluation).

use super::Client;
use crate::app::App;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

async fn listener() -> (String, Arc<Mutex<Vec<Value>>>) {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let s = seen.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(move |axum::Json(v): axum::Json<Value>| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push(v);
                StatusCode::OK
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}/hook"), seen)
}

async fn put(c: &mut Client, body: Value) -> (StatusCode, Value) {
    c.send("PUT", "/alerts/settings", Some(body), true).await
}

#[tokio::test]
async fn alert_settings_keep_webhook_secrets_write_only() {
    let (mut c, app) = Client::with(App::for_tests_isolated());
    let (s, _) = c.get("/alerts").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    c.sign_in(&app).await;

    let (s, b) = c.get("/alerts").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["settings"]["emailEnabled"], true);
    assert_eq!(b["settings"]["recipient"], "owner@elsewhere.example");
    assert_eq!(b["settings"]["recipientIsDefault"], true);
    assert_eq!(b["settings"]["sender"], "yunori-alerts@example.test");
    assert_eq!(b["settings"]["webhook"]["hasUrl"], false);
    assert_eq!(b["active"], json!([]));

    let url = "https://ntfy.example/very-secret-topic";
    let (s, b) = put(
        &mut c,
        json!({"emailEnabled": true, "recipient": "Ops@Elsewhere.example",
               "webhook": {"enabled": true, "kind": "ntfy", "url": url, "token": "tk_live_123"}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let text = b.to_string();
    assert!(
        !text.contains("very-secret-topic") && !text.contains("tk_live_123"),
        "{text}"
    );
    assert_eq!(b["settings"]["recipient"], "ops@elsewhere.example");
    assert_eq!(b["settings"]["webhook"]["host"], "ntfy.example");
    assert_eq!(b["settings"]["webhook"]["kind"], "ntfy");
    assert_eq!(b["settings"]["webhook"]["hasUrl"], true);
    assert_eq!(b["settings"]["webhook"]["hasToken"], true);
    assert_eq!(
        app.db
            .get_secret(crate::alerts::channels::SECRET_WEBHOOK_URL)
            .unwrap()
            .unwrap()
            .as_str(),
        url
    );

    // Saving again without URL or token keeps both.
    let (_, b) = put(
        &mut c,
        json!({"emailEnabled": false, "recipient": null, "webhook": {"enabled": true, "kind": "ntfy"}}),
    )
    .await;
    assert_eq!(b["settings"]["webhook"]["hasToken"], true);
    assert_eq!(b["settings"]["emailEnabled"], false);
    assert_eq!(b["settings"]["recipientIsDefault"], true);

    // A new URL never inherits the old token.
    let (_, b) = put(
        &mut c,
        json!({"emailEnabled": true, "webhook": {"enabled": true, "kind": "json", "url": "https://hooks.example/other"}}),
    )
    .await;
    assert_eq!(b["settings"]["webhook"]["host"], "hooks.example");
    assert_eq!(b["settings"]["webhook"]["hasToken"], false);

    // Bad input names its field.
    let (s, b) = put(
        &mut c,
        json!({"emailEnabled": true, "webhook": {"enabled": true, "url": "ftp://x/y"}}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(b["error"]["field"], "webhookUrl");
    let (s, b) = put(
        &mut c,
        json!({"emailEnabled": true, "recipient": "me@example.test", "webhook": {"enabled": false}}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "an address on this server is refused"
    );
    assert_eq!(b["error"]["field"], "recipient");
    let (_, b) = put(
        &mut c,
        json!({"emailEnabled": true, "webhook": {"enabled": false, "url": ""}}),
    )
    .await;
    assert_eq!(b["settings"]["webhook"]["hasUrl"], false);
    let (s, b) = put(
        &mut c,
        json!({"emailEnabled": true, "webhook": {"enabled": true}}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(b["error"]["field"], "webhookUrl");

    // Every change is in the ledger, and no secret is.
    let entries = app.db.audit_list(50).unwrap();
    let changes = entries
        .iter()
        .filter(|e| e.action == "alerts.settings")
        .count();
    assert_eq!(changes, 4);
    for e in &entries {
        let d = e.detail.clone().unwrap_or_default();
        assert!(
            !d.contains("tk_live_123") && !d.contains("very-secret-topic"),
            "{d}"
        );
    }
}

#[tokio::test]
async fn a_test_alert_goes_through_the_webhook_and_into_history() {
    let (mut c, app) = Client::with(App::for_tests_isolated());
    c.sign_in(&app).await;
    let (s, b) = put(
        &mut c,
        json!({"emailEnabled": false, "webhook": {"enabled": false}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = c.post("/alerts/test", json!({})).await;
    assert_eq!(s, StatusCode::CONFLICT, "nothing is on: {b}");

    let (hook, seen) = listener().await;
    put(
        &mut c,
        json!({"emailEnabled": false, "webhook": {"enabled": true, "kind": "json", "url": hook, "token": "abc"}}),
    )
    .await;
    let (s, b) = c.post("/alerts/test", json!({})).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["deliveries"][0]["channel"], "webhook");
    assert_eq!(b["deliveries"][0]["ok"], true, "{b}");
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["event"], "test");
    assert_eq!(got[0]["server"], "mail.example.test");

    let (_, b) = c.get("/alerts").await;
    assert_eq!(b["history"][0]["kind"], "test");
    assert_eq!(b["history"][0]["deliveries"][0]["ok"], true);
    assert!(
        app.db
            .audit_list(20)
            .unwrap()
            .iter()
            .any(|e| e.action == "alert.test" && e.actor == "owner@elsewhere.example")
    );

    // In development mode email is never sent, and says so.
    put(
        &mut c,
        json!({"emailEnabled": true, "webhook": {"enabled": false}}),
    )
    .await;
    let (_, b) = c.post("/alerts/test", json!({})).await;
    assert_eq!(b["deliveries"][0]["channel"], "email");
    assert_eq!(b["deliveries"][0]["ok"], false);
    assert!(
        b["deliveries"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("development mode")
    );
}

#[tokio::test]
async fn check_now_fires_once_and_shows_on_the_overview() {
    let (mut c, app) = Client::with(App::for_tests_isolated());
    c.sign_in(&app).await;
    let (hook, seen) = listener().await;
    put(
        &mut c,
        json!({"emailEnabled": false, "webhook": {"enabled": true, "url": hook}}),
    )
    .await;
    // The mock engine's queue holds a message deferred since 2026-11-05.
    let (s, b) = c.post("/alerts/check", json!({})).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let active = b["active"].as_array().unwrap();
    let queue = active
        .iter()
        .find(|a| a["id"] == "queue")
        .unwrap_or_else(|| panic!("queue alert: {b}"));
    assert_eq!(queue["severity"], "warning");
    assert!(
        queue["summary"]
            .as_str()
            .unwrap()
            .contains("waiting more than 6 hours")
    );
    assert!(!queue["remedy"].as_str().unwrap().is_empty());
    assert!(b["evaluatedAt"].is_string());

    let posts = |seen: &Arc<Mutex<Vec<Value>>>| {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|v| v["alert"]["id"] == "queue")
            .count()
    };
    assert_eq!(posts(&seen), 1);
    // Still firing: no second message.
    let (_, b) = c.post("/alerts/check", json!({})).await;
    assert!(
        b["active"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "queue")
    );
    assert_eq!(posts(&seen), 1);
    let firing = b["history"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|h| h["alertId"] == "queue")
        .count();
    assert_eq!(firing, 1);

    let (_, o) = c.get("/overview").await;
    assert!(
        o["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "queue" && a["title"] == "Outgoing mail is stuck"),
        "{o}"
    );
    let entries = app.db.audit_list(50).unwrap();
    assert!(entries.iter().any(|e| e.action == "alert.firing"
        && e.actor == "system"
        && e.target.as_deref() == Some("queue")));
    assert!(entries.iter().any(|e| e.action == "alerts.check"));
}

#[tokio::test]
async fn nothing_is_alerted_before_setup_is_complete() {
    let app = App::for_tests_isolated();
    let mut cfg = app.config().unwrap();
    cfg.install_complete = false;
    app.save_config(&cfg).unwrap();
    assert!(
        crate::alerts::evaluate(&app, std::time::Duration::ZERO)
            .await
            .is_empty()
    );
    assert!(crate::alerts::current(&app).is_empty());
}
