//! Engine upgrades with rollback (plan §7.6).
//!
//! snapshot → stop → swap the pinned binary → one start with Stalwart's
//! per-release flags (e.g. `STALWART_UPDATE_ROLE_PERMISSIONS`) → health gate →
//! normal restart → health gate. Any failure restores the previous binary
//! AND the data snapshot, then starts the old engine again.

use super::os;
use crate::app::App;
use anyhow::{Context, Result, bail};
use std::time::Duration;

/// Environment for the single post-upgrade start. Stalwart 1.0.3 asks admins to
/// restart once with this flag so missing role permissions are added; it is
/// harmless when nothing is missing, so Yunori always sets it for that start.
pub const UPGRADE_ENV: &[(&str, &str)] = &[("STALWART_UPDATE_ROLE_PERMISSIONS", "true")];

pub fn dropin_contents() -> String {
    let mut s =
        String::from("# Temporary — written and removed by `yunori-mail upgrade`.\n[Service]\n");
    for (k, v) in UPGRADE_ENV {
        s.push_str(&format!("Environment={k}={v}\n"));
    }
    s
}

fn dropin_path(app: &App) -> std::path::PathBuf {
    app.paths
        .systemd_dir()
        .join("stalwart.service.d")
        .join("yunori-upgrade.conf")
}

async fn healthy(app: &App, timeout: Duration) -> bool {
    let Ok(engine) = app.engine() else {
        return false;
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if engine.info().await.reachable {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub fn engine_is_current(app: &App) -> Result<bool> {
    let (_, sha) = os::engine_asset()?;
    let marker = app.paths.state_dir().join("engine.sha256");
    Ok(std::fs::read_to_string(marker)
        .map(|s| s.trim() == sha)
        .unwrap_or(false))
}

/// Upgrade the engine to the version pinned by this `yunori-mail` build.
pub async fn upgrade_engine(app: &App) -> Result<String> {
    if !os::is_root() {
        bail!("run upgrades as root");
    }
    if engine_is_current(app)? {
        return Ok(format!(
            "Stalwart {} is already current.",
            os::ENGINE.version
        ));
    }
    let p = &app.paths;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let snapshot = p.backups_dir().join(format!("pre-upgrade-{stamp}"));
    std::fs::create_dir_all(p.backups_dir())?;

    os::systemctl(&["stop", "stalwart"])?;
    os::run(
        "cp",
        &[
            "-a",
            &p.stalwart_data().to_string_lossy(),
            &snapshot.to_string_lossy(),
        ],
    )
    .context("snapshotting the engine data before upgrading")?;
    app.db.audit(
        "cli",
        "engine.upgrade_start",
        Some(os::ENGINE.version),
        Some(&snapshot.to_string_lossy()),
    )?;

    let result: Result<()> = async {
        os::install_engine_binary(p).await?;
        let dropin = dropin_path(app);
        std::fs::create_dir_all(dropin.parent().unwrap())?;
        std::fs::write(&dropin, dropin_contents())?;
        os::systemctl(&["daemon-reload"])?;
        os::systemctl(&["start", "stalwart"])?;
        let first = healthy(app, Duration::from_secs(120)).await;
        let _ = std::fs::remove_file(&dropin);
        os::systemctl(&["daemon-reload"])?;
        if !first {
            bail!("the new engine did not become healthy");
        }
        os::systemctl(&["restart", "stalwart"])?;
        if !healthy(app, Duration::from_secs(90)).await {
            bail!("the new engine did not come back after a normal restart");
        }
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            let mut cfg = app.config().context("missing config")?;
            cfg.stalwart_version = Some(os::ENGINE.version.to_string());
            app.save_config(&cfg)?;
            app.db
                .audit("cli", "engine.upgrade_done", Some(os::ENGINE.version), None)?;
            crate::jobs::push_certificate(app).await;
            Ok(format!(
                "Upgraded to Stalwart {}. The pre-upgrade snapshot is in {} (delete it once you are happy).",
                os::ENGINE.version,
                snapshot.display()
            ))
        }
        Err(e) => {
            // Roll back binary and data.
            let _ = os::systemctl(&["stop", "stalwart"]);
            let _ = std::fs::remove_file(dropin_path(app));
            let _ = os::systemctl(&["daemon-reload"]);
            if p.stalwart_bin_previous().exists() {
                std::fs::rename(p.stalwart_bin_previous(), p.stalwart_bin())?;
                let _ = std::fs::remove_file(p.state_dir().join("engine.sha256"));
            }
            let failed = p.backups_dir().join(format!("failed-upgrade-{stamp}"));
            let _ = std::fs::rename(p.stalwart_data(), &failed);
            std::fs::rename(&snapshot, p.stalwart_data()).context("restoring the data snapshot")?;
            os::systemctl(&["start", "stalwart"])?;
            let back = healthy(app, Duration::from_secs(90)).await;
            app.db.audit(
                "cli",
                "engine.upgrade_rolled_back",
                Some(os::ENGINE.version),
                Some(&format!("{e:#}")),
            )?;
            bail!(
                "upgrade failed ({e:#}) and was rolled back{}. The failed data is kept in {}.",
                if back {
                    "; the previous engine is running again"
                } else {
                    ", but the previous engine is not answering — run `yunori-mail doctor`"
                },
                failed.display()
            )
        }
    }
}

/// Rewrite the systemd units shipped with this build (no-op when unchanged).
pub fn refresh_units(app: &App) -> Result<bool> {
    let p = &app.paths;
    let changed = os::write_unit(p, "stalwart.service", &os::stalwart_unit(p))?
        | os::write_unit(p, "yunori-mail.service", &os::yunori_unit(p))?;
    if changed {
        os::systemctl(&["daemon-reload"])?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropin_sets_one_shot_flags() {
        let d = dropin_contents();
        assert!(d.starts_with("# Temporary"));
        assert!(d.contains("[Service]\nEnvironment=STALWART_UPDATE_ROLE_PERMISSIONS=true\n"));
    }

    #[test]
    fn current_engine_detection() {
        let app = App::for_tests();
        // No marker on disk in the test root → not current.
        assert!(!engine_is_current(&app).unwrap());
    }
}
