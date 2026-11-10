//! DNS automation through the API: the mock engine's record set, published
//! to a fake Cloudflare on a loopback port.

use super::Client;
use crate::app::App;
use crate::dns::provider::CLOUDFLARE_BASE;
use crate::dns::provider::fake::FakeCloudflare;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;

const TOKEN: &str = "cf-test-token-0123456789abcdefghijklmnop";

async fn setup() -> (Client, Arc<App>, FakeCloudflare, String) {
    let cf = FakeCloudflare::start(TOKEN, &[("zone1", "example.test")]).await;
    CLOUDFLARE_BASE.with(|b| *b.borrow_mut() = Some(cf.base.clone()));
    let (mut c, app) = Client::new();
    c.sign_in(&app).await;
    let (_, domains) = c.get("/domains").await;
    let id = domains
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "example.test")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    (c, app, cf, id)
}

fn audit_text(app: &App) -> String {
    app.db
        .audit_list(200)
        .unwrap()
        .iter()
        .map(|e| format!("{} {} {:?} {:?}", e.actor, e.action, e.target, e.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn connect(c: &mut Client, id: &str) -> Value {
    let (s, b) = c
        .send(
            "PUT",
            &format!("/domains/{id}/dns/provider"),
            Some(json!({"provider": "cloudflare", "token": TOKEN})),
            true,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b
}

#[tokio::test]
async fn connect_checks_the_token_and_never_returns_it() {
    let (mut c, app, _cf, id) = setup().await;
    let path = format!("/domains/{id}/dns/provider");

    let (s, b) = c.get(&path).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["connected"], Value::Null);
    assert_eq!(b["providers"][0]["id"], "cloudflare");

    // A token Cloudflare refuses: plain words, on the token field, nothing stored.
    let (s, b) = c
        .send(
            "PUT",
            &path,
            Some(
                json!({"provider": "cloudflare", "token": "wrong-token-0123456789abcdefghijklmn"}),
            ),
            true,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["error"]["code"], "provider_auth");
    assert_eq!(b["error"]["field"], "token");
    assert!(
        !app.db
            .has_secret("dns_provider.example.test.token")
            .unwrap()
    );

    // First connect needs a token.
    let (s, b) = c
        .send("PUT", &path, Some(json!({"provider": "cloudflare"})), true)
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(b["error"]["field"], "token");

    let b = connect(&mut c, &id).await;
    assert_eq!(b["connected"]["zoneName"], "example.test");
    assert_eq!(b["connected"]["hasSecret"], true);
    assert_eq!(b["connected"]["label"], "Cloudflare");
    // Write-only: the token is in no response, no kv row, no audit entry.
    let (_, got) = c.get(&path).await;
    for v in [&b, &got] {
        assert!(!v.to_string().contains(TOKEN));
    }
    assert!(
        !app.db
            .get_kv("dns_provider:example.test")
            .unwrap()
            .unwrap()
            .contains(TOKEN)
    );
    let audit = audit_text(&app);
    assert!(audit.contains("dns.provider.connect"));
    assert!(!audit.contains(TOKEN));
    assert_eq!(
        app.db
            .get_secret("dns_provider.example.test.token")
            .unwrap()
            .unwrap()
            .as_str(),
        TOKEN
    );

    // Blank token = re-check the stored one.
    let (s, b) = c
        .send(
            "PUT",
            &path,
            Some(json!({"provider": "cloudflare", "token": ""})),
            true,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // Disconnect forgets the token; publishing then has nothing to use.
    let (s, b) = c.send("DELETE", &path, None, true).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["connected"], Value::Null);
    assert!(
        !app.db
            .has_secret("dns_provider.example.test.token")
            .unwrap()
    );
    let (s, _) = c.get(&format!("/domains/{id}/dns/plan")).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(audit_text(&app).contains("dns.provider.disconnect"));
}

#[tokio::test]
async fn preview_then_publish_writes_only_what_is_missing_or_wrong() {
    let (mut c, app, cf, id) = setup().await;
    // What the zone already holds: mail going elsewhere, a site-verification TXT,
    // a correct SRV, and an unrelated web record.
    cf.seed("zone1", json!({"type": "MX", "name": "example.test", "content": "mx.old-host.example", "priority": 10}));
    cf.seed("zone1", json!({"type": "TXT", "name": "example.test", "content": "\"google-site-verification=abc\""}));
    cf.seed("zone1", json!({"type": "SRV", "name": "_imaps._tcp.example.test", "data": {"priority": 0, "weight": 1, "port": 993, "target": "mail.example.test"}}));
    cf.seed(
        "zone1",
        json!({"type": "A", "name": "www.example.test", "content": "192.0.2.80", "proxied": true}),
    );
    connect(&mut c, &id).await;

    let (s, plan) = c.get(&format!("/domains/{id}/dns/plan")).await;
    assert_eq!(s, StatusCode::OK, "{plan}");
    let item = |p: &Value, id: &str| {
        p["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(item(&plan, "a")["action"], "create");
    assert_eq!(item(&plan, "mx")["action"], "update");
    assert_eq!(
        item(&plan, "mx")["current"],
        json!(["10 mx.old-host.example"])
    );
    assert_eq!(item(&plan, "spf")["action"], "create");
    assert_eq!(item(&plan, "srv:_imaps._tcp")["action"], "unchanged");
    assert_eq!(item(&plan, "dmarc")["action"], "create");
    assert!(
        item(&plan, "dmarc")["value"]
            .as_str()
            .unwrap()
            .contains("p=none")
    );
    // The internal provider record id is not part of the contract.
    assert!(item(&plan, "mx").get("target").is_none());
    let creates = plan["counts"]["create"].as_u64().unwrap();
    assert!(creates >= 8, "{plan}");
    // Previewing changed nothing.
    assert_eq!(cf.records("zone1").len(), 4);

    let (s, out) = c
        .post(
            &format!("/domains/{id}/dns/publish"),
            json!({"planId": plan["planId"]}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["failed"], 0, "{out}");
    assert_eq!(out["created"].as_u64().unwrap(), creates);
    assert_eq!(out["updated"], 1);
    assert!(
        out["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["ok"] == true)
    );
    // The public-resolver check ran right after (the gate stays public DNS).
    assert!(
        out["dns"]["records"]
            .as_array()
            .is_some_and(|r| !r.is_empty())
    );

    let recs = cf.records("zone1");
    let has = |t: &str, name: &str, f: &dyn Fn(&Value) -> bool| {
        recs.iter()
            .any(|r| r["type"] == t && r["name"] == name && f(r))
    };
    assert!(has("MX", "example.test", &|r| r["content"]
        == "mail.example.test"));
    assert!(has("A", "mail.example.test", &|r| r["content"]
        == "203.0.113.10"
        && r["proxied"] == false));
    assert!(has("TXT", "example.test", &|r| r["content"]
        == "\"google-site-verification=abc\""));
    assert!(
        has("A", "www.example.test", &|r| r["proxied"] == true),
        "unrelated records untouched"
    );
    assert!(has(
        "TXT",
        "v1-rsa-20261105._domainkey.example.test",
        &|_| true
    ));
    assert!(cf.requests().iter().all(|r| !r.starts_with("DELETE")));
    assert!(audit_text(&app).contains("dns.publish"));

    // Publishing again has nothing left to do.
    let (_, again) = c.get(&format!("/domains/{id}/dns/plan")).await;
    assert_eq!(again["counts"]["create"], 0, "{again}");
    assert_eq!(again["counts"]["update"], 0, "{again}");
    let (_, st) = c.get(&format!("/domains/{id}/dns/provider")).await;
    assert_eq!(st["connected"]["lastPublish"]["updated"], 1);
}

#[tokio::test]
async fn publish_refuses_a_stale_preview_and_honours_a_selection() {
    let (mut c, _app, cf, id) = setup().await;
    connect(&mut c, &id).await;
    let (_, plan) = c.get(&format!("/domains/{id}/dns/plan")).await;
    // Someone changes the zone between preview and publish.
    cf.seed("zone1", json!({"type": "MX", "name": "example.test", "content": "elsewhere.example", "priority": 1}));
    let (s, b) = c
        .post(
            &format!("/domains/{id}/dns/publish"),
            json!({"planId": plan["planId"]}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["error"]["code"], "plan_changed");
    assert_eq!(cf.records("zone1").len(), 1);

    // Dry run: the plan comes back, nothing is written.
    let (s, b) = c
        .post(
            &format!("/domains/{id}/dns/publish"),
            json!({"dryRun": true}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["dryRun"], true);
    assert_eq!(cf.records("zone1").len(), 1);

    // Only the selected record.
    let (s, b) = c
        .post(
            &format!("/domains/{id}/dns/publish"),
            json!({"only": ["spf"]}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["results"].as_array().unwrap().len(), 1);
    assert_eq!(cf.records("zone1").len(), 2);
}

#[tokio::test]
async fn a_read_only_token_fails_each_write_in_plain_words() {
    let (mut c, _app, cf, id) = setup().await;
    connect(&mut c, &id).await;
    cf.set_read_only(true);
    let (s, b) = c
        .post(&format!("/domains/{id}/dns/publish"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["failed"].as_u64().unwrap() > 0);
    assert_eq!(b["created"], 0);
    let first = &b["results"][0];
    assert_eq!(first["ok"], false);
    assert!(
        first["error"].as_str().unwrap().contains("DNS → Edit"),
        "{first}"
    );
}

#[tokio::test]
async fn deleting_a_domain_forgets_its_provider() {
    let (mut c, app, cf, _) = setup().await;
    let (s, d) = c.post("/domains", json!({"name": "second.test"})).await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    let id = d["id"].as_str().unwrap().to_string();
    drop(cf);
    let cf2 = FakeCloudflare::start(TOKEN, &[("zone2", "second.test")]).await;
    CLOUDFLARE_BASE.with(|b| *b.borrow_mut() = Some(cf2.base.clone()));
    connect(&mut c, &id).await;
    assert!(app.db.has_secret("dns_provider.second.test.token").unwrap());
    let (s, _) = c
        .send("DELETE", &format!("/domains/{id}"), None, true)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(!app.db.has_secret("dns_provider.second.test.token").unwrap());
    assert!(app.db.get_kv("dns_provider:second.test").unwrap().is_none());
}
