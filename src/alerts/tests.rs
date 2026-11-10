//! The evaluation and de-duplication state machine (time injected), the
//! wording of each condition, and the webhook/ntfy channels against a local
//! listener.

use super::channels::{self, WebhookKind};
use super::*;
use crate::engine::EQueuedRecipient;
use crate::model::DnsRecord;
use chrono::TimeZone;
use std::sync::{Arc, Mutex};

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
}
fn mins(m: i64) -> ChronoDuration {
    ChronoDuration::minutes(m)
}
fn cond(id: &str, sev: Severity) -> Condition {
    Condition::new(id, sev, "Title", "Summary.", "Remedy.")
}
fn kinds(n: &[Notice]) -> Vec<(String, Kind)> {
    n.iter().map(|x| (x.condition.id.clone(), x.kind)).collect()
}

#[test]
fn notifies_on_start_and_never_twice_within_a_day() {
    let mut active = BTreeMap::new();
    let n = step(&mut active, vec![cond("disk", Severity::Warning)], t0());
    assert_eq!(kinds(&n), vec![("disk".into(), Kind::Firing)]);
    // Every 10 minutes for most of a day: silence.
    let mut t = t0();
    for _ in 0..140 {
        t += mins(10);
        assert!(step(&mut active, vec![cond("disk", Severity::Warning)], t).is_empty());
    }
    // 24 hours after the first message: one reminder, then silence again.
    let t = t0() + ChronoDuration::hours(24);
    let n = step(&mut active, vec![cond("disk", Severity::Warning)], t);
    assert_eq!(kinds(&n), vec![("disk".into(), Kind::Reminder)]);
    assert_eq!(n[0].since, t0(), "the reminder keeps the original start");
    assert!(
        step(
            &mut active,
            vec![cond("disk", Severity::Warning)],
            t + mins(10)
        )
        .is_empty()
    );
}

#[test]
fn getting_worse_is_one_message_and_better_is_none() {
    let mut active = BTreeMap::new();
    step(&mut active, vec![cond("disk", Severity::Warning)], t0());
    let n = step(
        &mut active,
        vec![cond("disk", Severity::Critical)],
        t0() + mins(10),
    );
    assert_eq!(kinds(&n), vec![("disk".into(), Kind::Worse)]);
    assert!(
        step(
            &mut active,
            vec![cond("disk", Severity::Critical)],
            t0() + mins(20)
        )
        .is_empty()
    );
    // Back to a warning, then critical again within the day: already told.
    assert!(
        step(
            &mut active,
            vec![cond("disk", Severity::Warning)],
            t0() + mins(30)
        )
        .is_empty()
    );
    assert!(
        step(
            &mut active,
            vec![cond("disk", Severity::Critical)],
            t0() + mins(40)
        )
        .is_empty()
    );
}

#[test]
fn resolves_after_two_clear_evaluations_only() {
    let mut active = BTreeMap::new();
    step(&mut active, vec![cond("queue", Severity::Warning)], t0());
    // One clear evaluation is not enough (a blip)...
    assert!(step(&mut active, vec![], t0() + mins(10)).is_empty());
    assert_eq!(active["queue"].clear_ticks, 1);
    // ...and firing again in between resets the count without a new message.
    assert!(
        step(
            &mut active,
            vec![cond("queue", Severity::Warning)],
            t0() + mins(20)
        )
        .is_empty()
    );
    assert_eq!(active["queue"].clear_ticks, 0);
    assert!(step(&mut active, vec![], t0() + mins(30)).is_empty());
    let n = step(&mut active, vec![], t0() + mins(40));
    assert_eq!(kinds(&n), vec![("queue".into(), Kind::Resolved)]);
    assert_eq!(n[0].since, t0());
    assert!(active.is_empty());
    // Nothing more once resolved.
    assert!(step(&mut active, vec![], t0() + mins(50)).is_empty());
    // A new episode is a new message.
    let n = step(
        &mut active,
        vec![cond("queue", Severity::Warning)],
        t0() + mins(60),
    );
    assert_eq!(kinds(&n), vec![("queue".into(), Kind::Firing)]);
}

#[test]
fn independent_alerts_do_not_interfere() {
    let mut active = BTreeMap::new();
    let n = step(
        &mut active,
        vec![
            cond("disk", Severity::Warning),
            cond("engine", Severity::Critical),
        ],
        t0(),
    );
    assert_eq!(n.len(), 2);
    step(
        &mut active,
        vec![cond("disk", Severity::Warning)],
        t0() + mins(10),
    );
    let n = step(
        &mut active,
        vec![cond("disk", Severity::Warning)],
        t0() + mins(20),
    );
    assert_eq!(kinds(&n), vec![("engine".into(), Kind::Resolved)]);
    assert!(active.contains_key("disk"));
}

#[test]
fn active_state_round_trips_as_json() {
    let mut active = BTreeMap::new();
    step(&mut active, vec![cond("disk", Severity::Critical)], t0());
    let s = serde_json::to_string(&active).unwrap();
    assert!(s.contains("\"notifiedSeverity\":\"critical\""), "{s}");
    let back: BTreeMap<String, Active> = serde_json::from_str(&s).unwrap();
    assert_eq!(back["disk"].condition, active["disk"].condition);
    assert_eq!(back["disk"].since, t0());
}

// ── conditions ──────────────────────────────────────────────────────────

fn healthy() -> Inputs {
    Inputs {
        now: t0(),
        hostname: "mail.example.com".into(),
        relayed: false,
        engine_reachable: true,
        cert: Some(CertFacts {
            state: "valid".into(),
            days_left: Some(60),
            failures: 0,
            problem: None,
        }),
        queue: vec![],
        ip: Some("203.0.113.10".into()),
        blocklists: vec![],
        dns: Some(vec![]),
        disk: Some((100_000_000_000, 60_000_000_000)),
        last_backup: None,
    }
}

fn ids(i: &Inputs) -> Vec<String> {
    conditions(i, &mut BTreeSet::new())
        .into_iter()
        .map(|c| c.id)
        .collect()
}

fn one(i: &Inputs) -> Condition {
    let mut v = conditions(i, &mut BTreeSet::new());
    assert_eq!(v.len(), 1, "{v:?}");
    v.remove(0)
}

#[test]
fn a_healthy_server_has_no_conditions() {
    assert!(ids(&healthy()).is_empty());
}

#[test]
fn every_condition_has_a_summary_and_a_remedy() {
    let mut i = healthy();
    i.engine_reachable = false;
    i.cert = Some(CertFacts {
        state: "expired".into(),
        days_left: Some(-1),
        failures: 5,
        problem: None,
    });
    i.disk = Some((100, 2));
    i.last_backup = Some(t0() - ChronoDuration::days(9));
    i.blocklists = vec![
        Check::new("dnsbl:a", "Spamhaus ZEN blocklist", Status::Fail, "listed")
            .help("https://check.spamhaus.org/"),
    ];
    for c in conditions(&i, &mut BTreeSet::new()) {
        assert!(c.summary.ends_with('.'), "{c:?}");
        assert!(!c.remedy.is_empty(), "{c:?}");
        assert!(!c.title.is_empty());
    }
}

#[test]
fn engine_down_is_critical() {
    let mut i = healthy();
    i.engine_reachable = false;
    let c = one(&i);
    assert_eq!((c.id.as_str(), c.severity), ("engine", Severity::Critical));
    assert!(c.remedy.contains("systemctl status stalwart"));
}

#[test]
fn certificate_expiry_and_renewal() {
    let mut i = healthy();
    i.cert.as_mut().unwrap().days_left = Some(9);
    i.cert.as_mut().unwrap().state = "expiring".into();
    let c = one(&i);
    assert_eq!(
        (c.id.as_str(), c.severity),
        ("certificate.expiry", Severity::Warning)
    );
    assert!(c.summary.contains("expires in 9 days"), "{}", c.summary);

    i.cert.as_mut().unwrap().days_left = Some(2);
    assert_eq!(one(&i).severity, Severity::Critical);

    i.cert.as_mut().unwrap().state = "expired".into();
    i.cert.as_mut().unwrap().problem = Some((
        "Let's Encrypt could not reach mail.example.com on port 80.".into(),
        "Open port 80 in your provider's firewall.".into(),
    ));
    let c = one(&i);
    assert_eq!(c.severity, Severity::Critical);
    assert_eq!(c.remedy, "Open port 80 in your provider's firewall.");

    // A valid certificate whose renewal keeps failing.
    let mut i = healthy();
    {
        let cert = i.cert.as_mut().unwrap();
        cert.days_left = Some(25);
        cert.failures = 2;
        cert.problem = Some((
            "CAA forbids Let's Encrypt.".into(),
            "Fix the CAA record.".into(),
        ));
    }
    assert!(ids(&i).is_empty(), "two failures are not yet a pattern");
    i.cert.as_mut().unwrap().failures = 3;
    let c = one(&i);
    assert_eq!(c.id, "certificate.renewal");
    assert!(c.summary.contains("failed 3 times"), "{}", c.summary);
    assert_eq!(c.remedy, "Fix the CAA record.");

    // A self-signed certificate during normal operation is the renewal alert's job, not expiry.
    let mut i = healthy();
    i.cert = Some(CertFacts {
        state: "self_signed".into(),
        days_left: Some(1),
        failures: 0,
        problem: None,
    });
    assert!(ids(&i).is_empty());
}

fn queued(id: &str, age_hours: i64, state: RcptState, retries: u64) -> EQueued {
    EQueued {
        id: id.into(),
        created_at: Some((t0() - ChronoDuration::hours(age_hours)).to_rfc3339()),
        next_retry: None,
        recipients: vec![EQueuedRecipient {
            address: "someone@gmail.example".into(),
            state,
            error_type: Some("connectionError".into()),
            response: None,
            retry_count: retries,
        }],
    }
}

#[test]
fn only_old_deferred_mail_counts_as_stuck() {
    let mut i = healthy();
    i.queue = vec![
        queued("fresh", 1, RcptState::TemporaryFailure, 3),
        queued("new-scheduled", 8, RcptState::Scheduled, 0),
        queued("done", 9, RcptState::Completed, 1),
    ];
    assert!(ids(&i).is_empty());
    i.queue
        .push(queued("old", 7, RcptState::TemporaryFailure, 4));
    let c = one(&i);
    assert_eq!(c.id, "queue");
    assert!(
        c.summary
            .starts_with("1 outgoing message has been waiting more than 6 hours"),
        "{}",
        c.summary
    );
    assert!(
        c.remedy.contains("port 25"),
        "network remedy for direct sending: {}",
        c.remedy
    );
    i.queue.push(queued("old2", 30, RcptState::Scheduled, 2));
    assert!(one(&i).summary.starts_with("2 outgoing messages have"));
    i.relayed = true;
    assert!(one(&i).remedy.contains("relay host"), "relay-aware remedy");
}

#[test]
fn blocklists_are_critical_unless_relayed() {
    let mut i = healthy();
    i.blocklists = vec![
        Check::new(
            "dnsbl:zen",
            "Spamhaus ZEN blocklist",
            Status::Fail,
            "listed",
        )
        .help("https://check.spamhaus.org/"),
        Check::new("dnsbl:b", "Barracuda blocklist", Status::Pass, "ok"),
        Check::new("dnsbl:s", "SpamCop blocklist", Status::Fail, "listed")
            .help("https://www.spamcop.net/bl.shtml"),
        Check::new("dnsbl:x", "Other blocklist", Status::Unknown, "refused"),
    ];
    let c = one(&i);
    assert_eq!(
        (c.id.as_str(), c.severity),
        ("blocklist", Severity::Critical)
    );
    assert!(
        c.summary
            .starts_with("203.0.113.10 is on the Spamhaus ZEN and SpamCop blocklists"),
        "{}",
        c.summary
    );
    assert!(c.remedy.contains("https://check.spamhaus.org/"));
    i.relayed = true;
    let c = one(&i);
    assert_eq!(c.severity, Severity::Warning);
    assert!(c.summary.contains("relay"));
}

fn rec(id: &str, rtype: &str, name: &str, status: Status) -> DnsRecord {
    DnsRecord {
        id: id.into(),
        rtype: rtype.into(),
        name: name.into(),
        expected: String::new(),
        actual: vec![],
        status,
        required: true,
        purpose: String::new(),
        remedy: None,
    }
}

fn dns(domain: &str, records: Vec<DnsRecord>) -> DomainDns {
    DomainDns {
        domain: domain.into(),
        records,
        checked_at: None,
        zone_file: String::new(),
    }
}

#[test]
fn dns_drift_is_only_a_record_that_used_to_pass() {
    let mut passed = BTreeSet::new();
    let mut i = healthy();
    // Never published: the domain checklist's job, not an alert.
    i.dns = Some(vec![dns(
        "example.com",
        vec![
            rec("mx", "MX", "example.com", Status::Fail),
            rec("spf", "TXT", "example.com", Status::Pass),
            rec("dkim", "TXT", "s1._domainkey.example.com", Status::Pass),
        ],
    )]);
    assert!(conditions(&i, &mut passed).is_empty());
    assert!(passed.contains("example.com|spf") && passed.contains("example.com|dkim"));

    // SPF and DKIM break later: drift (warning, not MX).
    i.dns = Some(vec![dns(
        "example.com",
        vec![
            rec("mx", "MX", "example.com", Status::Fail),
            rec("spf", "TXT", "example.com", Status::Fail),
            rec("dkim", "TXT", "s1._domainkey.example.com", Status::Fail),
        ],
    )]);
    let c = conditions(&i, &mut passed);
    assert_eq!(c.len(), 1);
    assert_eq!(
        (c[0].id.as_str(), c[0].severity),
        ("dns:example.com", Severity::Warning)
    );
    assert!(
        c[0].summary.starts_with(
            "The TXT example.com and TXT s1._domainkey.example.com records for example.com used to be correct but now fail"
        ),
        "{}",
        c[0].summary
    );

    // A lookup that could not be made is not drift.
    i.dns = Some(vec![dns(
        "example.com",
        vec![rec("spf", "TXT", "example.com", Status::Unknown)],
    )]);
    assert!(conditions(&i, &mut passed).is_empty());

    // An MX that passed and fails is critical.
    i.dns = Some(vec![dns(
        "example.com",
        vec![rec("mx", "MX", "example.com", Status::Pass)],
    )]);
    conditions(&i, &mut passed);
    i.dns = Some(vec![dns(
        "example.com",
        vec![rec("mx", "MX", "example.com", Status::Fail)],
    )]);
    assert_eq!(conditions(&i, &mut passed)[0].severity, Severity::Critical);

    // Unknown domain list (engine down) keeps the memory; a removed domain forgets it.
    i.dns = None;
    conditions(&i, &mut passed);
    assert!(passed.contains("example.com|mx"));
    i.dns = Some(vec![]);
    conditions(&i, &mut passed);
    assert!(passed.is_empty());
}

#[test]
fn disk_thresholds() {
    let mut i = healthy();
    i.disk = Some((100_000_000_000, 10_000_000_000)); // 90% exactly: fine
    assert!(ids(&i).is_empty());
    i.disk = Some((100_000_000_000, 6_000_000_000));
    let c = one(&i);
    assert_eq!((c.id.as_str(), c.severity), ("disk", Severity::Warning));
    assert_eq!(
        c.summary,
        "The mail disk is 94% full (6.0 GB free); once it fills up, the server stops accepting mail."
    );
    i.disk = Some((100_000_000_000, 2_000_000_000));
    assert_eq!(one(&i).severity, Severity::Critical);
}

#[test]
fn backups_only_when_one_was_ever_made() {
    let mut i = healthy();
    assert!(ids(&i).is_empty(), "no backup recorded: not alerted");
    i.last_backup = Some(t0() - ChronoDuration::days(3));
    assert!(ids(&i).is_empty());
    i.last_backup = Some(t0() - ChronoDuration::days(8));
    let c = one(&i);
    assert_eq!(c.id, "backup");
    assert!(c.summary.contains("8 days ago"));
}

// ── wording ─────────────────────────────────────────────────────────────

#[test]
fn messages_read_well() {
    let n = Notice {
        kind: Kind::Firing,
        condition: Condition::new(
            "disk",
            Severity::Warning,
            "Disk almost full",
            "The mail disk is 94% full.",
            "Free space.",
        ),
        since: t0(),
    };
    assert_eq!(
        channels::subject("mail.example.com", &n),
        "[mail.example.com] Disk almost full"
    );
    let b = channels::body("mail.example.com", &n);
    assert!(b.starts_with("The mail disk is 94% full.\n\nWhat to do: Free space.\n\nFirst noticed: 2026-11-10 12:00 UTC"), "{b}");
    assert!(b.contains("https://mail.example.com/console"));
    let r = Notice {
        kind: Kind::Resolved,
        ..n.clone()
    };
    assert_eq!(
        channels::subject("mail.example.com", &r),
        "[mail.example.com] Resolved: Disk almost full"
    );
    assert!(channels::body("mail.example.com", &r).contains("It was: The mail disk is 94% full."));
    let j = channels::webhook_json("mail.example.com", &n);
    assert_eq!(j["event"], "firing");
    assert_eq!(j["alert"]["severity"], "warning");
    assert_eq!(j["alert"]["since"], "2026-11-10T12:00:00Z");
}

// ── webhook + ntfy against a local listener ──────────────────────────────

#[derive(Debug, Clone)]
struct Seen {
    headers: axum::http::HeaderMap,
    body: String,
}

async fn listener(status: u16) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let s = seen.clone();
    let app = axum::Router::new().route(
        "/{*path}",
        axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push(Seen { headers, body });
                axum::http::StatusCode::from_u16(status).unwrap()
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

fn notice(kind: Kind, sev: Severity) -> Notice {
    Notice {
        kind,
        condition: Condition::new(
            "engine",
            sev,
            "Mail engine not responding",
            "The mail engine is not responding.",
            "Run doctor.",
        ),
        since: t0(),
    }
}

#[tokio::test]
async fn webhook_posts_json_with_the_token() {
    let (base, seen) = listener(204).await;
    let r = channels::post(
        &format!("{base}/hooks/abc"),
        Some("s3cret-token"),
        WebhookKind::Json,
        "mail.example.com",
        &notice(Kind::Firing, Severity::Critical),
    )
    .await
    .unwrap();
    assert_eq!(r, "delivered (HTTP 204)");
    let got = seen.lock().unwrap()[0].clone();
    assert_eq!(got.headers["authorization"], "Bearer s3cret-token");
    let v: serde_json::Value = serde_json::from_str(&got.body).unwrap();
    assert_eq!(v["event"], "firing");
    assert_eq!(v["server"], "mail.example.com");
    assert_eq!(v["alert"]["id"], "engine");
    assert_eq!(v["alert"]["severity"], "critical");
    assert_eq!(v["alert"]["remedy"], "Run doctor.");
}

#[tokio::test]
async fn ntfy_gets_title_priority_and_text() {
    let (base, seen) = listener(200).await;
    channels::post(
        &format!("{base}/yunori-topic"),
        None,
        WebhookKind::Ntfy,
        "mail.example.com",
        &notice(Kind::Firing, Severity::Critical),
    )
    .await
    .unwrap();
    channels::post(
        &format!("{base}/yunori-topic"),
        None,
        WebhookKind::Ntfy,
        "mail.example.com",
        &notice(Kind::Resolved, Severity::Critical),
    )
    .await
    .unwrap();
    let got = seen.lock().unwrap().clone();
    assert_eq!(
        got[0].headers["title"],
        "[mail.example.com] Mail engine not responding"
    );
    assert_eq!(got[0].headers["priority"], "urgent");
    assert!(got[0].headers.get("authorization").is_none());
    assert_eq!(
        got[0].body,
        "The mail engine is not responding.\n\nWhat to do: Run doctor."
    );
    assert_eq!(got[1].headers["priority"], "default");
    assert!(got[1].body.starts_with("Resolved."));
}

#[tokio::test]
async fn a_failing_receiver_is_reported_not_hidden() {
    let (base, _) = listener(500).await;
    let e = channels::post(
        &format!("{base}/x"),
        None,
        WebhookKind::Json,
        "h",
        &notice(Kind::Test, Severity::Warning),
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("HTTP 500"), "{e}");
    // Nothing listening at all.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let e = channels::post(
        &format!("http://127.0.0.1:{port}/x"),
        None,
        WebhookKind::Json,
        "h",
        &notice(Kind::Test, Severity::Warning),
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("could not reach 127.0.0.1"), "{e}");
}

#[test]
fn webhook_urls_are_checked() {
    assert!(channels::check_webhook_url("https://ntfy.sh/topic").is_ok());
    assert!(channels::check_webhook_url("http://10.0.0.5:8080/hook").is_ok());
    assert!(channels::check_webhook_url("ftp://x/y").is_err());
    assert!(channels::check_webhook_url("not a url").is_err());
    assert!(channels::check_webhook_url("file:///etc/passwd").is_err());
}

// ── the sender mailbox ───────────────────────────────────────────────────

#[tokio::test]
async fn the_sender_mailbox_is_created_once_and_its_password_sealed() {
    let app = crate::app::App::for_tests();
    let engine = app.engine().unwrap();
    let (addr, pw) = channels::ensure_sender(&app.db, engine.as_ref(), "example.test", false)
        .await
        .unwrap();
    assert_eq!(addr, "yunori-alerts@example.test");
    assert_eq!(
        app.db
            .get_secret(channels::SECRET_SENDER)
            .unwrap()
            .unwrap()
            .as_str(),
        pw.as_str()
    );
    let creds = crate::engine::UserCreds {
        email: addr.clone(),
        password: pw.clone(),
    };
    engine.user_verify(&creds).await.unwrap();
    // Second call: same mailbox, same password, no new account.
    let (_, pw2) = channels::ensure_sender(&app.db, engine.as_ref(), "example.test", false)
        .await
        .unwrap();
    assert_eq!(pw.as_str(), pw2.as_str());
    let n = engine
        .list_accounts()
        .await
        .unwrap()
        .iter()
        .filter(|a| a.address == addr)
        .count();
    assert_eq!(n, 1);
    // A reset (after the engine refused the password) gives it a new one.
    let (_, pw3) = channels::ensure_sender(&app.db, engine.as_ref(), "example.test", true)
        .await
        .unwrap();
    assert_ne!(pw.as_str(), pw3.as_str());
    assert!(engine.user_verify(&creds).await.is_err());
}
