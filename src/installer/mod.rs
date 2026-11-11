//! The resumable installer (plan §7.4).
//!
//! Two phases over one persisted step list ([`state::STEPS`]):
//!
//! - **Terminal** ([`state::TERMINAL_STEPS`], root, `yunori-mail install`):
//!   preflight, the one question (where the console opens), firewall, the
//!   pinned engine, its first run, the units and the HTTPS edge. Then it
//!   prints the one-time setup link and exits.
//! - **Browser onboarding** (everything else): mail domain and hostname, how
//!   mail goes out, and the verified gates (host record, certificate, DNS,
//!   reverse DNS), then the first mailbox. The service runs these steps (no
//!   root: nothing here needs it) every minute and whenever the console's
//!   wizard submits an answer, so every gate is checked for real, never
//!   self-attested.
//!
//! Installs started with the older all-in-the-terminal flow keep their saved
//! answers and step states and continue from where they are.

pub mod os;
pub mod panel;
pub mod state;
pub mod upgrade;

use crate::app::{App, ENGINE_HTTP_PORT};
use crate::checks;
use crate::dns::records;
use crate::dns::{DnsLookup, Resolver};
use crate::edge::certs;
use crate::engine::RelayConfig;
use crate::engine::stalwart::StalwartProvisioner;
use crate::model::{RelaySettings, SendingMode, ServerConfig, Status, relay_provider};
use crate::ops;
use anyhow::{Context, Result, anyhow, bail};
use panel::{PanelChoice, PanelKind};
use serde::{Deserialize, Serialize};
use state::{STEPS, StepStatus, TERMINAL_STEPS};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const BOOTSTRAP_PORT: u16 = 18081;
/// Set when the host-record gate passes: the next certificate attempt skips
/// any backoff left from before the record pointed here.
const KV_CERT_NOW: &str = "install.certificate_now";
/// "Skip for now" on reverse DNS (the console's button, or `--skip-ptr`).
pub const KV_SKIP_PTR: &str = "install.skip_ptr";
pub const KV_INPUTS: &str = "install.inputs";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayInput {
    pub provider: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Provider region (Amazon SES derives its SMTP host from it).
    #[serde(default)]
    pub region: Option<String>,
    #[serde(skip)]
    pub password: String,
}

fn yes() -> bool {
    true
}

/// The answers the old terminal wizard asked for. Now: given as flags (the
/// terminal applies them), or entered in the console's onboarding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstallInputs {
    pub domain: String,
    pub hostname: String,
    pub admin_email: String,
    /// Local part of the first mailbox the terminal creates (`--first-mailbox`);
    /// empty when the console creates it.
    #[serde(default)]
    pub first_mailbox: String,
    pub sending: SendingMode,
    /// Relay given as flags (the console's relay lives in kv `relay`).
    pub relay: Option<RelayInput>,
    pub client_origin: String,
    /// The sending mode has been chosen (always, for answers from flags or an
    /// older release; in the browser, once its sending step is answered).
    #[serde(default = "yes")]
    pub sending_confirmed: bool,
}

impl PartialEq for RelayInput {
    fn eq(&self, o: &Self) -> bool {
        self.provider == o.provider
            && self.host == o.host
            && self.port == o.port
            && self.username == o.username
            && self.region == o.region
    }
}

impl RelayInput {
    /// Validated relay settings (host derived from the region where the provider has one).
    /// Per-domain provider records are added later in the console.
    pub fn settings(&self) -> std::result::Result<RelaySettings, crate::relay::FieldError> {
        crate::relay::build_settings(
            &crate::relay::RelayDraft {
                provider: self.provider.clone(),
                host: self.host.clone(),
                port: self.port,
                username: self.username.clone(),
                region: self.region.clone(),
                spf_include: None,
                identities: None,
            },
            None,
            &[],
        )
    }
}

/// Why an identity answer was refused, and which field to point at.
#[derive(Debug)]
pub struct IdentityError {
    pub field: &'static str,
    pub message: String,
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Domain, hostname and the outside address, normalised. Shared by the flags
/// and the console so both say the same thing about the same mistake.
pub fn check_identity(
    domain: &str,
    hostname: &str,
    admin_email: &str,
) -> std::result::Result<(String, String, String), IdentityError> {
    let err = |field, message: String| IdentityError { field, message };
    let domain = crate::util::normalize_domain(domain).ok_or_else(|| {
        err(
            "domain",
            format!(
                "'{}' doesn't look like a domain. Try something like example.com.",
                domain.trim()
            ),
        )
    })?;
    let hostname = crate::util::normalize_domain(hostname).ok_or_else(|| {
        err(
            "hostname",
            format!("'{}' is not a valid hostname.", hostname.trim()),
        )
    })?;
    if hostname != domain && !hostname.ends_with(&format!(".{domain}")) {
        // Allowed (a shared mail host), but the usual mistake is a typo.
        tracing::debug!("mail hostname {hostname} is outside {domain}");
    }
    let admin_email = admin_email.trim().to_ascii_lowercase();
    if !crate::util::valid_email(&admin_email) {
        return Err(err(
            "adminEmail",
            format!("'{admin_email}' is not an email address."),
        ));
    }
    if admin_email.ends_with(&format!("@{domain}")) {
        return Err(err(
            "adminEmail",
            format!(
                "Use an address that does not depend on this server (not @{domain}): certificate and alert mail must reach you even if this server is down."
            ),
        ));
    }
    Ok((domain, hostname, admin_email))
}

impl InstallInputs {
    /// Domain, hostname, outside address and first mailbox (what every flow has).
    pub fn validate_identity(&self) -> Result<()> {
        check_identity(&self.domain, &self.hostname, &self.admin_email)
            .map_err(|e| anyhow!("{e}"))?;
        if !self.first_mailbox.is_empty() && !crate::util::valid_local_part(&self.first_mailbox) {
            bail!("'{}' is not a valid mailbox name", self.first_mailbox);
        }
        Ok(())
    }

    /// Everything, as given on the command line.
    pub fn validate(&self) -> Result<()> {
        self.validate_identity()?;
        if self.sending == SendingMode::Relay {
            let r = self.relay.as_ref().ok_or_else(|| {
                anyhow!(
                    "relay mode needs --relay-provider/--relay-host/--relay-user/--relay-password"
                )
            })?;
            relay_provider(&r.provider)
                .ok_or_else(|| anyhow!("unknown relay provider '{}'", r.provider))?;
            r.settings().map_err(|e| anyhow!("{}", e.message))?;
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
    /// The console's one-time setup link (it carries the setup token).
    /// `note` explains a caveat, e.g. the certificate is still pending.
    fn setup_link(&mut self, _url: &str, _note: Option<&str>) {}
    /// The first mailbox now exists (its password went through `secret`).
    fn mailbox(&mut self, _address: &str) {}
}

pub struct NullUi;
impl Ui for NullUi {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `yunori-mail install` as root.
    Cli,
    /// The service (`serve`): the browser onboarding. No root, no secrets on screen.
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

/// What the service says about a terminal step it cannot run.
pub const RUN_IN_TERMINAL: &str = "Run `sudo yunori-mail install` on the server.";

pub struct Installer {
    pub app: Arc<App>,
    pub mode: Mode,
    pub force: bool,
    pub skip_ptr: bool,
    /// Preflight results the wizard already showed (so step 1 doesn't run them twice).
    pub preflight_results: Option<Vec<crate::model::Check>>,
    /// Terminal runs without answers given as flags stop after the root
    /// steps: everything else is answered in the browser.
    pub terminal_only: bool,
    /// The first certificate attempt of a terminal run skips the backoff.
    force_certificate: AtomicBool,
}

impl Installer {
    pub fn new(app: Arc<App>, mode: Mode) -> Self {
        Self {
            app,
            mode,
            force: false,
            skip_ptr: false,
            preflight_results: None,
            terminal_only: false,
            force_certificate: AtomicBool::new(mode == Mode::Cli),
        }
    }

    fn inputs(&self) -> Result<Option<InstallInputs>> {
        self.app.db.get_json::<InstallInputs>(KV_INPUTS)
    }

    fn config(&self) -> Result<ServerConfig> {
        self.app
            .config()
            .ok_or_else(|| anyhow!("missing config — run `sudo yunori-mail install`"))
    }

    fn log(&self, level: &str, step: &str, msg: &str) {
        let _ = self.app.db.install_log(level, Some(step), msg);
        if self.mode == Mode::Cli {
            return; // The terminal UI already printed it, in plain words.
        }
        match level {
            "error" => tracing::error!("[{step}] {msg}"),
            "warn" => tracing::warn!("[{step}] {msg}"),
            _ => tracing::info!("[{step}] {msg}"),
        }
    }

    /// Save answers given as flags. The relay password goes to the vault, never into kv.
    pub fn save_inputs(&self, inputs: &InstallInputs) -> Result<()> {
        inputs.validate()?;
        if let Some(r) = &inputs.relay {
            self.app.db.put_secret("relay.password", &r.password)?;
        }
        self.app.db.set_json(KV_INPUTS, inputs)?;
        self.app.db.init_steps()?;
        Ok(())
    }

    /// Run every remaining step in order; stops at the first waiting gate.
    pub async fn run(&self, ui: &mut dyn Ui) -> Result<RunResult> {
        let _turn = match self.mode {
            Mode::Cli => self.app.install_run.lock().await,
            Mode::Service => match self.app.install_run.try_lock() {
                Ok(g) => g,
                // Another check is running right now; it does the same work.
                Err(_) => {
                    return Ok(RunResult::Waiting {
                        step: String::new(),
                        detail: "A check is already running.".into(),
                    });
                }
            },
        };
        self.app.db.init_steps()?;
        for (id, title) in STEPS {
            let status = self.app.db.step_status(id)?;
            if matches!(status, StepStatus::Done | StepStatus::Skipped) {
                continue;
            }
            if self.terminal_only && !TERMINAL_STEPS.contains(id) {
                return Ok(RunResult::Waiting {
                    step: id.to_string(),
                    detail: "Continue in the browser.".into(),
                });
            }
            if self.mode == Mode::Service && TERMINAL_STEPS.contains(id) {
                return Ok(RunResult::Waiting {
                    step: id.to_string(),
                    detail: RUN_IN_TERMINAL.into(),
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
            "panel" => self.panel().await,
            "firewall" => Ok(Outcome::Done(os::open_firewall()?)),
            "install_engine" => self.install_engine().await,
            "bootstrap_engine" => self.bootstrap_engine().await,
            "edge_tls" => self.edge_tls().await,
            "inputs" => self.identity().await,
            "sending" => self.sending().await,
            "host_record" => self.host_record().await,
            "certificate" => self.certificate().await,
            "dns" => self.dns_gate(ui).await,
            "ptr" => self.ptr_gate().await,
            "first_mailbox" => self.first_mailbox(ui).await,
            "done" => self.finish(ui).await,
            other => bail!("unknown step {other}"),
        }
    }

    // ── terminal ─────────────────────────────────────────────────────────

    async fn preflight(&self, ui: &mut dyn Ui) -> Result<Outcome> {
        if !os::is_root() {
            bail!("the installer must run as root (try: sudo yunori-mail install)");
        }
        let results = match &self.preflight_results {
            // Already on screen from the wizard.
            Some(r) => r.clone(),
            None => {
                let r = checks::preflight(&self.app.paths, None).await;
                ui.checks(&r);
                r
            }
        };
        let warnings = results.iter().filter(|c| c.status == Status::Warn).count();
        if checks::has_blocking_failure(&results) && !self.force {
            let failed: Vec<String> = results
                .iter()
                .filter(|c| c.status == Status::Fail)
                .map(|c| format!("{}: {}", c.label, c.detail))
                .collect();
            bail!("this server is not ready: {}", failed.join("; "));
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

    /// Where the console opens (decided by the terminal's one question), and
    /// the configuration the engine starts from: the answers given as flags,
    /// an earlier release's answers, or names derived from the console's
    /// address that the browser onboarding replaces.
    async fn panel(&self) -> Result<Outcome> {
        let choice = self
            .app
            .db
            .get_json::<PanelChoice>(panel::KV)?
            .ok_or_else(|| {
                anyhow!("choose where the console opens: run `sudo yunori-mail install`")
            })?;
        let (v4, v6) = checks::public_ips().await;
        let previous = self.app.config();
        let mut cfg = match (&previous, self.inputs()?) {
            (Some(c), _) => c.clone(),
            (None, Some(i)) => config_from_inputs(&i),
            (None, None) => {
                let ip = v4.as_deref().or(v6.as_deref()).and_then(|s| s.parse().ok());
                let (hostname, domain) = panel::placeholders(&choice.host, ip);
                ServerConfig {
                    hostname,
                    primary_domain: domain,
                    admin_email: String::new(),
                    client_origin: crate::app::DEFAULT_CLIENT_ORIGIN.into(),
                    engine_url: format!("http://127.0.0.1:{ENGINE_HTTP_PORT}"),
                    sending_mode: SendingMode::Direct,
                    public_ipv4: None,
                    public_ipv6: None,
                    stalwart_version: None,
                    install_complete: false,
                    panel_host: None,
                }
            }
        };
        cfg.public_ipv4 = v4.or_else(|| previous.as_ref().and_then(|p| p.public_ipv4.clone()));
        cfg.public_ipv6 = v6.or_else(|| previous.as_ref().and_then(|p| p.public_ipv6.clone()));
        cfg.panel_host = Some(choice.host.clone());
        self.app.save_config(&cfg)?;
        if let Some(dir) = crate::edge::acme::env_directory() {
            // The service starts without this environment: remember the CA.
            crate::edge::acme::save_directory(&self.app.db, &dir)?;
        }
        if self
            .app
            .db
            .get_kv(&dmarc_key(&cfg.primary_domain))?
            .is_none()
        {
            self.app
                .db
                .set_kv(&dmarc_key(&cfg.primary_domain), "none")?;
        }
        Ok(Outcome::Done(format!(
            "The console opens at {}.",
            panel::console_url(&choice.host)
        )))
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
        let resolver = os::ensure_unbound()?;
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
            "Stalwart {} {}; {resolver}.",
            os::ENGINE.version,
            if downloaded {
                "downloaded and verified"
            } else {
                "already installed"
            },
        )))
    }

    async fn bootstrap_engine(&self) -> Result<Outcome> {
        let p = &self.app.paths;
        let cfg = self.config()?;
        let base = format!("http://127.0.0.1:{BOOTSTRAP_PORT}");

        // A first run interrupted after the engine wrote its config but before
        // its administrator was sealed (lost response, Ctrl-C) can never be
        // driven: nobody knows that password. Nothing but the bootstrap lives
        // in the engine yet (it was never configured), so start it over.
        let configured = self
            .app
            .db
            .get_kv("engine.configured")?
            .is_some_and(|v| v == "true");
        if p.stalwart_config().exists()
            && !configured
            && self.app.db.get_secret("engine.admin")?.is_none()
        {
            self.log(
                "warn",
                "bootstrap_engine",
                "The engine's first run did not finish; starting it again",
            );
            std::fs::remove_file(p.stalwart_config())?;
            if p.stalwart_data().exists() {
                std::fs::remove_dir_all(p.stalwart_data())?;
                std::fs::create_dir_all(p.stalwart_data())?;
                os::chown_r("stalwart", &p.stalwart_data())?;
            }
        }

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
            // The hostname and domain may be placeholders until the browser
            // onboarding names them (it renames the domain in place).
            let result = prov
                .bootstrap(
                    &cfg.hostname,
                    &cfg.primary_domain,
                    &format!("{}/", p.stalwart_data().display()),
                    &format!("{}/", p.stalwart_logs().display()),
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
        // Ban addresses after repeated failed sign-ins (Stalwart's own default
        // is far looser and its bans never expire). Idempotent.
        if crate::ipaccess::ensure_auth_ban_policy(&self.app).await? {
            self.log(
                "info",
                "bootstrap_engine",
                "Addresses with 20 failed sign-ins in an hour are blocked for a day",
            );
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

    async fn edge_tls(&self) -> Result<Outcome> {
        os::systemctl(&["enable", "--now", "yunori-mail"])?;
        os::systemctl(&["restart", "yunori-mail"])?;
        // Wait for the edge to listen on :80 (ACME challenges) and :443.
        let mut up = false;
        for _ in 0..30 {
            if tokio::net::TcpStream::connect(("127.0.0.1", 80))
                .await
                .is_ok()
                && tokio::net::TcpStream::connect(("127.0.0.1", 443))
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
        Ok(Outcome::Done(
            "The console and HTTPS edge are running on ports 80 and 443.".into(),
        ))
    }

    /// The console's own certificate, asked for once by the terminal right
    /// after the edge starts (through `certs::ensure`, like every other
    /// certificate). With an sslip.io name that the CA cannot validate (port
    /// 80 blocked), the console falls back to the bare IP address and its
    /// temporary self-signed certificate; the browser onboarding gets the
    /// mail certificate once the cause is fixed.
    pub async fn console_certificate(&self, ui: &mut dyn Ui) -> Result<()> {
        let mut cfg = self.config()?;
        let host = cfg.console_host().to_string();
        if panel::kind(&host) == PanelKind::Ip {
            ui.info(&format!(
                "The console uses a temporary self-signed certificate at {}: your browser asks once whether to continue.",
                panel::url_host(&host)
            ));
            return Ok(());
        }
        ui.step(
            "console_certificate",
            &format!("Get a certificate for {host}"),
        );
        let outcome = certs::ensure(&self.app, true).await;
        match outcome {
            certs::Outcome::Current | certs::Outcome::Issued(_) => {
                ui.done(
                    "certificate",
                    &format!("{host} has a trusted certificate; the browser shows no warning."),
                );
                wait_until_served(&self.app.paths, &host).await;
            }
            certs::Outcome::Failed { problem, .. }
            | certs::Outcome::Later {
                problem: Some(problem),
                ..
            } => {
                ui.waiting("certificate", &problem.sentence());
                let ip = cfg.public_ipv4.clone().or_else(|| cfg.public_ipv6.clone());
                match (panel::kind(&host), ip) {
                    (PanelKind::Sslip, Some(ip)) => {
                        cfg.panel_host = Some(ip.clone());
                        self.app.save_config(&cfg)?;
                        self.app
                            .db
                            .set_step("panel", StepStatus::Done, Some(&format!(
                                "The console opens at {} (temporary self-signed certificate: {host} could not get one yet).",
                                panel::console_url(&ip)
                            )))?;
                        ui.info(&format!(
                            "Using this server's address instead: {}. Your browser asks once whether to continue (temporary self-signed certificate).",
                            panel::console_url(&ip)
                        ));
                    }
                    _ => ui.info(
                        "Until the certificate is issued your browser will warn about the console; it switches to the real certificate by itself.",
                    ),
                }
            }
            certs::Outcome::Later { .. } | certs::Outcome::Busy => {}
            certs::Outcome::NotConfigured => {}
        }
        Ok(())
    }

    /// Print the console's one-time setup link (terminal only, while nobody
    /// has claimed the console yet). The token is shown once, never logged.
    pub fn offer_setup_link(&self, ui: &mut dyn Ui) -> Result<bool> {
        if self.mode != Mode::Cli || self.app.db.admin_count_active()? > 0 {
            return Ok(false);
        }
        let cfg = self.config()?;
        let token = self.app.db.issue_setup_token()?;
        let note = (panel::kind(cfg.console_host()) == PanelKind::Ip).then_some(
            "The console uses a temporary self-signed certificate on this address: your browser asks once whether to continue.",
        );
        ui.setup_link(&setup_link(cfg.console_host(), &token), note);
        Ok(true)
    }

    // ── browser onboarding ───────────────────────────────────────────────

    /// The mail domain, the mail hostname and the outside address for
    /// certificates and alerts, applied to the engine.
    async fn identity(&self) -> Result<Outcome> {
        let Some(inputs) = self.inputs()? else {
            return Ok(Outcome::Waiting(
                "Choose your mail domain and the mail server's hostname in the console.".into(),
            ));
        };
        let detail = apply_identity(&self.app, &inputs, "installer").await?;
        Ok(Outcome::Done(detail))
    }

    async fn sending(&self) -> Result<Outcome> {
        let Some(inputs) = self.inputs()? else {
            return Ok(Outcome::Waiting("Choose your mail domain first.".into()));
        };
        if !inputs.sending_confirmed {
            return Ok(Outcome::Waiting(
                "Choose how this server sends mail: directly, or through a relay provider.".into(),
            ));
        }
        let mut cfg = self.config()?;
        // A relay given as flags: apply it once (the console's relay form
        // applies its own as it is saved).
        if inputs.sending == SendingMode::Relay
            && self.app.db.get_json::<RelaySettings>("relay")?.is_none()
            && let Some(r) = &inputs.relay
        {
            let password = self
                .app
                .db
                .get_secret("relay.password")?
                .map(|s| s.to_string())
                .unwrap_or_default();
            let settings = r.settings().map_err(|e| anyhow!("{}", e.message))?;
            self.app
                .engine()?
                .set_relay(Some(&RelayConfig {
                    host: settings.host.clone(),
                    port: settings.port,
                    implicit_tls: settings.port == 465,
                    username: settings.username.clone(),
                    password,
                }))
                .await?;
            self.app.db.set_json("relay", &settings)?;
            cfg.sending_mode = SendingMode::Relay;
            self.app.save_config(&cfg)?;
        }
        Ok(Outcome::Done(match cfg.sending_mode {
            SendingMode::Relay => format!(
                "Mail goes out through {}.",
                ops::relay_settings(&self.app)
                    .and_then(|r| relay_provider(&r.provider).map(|p| p.label.to_string()))
                    .unwrap_or_else(|| "a relay provider".into())
            ),
            SendingMode::Direct => "This server delivers mail itself.".into(),
        }))
    }

    async fn host_record(&self) -> Result<Outcome> {
        let cfg = self.config()?;
        // With a connected DNS provider, publish the host record first.
        match crate::dns_publish::publish_host_record(&self.app, "install").await {
            Ok(Some(msg)) => self.log("info", "host_record", &msg),
            Ok(None) => {}
            Err(e) => self.log("warn", "host_record", &format!("DNS provider: {e}")),
        }
        let c = checks::host_record_now(
            &cfg.hostname,
            cfg.public_ipv4.as_deref(),
            cfg.public_ipv6.as_deref(),
        )
        .await;
        if c.status != Status::Pass {
            return Ok(Outcome::Waiting(
                format!("{} {}", c.detail, c.remedy.unwrap_or_default())
                    .trim()
                    .to_string(),
            ));
        }
        // The same checks the certificate step makes (stale IPv6, CAA), now,
        // while the person is still at their DNS provider.
        let directory = crate::edge::acme::directory(&self.app.db);
        if let Err(p) = certs::precheck(
            &cfg.hostname,
            cfg.public_ipv4.as_deref(),
            cfg.public_ipv6.as_deref(),
            &crate::dns::Truth::for_name(&cfg.hostname).await,
            crate::edge::acme::caa_identity(&directory),
        )
        .await
        {
            return Ok(Outcome::Waiting(p.sentence()));
        }
        // Attempts made before the record pointed here may have left a long
        // backoff: the certificate step starts fresh.
        self.app.db.set_kv(KV_CERT_NOW, "true")?;
        Ok(Outcome::Done(c.detail))
    }

    /// Automatic HTTPS for the mail hostname. Runs from the terminal and from
    /// `serve`, so a certificate that could not be issued yet (firewall, DNS
    /// still spreading) arrives on its own once the cause is fixed.
    async fn certificate(&self) -> Result<Outcome> {
        let fresh = self
            .app
            .db
            .get_kv(KV_CERT_NOW)?
            .is_some_and(|v| v == "true");
        if fresh {
            self.app.db.set_kv(KV_CERT_NOW, "false")?;
        }
        let force = self.force_certificate.swap(false, Ordering::SeqCst) || fresh;
        let ca = crate::edge::acme::ca_name(&crate::edge::acme::directory(&self.app.db));
        Ok(match certs::ensure(&self.app, force).await {
            certs::Outcome::Current | certs::Outcome::Issued(_) => {
                let st = certs::status(&self.app);
                let until = st
                    .not_after
                    .as_deref()
                    .and_then(|t| t.get(..10))
                    .map(|d| format!(", valid until {d}"))
                    .unwrap_or_default();
                Outcome::Done(format!(
                    "{} issued a certificate for {}{until}; it renews automatically.",
                    st.issuer
                        .as_deref()
                        .filter(|i| i.contains("Let's Encrypt"))
                        .map(|_| "Let's Encrypt".to_string())
                        .unwrap_or(ca),
                    st.hostnames.join(", ")
                ))
            }
            certs::Outcome::Failed {
                problem,
                next_attempt,
            }
            | certs::Outcome::Later {
                problem: Some(problem),
                next_attempt,
            } => Outcome::Waiting(format!(
                "{} Retrying by itself {}.",
                problem.sentence(),
                crate::util::when(&next_attempt)
            )),
            certs::Outcome::Later {
                problem: None,
                next_attempt,
            } => Outcome::Waiting(format!(
                "Requesting the certificate again {}.",
                crate::util::when(&next_attempt)
            )),
            certs::Outcome::Busy => {
                Outcome::Waiting("A certificate is being requested right now.".into())
            }
            certs::Outcome::NotConfigured => bail!("missing config"),
        })
    }

    async fn dns_gate(&self, ui: &mut dyn Ui) -> Result<Outcome> {
        let cfg = self.config()?;
        let engine = self.app.engine()?;
        let domain = engine
            .list_domains()
            .await?
            .into_iter()
            .find(|d| d.name == cfg.primary_domain)
            .ok_or_else(|| anyhow!("the primary domain is missing from the engine"))?;
        // With a connected DNS provider, publish what is missing or wrong first.
        let mut published = false;
        if crate::dns_publish::link(&self.app, &domain.name)?.is_some() {
            let req = crate::dns_publish::PublishRequest {
                only: None,
                plan_id: None,
                dry_run: false,
                check: false,
            };
            match crate::dns_publish::publish(&self.app, &domain, req, "install").await {
                Ok(o) => {
                    let s = crate::dns_publish::sentence(&o);
                    ui.info(&s);
                    self.log("info", "dns", &s);
                    published = o.failed == 0;
                }
                Err(e) => {
                    let s = format!("DNS provider: {e}");
                    ui.info(&s);
                    self.log("warn", "dns", &s);
                }
            }
        }
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
            Ok(Outcome::Waiting(if published {
                format!(
                    "Published through your DNS provider; waiting for public resolvers to see {} record{}: {}.",
                    failing.len(),
                    if failing.len() == 1 { "" } else { "s" },
                    names.join(", ")
                )
            } else {
                format!(
                    "Publish {} record{} at your DNS provider: {}.",
                    failing.len(),
                    if failing.len() == 1 { "" } else { "s" },
                    names.join(", ")
                )
            }))
        }
    }

    async fn ptr_gate(&self) -> Result<Outcome> {
        let cfg = self.config()?;
        let relayed = cfg.sending_mode == SendingMode::Relay;
        // An IPv6-only server's reverse DNS is the IPv6 address's.
        let Some(ip) = cfg.public_ipv4.clone().or_else(|| cfg.public_ipv6.clone()) else {
            return Ok(Outcome::Skipped("No public IP address to check.".into()));
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
        if self.skip_ptr
            || self
                .app
                .db
                .get_kv(KV_SKIP_PTR)?
                .is_some_and(|v| v == "true")
        {
            return Ok(Outcome::Skipped(PTR_SKIPPED.into()));
        }
        let names = public.ptr(ip.parse()?).await.unwrap_or_default();
        let g = crate::dns::reputation::guidance(crate::dns::reputation::detect_provider(&names));
        Ok(Outcome::Waiting(format!(
            "Set reverse DNS for {ip} to {} — {} (Or switch to relay sending.)",
            cfg.hostname, g.ptr
        )))
    }

    async fn first_mailbox(&self, ui: &mut dyn Ui) -> Result<Outcome> {
        let cfg = self.config()?;
        let engine = self.app.engine()?;
        let accounts = engine.list_accounts().await?;
        if let Some(a) = accounts.iter().find(|a| !a.is_admin) {
            return Ok(Outcome::Done(format!("{} exists.", a.address)));
        }
        // The terminal creates it only when it was named on the command line.
        let local = self
            .inputs()?
            .map(|i| i.first_mailbox)
            .filter(|l| !l.is_empty());
        let (Mode::Cli, Some(local)) = (self.mode, local) else {
            return Ok(Outcome::Waiting(
                "Create your first mailbox in the console.".into(),
            ));
        };
        let password = crate::util::generate_password(20);
        let acct =
            ops::create_first_mailbox(&self.app, &local, None, &password, "installer").await?;
        ui.secret(&format!("Password for {}", acct.address), &password);
        ui.mailbox(&acct.address);
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
        let mut cfg = self.config()?;
        cfg.install_complete = true;
        self.app.save_config(&cfg)?;
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

pub const PTR_SKIPPED: &str = "Skipped for now. Gmail and Outlook will likely reject mail sent directly from this server until reverse DNS is set (receiving is not affected).";

fn dmarc_key(domain: &str) -> String {
    format!("dmarc_policy:{domain}")
}

/// The configuration answers given as flags describe.
pub fn config_from_inputs(i: &InstallInputs) -> ServerConfig {
    ServerConfig {
        hostname: i.hostname.clone(),
        primary_domain: i.domain.clone(),
        admin_email: i.admin_email.clone(),
        client_origin: i.client_origin.clone(),
        engine_url: format!("http://127.0.0.1:{ENGINE_HTTP_PORT}"),
        sending_mode: i.sending,
        public_ipv4: None,
        public_ipv6: None,
        stalwart_version: None,
        install_complete: false,
        panel_host: None,
    }
}

/// Make the engine and the configuration carry the chosen domain and
/// hostname. The engine started with placeholder names (it needs both at its
/// first run): its domain is renamed in place (DKIM keys and the
/// administrator follow it), its hostname set through the management API.
/// Idempotent; refuses to rename a domain that already holds mailboxes.
pub async fn apply_identity(app: &App, inputs: &InstallInputs, actor: &str) -> Result<String> {
    inputs.validate_identity()?;
    let mut cfg = app
        .config()
        .ok_or_else(|| anyhow!("missing config — run `sudo yunori-mail install`"))?;
    let engine = app.engine()?;
    let domains = engine.list_domains().await?;
    if !domains.iter().any(|d| d.name == inputs.domain) {
        let current = domains
            .iter()
            .find(|d| d.name == cfg.primary_domain)
            .filter(|_| !cfg.install_complete);
        match current {
            Some(old) => {
                let accounts = engine.list_accounts().await?;
                if accounts
                    .iter()
                    .any(|a| a.domain_id == old.id && !a.is_admin)
                {
                    bail!(
                        "{} already has mailboxes, so it cannot be renamed; add {} under Domains instead",
                        old.name,
                        inputs.domain
                    );
                }
                engine.rename_domain(&old.id, &inputs.domain).await?;
                follow_admin_rename(app, &old.name, &inputs.domain)?;
                app.db.audit(
                    actor,
                    "domain.rename",
                    Some(&inputs.domain),
                    Some(&format!("was {}", old.name)),
                )?;
            }
            None => {
                engine.create_domain(&inputs.domain).await?;
                app.db
                    .audit(actor, "domain.create", Some(&inputs.domain), None)?;
            }
        }
    }
    let engine = app.engine()?;
    if engine.hostname().await.ok().as_deref() != Some(inputs.hostname.as_str()) {
        engine.set_hostname(&inputs.hostname).await?;
        app.db
            .audit(actor, "engine.hostname", Some(&inputs.hostname), None)?;
    }
    if app.db.get_kv(&dmarc_key(&inputs.domain))?.is_none() {
        app.db.set_kv(&dmarc_key(&inputs.domain), "none")?;
    }
    cfg.hostname = inputs.hostname.clone();
    cfg.primary_domain = inputs.domain.clone();
    cfg.admin_email = inputs.admin_email.clone();
    cfg.client_origin = inputs.client_origin.clone();
    app.save_config(&cfg)?;
    Ok(format!(
        "{} on {}; alerts go to {}.",
        inputs.domain, inputs.hostname, inputs.admin_email
    ))
}

/// The engine's administrator signs in as `admin@<domain>`: after a rename
/// only the new name works once the engine restarts (verified on 1.0.3), so
/// the sealed credential follows at once.
fn follow_admin_rename(app: &App, old: &str, new: &str) -> Result<()> {
    let Some(creds) = app.db.get_secret("engine.admin")? else {
        return Ok(());
    };
    let (user, pass) = creds.split_once(':').unwrap_or((creds.as_str(), ""));
    if let Some(local) = user.strip_suffix(&format!("@{old}")) {
        app.db
            .put_secret("engine.admin", &format!("{local}@{new}:{pass}"))?;
        app.reload_engine()?;
    }
    Ok(())
}

/// Wait (briefly) until the edge serves the certificate on disk for `host`,
/// so the link printed next does not open on the old self-signed one.
async fn wait_until_served(paths: &crate::paths::Paths, host: &str) {
    let Some(want) = crate::edge::tls::disk_fingerprint(paths) else {
        return;
    };
    for _ in 0..20 {
        if crate::edge::tls::served_fingerprint(443, host, false)
            .await
            .is_ok_and(|fp| fp == want)
        {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// The console's first-run page with the setup token in the URL fragment: the
/// fragment never reaches the server's logs or a Referer header, and the page
/// removes it from the address bar as soon as it has read it.
pub fn setup_link(host: &str, token: &str) -> String {
    format!(
        "https://{}/console/setup#token={token}",
        panel::url_host(host)
    )
}

/// The host links to this server should use (invites, device profiles): the
/// mail hostname once it has its certificate, the console's address before.
pub fn public_host(app: &App) -> Option<String> {
    let cfg = app.config()?;
    let mail_ready = cfg.install_complete
        || cfg.panel_host.is_none()
        || matches!(
            app.db.step_status("certificate"),
            Ok(StepStatus::Done | StepStatus::Skipped)
        );
    Some(if mail_ready {
        cfg.hostname
    } else {
        panel::url_host(cfg.console_host())
    })
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

/// Called periodically by `serve`, and after every onboarding answer:
/// advance the onboarding's verified gates without root.
pub async fn advance_gates(app: Arc<App>) {
    if app.config().is_none_or(|c| c.install_complete) {
        return;
    }
    let inst = Installer::new(app, Mode::Service);
    if let Err(e) = inst.run(&mut NullUi).await {
        tracing::debug!("installer gate check: {e:#}");
    }
}

#[cfg(test)]
mod tests;
