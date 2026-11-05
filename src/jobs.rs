//! Background jobs inside `serve`: DNS re-checks, host deliverability checks,
//! certificate renewal + push into the engine, served-certificate probe, and
//! session cleanup. Each job logs and carries on — never takes the edge down.

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

/// Renew when fewer than 30 days remain (or the cert is self-signed), then push
/// the certificate into the engine and verify what it serves.
pub async fn certificate_maintenance(app: &App) {
    let Some(cfg) = app.config() else { return };
    if !cfg.install_complete && app.db.get_kv("tls.acme_ok").ok().flatten().is_none() {
        return; // the installer drives the first issuance
    }
    let info = crate::edge::tls::disk_certificate_info(&app.paths);
    let needs = info
        .as_ref()
        .is_none_or(|i| i.self_signed || i.days_left < 30);
    if needs {
        let domains: Vec<String> = match app.engine() {
            Ok(e) => e
                .list_domains()
                .await
                .map(|d| d.into_iter().map(|x| x.name).collect())
                .unwrap_or_default(),
            Err(_) => vec![cfg.primary_domain.clone()],
        };
        let names =
            crate::edge::acme::wanted_names(&cfg.hostname, &domains, cfg.public_ipv4.as_deref())
                .await;
        match crate::edge::acme::issue(&app.paths, &app.db, &names, &cfg.admin_email).await {
            Ok(()) => {
                let _ = app.db.set_kv("tls.acme_ok", "true");
                tracing::info!("certificate issued for {}", names.join(", "));
            }
            Err(e) => {
                tracing::error!("certificate renewal failed: {e:#}");
                let _ = app.db.audit(
                    "system",
                    "certificate.renew_failed",
                    Some(&cfg.hostname),
                    Some(&format!("{e:#}")),
                );
                return;
            }
        }
    }
    push_certificate(app).await;
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
        loop {
            ops::host_checks(&a, true).await;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            certificate_maintenance(&a).await;
            let _ = a.db.purge_expired_sessions();
            tokio::time::sleep(Duration::from_secs(12 * 3600)).await;
        }
    });
}
