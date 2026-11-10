//! Live test of the queue manager and the login-abuse controls against a REAL
//! Stalwart binary (ignored by default; same harness as `live_tests.rs`):
//!
//! ```sh
//! YUNORI_STALWART_BIN=/path/to/stalwart \
//!   cargo test live_engine_queue -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Gets mail stuck by routing all outbound mail to a relay on a closed local
//! port, then lists, retries (also while paused), and cancels it. Produces
//! failed IMAP and SMTP sign-ins from 127.0.0.2/127.0.0.3 (Stalwart never bans
//! 127.0.0.1, but does ban the rest of 127/8) and checks the ban, the unblock
//! and the allowlist. Binds the real mail ports — run on a throwaway host or in
//! a private network namespace.

use super::live_tests::{
    BOOT_PORT, DOMAIN, Guard, HOST, HTTP_PORT, root, spawn_normal, step, wait_reachable,
};
use super::stalwart::{StalwartEngine, StalwartProvisioner};
use super::{BanRate, EAuthBanPolicy, Engine, EngineError, RcptState, RelayConfig};
use crate::installer::os::EngineChild;
use base64::Engine as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpSocket, TcpStream};

const USER: &str = "bob";
const PASS: &str = "Bob-Strong-Pass-123!";

#[derive(Debug)]
struct AcceptAny;
impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

type Tls = tokio_rustls::client::TlsStream<TcpStream>;

/// Implicit-TLS connection to a local mail port from `source` (an address on lo).
async fn connect(source: &str, port: u16) -> std::io::Result<BufReader<Tls>> {
    let sock = TcpSocket::new_v4()?;
    sock.bind(format!("{source}:0").parse().unwrap())?;
    let tcp = sock
        .connect(format!("127.0.0.1:{port}").parse().unwrap())
        .await?;
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(AcceptAny))
    .with_no_client_auth();
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(HOST.to_string()).unwrap(), tcp)
        .await?;
    Ok(BufReader::new(tls))
}

/// Read one (possibly multi-line) SMTP reply; returns the last line.
async fn smtp_reply(s: &mut BufReader<Tls>) -> std::io::Result<String> {
    loop {
        let mut line = String::new();
        if s.read_line(&mut line).await? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        if line.len() < 4 || line.as_bytes()[3] != b'-' {
            return Ok(line.trim_end().to_string());
        }
    }
}

async fn smtp(s: &mut BufReader<Tls>, cmd: &str) -> std::io::Result<String> {
    s.get_mut()
        .write_all(format!("{cmd}\r\n").as_bytes())
        .await?;
    smtp_reply(s).await
}

fn auth_plain(user: &str, pass: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("\0{user}\0{pass}"))
}

/// Submit a message on 465 as the test user; returns the final DATA reply.
async fn submit(rcpts: &[&str]) -> String {
    let mut s = connect("127.0.0.1", 465).await.expect("connect 465");
    assert!(smtp_reply(&mut s).await.unwrap().starts_with("220"));
    smtp(&mut s, "EHLO client.live.test").await.unwrap();
    let r = smtp(
        &mut s,
        &format!(
            "AUTH PLAIN {}",
            auth_plain(&format!("{USER}@{DOMAIN}"), PASS)
        ),
    )
    .await
    .unwrap();
    assert!(r.starts_with("235"), "AUTH: {r}");
    let from = format!("{USER}@{DOMAIN}");
    assert!(
        smtp(&mut s, &format!("MAIL FROM:<{from}>"))
            .await
            .unwrap()
            .starts_with("250")
    );
    for r in rcpts {
        let a = smtp(&mut s, &format!("RCPT TO:<{r}>")).await.unwrap();
        assert!(a.starts_with("250"), "RCPT {r}: {a}");
    }
    assert!(smtp(&mut s, "DATA").await.unwrap().starts_with("354"));
    let body = format!(
        "From: {from}\r\nTo: {}\r\nSubject: stuck in the queue\r\n\r\nhello\r\n.",
        rcpts.join(", ")
    );
    let r = smtp(&mut s, &body).await.unwrap();
    let _ = smtp(&mut s, "QUIT").await;
    r
}

/// One failed IMAP LOGIN from `source`. `Ok(true)`: the server answered NO;
/// `Err`: the connection was refused/dropped (blocked).
async fn bad_imap_login(source: &str, n: usize) -> std::io::Result<bool> {
    let mut s = connect(source, 993).await?;
    let mut greet = String::new();
    if s.read_line(&mut greet).await? == 0 {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    s.get_mut()
        .write_all(format!("a1 LOGIN {USER}@{DOMAIN} \"wrong-{n}\"\r\n").as_bytes())
        .await?;
    let mut line = String::new();
    if s.read_line(&mut line).await? == 0 {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    Ok(line.contains("NO"))
}

async fn bad_smtp_auth(source: &str, n: usize) -> std::io::Result<String> {
    let mut s = connect(source, 465).await?;
    smtp_reply(&mut s).await?;
    smtp(&mut s, "EHLO client.live.test").await?;
    smtp(
        &mut s,
        &format!(
            "AUTH PLAIN {}",
            auth_plain(&format!("{USER}@{DOMAIN}"), &format!("wrong-{n}"))
        ),
    )
    .await
}

async fn blocked(engine: &StalwartEngine, ip: &str) -> Option<super::EBlockedIp> {
    engine
        .blocked_ips()
        .await
        .expect("blocked ips")
        .into_iter()
        .find(|b| b.address == ip)
}

async fn wait_for<F, Fut>(what: &str, secs: u64, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f().await {
        assert!(Instant::now() < deadline, "timed out waiting: {what}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test]
#[ignore]
async fn live_engine_queue_and_login_abuse() {
    let Some(paths) = root() else {
        eprintln!("YUNORI_STALWART_BIN not set — skipping live test");
        return;
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    let recovery_pw = "live-recovery-0123456789abcdef";
    let prov = StalwartProvisioner::new(
        &format!("http://127.0.0.1:{BOOT_PORT}"),
        "admin",
        recovery_pw,
    );

    step("1. provision an engine (bootstrap → recovery → normal)");
    let child =
        EngineChild::spawn(&paths, &format!("admin:{recovery_pw}"), BOOT_PORT, false).unwrap();
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
    let child =
        EngineChild::spawn(&paths, &format!("admin:{recovery_pw}"), BOOT_PORT, true).unwrap();
    prov.wait_ready(Duration::from_secs(60)).await.unwrap();
    prov.configure(HOST, HTTP_PORT).await.unwrap();
    child.stop().unwrap();
    let _normal = Guard(Some(spawn_normal(&paths)));
    let engine = StalwartEngine::new(
        &format!("http://127.0.0.1:{HTTP_PORT}"),
        &boot.admin_user,
        &boot.admin_secret,
    );
    wait_reachable(&engine).await;
    let domain = engine
        .list_domains()
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.name == DOMAIN)
        .unwrap();
    engine
        .create_account(USER, &domain.id, None, None, PASS)
        .await
        .expect("create the sending mailbox");

    step("2. sign-in ban policy: Stalwart's default, then Yunori's");
    let stock = engine.auth_ban_policy().await.expect("x:Security/get");
    eprintln!("engine default: {stock:?}");
    assert_eq!(
        stock,
        EAuthBanPolicy {
            rate: Some(crate::ipaccess::STALWART_DEFAULT_RATE),
            ban_for_ms: None
        },
        "Stalwart 1.0.3 default: 100 failures a day, permanent ban"
    );
    engine
        .set_auth_ban_policy(&crate::ipaccess::YUNORI_AUTH_BAN)
        .await
        .expect("set policy");
    assert_eq!(
        engine.auth_ban_policy().await.unwrap(),
        crate::ipaccess::YUNORI_AUTH_BAN
    );
    // A low threshold for the test itself.
    let test_policy = EAuthBanPolicy {
        rate: Some(BanRate {
            count: 5,
            period_ms: 3_600_000,
        }),
        ban_for_ms: Some(600_000),
    };
    engine.set_auth_ban_policy(&test_policy).await.unwrap();

    step("3. failed IMAP sign-ins from 127.0.0.2 get it blocked; 127.0.0.1 never is");
    let mut answered = 0;
    for n in 0..8 {
        match bad_imap_login("127.0.0.2", n).await {
            Ok(no) => {
                assert!(no, "a wrong password is refused");
                answered += 1;
            }
            Err(e) => eprintln!("attempt {n}: connection dropped ({e})"),
        }
    }
    assert!(answered <= 6, "the address was cut off after the threshold");
    let b = blocked(&engine, "127.0.0.2")
        .await
        .expect("127.0.0.2 is blocked");
    eprintln!("blocked: {b:?}");
    assert_eq!(b.reason, "authFailure");
    assert!(b.expires_at.is_some(), "ban period set → the block expires");
    assert!(
        bad_imap_login("127.0.0.2", 99).await.is_err(),
        "a blocked address is dropped"
    );
    for n in 0..8 {
        assert!(bad_imap_login("127.0.0.1", n).await.unwrap());
    }
    assert!(blocked(&engine, "127.0.0.1").await.is_none());

    step("4. failed SMTP AUTH from 127.0.0.3 counts too");
    for n in 0..8 {
        let r = bad_smtp_auth("127.0.0.3", n).await;
        eprintln!("SMTP AUTH {n}: {r:?}");
    }
    wait_for("127.0.0.3 blocked", 10, || async {
        blocked(&engine, "127.0.0.3").await.is_some()
    })
    .await;

    step("5. unblock (destroy + ReloadBlockedIps) lets the address back in");
    engine.unblock_ip(&b.id).await.expect("unblock");
    assert!(blocked(&engine, "127.0.0.2").await.is_none());
    wait_for("127.0.0.2 can connect again", 10, || async {
        bad_imap_login("127.0.0.2", 100).await.is_ok()
    })
    .await;
    assert!(matches!(
        engine.unblock_ip(&b.id).await,
        Err(EngineError::NotFound)
    ));

    step("6. allowlist: an allowed network is never blocked");
    assert!(
        matches!(
            engine.allow_ip("10.0.0.0/33", None).await,
            Err(EngineError::Rejected(_))
        ),
        "the engine refuses a bad network too"
    );
    let allowed = engine
        .allow_ip("127.0.0.0/30", Some("live test"))
        .await
        .expect("allow");
    assert_eq!(allowed.address, "127.0.0.0/30");
    assert_eq!(allowed.reason.as_deref(), Some("live test"));
    assert!(
        engine
            .allowed_ips()
            .await
            .unwrap()
            .iter()
            .any(|a| a.id == allowed.id)
    );
    for n in 0..10 {
        assert!(
            bad_imap_login("127.0.0.2", 200 + n).await.unwrap(),
            "allowlisted: still answered after {n} failures"
        );
    }
    assert!(blocked(&engine, "127.0.0.2").await.is_none());
    engine
        .remove_allowed_ip(&allowed.id)
        .await
        .expect("remove allowlist entry");
    assert!(engine.allowed_ips().await.unwrap().is_empty());
    for n in 0..8 {
        let _ = bad_imap_login("127.0.0.2", 300 + n).await;
    }
    assert!(
        blocked(&engine, "127.0.0.2").await.is_some(),
        "banned again once off the allowlist"
    );

    step("7. queue: mail to a relay on a closed port gets stuck");
    engine
        .set_relay(Some(&RelayConfig {
            host: "127.0.0.1".into(),
            port: 9,
            implicit_tls: false,
            username: "relay".into(),
            password: "relay-pass-not-real".into(),
        }))
        .await
        .expect("route everything to a closed port");
    let r = submit(&["alice@remote.example", "carol@remote.example"]).await;
    assert!(r.starts_with("250"), "DATA: {r}");
    wait_for("the message is queued with a failure", 30, || async {
        engine.queue().await.unwrap().first().is_some_and(|m| {
            m.recipients.len() == 2 && m.recipients.iter().all(|r| r.retry_count >= 1)
        })
    })
    .await;
    let q = engine.queue().await.unwrap();
    let id = q[0].id.clone();
    let m = q.iter().find(|m| m.id == id).unwrap();
    eprintln!("queued: {m:#?}");
    assert_eq!(m.sender.as_deref(), Some(&*format!("{USER}@{DOMAIN}")));
    assert!(m.size.unwrap_or(0) > 0);
    assert_eq!(m.recipients.len(), 2);
    let r0 = &m.recipients[0];
    assert_eq!(r0.state, RcptState::TemporaryFailure);
    assert!(r0.next_retry.is_some() && r0.expires_at.is_some());
    let (kind, problem) = crate::mailqueue::explain(r0, true).expect("explained");
    eprintln!("explained: {kind}: {problem}");
    assert_eq!(kind, "network");
    let attempts = |q: &[super::EQueued]| {
        q.iter()
            .find(|m| m.id == id)
            .map(|m| m.recipients[0].retry_count)
    };
    let before = r0.retry_count;

    step("8. retry now");
    assert_eq!(
        engine.queue_retry(std::slice::from_ref(&id)).await.unwrap(),
        1
    );
    wait_for("another attempt", 15, || async {
        attempts(&engine.queue().await.unwrap()).unwrap_or(0) > before
    })
    .await;
    assert!(matches!(
        engine.queue_retry(&["nope0000".to_string()]).await,
        Err(EngineError::NotFound)
    ));

    step("9. paused: a retry waits; resumed: it runs");
    engine.queue_set_paused(true).await.expect("pause");
    let before = attempts(&engine.queue().await.unwrap()).unwrap();
    engine.queue_retry(std::slice::from_ref(&id)).await.unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        attempts(&engine.queue().await.unwrap()),
        Some(before),
        "no attempt while paused"
    );
    engine.queue_set_paused(false).await.expect("resume");
    wait_for("the attempt after resuming", 15, || async {
        attempts(&engine.queue().await.unwrap()).unwrap_or(0) > before
    })
    .await;

    step("10. cancel");
    engine.queue_cancel(&id).await.expect("cancel");
    assert!(engine.queue().await.unwrap().iter().all(|m| m.id != id));
    assert!(matches!(
        engine.queue_cancel(&id).await,
        Err(EngineError::NotFound)
    ));
    engine.set_relay(None).await.unwrap();
    eprintln!("\nLIVE QUEUE + ABUSE TEST PASSED against the real engine");
}
