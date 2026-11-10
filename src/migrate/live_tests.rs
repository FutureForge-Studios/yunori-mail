//! Live end-to-end test of "Move your mail here" against a REAL Stalwart,
//! used as both the old server (IMAP on 993) and the new mailbox (JMAP).
//!
//! ```sh
//! YUNORI_STALWART_BIN=/path/to/stalwart \
//!   cargo test live_migrate -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Seeds a source account over IMAP APPEND (several folders, flags, dates),
//! imports it into another account the way the portal does (temporary app
//! password), checks folders, counts, flags and dates over IMAP as the new
//! account, runs again (nothing duplicated), then once more as the CLI does
//! (engine admin acting for the user, `user%admin`) after a new message.

use super::imap::{Client, Security, Target as ImapTarget, encode_mutf7};
use super::*;
use crate::engine::Engine;
use crate::engine::live_tests::{LIVE_DOMAIN, LIVE_HOST, LIVE_HTTP_PORT, boot_live_engine};
use crate::model::ServerConfig;
use crate::paths::Paths;

const SRC_PW: &str = "Source-Account-Pass-1!";
const DST_PW: &str = "Destination-Pass-2024!";

fn step(msg: &str) {
    eprintln!("\n── {msg}");
}

fn message(id: &str, subject: &str) -> Vec<u8> {
    format!(
        "From: Someone <someone@elsewhere.example>\r\nTo: old@{LIVE_DOMAIN}\r\nSubject: {subject}\r\nMessage-ID: <{id}>\r\nDate: Tue, 2 Jan 2024 10:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from {subject}.\r\n"
    )
    .into_bytes()
}

async fn imap_as(policy: &NetPolicy, user: &str, pass: &str) -> Client {
    let mut c = Client::connect(
        &ImapTarget {
            host: LIVE_HOST,
            addr: SocketAddr::new(policy.resolve_to.unwrap(), 993),
            security: Security::Implicit,
            extra_roots: &policy.extra_roots,
        },
        1 << 26,
    )
    .await
    .expect("IMAP connect (verified TLS)");
    c.login(user, pass).await.expect("IMAP login");
    c
}

/// name → (count, [(flags, internal date)]) as the account sees it over IMAP.
async fn snapshot(c: &mut Client) -> HashMap<String, Vec<(Vec<String>, Option<String>)>> {
    let mut out = HashMap::new();
    for (raw, _, attrs) in c.list().await.unwrap() {
        if attrs.iter().any(|a| a == "\\noselect") {
            continue;
        }
        let name = imap::decode_mutf7(&raw);
        let (n, _) = c.examine(&raw).await.unwrap();
        let mut msgs = Vec::new();
        if n > 0 {
            for items in c
                .uid_fetch("1:*", "(UID FLAGS INTERNALDATE)")
                .await
                .unwrap()
            {
                let mut flags = Vec::new();
                let mut date = None;
                for (k, v) in items {
                    match k.as_str() {
                        "FLAGS" => {
                            flags = v
                                .list()
                                .unwrap()
                                .iter()
                                .filter_map(imap::V::text)
                                .filter(|f| f != "\\Recent")
                                .collect();
                            flags.sort();
                        }
                        "INTERNALDATE" => date = v.text().and_then(|d| imap::internal_date_utc(&d)),
                        _ => {}
                    }
                }
                msgs.push((flags, date));
            }
            msgs.sort();
        }
        out.insert(name, msgs);
    }
    out
}

async fn wait(job: Arc<Job>) -> Progress {
    tokio::time::timeout(Duration::from_secs(120), job.wait())
        .await
        .expect("import finishes");
    let p = job.progress();
    eprintln!("{p:#?}");
    p
}

#[tokio::test]
#[ignore]
async fn live_migrate_between_two_accounts() {
    let Some(live) = boot_live_engine().await else {
        eprintln!("YUNORI_STALWART_BIN not set — skipping live test");
        return;
    };
    let engine = &live.engine;

    step("1. a certificate the IMAP client can verify (as a trust anchor of its own)");
    let (cert, key) = crate::edge::tls::self_signed(&[LIVE_HOST.to_string()]).unwrap();
    engine.install_certificate(&cert, &key).await.unwrap();
    let want = crate::util::sha256_hex(crate::edge::tls::parse_certs(&cert).unwrap()[0].as_ref());
    let deadline = Instant::now() + Duration::from_secs(20);
    while crate::edge::tls::served_fingerprint(993, LIVE_HOST, false)
        .await
        .ok()
        .as_ref()
        != Some(&want)
    {
        assert!(Instant::now() < deadline, "993 serves the new certificate");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let policy = NetPolicy {
        allow_private: true,
        resolve_to: Some("127.0.0.1".parse().unwrap()),
        extra_roots: crate::edge::tls::parse_certs(&cert).unwrap(),
    };
    // Without the test anchor, verification stays on and refuses the connection.
    let strict = NetPolicy {
        extra_roots: vec![],
        ..policy.clone()
    };
    let refused = inspect(
        SourceSpec {
            host: LIVE_HOST.into(),
            port: 993,
            username: format!("old@{LIVE_DOMAIN}"),
            password: Zeroizing::new(SRC_PW.into()),
        },
        strict,
    )
    .await
    .expect_err("an unverifiable certificate is refused");
    eprintln!("unverified certificate: {refused:?}");

    step("2. two mailboxes: old@ (the source) and new@ (the destination)");
    let domain = engine
        .list_domains()
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.name == LIVE_DOMAIN)
        .unwrap();
    engine
        .create_account("old", &domain.id, Some("Old"), None, SRC_PW)
        .await
        .unwrap();
    engine
        .create_account("new", &domain.id, Some("New"), None, DST_PW)
        .await
        .unwrap();
    let src_user = format!("old@{LIVE_DOMAIN}");
    let dst_user = format!("new@{LIVE_DOMAIN}");

    step("3. seed the source over IMAP APPEND");
    let mut c = imap_as(&policy, &src_user, SRC_PW).await;
    let folders = c.list().await.unwrap();
    for (raw, _, attrs) in &folders {
        eprintln!("  source folder {:?} {attrs:?}", imap::decode_mutf7(raw));
    }
    let sent_raw = folders
        .iter()
        .find(|(_, _, a)| a.iter().any(|x| x == "\\sent"))
        .map(|(r, _, _)| r.clone())
        .expect("the engine marks its Sent folder (SPECIAL-USE)");
    let drafts_raw = folders
        .iter()
        .find(|(_, _, a)| a.iter().any(|x| x == "\\drafts"))
        .map(|(r, _, _)| r.clone())
        .unwrap();
    for f in ["Projects", "Projects/2024", "Receipts", "Résumés"] {
        c.create(&encode_mutf7(f)).await.unwrap();
    }
    let seed: Vec<(Vec<u8>, &str, &str, &str)> = vec![
        (
            b"INBOX".to_vec(),
            "\\Seen \\Flagged",
            "01-Feb-2023 08:30:00 +0000",
            "in1@x",
        ),
        (
            b"INBOX".to_vec(),
            "\\Answered \\Seen",
            "15-Mar-2023 12:00:00 +0200",
            "in2@x",
        ),
        (b"INBOX".to_vec(), "", "20-Apr-2023 23:59:59 -0500", "in3@x"),
        (
            sent_raw.clone(),
            "\\Seen",
            "02-May-2023 09:00:00 +0000",
            "sent1@x",
        ),
        (
            drafts_raw.clone(),
            "\\Draft",
            "03-Jun-2023 10:00:00 +0000",
            "draft1@x",
        ),
        (
            encode_mutf7("Projects/2024"),
            "\\Seen",
            "04-Jul-2023 11:00:00 +0000",
            "p1@x",
        ),
        (
            encode_mutf7("Projects/2024"),
            "",
            "05-Aug-2023 12:00:00 +0000",
            "p2@x",
        ),
        (
            encode_mutf7("Receipts"),
            "\\Seen",
            "06-Sep-2023 13:00:00 +0000",
            "r1@x",
        ),
        (
            encode_mutf7("Résumés"),
            "\\Flagged",
            "07-Oct-2023 14:00:00 +0000",
            "cv1@x",
        ),
    ];
    for (folder, flags, date, id) in &seed {
        c.append(folder, flags, date, &message(id, id))
            .await
            .unwrap();
    }
    let source_view = snapshot(&mut c).await;
    c.logout().await;
    eprintln!("source: {source_view:#?}");

    step("4. an App wired to the live engine, as `serve` has it");
    let root = std::env::temp_dir().join(format!("yunori-live-migrate-app-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let app = App::open(Paths::with_root(&root), true, false).unwrap();
    app.save_config(&ServerConfig {
        hostname: LIVE_HOST.into(),
        primary_domain: LIVE_DOMAIN.into(),
        admin_email: "owner@elsewhere.example".into(),
        client_origin: crate::app::DEFAULT_CLIENT_ORIGIN.into(),
        engine_url: format!("http://127.0.0.1:{LIVE_HTTP_PORT}"),
        sending_mode: crate::model::SendingMode::Direct,
        public_ipv4: None,
        public_ipv6: None,
        stalwart_version: Some("1.0.3".into()),
        install_complete: true,
    })
    .unwrap();
    app.db
        .put_secret(
            "engine.admin",
            &format!("{}:{}", live.admin_user, live.admin_secret),
        )
        .unwrap();
    app.reload_engine().unwrap();

    step("5. test the connection: folders, counts and the plan");
    let spec = SourceSpec {
        host: LIVE_HOST.into(),
        port: 993,
        username: src_user.clone(),
        password: Zeroizing::new(SRC_PW.into()),
    };
    let plans = inspect(spec.clone(), policy.clone()).await.unwrap();
    for p in &plans {
        eprintln!(
            "  {:<20} {:>2} → {:<28} selected={}",
            p.name, p.messages, p.destination, p.selected
        );
    }
    let by_name = |n: &str| plans.iter().find(|p| p.name == n).unwrap();
    assert_eq!(
        by_name("INBOX").target,
        Target::Role {
            role: "inbox".into()
        }
    );
    assert_eq!(by_name("INBOX").messages, 3);
    let sent_name = imap::decode_mutf7(&sent_raw);
    assert_eq!(
        by_name(&sent_name).target,
        Target::Role {
            role: "sent".into()
        }
    );
    assert_eq!(by_name("Projects/2024").destination, "Projects / 2024");
    assert_eq!(by_name("Résumés").messages, 1);
    let wrong = inspect(
        SourceSpec {
            password: Zeroizing::new("not-the-password".into()),
            ..spec.clone()
        },
        policy.clone(),
    )
    .await
    .err()
    .unwrap();
    match wrong {
        StartError::Source(m) => assert!(m.contains("app password"), "{m}"),
        other => panic!("{other:?}"),
    }

    step("6. import as the portal does (temporary app password of new@)");
    let dst = UserCreds {
        email: dst_user.clone(),
        password: Zeroizing::new(DST_PW.into()),
    };
    let req = |source: SourceSpec| JobRequest {
        account: dst_user.clone(),
        actor: dst_user.clone(),
        source,
        folders: None,
        dry_run: false,
    };
    let job = start(
        &app,
        req(spec.clone()),
        DestAuth::TemporaryAppPassword(dst.clone()),
        policy.clone(),
    )
    .await
    .unwrap();
    let p = wait(job).await;
    assert_eq!(p.state, JobState::Done, "{p:?}");
    assert_eq!(p.messages_copied, seed.len() as u64);
    assert_eq!(p.messages_failed, 0);
    assert!(
        engine
            .user_app_passwords(&dst)
            .await
            .unwrap()
            .iter()
            .all(|a| a.description != TEMP_APP_PASSWORD),
        "the temporary app password is revoked"
    );

    step("7. what new@ sees over IMAP");
    let mut d = imap_as(&policy, &dst_user, DST_PW).await;
    let dest_view = snapshot(&mut d).await;
    eprintln!("destination: {dest_view:#?}");
    // Special folders by role (the engine names them its own way).
    let dfolders = d.list().await.unwrap();
    let by_attr = |attr: &str| {
        dfolders
            .iter()
            .find(|(_, _, a)| a.iter().any(|x| x == attr))
            .map(|(r, _, _)| imap::decode_mutf7(r))
            .unwrap()
    };
    let same = |src: &str, dst: &str| {
        assert_eq!(
            source_view[src], dest_view[dst],
            "{src} → {dst}: same count, flags and internal dates"
        );
    };
    same("INBOX", "INBOX");
    same(&sent_name, &by_attr("\\sent"));
    same(&imap::decode_mutf7(&drafts_raw), &by_attr("\\drafts"));
    same("Projects/2024", "Projects/2024");
    same("Receipts", "Receipts");
    same("Résumés", "Résumés");
    assert_eq!(dest_view["INBOX"].len(), 3);
    assert!(
        dest_view["INBOX"]
            .iter()
            .any(|(f, d)| f.contains(&"\\Flagged".to_string())
                && d.as_deref() == Some("2023-02-01T08:30:00Z"))
    );
    let total = |v: &HashMap<String, Vec<_>>| v.values().map(Vec::len).sum::<usize>();
    assert_eq!(total(&dest_view), seed.len());
    d.logout().await;

    step("8. again: nothing is copied twice");
    let p = wait(
        start(
            &app,
            req(spec.clone()),
            DestAuth::TemporaryAppPassword(dst.clone()),
            policy.clone(),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(p.state, JobState::Done);
    assert_eq!(
        (p.messages_copied, p.messages_skipped),
        (0, seed.len() as u64)
    );
    let mut d = imap_as(&policy, &dst_user, DST_PW).await;
    assert_eq!(total(&snapshot(&mut d).await), seed.len());
    d.logout().await;

    step("9. as the CLI does: the engine admin acting for new@ (user%admin), after a new message");
    let mut c = imap_as(&policy, &src_user, SRC_PW).await;
    c.append(
        b"INBOX",
        "",
        "08-Nov-2023 15:00:00 +0000",
        &message("late@x", "late"),
    )
    .await
    .unwrap();
    c.logout().await;
    let impersonated = UserCreds {
        email: format!("{dst_user}%{}", live.admin_user),
        password: Zeroizing::new(live.admin_secret.clone()),
    };
    let p = wait(
        start(
            &app,
            req(spec.clone()),
            DestAuth::Direct(impersonated),
            policy.clone(),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(p.state, JobState::Done, "{p:?}");
    assert_eq!(
        (p.messages_copied, p.messages_skipped),
        (1, seed.len() as u64)
    );
    let mut d = imap_as(&policy, &dst_user, DST_PW).await;
    let view = snapshot(&mut d).await;
    assert_eq!(view["INBOX"].len(), 4, "landed in new@'s own Inbox");
    assert_eq!(total(&view), seed.len() + 1);
    d.logout().await;

    let audit: Vec<String> = app
        .db
        .audit_list(50)
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    assert_eq!(
        audit
            .iter()
            .filter(|a| *a == "mailbox.migrate_start")
            .count(),
        3
    );
    assert_eq!(
        audit
            .iter()
            .filter(|a| *a == "mailbox.migrate_done")
            .count(),
        3
    );
    let _ = std::fs::remove_dir_all(&root);
    eprintln!("\nLIVE MIGRATION TEST PASSED against the real engine");
}
