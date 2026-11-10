//! Background jobs inside `serve`: DNS re-checks, host deliverability checks,
//! certificate renewal + push into the engine, served-certificate probe, the
//! daily update check, proactive alerts (`alerts`), and session cleanup.
//! Each job logs and carries on — never takes the edge down.

use crate::app::App;
use crate::ops;
use std::sync::Arc;
use std::time::Duration;

pub async fn refresh_all_dns(app: &App) {
    let Ok(engine) = app.engine() else { return };
    let Ok(domains) = engine.list_domains().await else {
        return;
    };
    for d in domains {
        if let Err(e) = ops::domain_dns(app, &d, true).await {
            tracing::warn!("DNS check for {} failed: {e:#}", d.name);
        }
    }
}

/// Automatic HTTPS: issue, renew or extend the certificate when it is time
/// (see `edge::certs`; it backs off on failure, so a short tick is cheap),
/// then make sure the engine serves the same certificate (`recheck`: verify
/// what it serves even if it matched last time).
pub async fn certificate_maintenance(app: &App, recheck: bool) {
    use crate::edge::certs::Outcome;
    match crate::edge::certs::ensure(app, false).await {
        // `ensure` already pushed a new certificate into the engine.
        Outcome::Issued(_) => {}
        Outcome::Failed { problem, .. } => {
            let _ = app.db.audit(
                "system",
                "certificate.renew_failed",
                app.config().map(|c| c.hostname).as_deref(),
                Some(&problem.sentence()),
            );
        }
        _ => {
            let served = app.db.get_kv("tls.served_matches").ok().flatten();
            if recheck || served.as_deref() != Some("true") {
                push_certificate(app).await;
            }
        }
    }
}

/// Install the disk certificate into the engine if what it serves differs.
pub async fn push_certificate(app: &App) {
    let Some(cfg) = app.config() else { return };
    let Ok(engine) = app.engine() else { return };
    let (matches, _) = crate::edge::tls::verify_served(&app.paths, &cfg.hostname).await;
    if matches {
        let _ = app.db.set_kv("tls.served_matches", "true");
        return;
    }
    if let Some((c, k)) = crate::edge::tls::read_cert_files(&app.paths) {
        match engine.install_certificate(&c, &k).await {
            Ok(()) => {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let (ok, detail) = crate::edge::tls::verify_served(&app.paths, &cfg.hostname).await;
                let _ = app
                    .db
                    .set_kv("tls.served_matches", if ok { "true" } else { "false" });
                if !ok {
                    tracing::warn!(
                        "engine still serves an old certificate: {}",
                        detail.join("; ")
                    );
                }
            }
            Err(e) => tracing::error!("could not install certificate into the engine: {e}"),
        }
    }
}

pub fn spawn_all(app: Arc<App>) {
    if app.dev {
        return; // no network side effects in development
    }
    let a = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            refresh_all_dns(&a).await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        // Daily: is there a newer release on yunori.email? Only noted for the console.
        tokio::time::sleep(Duration::from_secs(90)).await;
        loop {
            crate::updates::check(&a).await;
            tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
        }
    });
    // Proactive alerts: evaluate every 10 minutes, notify on changes.
    tokio::spawn(crate::alerts::run_forever(app.clone()));
    let a = app.clone();
    tokio::spawn(async move {
        loop {
            ops::host_checks(&a, true).await;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        // Every minute: keep a paused queue paused across engine restarts, and
        // (until it has worked once) apply the sign-in ban policy — the upgrade
        // path for servers installed before Yunori set it.
        tokio::time::sleep(Duration::from_secs(15)).await;
        let mut policy_done = false;
        loop {
            crate::mailqueue::reassert_pause(&a).await;
            if !policy_done {
                match crate::ipaccess::ensure_auth_ban_policy(&a).await {
                    Ok(changed) => {
                        policy_done = true;
                        if changed {
                            tracing::info!("sign-in ban policy set on the engine");
                        }
                    }
                    Err(e) => tracing::debug!("sign-in ban policy not applied yet: {e:#}"),
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        let mut ticks: u64 = 0;
        loop {
            // Hourly: re-verify what the engine serves, and tidy sessions.
            let hourly = ticks % 12 == 0;
            certificate_maintenance(&a, hourly).await;
            if hourly {
                let _ = a.db.purge_expired_sessions();
            }
            ticks += 1;
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}
