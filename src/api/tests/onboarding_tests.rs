//! The browser onboarding through the API, against the in-memory engine as
//! the terminal leaves it (one placeholder domain, setup not complete).

use super::Client;
use crate::app::App;
use crate::installer::state::{StepStatus, TERMINAL_STEPS};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;

/// The terminal finished with `panel` as the console's address.
fn after_terminal(panel: &str) -> (Client, Arc<App>) {
    let app = App::for_tests_fresh(panel);
    app.db.init_steps().unwrap();
    for id in TERMINAL_STEPS {
        app.db.set_step(id, StepStatus::Done, None).unwrap();
    }
    Client::with(app)
}

fn step<'a>(state: &'a Value, id: &str) -> &'a Value {
    state["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no step {id} in {state}"))
}

async fn put(c: &mut Client, path: &str, body: Value) -> (StatusCode, Value) {
    c.send("PUT", path, Some(body), true).await
}

#[tokio::test]
async fn the_wizard_reads_one_state_and_every_answer_is_applied_for_real() {
    let (mut c, app) = after_terminal("panel.example.com");
    c.sign_in(&app).await;

    let (s, b) = c.get("/onboarding").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["complete"], false);
    assert_eq!(b["terminalDone"], true);
    assert_eq!(b["panel"]["host"], "panel.example.com");
    assert_eq!(b["panel"]["kind"], "domain");
    assert_eq!(b["panel"]["url"], "https://panel.example.com/console");
    // Suggestions follow the console's name; the owner's own address is the
    // natural alert address.
    assert_eq!(b["suggested"]["domain"], "example.com");
    assert_eq!(b["suggested"]["hostname"], "mail.example.com");
    assert_eq!(b["suggested"]["adminEmail"], "owner@elsewhere.example");
    assert_eq!(b["identity"], Value::Null);
    assert_eq!(b["sending"]["confirmed"], false);
    assert_eq!(b["ptrSkipped"], false);
    let ids: Vec<&str> = b["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids[6], "inputs", "{ids:?}");

    // The session tells the console where it is opened.
    let (_, sess) = c.get("/session").await;
    assert_eq!(sess["server"]["consoleHost"], "panel.example.com");

    // Bad answers name their field.
    let (s, e) = put(
        &mut c,
        "/onboarding/identity",
        json!({"domain": "example.com", "hostname": "mail.example.com", "adminEmail": "me@example.com"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(e["error"]["field"], "adminEmail");
    let (s, e) = put(
        &mut c,
        "/onboarding/identity",
        json!({"domain": "not a domain", "hostname": "mail.example.com", "adminEmail": "me@gmail.com"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(e["error"]["field"], "domain");

    // Reverse DNS "skip for now" is remembered (and can be undone).
    let (s, b) = put(&mut c, "/onboarding/ptr", json!({"skip": true})).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["ptrSkipped"], true);

    // The domain and hostname are applied to the engine at once (the console's
    // parent domain was the engine's placeholder, so only the hostname changes).
    let (s, b) = put(
        &mut c,
        "/onboarding/identity",
        json!({"domain": "Example.com", "hostname": "mail.example.com", "adminEmail": "me@gmail.com"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(step(&b, "inputs")["status"], "done");
    assert_eq!(b["identity"]["domain"], "example.com");
    assert_eq!(b["mailHostname"], "mail.example.com");
    assert_eq!(b["primaryDomain"], "example.com");
    assert_eq!(b["primaryDomainId"], "d1");
    let engine = app.engine().unwrap();
    let names: Vec<String> = engine
        .list_domains()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(names, vec!["example.com".to_string()]);
    assert_eq!(engine.hostname().await.unwrap(), "mail.example.com");

    // Sending is answered through the usual form; its secret stays write-only.
    let (s, b) = c
        .send(
            "PUT",
            "/sending",
            Some(json!({"mode": "relay", "relay": {"provider": "postmark", "host": "smtp.postmarkapp.com", "port": 587, "username": "token", "password": "relay-secret"}})),
            true,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(!b.to_string().contains("relay-secret"));
    let (_, b) = c.get("/onboarding").await;
    assert_eq!(b["sending"]["confirmed"], true);
    assert_eq!(b["sending"]["mode"], "relay");

    // The first mailbox with an invite link instead of a password.
    let (s, b) = c
        .post(
            "/install/first-mailbox",
            json!({"local": "hello", "invite": true}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["mailbox"]["address"], "hello@example.com");
    assert!(b.get("password").is_none(), "{b}");
    assert!(
        b["invite"]["url"]
            .as_str()
            .unwrap()
            .contains("/account/welcome#invite="),
        "{b}"
    );

    // Everything above was audited, and the chain verifies.
    let (_, a) = c.get("/audit").await;
    assert_eq!(a["chainOk"], true);
    let actions: Vec<&str> = a["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for want in [
        "onboarding.identity",
        "engine.hostname",
        "onboarding.ptr_skipped",
        "mailbox.invite",
    ] {
        assert!(actions.contains(&want), "{want} missing from {actions:?}");
    }

    // Once setup is complete, the wizard's answers are changed elsewhere.
    let mut cfg = app.config().unwrap();
    cfg.install_complete = true;
    app.save_config(&cfg).unwrap();
    let (s, _) = put(
        &mut c,
        "/onboarding/identity",
        json!({"domain": "example.net", "hostname": "mail.example.net", "adminEmail": "me@gmail.com"}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn changing_the_names_checks_their_gates_again() {
    let (mut c, app) = after_terminal("203-0-113-10.sslip.io");
    c.sign_in(&app).await;
    let (_, b) = c.get("/onboarding").await;
    assert_eq!(b["panel"]["kind"], "sslip");
    assert_eq!(b["suggested"]["domain"], Value::Null);
    // With no domain to suggest, the server's free name is offered instead.
    assert!(
        b["server"]["freeName"]
            .as_str()
            .is_some_and(|n| n.ends_with(".sslip.io")),
        "{b}"
    );

    let (s, _) = put(
        &mut c,
        "/onboarding/identity",
        json!({"domain": "example.com", "hostname": "mail.example.com", "adminEmail": "me@gmail.com"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    for id in ["host_record", "certificate", "dns"] {
        app.db.set_step(id, StepStatus::Done, None).unwrap();
    }
    let (s, b) = put(
        &mut c,
        "/onboarding/identity",
        json!({"domain": "example.net", "hostname": "mx.example.net", "adminEmail": "me@gmail.com"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    for id in ["host_record", "certificate", "dns"] {
        assert_ne!(step(&b, id)["status"], "done", "{id} must be checked again");
    }
    // The engine's administrator follows the renamed domain.
    assert_eq!(
        app.db.get_secret("engine.admin").unwrap().unwrap().as_str(),
        "admin@example.net:engine-secret"
    );
    assert_eq!(
        app.config().unwrap().console_host(),
        "203-0-113-10.sslip.io"
    );
}

#[tokio::test]
async fn the_wizard_needs_an_admin_and_the_terminal_first() {
    let app = App::for_tests_fresh("203.0.113.10");
    app.db.init_steps().unwrap();
    let (mut c, app) = Client::with(app);
    let (s, _) = c.get("/onboarding").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    c.sign_in(&app).await;
    let (s, b) = c.get("/onboarding").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["terminalDone"], false);
    assert_eq!(b["panel"]["kind"], "ip");
    let (s, e) = put(
        &mut c,
        "/onboarding/identity",
        json!({"domain": "example.com", "hostname": "mail.example.com", "adminEmail": "me@gmail.com"}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(e.to_string().contains("sudo yunori-mail install"), "{e}");
}
