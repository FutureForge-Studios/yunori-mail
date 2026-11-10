//! Live probe of Stalwart's own DNS management (ignored by default): what
//! `x:DnsServer` + `Domain.dnsManagement = Automatic` do on a real engine.
//! The facts it prints are recorded in docs/stalwart-integration.md.
//!
//! ```sh
//! YUNORI_STALWART_BIN=/path/to/stalwart \
//!   cargo test live_dns_management -- --ignored --nocapture --test-threads=1
//! ```
//! Binds the real mail ports (25/465/587/993/4190): run on a throwaway host.

use crate::engine::Engine;
use crate::engine::jmap::{Auth, Call, JmapClient, get_list};
use crate::engine::stalwart::{StalwartEngine, StalwartProvisioner};
use crate::installer::os::EngineChild;
use crate::paths::Paths;
use serde_json::{Value, json};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BOOT_PORT: u16 = 18381;
const HTTP_PORT: u16 = 18380;
const HOST: &str = "mail.live.test";
const DOMAIN: &str = "live.test";

#[tokio::test]
#[ignore]
async fn live_dns_management_probe() {
    let Ok(bin) = std::env::var("YUNORI_STALWART_BIN") else {
        eprintln!("YUNORI_STALWART_BIN not set — skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("yunori-live-dns-{}", std::process::id()));
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
    let pw = "live-recovery-0123456789abcdef";
    let prov = StalwartProvisioner::new(&format!("http://127.0.0.1:{BOOT_PORT}"), "admin", pw);
    let child = EngineChild::spawn(&paths, &format!("admin:{pw}"), BOOT_PORT, false).unwrap();
    prov.wait_ready(Duration::from_secs(60)).await.unwrap();
    let boot = prov
        .bootstrap(
            HOST,
            DOMAIN,
            &format!("{}/", paths.stalwart_data().display()),
            &format!("{}/", paths.stalwart_logs().display()),
        )
        .await
        .unwrap();
    child.stop().unwrap();
    let child = EngineChild::spawn(&paths, &format!("admin:{pw}"), BOOT_PORT, true).unwrap();
    prov.wait_ready(Duration::from_secs(60)).await.unwrap();
    prov.configure(HOST, HTTP_PORT).await.unwrap();
    child.stop().unwrap();

    let log = std::fs::File::create(paths.stalwart_logs().join("normal.stdout")).unwrap();
    let mut normal = Command::new(paths.stalwart_bin())
        .arg(format!("--config={}", paths.stalwart_config().display()))
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let base = format!("http://127.0.0.1:{HTTP_PORT}");
    let engine = StalwartEngine::new(&base, &boot.admin_user, &boot.admin_secret);
    let deadline = Instant::now() + Duration::from_secs(90);
    while !engine.info().await.reachable {
        assert!(Instant::now() < deadline, "engine not reachable");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let domain_id = engine
        .list_domains()
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.name == DOMAIN)
        .unwrap()
        .id;
    let jmap = JmapClient::new(&base);
    let auth = Auth::basic(&boot.admin_user, &boot.admin_secret);
    let call = |m: &str, a: Value| Call::new(m, a);

    eprintln!("\n── x:DnsServer/set create (Cloudflare, dummy token)");
    let r = jmap
        .call(
            &auth,
            vec![call(
                "x:DnsServer/set",
                json!({"create": {"n": {"@type": "Cloudflare", "description": "probe",
                    "secret": {"@type": "Value", "secret": "dummy-token-0123456789abcdefghijklmn"}}}}),
            )],
        )
        .await
        .unwrap();
    eprintln!("{}", r[0]);
    let server_id = r[0]["created"]["n"]["id"]
        .as_str()
        .expect("DnsServer created")
        .to_string();

    eprintln!("\n── x:DnsServer/get (is the secret returned?)");
    let r = jmap
        .call(
            &auth,
            vec![call("x:DnsServer/get", json!({"ids": [server_id]}))],
        )
        .await
        .unwrap();
    eprintln!("{}", r[0]);
    let got = get_list(&r[0]);
    let secret_echoed = got
        .first()
        .map(|s| s.to_string().contains("dummy-token"))
        .unwrap_or(false);
    eprintln!("secret echoed back: {secret_echoed}");

    eprintln!("\n── x:Domain/set dnsManagement = Automatic (mx + spf only)");
    let r = jmap
        .call(
            &auth,
            vec![call(
                "x:Domain/set",
                json!({"update": {domain_id.clone(): {"dnsManagement": {"@type": "Automatic",
                    "dnsServerId": server_id, "publishRecords": {"mx": true, "spf": true}}}}}),
            )],
        )
        .await
        .unwrap();
    eprintln!("{}", r[0]);

    eprintln!("\n── x:Task (DnsManagement) over ~20 s");
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let r = jmap
            .call(
                &auth,
                vec![
                    call("x:Task/query", json!({})),
                    call(
                        "x:Task/get",
                        json!({"#ids": {"resultOf": "c0", "name": "x:Task/query", "path": "/ids"}}),
                    ),
                ],
            )
            .await;
        match r {
            Ok(r) => {
                for t in get_list(&r[1])
                    .iter()
                    .filter(|t| t["@type"] == "DnsManagement")
                {
                    eprintln!("{t}");
                }
            }
            Err(e) => eprintln!("task query: {e}"),
        }
    }

    eprintln!("\n── x:Domain/get after Automatic");
    let r = jmap
        .call(
            &auth,
            vec![call(
                "x:Domain/get",
                json!({"ids": [domain_id.clone()], "properties": ["dnsManagement"]}),
            )],
        )
        .await
        .unwrap();
    eprintln!("{}", r[0]);

    // Back to manual, server removed.
    let r = jmap
        .call(
            &auth,
            vec![
                call(
                    "x:Domain/set",
                    json!({"update": {domain_id: {"dnsManagement": {"@type": "Manual"}}}}),
                ),
                call("x:DnsServer/set", json!({"destroy": [server_id]})),
            ],
        )
        .await
        .unwrap();
    eprintln!("\n── revert: {} / {}", r[0], r[1]);

    let _ = Command::new("kill")
        .args(["-INT", &normal.id().to_string()])
        .status();
    std::thread::sleep(Duration::from_secs(2));
    let _ = normal.kill();
    let _ = normal.wait();
    let tail =
        std::fs::read_to_string(paths.stalwart_logs().join("normal.stdout")).unwrap_or_default();
    for line in tail
        .lines()
        .filter(|l| l.to_ascii_lowercase().contains("dns"))
        .take(20)
    {
        eprintln!("log: {line}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
