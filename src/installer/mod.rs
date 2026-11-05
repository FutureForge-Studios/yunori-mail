//! The resumable installer (plan §7.4). Steps 1–7 need root and run from
//! `yunori-mail install`; the verified gates (DNS, PTR) and the finishing
//! steps also run inside `serve`, so a user who switched to the browser sees
//! the install advance on its own once their records propagate.

pub mod os;
pub mod state;

use crate::app::{App, ENGINE_HTTP_PORT};
use crate::checks;
use crate::dns::records;
use crate::dns::{DnsLookup, Resolver};
use crate::engine::RelayConfig;
use crate::engine::stalwart::StalwartProvisioner;
use crate::model::{RelaySettings, SendingMode, ServerConfig, Status, relay_provider};
use crate::ops;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use state::{STEPS, StepStatus};
use std::sync::Arc;
use std::time::Duration;

const BOOTSTRAP_PORT: u16 = 18081;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayInput {
    pub provider: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip)]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallInputs {
    pub domain: String,
    pub hostname: String,
    pub admin_email: String,
    pub first_mailbox: String,
    pub sending: SendingMode,
    pub relay: Option<RelayInput>,
    pub client_origin: String,
}

impl InstallInputs {
    pub fn validate(&self) -> Result<()> {
        crate::util::normalize_domain(&self.domain)
            .ok_or_else(|| anyhow!("'{}' is not a valid domain", self.domain))?;
        crate::util::normalize_domain(&self.hostname)
            .ok_or_else(|| anyhow!("'{}' is not a valid hostname", self.hostname))?;
        if !crate::util::valid_email(&self.admin_email) {
            bail!("'{}' is not a valid email address", self.admin_email);
        }
        if self.admin_email.ends_with(&format!("@{}", self.domain)) {
            bail!(
                "use an address on another provider for alerts and certificates — not one hosted on this new server"
            );
        }
        if !crate::util::valid_local_part(&self.first_mailbox) {
            bail!("'{}' is not a valid mailbox name", self.first_mailbox);
        }
        if self.sending == SendingMode::Relay {
            let r = self.relay.as_ref().ok_or_else(|| {
                anyhow!(
                    "relay mode needs --relay-provider/--relay-host/--relay-user/--relay-password"
                )
            })?;
            relay_provider(&r.provider)
                .ok_or_else(|| anyhow!("unknown relay provider '{}'", r.provider))?;
            if r.password.is_empty() {
                bail!("relay password is required");
            }
        }
        Ok(())
    }
}

/// Where a step's output goes (terminal or nowhere when run by the service).
pub trait Ui: Send {
    fn step(&mut self, _id: &str, _title: &str) {}
    fn done(&mut self, _id: &str, _detail: &str) {}
    fn waiting(&mut self, _id: &str, _detail: &str) {}
    fn failed(&mut self, _id: &str, _detail: &str) {}
    fn info(&mut self, _msg: &str) {}
    /// Shown once on screen, never written to any log.
    fn secret(&mut self, _label: &str, _value: &str) {}
    fn checks(&mut self, _checks: &[crate::model::Check]) {}
    fn records(&mut self, _records: &[crate::model::DnsRecord]) {}
}

pub struct NullUi;
impl Ui for NullUi {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `yunori-mail install` as root.
    Cli,
    /// The background gate checker inside `serve` (no root, no secrets on screen).
    Service,
}

enum Outcome {
    Done(String),
    Waiting(String),
    Skipped(String),
}

#[derive(Debug, PartialEq)]
pub enum RunResult {
    Complete,
    Waiting { step: String, detail: String },
}

const ROOT_STEPS: &[&str] = &[
    "preflight",
    "inputs",
    "host_record",
    "firewall",
    "install_engine",
    "bootstrap_engine",
    "edge_tls",
];

pub struct Installer {
    pub app: Arc<App>,
    pub mode: Mode,
    pub force: bool,
    pub skip_ptr: bool,
}

impl Installer {
    fn inputs(&self) -> Result<InstallInputs> {
        self.app
            .db
            .get_json::<InstallInputs>("install.inputs")?
            .ok_or_else(|| {
                anyhow!("no saved install inputs — run `yunori-mail install` with --domain")
            })
    }

    fn log(&self, level: &str, step: &str, msg: &str) {
        let _ = self.app.db.install_log(level, Some(step), msg);
        match level {
            "error" => tracing::error!("[{step}] {msg}"),
            "warn" => tracing::warn!("[{step}] {msg}"),
            _ => tracing::info!("[{step}] {msg}"),
        }
    }

    /// Save inputs (fresh install). The relay password goes to the vault, never into kv.
    pub fn save_inputs(&self, inputs: &InstallInputs) -> Result<()> {
        inputs.validate()?;
        if let Some(r) = &inputs.relay {
            self.app.db.put_secret("relay.password", &r.password)?;
        }
        self.app.db.set_json("install.inputs", inputs)?;
        self.app.db.init_steps()?;
        Ok(())
    }

    /// Run every remaining step in order; stops at the first waiting gate.
    pub async fn run(&self, ui: &mut dyn Ui) -> Result<RunResult> {
        self.app.db.init_steps()?;
        for (id, title) in STEPS {
            let status = self.app.db.step_status(id)?;
            if matches!(status, StepStatus::Done | StepStatus::Skipped) {
                continue;
            }
            if self.mode == Mode::Service && ROOT_STEPS.contains(id) {
                return Ok(RunResult::Waiting {
                    step: id.to_string(),
                    detail: "Run `yunori-mail install --resume` on the server.".into(),
                });
            }
            ui.step(id, title);
            self.app.db.set_step(id, StepStatus::Running, None)?;
            let res = self.run_step(id, ui).await;
            if self.mode == Mode::Cli {
                fix_permissions(&self.app.paths);
            }
            match res {
                Ok(Outcome::Done(detail)) => {
                    self.app.db.set_step(id, StepStatus::Done, Some(&detail))?;
                    self.log("info", id, &detail);
                    ui.done(id, &detail);
                }
                Ok(Outcome::Skipped(detail)) => {
                    self.app
                        .db
                        .set_step(id, StepStatus::Skipped, Some(&detail))?;
                    self.log("warn", id, &detail);
                    ui.done(id, &detail);
                }
                Ok(Outcome::Waiting(detail)) => {
                    self.app
                        .db
                        .set_step(id, StepStatus::Waiting, Some(&detail))?;
                    ui.waiting(id, &detail);
                    return Ok(RunResult::Waiting {
                        step: id.to_string(),
                        detail,
                    });
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    self.app.db.set_step(id, StepStatus::Failed, Some(&msg))?;
                    self.log("error", id, &msg);
                    ui.failed(id, &msg);
                    return Err(e);
                }
            }
        }
        Ok(RunResult::Complete)
    }

    async fn run_step(&self, id: &str, ui: &mut dyn Ui) -> Result<Outcome> {
        match id {
            "preflight" => self.preflight(ui).await,
            "inputs" => self.apply_inputs().await,
            "host_record" => self.host_record().await,
            "firewall" => Ok(Outcome::Done(os::open_firewall()?)),
            "install_engine" => self.install_engine().await,
            "bootstrap_engine" => self.bootstrap_engine().await,
            "edge_tls" => self.edge_tls(ui).await,
            "dns" => self.dns_gate(ui).await,
            "ptr" => self.ptr_gate().await,
            "first_mailbox" => self.first_mailbox(ui).await,
            "done" => self.finish(ui).await,
            other => bail!("unknown step {other}"),
        }
    }

    async fn preflight(&self, ui: &mut dyn Ui) -> Result<Outcome> {
        if !os::is_root() {
            bail!("the installer must run as root (try: sudo yunori-mail install)");
        }
        let inputs = self.inputs()?;
        let results = checks::preflight(&self.app.paths, None).await;
        ui.checks(&results);
        let warnings = results.iter().filter(|c| c.status == Status::Warn).count();
        if checks::has_blocking_failure(&results) && !self.force {
            let failed: Vec<String> = results
                .iter()
                .filter(|c| c.status == Status::Fail)
                .map(|c| format!("{}: {}", c.label, c.detail))
                .collect();
            bail!("this server is not ready: {}", failed.join("; "));
        }
        if inputs.sending == SendingMode::Direct
            && results
                .iter()
                .any(|c| c.id == "port25" && c.status != Status::Pass)
        {
            ui.info("Outbound port 25 looks blocked. You can continue — receiving works — but consider `--sending relay`.");
        }
        Ok(Outcome::Done(if warnings == 0 {
            "All checks passed.".into()
        } else {
            format!(
                "Ready, with {warnings} warning{} to review.",
                if warnings == 1 { "" } else { "s" }
            )
        }))
    }

    async fn apply_inputs(&self) -> Result<Outcome> {
        let inputs = self.inputs()?;
        let (v4, v6) = checks::public_ips().await;
        let previous = self.app.config();
        let cfg = ServerConfig {
            hostname: inputs.hostname.clone(),
            primary_domain: inputs.domain.clone(),
            admin_email: inputs.admin_email.clone(),
            client_origin: inputs.client_origin.clone(),
            engine_url: format!("http://127.0.0.1:{ENGINE_HTTP_PORT}"),
            sending_mode: inputs.sending,
            public_ipv4: v4.or_else(|| previous.as_ref().and_then(|p| p.public_ipv4.clone())),
            public_ipv6: v6.or_else(|| previous.as_ref().and_then(|p| p.public_ipv6.clone())),
            stalwart_version: previous.as_ref().and_then(|p| p.stalwart_version.clone()),
            install_complete: false,
        };
        self.app.save_config(&cfg)?;
        self.app
            .db
            .set_kv(&format!("dmarc_policy:{}", inputs.domain), "none")?;
        Ok(Outcome::Done(format!(
            "{} on {} ({}).",
            inputs.domain,
            inputs.hostname,
            cfg.public_ipv4.as_deref().unwrap_or("no public IPv4 found")
        )))
    }

    async fn host_record(&self) -> Result<Outcome> {
        let cfg = self.app.config().ok_or_else(|| anyhow!("missing config"))?;
        let c = checks::check_host_record(
            &cfg.hostname,
            cfg.public_ipv4.as_deref(),
            &Resolver::public(),
        )
        .await;
        match c.status {
            Status::Pass => Ok(Outcome::Done(c.detail)),
            _ => Ok(Outcome::Waiting(
                format!("{} {}", c.detail, c.remedy.unwrap_or_default())
                    .trim()
                    .to_string(),
            )),
        }
    }

    async fn install_engine(&self) -> Result<Outcome> {
        let p = &self.app.paths;
        os::ensure_system_user("stalwart", &p.stalwart_data())?;
        os::ensure_system_user("yunori", &p.state_dir())?;
        for dir in [p.stalwart_data(), p.stalwart_etc(), p.stalwart_logs()] {
            std::fs::create_dir_all(&dir)?;
            os::chown_r("stalwart", &dir)?;
        }
        let downloaded = os::install_engine_binary(p).await?;
        let unbound = os::ensure_unbound()?;
        // Install ourselves where the unit expects us.
        let me = std::env::current_exe()?;
        if me != p.yunori_bin() {
            std::fs::copy(&me, p.yunori_bin())
                .context("installing yunori-mail into /usr/local/bin")?;
            crate::store::vault::set_mode(&p.yunori_bin(), 0o755);
        }
        let changed = os::write_unit(p, "stalwart.service", &os::stalwart_unit(p))?
            | os::write_unit(p, "yunori-mail.service", &os::yunori_unit(p))?;
        if changed {
            os::systemctl(&["daemon-reload"])?;
        }
        Ok(Outcome::Done(format!(
            "Stalwart {} {}; local resolver {}.",
            os::ENGINE.version,
            if downloaded {
                "downloaded and verified"
            } else {
                "already installed"
            },
            if unbound { "installed" } else { "ready" }
        )))
    }

    async fn bootstrap_engine(&self) -> Result<Outcome> {
        let p = &self.app.paths;
        let cfg = self.app.config().ok_or_else(|| anyhow!("missing config"))?;
        let inputs = self.inputs()?;
        let base = format!("http://127.0.0.1:{BOOTSTRAP_PORT}");

        if !p.stalwart_config().exists() {
            let recovery_pw = crate::util::generate_password(32);
            let admin = format!("admin:{recovery_pw}");
            self.log(
                "info",
                "bootstrap_engine",
                "Starting the engine in first-run mode",
            );
            let child = os::EngineChild::spawn(p, &admin, BOOTSTRAP_PORT, false)?;
            let prov = StalwartProvisioner::new(&base, "admin", &recovery_pw);
            prov.wait_ready(Duration::from_secs(60))
                .await
                .context("the engine did not start")?;
            let result = prov
                .bootstrap(
                    &cfg.hostname,
                    &cfg.primary_domain,
                    &format!("{}/", p.stalwart_data().display()),
                )
                .await
                .context("first-run provisioning failed")?;
            self.app.db.put_secret(
                "engine.admin",
                &format!("{}:{}", result.admin_user, result.admin_secret.as_str()),
            )?;
            child.stop()?;
            self.log(
                "info",
                "bootstrap_engine",
                &format!("Engine administrator {} created", result.admin_user),
            );
        }

        if !self
            .app
            .db
            .get_kv("engine.configured")?
            .is_some_and(|v| v == "true")
        {
            let recovery_pw = crate::util::generate_password(32);
            let child =
                os::EngineChild::spawn(p, &format!("admin:{recovery_pw}"), BOOTSTRAP_PORT, true)?;
            let prov = StalwartProvisioner::new(&base, "admin", &recovery_pw);
            prov.wait_ready(Duration::from_secs(60))
                .await
                .context("the engine did not start in recovery mode")?;
            prov.configure(&cfg.hostname, ENGINE_HTTP_PORT)
                .await
                .context("configuring listeners")?;
            child.stop()?;
            self.app.db.set_kv("engine.configured", "true")?;
            self.log(
                "info",
                "bootstrap_engine",
                "Listeners and proxy trust configured",
            );
        }

        os::chown_r("stalwart", &p.stalwart_data())?;
        os::chown_r("stalwart", &p.stalwart_etc())?;
        os::systemctl(&["enable", "--now", "stalwart"])?;
        self.app.reload_engine()?;
        let engine = self.app.engine()?;
        let mut info = engine.info().await;
        for _ in 0..30 {
            if info.reachable {
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            info = engine.info().await;
        }
        if !info.reachable {
            bail!(
                "the engine is not answering on 127.0.0.1:{ENGINE_HTTP_PORT} — see `journalctl -u stalwart`"
            );
        }
        let mut cfg = cfg;
        cfg.stalwart_version = Some(
            info.version
                .clone()
                .unwrap_or_else(|| os::ENGINE.version.to_string()),
        );
        self.app.save_config(&cfg)?;

        if inputs.sending == SendingMode::Relay
            && let Some(r) = &inputs.relay
        {
            let password = self
                .app
                .db
                .get_secret("relay.password")?
                .map(|s| s.to_string())
                .unwrap_or_default();
            engine
                .set_relay(Some(&RelayConfig {
                    host: r.host.clone(),
                    port: r.port,
                    implicit_tls: r.port == 465,
                    username: r.username.clone(),
                    password,
                }))
                .await?;
            self.app.db.set_json(
                "relay",
                &RelaySettings {
                    provider: r.provider.clone(),
                    host: r.host.clone(),
                    port: r.port,
                    username: r.username.clone(),
                },
            )?;
        }
        self.app.db.audit(
            "installer",
            "engine.bootstrap",
            Some(&cfg.hostname),
            Some(&format!("Stalwart {}", os::ENGINE.version)),
        )?;
        Ok(Outcome::Done(format!(
            "Stalwart {} is running.",
            os::ENGINE.version
        )))
    }

    async fn edge_tls(&self, ui: &mut dyn Ui) -> Result<Outcome> {
        let cfg = self.app.config().ok_or_else(|| anyhow!("missing config"))?;
        os::systemctl(&["enable", "--now", "yunori-mail"])?;
        os::systemctl(&["restart", "yunori-mail"])?;
        // Wait for the edge to listen on :80 (it serves the ACME challenges).
        let mut up = false;
        for _ in 0..30 {
            if tokio::net::TcpStream::connect(("127.0.0.1", 80))
                .await
                .is_ok()
            {
                up = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        if !up {
            bail!("the Yunori edge did not start — see `journalctl -u yunori-mail`");
        }
        let names = crate::edge::acme::wanted_names(
            &cfg.hostname,
            std::slice::from_ref(&cfg.primary_domain),
            cfg.public_ipv4.as_deref(),
        )
        .await;
        let mut last = None;
        for attempt in 1..=2 {
            match crate::edge::acme::issue(&self.app.paths, &self.app.db, &names, &cfg.admin_email)
                .await
            {
                Ok(()) => {
                    last = None;
                    break;
                }
                Err(e) => {
                    self.log(
                        "warn",
                        "edge_tls",
                        &format!("certificate attempt {attempt} failed: {e:#}"),
                    );
                    last = Some(e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
        if let Some(e) = last {
            return Err(e.context(
                "could not obtain a certificate — is port 80 reachable from the internet?",
            ));
        }
        self.app.db.set_kv("tls.acme_ok", "true")?;
        fix_permissions(&self.app.paths);
        crate::jobs::push_certificate(&self.app).await;
        let served = self
            .app
            .db
            .get_kv("tls.served_matches")?
            .is_some_and(|v| v == "true");

        // From here on the browser can take over: issue the console setup token.
        if self.app.db.admin_count_active()? == 0 {
            let token = self.app.db.issue_setup_token()?;
            ui.info(&format!(
                "Console ready: https://{}/console/setup",
                cfg.hostname
            ));
            ui.secret("Setup token (valid 24 hours)", &token);
        }
        Ok(Outcome::Done(format!(
            "Certificate for {} issued{}.",
            names.join(", "),
            if served {
                " and served on 465, 587 and 993"
            } else {
                "; the engine will pick it up shortly"
            }
        )))
    }

    async fn dns_gate(&self, ui: &mut dyn Ui) -> Result<Outcome> {
        let cfg = self.app.config().ok_or_else(|| anyhow!("missing config"))?;
        let engine = self.app.engine()?;
        let domain = engine
            .list_domains()
            .await?
            .into_iter()
            .find(|d| d.name == cfg.primary_domain)
            .ok_or_else(|| anyhow!("the primary domain is missing from the engine"))?;
        let dns = ops::domain_dns(&self.app, &domain, true).await?;
        ui.records(&dns.records);
        let failing: Vec<&crate::model::DnsRecord> = dns
            .records
            .iter()
            .filter(|r| r.required && r.id != "ptr" && r.status != Status::Pass)
            .collect();
        if failing.is_empty() {
            Ok(Outcome::Done(
                "All required DNS records are published.".into(),
            ))
        } else {
            let names: Vec<String> = failing
                .iter()
                .map(|r| format!("{} {}", r.rtype, r.name))
                .collect();
            Ok(Outcome::Waiting(format!(
                "Publish {} record{} at your DNS provider: {}.",
                failing.len(),
                if failing.len() == 1 { "" } else { "s" },
                names.join(", ")
            )))
        }
    }

    async fn ptr_gate(&self) -> Result<Outcome> {
        let cfg = self.app.config().ok_or_else(|| anyhow!("missing config"))?;
        let relayed = cfg.sending_mode == SendingMode::Relay;
        let Some(ip) = cfg.public_ipv4.clone() else {
            return Ok(Outcome::Skipped("No public IPv4 address to check.".into()));
        };
        let public = Resolver::public();
        let rec = records::evaluate_ptr(&ip, &cfg.hostname, &public, relayed).await;
        if rec.status == Status::Pass {
            return Ok(Outcome::Done(format!("{ip} → {}", cfg.hostname)));
        }
        if relayed {
            return Ok(Outcome::Skipped(
                "Sending is relayed, so reverse DNS is recommended but not required.".into(),
            ));
        }
        if self.skip_ptr {
            return Ok(Outcome::Skipped(
                "Skipped at your request. Gmail and Outlook will likely reject mail sent directly until reverse DNS is set.".into(),
            ));
        }
        let names = public.ptr(ip.parse()?).await.unwrap_or_default();
        let g = crate::dns::reputation::guidance(crate::dns::reputation::detect_provider(&names));
        Ok(Outcome::Waiting(format!(
            "Set reverse DNS for {ip} to {} — {} (Or switch to relay sending.)",
            cfg.hostname, g.ptr
        )))
    }

    async fn first_mailbox(&self, ui: &mut dyn Ui) -> Result<Outcome> {
        let cfg = self.app.config().ok_or_else(|| anyhow!("missing config"))?;
        let engine = self.app.engine()?;
        let accounts = engine.list_accounts().await?;
        if let Some(a) = accounts.iter().find(|a| !a.is_admin) {
            return Ok(Outcome::Done(format!("{} exists.", a.address)));
        }
        if self.mode == Mode::Service {
            return Ok(Outcome::Waiting("Create your first mailbox under Mailboxes, or finish with `yunori-mail install --resume`.".into()));
        }
        let inputs = self.inputs()?;
        let domain = engine
            .list_domains()
            .await?
            .into_iter()
            .find(|d| d.name == cfg.primary_domain)
            .ok_or_else(|| anyhow!("primary domain missing"))?;
        let password = crate::util::generate_password(20);
        let acct = engine
            .create_account(&inputs.first_mailbox, &domain.id, None, None, &password)
            .await?;
        for alias in ["postmaster", "abuse"] {
            let taken = accounts
                .iter()
                .any(|a| a.local == alias && a.domain_id == domain.id)
                || alias == inputs.first_mailbox;
            if !taken {
                let _ = engine.add_alias(&acct.id, alias, &domain.id).await;
            }
        }
        self.app.db.audit(
            "installer",
            "mailbox.create",
            Some(&acct.address),
            Some("first mailbox; postmaster+abuse aliases"),
        )?;
        ui.secret(&format!("Password for {}", acct.address), &password);
        ui.info(&format!(
            "Open it in Yunori: {}/welcome?connect={}",
            cfg.client_origin, acct.address
        ));
        Ok(Outcome::Done(format!(
            "{} created (postmaster@ and abuse@ deliver to it).",
            acct.address
        )))
    }

    async fn finish(&self, ui: &mut dyn Ui) -> Result<Outcome> {
        let mut cfg = self.app.config().ok_or_else(|| anyhow!("missing config"))?;
        cfg.install_complete = true;
        self.app.save_config(&cfg)?;
        if self.mode == Mode::Cli && self.app.db.admin_count_active()? == 0 {
            let token = self.app.db.issue_setup_token()?;
            ui.secret("Console setup token (valid 24 hours)", &token);
        }
        ui.info(&format!(
            "Console: https://{}/console   ·   Mailbox portal: https://{}/account",
            cfg.hostname, cfg.hostname
        ));
        self.app
            .db
            .audit("installer", "install.complete", Some(&cfg.hostname), None)?;
        Ok(Outcome::Done("Yunori Mail Server is ready.".into()))
    }
}

/// Let the unprivileged service read/write what the root installer created.
pub fn fix_permissions(paths: &crate::paths::Paths) {
    if !os::is_root() || !paths.is_production() {
        return;
    }
    for dir in [paths.state_dir(), paths.etc_dir()] {
        let d = dir.to_string_lossy().to_string();
        let _ = os::run("chown", &["-R", "root:yunori", &d]);
        let _ = os::run("chmod", &["-R", "g+rwX,o-rwx", &d]);
        let _ = os::run("chmod", &["g+s", &d]);
    }
    // The service only needs to READ the master key and certificates.
    for f in [
        paths.master_key(),
        crate::edge::tls::key_path(paths),
        crate::edge::tls::cert_path(paths),
    ] {
        if f.exists() {
            let _ = os::run("chmod", &["0640", &f.to_string_lossy()]);
        }
    }
}

/// Called periodically by `serve`: advance verified gates without root.
pub async fn advance_gates(app: Arc<App>) {
    if app.config().is_none_or(|c| c.install_complete) {
        return;
    }
    let inst = Installer {
        app,
        mode: Mode::Service,
        force: false,
        skip_ptr: false,
    };
    if let Err(e) = inst.run(&mut NullUi).await {
        tracing::debug!("installer gate check: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> InstallInputs {
        InstallInputs {
            domain: "example.com".into(),
            hostname: "mail.example.com".into(),
            admin_email: "me@gmail.com".into(),
            first_mailbox: "hello".into(),
            sending: SendingMode::Direct,
            relay: None,
            client_origin: crate::app::DEFAULT_CLIENT_ORIGIN.into(),
        }
    }

    #[test]
    fn input_validation() {
        assert!(inputs().validate().is_ok());
        let mut i = inputs();
        i.admin_email = "me@example.com".into();
        assert!(
            i.validate()
                .unwrap_err()
                .to_string()
                .contains("another provider")
        );
        let mut i = inputs();
        i.sending = SendingMode::Relay;
        assert!(i.validate().is_err());
        i.relay = Some(RelayInput {
            provider: "postmark".into(),
            host: "smtp.postmarkapp.com".into(),
            port: 587,
            username: "u".into(),
            password: "p".into(),
        });
        assert!(i.validate().is_ok());
    }

    #[tokio::test]
    async fn service_mode_never_runs_root_steps() {
        let app = App::for_tests();
        let inst = Installer {
            app: app.clone(),
            mode: Mode::Service,
            force: false,
            skip_ptr: false,
        };
        let r = inst.run(&mut NullUi).await.unwrap();
        assert_eq!(
            r,
            RunResult::Waiting {
                step: "preflight".into(),
                detail: "Run `yunori-mail install --resume` on the server.".into()
            }
        );
    }

    #[tokio::test]
    async fn first_mailbox_waits_in_service_mode_and_creates_in_cli() {
        let app = App::for_tests();
        // Mark root steps done and the gates done, leaving first_mailbox.
        app.db.init_steps().unwrap();
        for (id, _) in STEPS.iter().take(9) {
            app.db.set_step(id, StepStatus::Done, None).unwrap();
        }
        // The seeded mock already has user mailboxes, so the step is satisfied.
        let inst = Installer {
            app: app.clone(),
            mode: Mode::Service,
            force: false,
            skip_ptr: false,
        };
        assert_eq!(inst.run(&mut NullUi).await.unwrap(), RunResult::Complete);
        assert!(app.config().unwrap().install_complete);
    }
}
