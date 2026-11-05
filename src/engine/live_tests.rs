//! Live end-to-end test against a REAL Stalwart binary (ignored by default).
//!
//! ```sh
//! YUNORI_STALWART_BIN=/path/to/stalwart \
//!   cargo test live_engine -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Drives the same code paths as the installer (bootstrap mode → recovery mode →
//! normal mode) and then every Engine operation the console and CLI use.
//! Binds the real mail ports (25/465/587/993/4190) — run on a throwaway host.

use super::stalwart::{StalwartEngine, StalwartProvisioner};
use super::{Engine, EngineError, RelayConfig, UserCreds};
use crate::installer::os::EngineChild;
use crate::paths::Paths;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const BOOT_PORT: u16 = 18181;
const HTTP_PORT: u16 = 18180;
const HOST: &str = "mail.live.test";
const DOMAIN: &str = "live.test";

fn step(msg: &str) {
    eprintln!("\n── {msg}");
}

fn root() -> Option<Paths> {
    let bin = std::env::var("YUNORI_STALWART_BIN").ok()?;
    let dir = std::env::var("YUNORI_LIVE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::temp_dir().join(format!("yunori-live-{}", std::process::id()))
        });
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::with_root(&dir);
    std::fs::create_dir_all(paths.stalwart_bin().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&bin, paths.stalwart_bin()).unwrap();
    for d in [
        paths.stalwart_etc(),
        paths.stalwart_data(),
        paths.stalwart_logs(),
    ] {
        std::fs::create_dir_all(d).unwrap();
    }
    eprintln!("live root: {}", dir.display());
    Some(paths)
}

fn spawn_normal(paths: &Paths) -> Child {
    let log = std::fs::File::create(paths.stalwart_logs().join("normal.stdout")).unwrap();
    Command::new(paths.stalwart_bin())
        .arg(format!("--config={}", paths.stalwart_config().display()))
        .env_remove("STALWART_RECOVERY_MODE")
        .env_remove("STALWART_RECOVERY_ADMIN")
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .expect("start engine in normal mode")
}

async fn wait_reachable(engine: &StalwartEngine) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !engine.info().await.reachable {
        assert!(
            Instant::now() < deadline,
            "engine did not become reachable on :{HTTP_PORT}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

struct Guard(Option<Child>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(c) = self.0.as_mut() {
            let _ = Command::new("kill")
                .args(["-INT", &c.id().to_string()])
                .status();
            std::thread::sleep(Duration::from_secs(2));
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[tokio::test]
#[ignore]
async fn live_engine_end_to_end() {
    let Some(paths) = root() else {
        eprintln!("YUNORI_STALWART_BIN not set — skipping live test");
        return;
    };
    let recovery_pw = "live-recovery-0123456789abcdef";
    let base = format!("http://127.0.0.1:{BOOT_PORT}");
    let prov = StalwartProvisioner::new(&base, "admin", recovery_pw);

    step("1. bootstrap mode: provision store, domain, admin");
    let child =
        EngineChild::spawn(&paths, &format!("admin:{recovery_pw}"), BOOT_PORT, false).unwrap();
    prov.wait_ready(Duration::from_secs(60))
        .await
        .expect("bootstrap listener");
    let boot = prov
        .bootstrap(
            HOST,
            DOMAIN,
            &format!("{}/", paths.stalwart_data().display()),
            &format!("{}/", paths.stalwart_logs().display()),
        )
        .await
        .expect("x:Bootstrap/set");
    eprintln!("admin user: {}", boot.admin_user);
    assert!(boot.admin_user.ends_with(&format!("@{DOMAIN}")));
    child.stop().unwrap();
    assert!(
        paths.stalwart_config().exists(),
        "bootstrap must write config.json"
    );

    step("2. recovery mode: Yunori listener set + proxy trust (twice: idempotent)");
    let child =
        EngineChild::spawn(&paths, &format!("admin:{recovery_pw}"), BOOT_PORT, true).unwrap();
    prov.wait_ready(Duration::from_secs(60))
        .await
        .expect("recovery listener");
    prov.configure(HOST, HTTP_PORT)
        .await
        .expect("configure listeners");
    prov.configure(HOST, HTTP_PORT)
        .await
        .expect("configure again (idempotent)");
    child.stop().unwrap();

    step("3. normal mode: admin JMAP over the internal HTTP listener");
    let _normal = Guard(Some(spawn_normal(&paths)));
    let engine = StalwartEngine::new(
        &format!("http://127.0.0.1:{HTTP_PORT}"),
        &boot.admin_user,
        &boot.admin_secret,
    );
    wait_reachable(&engine).await;

    step("4. domains + zone file (DKIM generated asynchronously)");
    let domains = engine.list_domains().await.expect("list domains");
    let primary = domains
        .iter()
        .find(|d| d.name == DOMAIN)
        .expect("primary domain exists")
        .clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    let zone = loop {
        let z = engine.zone_file(&primary.id).await.expect("zone file");
        if z.contains("_domainkey") || Instant::now() > deadline {
            break z;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    eprintln!("zone file:\n{zone}");
    let recs = crate::dns::zone::parse(&zone, DOMAIN);
    assert!(recs.iter().any(|r| r.rtype == "MX"), "zone has MX");
    assert!(
        recs.iter()
            .any(|r| r.rtype == "TXT" && r.value.starts_with("v=spf1")),
        "zone has SPF"
    );
    assert!(
        recs.iter().any(|r| r.name.contains("._domainkey.")),
        "zone has DKIM"
    );
    let ctx = crate::dns::records::RecordContext {
        hostname: HOST.into(),
        ipv4: Some("203.0.113.10".into()),
        ipv6: None,
        is_primary: true,
        relay_spf_include: None,
        dmarc_policy: crate::model::DmarcPolicy::None,
    };
    let expected = crate::dns::records::expected_records(DOMAIN, &zone, &ctx);
    let ids: Vec<&str> = expected.iter().map(|e| e.id.as_str()).collect();
    eprintln!("expected record ids: {ids:?}");
    assert!(ids.contains(&"mx") && ids.contains(&"spf") && ids.contains(&"dmarc"));
    assert!(
        !ids.contains(&"dkim"),
        "DKIM came from the zone, not the pending placeholder"
    );

    let d2 = engine
        .create_domain("second.test")
        .await
        .expect("create domain");
    let d2u = engine
        .update_domain(&d2.id, Some(Some("ana@live.test".into())), None)
        .await
        .expect("set catch-all");
    assert_eq!(d2u.catch_all.as_deref(), Some("ana@live.test"));
    let d2u = engine
        .update_domain(&d2.id, Some(None), Some(false))
        .await
        .expect("clear catch-all, disable");
    assert!(d2u.catch_all.is_none() && !d2u.enabled);

    step("5. accounts: create, update, aliases, password");
    let acct = engine
        .create_account(
            "ana",
            &primary.id,
            Some("Ana Ito"),
            Some(1 << 30),
            "Correct-Horse-Battery-9!",
        )
        .await
        .expect("create account");
    assert_eq!(acct.address, "ana@live.test");
    assert_eq!(acct.display_name.as_deref(), Some("Ana Ito"));
    assert_eq!(acct.quota_bytes, Some(1 << 30));
    let upd = engine
        .update_account(&acct.id, Some(Some("Ana I.".into())), Some(None))
        .await
        .expect("update");
    assert_eq!(upd.display_name.as_deref(), Some("Ana I."));
    assert_eq!(upd.quota_bytes, None);

    engine
        .add_alias(&acct.id, "hello", &primary.id)
        .await
        .expect("add alias");
    engine
        .add_alias(&acct.id, "sales", &primary.id)
        .await
        .expect("add second alias");
    let a = engine
        .list_accounts()
        .await
        .unwrap()
        .into_iter()
        .find(|x| x.id == acct.id)
        .unwrap();
    let locals: Vec<&str> = a.aliases.iter().map(|x| x.local.as_str()).collect();
    assert_eq!(locals, vec!["hello", "sales"]);
    let hello_key = a
        .aliases
        .iter()
        .find(|x| x.local == "hello")
        .unwrap()
        .key
        .clone();
    engine
        .remove_alias(&acct.id, &hello_key)
        .await
        .expect("remove alias");
    let a = engine
        .list_accounts()
        .await
        .unwrap()
        .into_iter()
        .find(|x| x.id == acct.id)
        .unwrap();
    assert_eq!(
        a.aliases
            .iter()
            .map(|x| x.local.as_str())
            .collect::<Vec<_>>(),
        vec!["sales"]
    );

    let creds = |pw: &str| UserCreds {
        email: "ana@live.test".into(),
        password: Zeroizing::new(pw.into()),
    };
    engine
        .user_verify(&creds("Correct-Horse-Battery-9!"))
        .await
        .expect("user login works");
    assert!(matches!(
        engine.user_verify(&creds("wrong-password-x")).await,
        Err(EngineError::Unauthorized)
    ));
    engine
        .set_password(&acct.id, "Another-Strong-Pass-42!")
        .await
        .expect("admin password reset");
    engine
        .user_verify(&creds("Another-Strong-Pass-42!"))
        .await
        .expect("new password works");
    assert!(
        engine
            .user_verify(&creds("Correct-Horse-Battery-9!"))
            .await
            .is_err(),
        "old password rejected"
    );

    step("6. self-service as the user: app passwords + password change");
    let me = creds("Another-Strong-Pass-42!");
    let (ap, secret) = engine
        .user_create_app_password(&me, "Phone")
        .await
        .expect("create app password");
    assert!(!secret.is_empty());
    let list = engine
        .user_app_passwords(&me)
        .await
        .expect("list app passwords");
    assert!(
        list.iter()
            .any(|p| p.id == ap.id && p.description == "Phone"),
        "{list:?}"
    );
    engine
        .user_verify(&UserCreds {
            email: "ana@live.test".into(),
            password: Zeroizing::new(secret.clone()),
        })
        .await
        .expect("app password authenticates");
    engine
        .user_delete_app_password(&me, &ap.id)
        .await
        .expect("revoke app password");
    assert!(
        engine
            .user_app_passwords(&me)
            .await
            .unwrap()
            .iter()
            .all(|p| p.id != ap.id)
    );
    engine
        .user_change_password(&me, "Third-Strong-Pass-77!")
        .await
        .expect("self-service password change");
    engine
        .user_verify(&creds("Third-Strong-Pass-77!"))
        .await
        .expect("changed password works");

    step("7. operations: queue, reload, certificate push, relay route");
    let q = engine.queue().await.expect("queue");
    eprintln!("queued messages: {}", q.len());
    engine.reload_settings().await.expect("reload settings");
    let fp_of = |pem: &str| {
        crate::edge::tls::parse_certs(pem)
            .unwrap()
            .into_iter()
            .next()
            .map(|c| crate::util::sha256_hex(c.as_ref()))
    };
    async fn served_all(want: &Option<String>, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        for (port, starttls) in [(993u16, false), (465, false), (587, true)] {
            loop {
                let got = crate::edge::tls::served_fingerprint(port, HOST, starttls).await;
                match got {
                    Ok(fp) if Some(&fp) == want.as_ref() => {
                        eprintln!("port {port}: serves the {label} certificate");
                        break;
                    }
                    other if Instant::now() > deadline => {
                        panic!("port {port}: expected the {label} certificate, got {other:?}")
                    }
                    _ => tokio::time::sleep(Duration::from_millis(500)).await,
                }
            }
        }
    }
    let (c1, k1) = crate::edge::tls::self_signed(&[HOST.to_string()]).unwrap();
    engine
        .install_certificate(&c1, &k1)
        .await
        .expect("install certificate");
    served_all(&fp_of(&c1), "first").await;
    let (c2, k2) = crate::edge::tls::self_signed(&[HOST.to_string()]).unwrap();
    engine
        .install_certificate(&c2, &k2)
        .await
        .expect("replace certificate");
    served_all(&fp_of(&c2), "replacement").await;
    let relay = RelayConfig {
        host: "smtp.postmarkapp.com".into(),
        port: 587,
        implicit_tls: false,
        username: "relay-user".into(),
        password: "relay-pass-not-real".into(),
    };
    engine.set_relay(Some(&relay)).await.expect("enable relay");
    engine
        .set_relay(Some(&relay))
        .await
        .expect("update relay (idempotent)");
    engine.set_relay(None).await.expect("disable relay");

    step("8. cleanup: delete account and second domain");
    engine
        .delete_account(&acct.id)
        .await
        .expect("delete account");
    assert!(
        engine
            .list_accounts()
            .await
            .unwrap()
            .iter()
            .all(|x| x.id != acct.id)
    );
    engine.delete_domain(&d2.id).await.expect("delete domain");
    assert!(
        engine
            .list_domains()
            .await
            .unwrap()
            .iter()
            .all(|d| d.id != d2.id)
    );
    eprintln!("\nLIVE TEST PASSED against the real engine");
    // YUNORI_LIVE_KEEP=1 leaves the engine running for `serve --dev --engine-url`.
    if std::env::var("YUNORI_LIVE_KEEP").is_ok_and(|v| v == "1") {
        let creds = format!("{}:{}", boot.admin_user, boot.admin_secret.as_str());
        crate::store::vault::write_private(
            &paths.stalwart_etc().join("admin.txt"),
            creds.as_bytes(),
        )
        .unwrap();
        eprintln!(
            "engine left running on 127.0.0.1:{HTTP_PORT}; admin credentials in etc/stalwart/admin.txt"
        );
        std::mem::forget(_normal);
    }
}
