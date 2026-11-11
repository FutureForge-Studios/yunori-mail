//! Live test of the browser onboarding's engine path against a REAL Stalwart
//! binary (ignored by default; same harness as `live_tests.rs`):
//!
//! ```sh
//! YUNORI_STALWART_BIN=/path/to/stalwart \
//!   cargo test live_engine_late_identity -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The terminal bootstraps the engine before anyone has named a mail domain,
//! with names derived from the console's address (here the sslip.io name of
//! a documentation IP). The browser then names the domain and the hostname,
//! and `installer::apply_identity` changes the running engine through the
//! management API only: no root, no restart. Binds the real mail ports — run
//! on a throwaway host or in a private network namespace.

use super::Engine;
use super::live_tests::{BOOT_PORT, Guard, HTTP_PORT, root, spawn_normal, step, wait_reachable};
use super::stalwart::{StalwartEngine, StalwartProvisioner};
use crate::app::App;
use crate::installer::os::EngineChild;
use crate::installer::{InstallInputs, apply_identity, panel};
use crate::model::{SendingMode, ServerConfig};
use crate::paths::Paths;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// The SMTP greeting and the EHLO answer on port 25.
async fn smtp_names() -> (String, String) {
    let s = tokio::net::TcpStream::connect(("127.0.0.1", 25))
        .await
        .unwrap();
    let mut r = BufReader::new(s);
    let mut greeting = String::new();
    r.read_line(&mut greeting).await.unwrap();
    r.get_mut().write_all(b"EHLO probe.test\r\n").await.unwrap();
    let mut ehlo = String::new();
    r.read_line(&mut ehlo).await.unwrap();
    (greeting.trim().to_string(), ehlo.trim().to_string())
}

#[tokio::test]
#[ignore]
async fn live_engine_late_identity() {
    let Some(paths) = root() else {
        eprintln!("YUNORI_STALWART_BIN not set — skipping live test");
        return;
    };
    let pw = "live-recovery-0123456789abcdef";
    let prov = StalwartProvisioner::new(&format!("http://127.0.0.1:{BOOT_PORT}"), "admin", pw);
    let console = panel::sslip_name("203.0.113.10".parse().unwrap());
    let (ph_host, ph_domain) = panel::placeholders(&console, None);

    step(
        "0. a reserved .invalid domain is refused at first run (so placeholders are real-looking names)",
    );
    let child = EngineChild::spawn(&paths, &format!("admin:{pw}"), BOOT_PORT, false).unwrap();
    prov.wait_ready(Duration::from_secs(60)).await.unwrap();
    let refused = prov
        .bootstrap(
            &ph_host,
            "setup.yunori.invalid",
            &format!("{}/", paths.stalwart_data().display()),
            &format!("{}/", paths.stalwart_logs().display()),
        )
        .await;
    eprintln!("bootstrap with .invalid: {:?}", refused.as_ref().err());
    assert!(refused.is_err());

    step("1. first run with the console's sslip.io name as hostname and domain");
    let boot = prov
        .bootstrap(
            &ph_host,
            &ph_domain,
            &format!("{}/", paths.stalwart_data().display()),
            &format!("{}/", paths.stalwart_logs().display()),
        )
        .await
        .expect("bootstrap with placeholders");
    assert_eq!(boot.admin_user, format!("admin@{ph_domain}"));
    child.stop().unwrap();
    let child = EngineChild::spawn(&paths, &format!("admin:{pw}"), BOOT_PORT, true).unwrap();
    prov.wait_ready(Duration::from_secs(60)).await.unwrap();
    prov.configure(&ph_host, HTTP_PORT).await.unwrap();
    child.stop().unwrap();

    step("2. normal mode, and an App wired to it as `serve` has it");
    let normal = Guard(Some(spawn_normal(&paths)));
    let app_root = paths.state_dir().join("app");
    let app = App::open(Paths::with_root(&app_root), true, false).unwrap();
    app.save_config(&ServerConfig {
        hostname: ph_host.clone(),
        primary_domain: ph_domain.clone(),
        admin_email: String::new(),
        client_origin: crate::app::DEFAULT_CLIENT_ORIGIN.into(),
        engine_url: format!("http://127.0.0.1:{HTTP_PORT}"),
        sending_mode: SendingMode::Direct,
        public_ipv4: Some("203.0.113.10".into()),
        public_ipv6: None,
        stalwart_version: Some("1.0.3".into()),
        install_complete: false,
        panel_host: Some(console.clone()),
    })
    .unwrap();
    app.db
        .put_secret(
            "engine.admin",
            &format!("{}:{}", boot.admin_user, boot.admin_secret.as_str()),
        )
        .unwrap();
    app.reload_engine().unwrap();
    let probe = StalwartEngine::new(
        &format!("http://127.0.0.1:{HTTP_PORT}"),
        &boot.admin_user,
        &boot.admin_secret,
    );
    wait_reachable(&probe).await;
    let (greeting, _) = smtp_names().await;
    eprintln!("greeting before: {greeting}");
    assert!(greeting.contains(&ph_host));

    step("3. the browser names the domain and hostname: rename + hostname, live");
    let inputs = InstallInputs {
        domain: "example.org".into(),
        hostname: "mail.example.org".into(),
        admin_email: "owner@elsewhere.example".into(),
        first_mailbox: String::new(),
        sending: SendingMode::Direct,
        relay: None,
        client_origin: crate::app::DEFAULT_CLIENT_ORIGIN.into(),
        sending_confirmed: false,
    };
    let detail = apply_identity(&app, &inputs, "live-test").await.unwrap();
    eprintln!("{detail}");
    let engine = app.engine().unwrap();
    let domains = engine.list_domains().await.unwrap();
    assert_eq!(domains.len(), 1, "{domains:?}");
    assert_eq!(domains[0].name, "example.org");
    assert_eq!(engine.hostname().await.unwrap(), "mail.example.org");
    let admin = app.db.get_secret("engine.admin").unwrap().unwrap();
    assert!(admin.starts_with("admin@example.org:"));
    let (greeting, ehlo) = smtp_names().await;
    eprintln!("greeting after: {greeting} | {ehlo}");
    assert!(greeting.contains("mail.example.org"), "{greeting}");
    assert!(ehlo.contains("mail.example.org"), "{ehlo}");
    // Idempotent.
    apply_identity(&app, &inputs, "live-test").await.unwrap();

    step("4. the zone file follows: MX/SRV targets and the DKIM keys made at first run");
    let deadline = Instant::now() + Duration::from_secs(60);
    let zone = loop {
        let z = engine.zone_file(&domains[0].id).await.unwrap();
        if z.contains("_domainkey") || Instant::now() > deadline {
            break z;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    eprintln!("{zone}");
    assert!(zone.contains("example.org. IN MX 10 mail.example.org."));
    assert!(zone.contains("._domainkey.example.org."));
    assert!(!zone.contains(&ph_domain));

    step("5. listeners keep serving the certificate Yunori pushes, for the new name");
    let (cert, key) = crate::edge::tls::self_signed(&["mail.example.org".into()]).unwrap();
    engine.install_certificate(&cert, &key).await.unwrap();
    let want = crate::util::sha256_hex(crate::edge::tls::parse_certs(&cert).unwrap()[0].as_ref());
    tokio::time::sleep(Duration::from_secs(2)).await;
    for (port, starttls) in [(465u16, false), (993, false), (587, true)] {
        let got = crate::edge::tls::served_fingerprint(port, "mail.example.org", starttls)
            .await
            .unwrap();
        assert_eq!(got, want, "port {port}");
    }

    step("6. after a restart only the new administrator name signs in");
    drop(normal);
    let _normal = Guard(Some(spawn_normal(&paths)));
    app.reload_engine().unwrap();
    let engine = app.engine().unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    while !engine.info().await.reachable {
        assert!(
            Instant::now() < deadline,
            "the renamed admin cannot sign in"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        !probe.info().await.reachable,
        "the old admin name must stop working"
    );
    let (greeting, _) = smtp_names().await;
    assert!(greeting.contains("mail.example.org"), "{greeting}");
    assert_eq!(engine.hostname().await.unwrap(), "mail.example.org");
}
