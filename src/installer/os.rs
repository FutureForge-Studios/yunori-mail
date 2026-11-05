//! Host operations the installer performs. Every function is idempotent and
//! logs what it changes; nothing here runs unless `install` is invoked as root.

use crate::paths::Paths;
use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use std::process::{Command, Stdio};

/// Stalwart builds this `yunori-mail` release is tested against. Upstream
/// publishes no per-asset checksums, so these are pinned by Yunori
/// (measured over TLS from the GitHub release, 2026-11-05).
pub struct EngineRelease {
    pub version: &'static str,
    pub x86_64_sha256: &'static str,
    pub aarch64_sha256: &'static str,
}

pub const ENGINE: EngineRelease = EngineRelease {
    version: "1.0.3",
    x86_64_sha256: "8ae481ff410cd09dd53d7f49f528dcad35d90b4b2ffb0c49c9ae23aaf43c4831",
    aarch64_sha256: "b1e49a941ac38bb5e4e5586933dc736bdf5014b1892105a98e4cdf0bd829c461",
};

pub fn engine_asset() -> Result<(String, &'static str)> {
    let (triple, sha) = match std::env::consts::ARCH {
        "x86_64" => ("x86_64-unknown-linux-gnu", ENGINE.x86_64_sha256),
        "aarch64" => ("aarch64-unknown-linux-gnu", ENGINE.aarch64_sha256),
        other => bail!("no engine build for {other}"),
    };
    Ok((
        format!(
            "https://github.com/stalwartlabs/stalwart/releases/download/v{}/stalwart-{triple}.tar.gz",
            ENGINE.version
        ),
        sha,
    ))
}

pub fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {cmd}"))?;
    if !out.status.success() {
        bail!(
            "`{cmd} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn is_root() -> bool {
    run("id", &["-u"]).map(|s| s.trim() == "0").unwrap_or(false)
}

/// Download the pinned engine archive, verify its SHA-256, and install the binary.
/// Returns false when the right binary is already in place.
pub async fn install_engine_binary(paths: &Paths) -> Result<bool> {
    let bin = paths.stalwart_bin();
    let marker = paths.state_dir().join("engine.sha256");
    let (url, sha) = engine_asset()?;
    if bin.exists()
        && std::fs::read_to_string(&marker)
            .map(|s| s.trim() == sha)
            .unwrap_or(false)
    {
        return Ok(false);
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("downloading {url}"))?;
    if !resp.status().is_success() {
        bail!("download failed: HTTP {}", resp.status());
    }
    let bytes = resp.bytes().await?;
    let got = crate::util::sha256_hex(&bytes);
    if got != sha {
        bail!(
            "checksum mismatch for the engine download (expected {sha}, got {got}) — refusing to install"
        );
    }
    let work = paths.state_dir().join("download");
    std::fs::create_dir_all(&work)?;
    let archive = work.join("stalwart.tar.gz");
    std::fs::write(&archive, &bytes)?;
    run(
        "tar",
        &[
            "-xzf",
            &archive.to_string_lossy(),
            "-C",
            &work.to_string_lossy(),
            "stalwart",
        ],
    )?;
    if bin.exists() {
        std::fs::copy(&bin, paths.stalwart_bin_previous())?;
    }
    if let Some(dir) = bin.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let staged = bin.with_extension("new");
    std::fs::copy(work.join("stalwart"), &staged)?;
    crate::store::vault::set_mode(&staged, 0o755);
    std::fs::rename(&staged, &bin)?;
    std::fs::write(&marker, sha)?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(true)
}

pub fn ensure_system_user(name: &str, home: &Path) -> Result<()> {
    if run("id", &["-u", name]).is_ok() {
        return Ok(());
    }
    run(
        "useradd",
        &[
            "--system",
            "--home-dir",
            &home.to_string_lossy(),
            "--no-create-home",
            "--shell",
            "/usr/sbin/nologin",
            name,
        ],
    )?;
    Ok(())
}

pub fn chown_r(user: &str, path: &Path) -> Result<()> {
    run(
        "chown",
        &["-R", &format!("{user}:{user}"), &path.to_string_lossy()],
    )?;
    Ok(())
}

pub fn stalwart_unit(paths: &Paths) -> String {
    format!(
        r#"# Managed by yunori-mail — changes are overwritten on upgrade.
[Unit]
Description=Stalwart mail engine (Yunori Mail Server)
Conflicts=postfix.service sendmail.service exim4.service
ConditionPathExists={config}
After=network-online.target unbound.service
Wants=network-online.target

[Service]
Type=notify
LimitNOFILE=65536
KillMode=process
KillSignal=SIGINT
Restart=on-failure
RestartSec=5
ExecStart={bin} --config={config}
SyslogIdentifier=stalwart
User=stalwart
Group=stalwart
AmbientCapabilities=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths={data} {etc} {logs}

[Install]
WantedBy=multi-user.target
"#,
        bin = paths.stalwart_bin().display(),
        config = paths.stalwart_config().display(),
        data = paths.stalwart_data().display(),
        etc = paths.stalwart_etc().display(),
        logs = paths.stalwart_logs().display(),
    )
}

pub fn yunori_unit(paths: &Paths) -> String {
    format!(
        r#"# Managed by yunori-mail — changes are overwritten on upgrade.
[Unit]
Description=Yunori Mail Server (console, HTTPS edge, jobs)
After=network-online.target stalwart.service
Wants=network-online.target

[Service]
Type=simple
ExecStart={bin} serve
Restart=on-failure
RestartSec=3
User=yunori
Group=yunori
AmbientCapabilities=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths={state}
ReadOnlyPaths={etc}
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
"#,
        bin = paths.yunori_bin().display(),
        state = paths.state_dir().display(),
        etc = paths.etc_dir().display(),
    )
}

pub fn write_unit(paths: &Paths, name: &str, contents: &str) -> Result<bool> {
    let path = paths.systemd_dir().join(name);
    if std::fs::read_to_string(&path)
        .map(|c| c == contents)
        .unwrap_or(false)
    {
        return Ok(false);
    }
    std::fs::create_dir_all(paths.systemd_dir())?;
    std::fs::write(&path, contents)?;
    Ok(true)
}

pub fn systemctl(args: &[&str]) -> Result<String> {
    run("systemctl", args)
}

pub fn is_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Install the local validating resolver (Spamhaus refuses public resolvers).
pub fn ensure_unbound() -> Result<bool> {
    if Path::new("/usr/sbin/unbound").exists() {
        let _ = systemctl(&["enable", "--now", "unbound"]);
        return Ok(false);
    }
    run("apt-get", &["update", "-qq"])?;
    let status = Command::new("apt-get")
        .args(["install", "-y", "-qq", "unbound"])
        .env("DEBIAN_FRONTEND", "noninteractive")
        .status()?;
    if !status.success() {
        bail!("apt-get install unbound failed");
    }
    systemctl(&["enable", "--now", "unbound"])?;
    Ok(true)
}

pub const PUBLIC_PORTS: &[u16] = &[25, 80, 443, 465, 587, 993, 4190];

/// SSH ports currently listening (so we never lock the operator out).
pub fn ssh_ports() -> Vec<u16> {
    let out = run("ss", &["-H", "-ltnp"]).unwrap_or_default();
    let mut ports: Vec<u16> = out
        .lines()
        .filter(|l| l.contains("sshd"))
        .filter_map(|l| l.split_whitespace().nth(3))
        .filter_map(|addr| addr.rsplit(':').next().and_then(|p| p.parse().ok()))
        .collect();
    ports.sort_unstable();
    ports.dedup();
    if ports.is_empty() {
        ports.push(22);
    }
    ports
}

#[derive(Debug, PartialEq)]
pub enum Firewall {
    Ufw { active: bool },
    Firewalld,
    None,
}

pub fn detect_firewall() -> Firewall {
    if Path::new("/usr/sbin/ufw").exists() {
        let active = run("ufw", &["status"])
            .map(|s| s.contains("Status: active"))
            .unwrap_or(false);
        return Firewall::Ufw { active };
    }
    if is_active("firewalld") {
        return Firewall::Firewalld;
    }
    Firewall::None
}

/// Open the mail ports in an active host firewall. Returns a human summary.
pub fn open_firewall() -> Result<String> {
    match detect_firewall() {
        Firewall::Ufw { active: true } => {
            for p in ssh_ports() {
                run("ufw", &["allow", &format!("{p}/tcp")])?;
            }
            for p in PUBLIC_PORTS {
                run("ufw", &["allow", &format!("{p}/tcp")])?;
            }
            Ok(format!("Opened {} in ufw.", ports_text()))
        }
        Firewall::Firewalld => {
            for p in PUBLIC_PORTS {
                run(
                    "firewall-cmd",
                    &["--permanent", &format!("--add-port={p}/tcp")],
                )?;
            }
            run("firewall-cmd", &["--reload"])?;
            Ok(format!("Opened {} in firewalld.", ports_text()))
        }
        Firewall::Ufw { active: false } | Firewall::None => Ok(format!(
            "No active host firewall, so nothing to open. If your provider has a cloud firewall or security group, allow TCP {}.",
            ports_text()
        )),
    }
}

fn ports_text() -> String {
    PUBLIC_PORTS
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A Stalwart process started by the installer for bootstrap/recovery mode.
pub struct EngineChild {
    child: std::process::Child,
}

impl EngineChild {
    /// `recovery`: pass `STALWART_RECOVERY_MODE=1` (store exists). Without it, a
    /// missing config file puts Stalwart in bootstrap mode.
    pub fn spawn(paths: &Paths, admin: &str, port: u16, recovery: bool) -> Result<Self> {
        let mut cmd = Command::new(paths.stalwart_bin());
        cmd.arg(format!("--config={}", paths.stalwart_config().display()))
            .env("STALWART_RECOVERY_ADMIN", admin)
            .env("STALWART_RECOVERY_MODE_PORT", port.to_string())
            .env_remove("STALWART_RECOVERY_MODE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if recovery {
            cmd.env("STALWART_RECOVERY_MODE", "1");
        }
        // Run as the stalwart user so created files have the right owner.
        // Drop to the `stalwart` user when it exists (always, after the installer's
        // install_engine step); dev/test runs without that user keep the caller's uid.
        if is_root() && run("id", &["-u", "stalwart"]).is_ok() {
            use std::os::unix::process::CommandExt;
            let uid: u32 = run("id", &["-u", "stalwart"])?.trim().parse()?;
            let gid: u32 = run("id", &["-g", "stalwart"])?.trim().parse()?;
            cmd.uid(uid).gid(gid);
        }
        let child = cmd.spawn().context("starting the engine")?;
        Ok(Self { child })
    }

    pub fn stop(mut self) -> Result<()> {
        // SIGINT is Stalwart's graceful stop.
        let _ = run("kill", &["-INT", &self.child.id().to_string()]);
        for _ in 0..50 {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        self.child
            .kill()
            .map_err(|e| anyhow!("could not stop the engine: {e}"))?;
        let _ = self.child.wait();
        Ok(())
    }
}

impl Drop for EngineChild {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_hardened_and_point_at_paths() {
        let p = Paths::with_root("/");
        let s = stalwart_unit(&p);
        assert!(s.contains("ExecStart=/usr/local/bin/stalwart --config=/etc/stalwart/config.json"));
        assert!(s.contains("ProtectSystem=strict"));
        assert!(s.contains("LimitNOFILE=65536"));
        let y = yunori_unit(&p);
        assert!(y.contains("ExecStart=/usr/local/bin/yunori-mail serve"));
        assert!(y.contains("User=yunori"));
    }

    #[test]
    fn engine_asset_is_pinned() {
        let (url, sha) = engine_asset().unwrap();
        assert!(url.contains(&format!("v{}", ENGINE.version)));
        assert_eq!(sha.len(), 64);
    }
}
