//! Command-line interface. Calm output: one line per step, plain words,
//! colour only when attached to a terminal (and never if NO_COLOR is set).

use crate::app::App;
use crate::installer::{InstallInputs, Installer, Mode, RelayInput, RunResult, Ui, os};
use crate::model::{Check, DnsRecord, SendingMode, Status};
use crate::paths::Paths;
use crate::{edge, ops};
use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "yunori-mail",
    version,
    about = "Yunori Mail Server — install and run your own mail server"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Install (or resume installing) Yunori Mail Server on this machine.
    Install(InstallArgs),
    /// Run the console, HTTPS edge and background jobs (started by systemd).
    Serve(ServeArgs),
    /// Check this server end to end and explain anything wrong.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Show install progress and the one-sentence status.
    Status,
    /// Issue a new one-time token for creating the first console owner.
    SetupToken,
    /// Manage mail domains.
    #[command(subcommand)]
    Domain(DomainCmd),
    /// Manage mailboxes.
    #[command(subcommand)]
    Mailbox(MailboxCmd),
    /// Manage aliases.
    #[command(subcommand)]
    Alias(AliasCmd),
    /// TLS certificate status and renewal.
    #[command(subcommand)]
    Certificate(CertCmd),
    /// Write an encrypted backup of the mail store and Yunori state.
    Backup {
        /// Directory to write the backup into.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Restore an encrypted backup (stops services while restoring).
    Restore {
        file: std::path::PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// Stop and remove the services (keeps data unless --purge).
    Uninstall {
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub struct InstallArgs {
    /// Your mail domain, e.g. example.com
    #[arg(long)]
    domain: Option<String>,
    /// Mail server hostname (default: mail.<domain>)
    #[arg(long)]
    hostname: Option<String>,
    /// An address on ANOTHER provider for certificates and alerts.
    #[arg(long)]
    admin_email: Option<String>,
    /// First mailbox to create (local part).
    #[arg(long, default_value = "hello")]
    first_mailbox: String,
    /// direct (this server delivers) or relay (an email provider delivers).
    #[arg(long, value_enum, default_value_t = SendingArg::Direct)]
    sending: SendingArg,
    #[arg(long)]
    relay_provider: Option<String>,
    #[arg(long)]
    relay_host: Option<String>,
    #[arg(long, default_value_t = 587)]
    relay_port: u16,
    #[arg(long)]
    relay_user: Option<String>,
    /// Name of an environment variable holding the relay password (never pass secrets as arguments).
    #[arg(long)]
    relay_password_env: Option<String>,
    /// Yunori client origin for "Open in Yunori" links.
    #[arg(long, default_value = crate::app::DEFAULT_CLIENT_ORIGIN)]
    client_origin: String,
    /// Continue a previous install with its saved answers.
    #[arg(long)]
    resume: bool,
    /// Do not wait at DNS / reverse-DNS gates; exit and resume later.
    #[arg(long)]
    no_wait: bool,
    /// Continue past blocking preflight failures (not recommended).
    #[arg(long)]
    force: bool,
    /// Continue without reverse DNS (direct sending will likely be rejected).
    #[arg(long)]
    skip_ptr: bool,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SendingArg {
    Direct,
    Relay,
}

#[derive(Args)]
pub struct ServeArgs {
    /// Plain HTTP on --listen, no ACME, no jobs (for development).
    #[arg(long)]
    pub dev: bool,
    /// Use the in-memory engine (development only).
    #[arg(long, requires = "dev")]
    pub mock_engine: bool,
    #[arg(long, default_value = "127.0.0.1:8787")]
    pub listen: std::net::SocketAddr,
}

#[derive(Subcommand)]
pub enum DomainCmd {
    List,
    Add {
        name: String,
    },
    Remove {
        name: String,
    },
    /// Show expected vs published DNS records.
    Dns {
        name: String,
    },
}

#[derive(Subcommand)]
pub enum MailboxCmd {
    List,
    Add {
        address: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        quota_mb: Option<u64>,
    },
    /// Set a new generated password.
    Passwd {
        address: String,
    },
    Remove {
        address: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum AliasCmd {
    List,
    Add { alias: String, mailbox: String },
    Remove { alias: String },
}

#[derive(Subcommand)]
pub enum CertCmd {
    Status,
    Renew,
}

// ── terminal UI ──────────────────────────────────────────────────────────

struct Term {
    color: bool,
}

impl Term {
    fn new() -> Self {
        Self {
            color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }
    fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn status_tag(&self, s: Status) -> String {
        match s {
            Status::Pass => self.paint("32", "  ok   "),
            Status::Warn => self.paint("33", " warn  "),
            Status::Fail => self.paint("31", "problem"),
            Status::Info => self.paint("2", " info  "),
            Status::Unknown => self.paint("2", "  ?    "),
        }
    }
}

impl Ui for Term {
    fn step(&mut self, _id: &str, title: &str) {
        println!("\n{} {}", self.paint("2", "·"), self.paint("1", title));
    }
    fn done(&mut self, _id: &str, detail: &str) {
        println!("  {} {detail}", self.paint("32", "✓"));
    }
    fn waiting(&mut self, _id: &str, detail: &str) {
        println!("  {} {detail}", self.paint("33", "…"));
    }
    fn failed(&mut self, _id: &str, detail: &str) {
        println!("  {} {detail}", self.paint("31", "✗"));
    }
    fn info(&mut self, msg: &str) {
        println!("  {msg}");
    }
    fn secret(&mut self, label: &str, value: &str) {
        println!(
            "\n  {}\n    {}\n  {}",
            label,
            self.paint("1", value),
            self.paint("2", "Shown once — store it in your password manager.")
        );
    }
    fn checks(&mut self, checks: &[Check]) {
        print_checks(self, checks);
    }
    fn records(&mut self, records: &[DnsRecord]) {
        print_records(self, records);
    }
}

fn print_checks(t: &Term, checks: &[Check]) {
    for c in checks {
        println!(
            "  [{}] {:<28} {}",
            t.status_tag(c.status),
            c.label,
            c.detail
        );
        if let Some(r) = &c.remedy
            && c.status != Status::Pass
        {
            println!("            {:<28} {}", "", t.paint("2", &format!("→ {r}")));
        }
    }
}

fn print_records(t: &Term, records: &[DnsRecord]) {
    for r in records {
        println!(
            "  [{}] {:<5} {}{}",
            t.status_tag(r.status),
            r.rtype,
            r.name,
            if r.required { "" } else { "  (optional)" }
        );
        if r.status != Status::Pass && !r.expected.is_empty() {
            println!("            value: {}", r.expected);
            if !r.actual.is_empty() {
                println!("            found: {}", r.actual.join(" | "));
            }
        }
    }
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("{label} is required (pass it as a flag when not running interactively)");
    }
    match default {
        Some(d) => print!("{label} [{d}]: "),
        None => print!("{label}: "),
    }
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let v = line.trim().to_string();
    Ok(if v.is_empty() {
        default.unwrap_or_default().to_string()
    } else {
        v
    })
}

fn open_app(dev: bool, mock: bool) -> Result<Arc<App>> {
    App::open(Paths::from_env(), dev, mock)
}

// ── commands ─────────────────────────────────────────────────────────────

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Install(args) => install(args).await,
        Command::Serve(args) => serve(args).await,
        Command::Doctor { json } => doctor(json).await,
        Command::Status => status().await,
        Command::SetupToken => setup_token(),
        Command::Domain(c) => domain_cmd(c).await,
        Command::Mailbox(c) => mailbox_cmd(c).await,
        Command::Alias(c) => alias_cmd(c).await,
        Command::Certificate(c) => cert_cmd(c).await,
        Command::Backup { out } => backup(out).await,
        Command::Restore { file, yes } => restore(file, yes).await,
        Command::Uninstall { purge, yes } => uninstall(purge, yes),
    }
}

async fn install(args: InstallArgs) -> Result<()> {
    let paths = Paths::from_env();
    if paths.is_production() && !os::is_root() {
        bail!("run the installer as root: sudo yunori-mail install");
    }
    if paths.is_production() {
        // New files must stay group-accessible to the unprivileged service.
        unsafe {
            libc::umask(0o007);
        }
    }
    let app = App::open(paths, false, false)?;
    let mut term = Term::new();
    println!("{}", term.paint("1", "Yunori Mail Server"));
    println!(
        "{}",
        term.paint(
            "2",
            "Installs a real mail server on this machine. Every step can be resumed."
        )
    );

    let inst = Installer {
        app: app.clone(),
        mode: Mode::Cli,
        force: args.force,
        skip_ptr: args.skip_ptr,
    };
    let has_saved = app.db.get_kv("install.inputs")?.is_some();
    if !(args.resume && has_saved) {
        let domain = match args.domain {
            Some(d) => d,
            None => prompt("Mail domain (e.g. example.com)", None)?,
        };
        let domain = crate::util::normalize_domain(&domain)
            .ok_or_else(|| anyhow!("'{domain}' is not a valid domain"))?;
        let hostname = match args.hostname {
            Some(h) => h,
            None => prompt("Mail server hostname", Some(&format!("mail.{domain}")))?,
        };
        let admin_email = match args.admin_email {
            Some(e) => e,
            None => prompt(
                "Your email on another provider (certificates & alerts)",
                None,
            )?,
        };
        let sending = match args.sending {
            SendingArg::Direct => SendingMode::Direct,
            SendingArg::Relay => SendingMode::Relay,
        };
        let relay = if sending == SendingMode::Relay {
            let provider = args
                .relay_provider
                .clone()
                .unwrap_or_else(|| "custom".into());
            let preset = crate::model::relay_provider(&provider);
            let host = args
                .relay_host
                .clone()
                .or_else(|| preset.and_then(|p| p.host).map(str::to_string))
                .ok_or_else(|| anyhow!("--relay-host is required for this provider"))?;
            let username = args
                .relay_user
                .clone()
                .ok_or_else(|| anyhow!("--relay-user is required"))?;
            let password = match &args.relay_password_env {
                Some(var) => std::env::var(var)
                    .with_context(|| format!("environment variable {var} is not set"))?,
                None => rpassword::prompt_password("Relay password: ")?,
            };
            Some(RelayInput {
                provider,
                host,
                port: args.relay_port,
                username,
                password,
            })
        } else {
            None
        };
        let inputs = InstallInputs {
            domain,
            hostname: hostname.trim().to_ascii_lowercase(),
            admin_email: admin_email.trim().to_ascii_lowercase(),
            first_mailbox: args.first_mailbox.trim().to_ascii_lowercase(),
            sending,
            relay,
            client_origin: args.client_origin.trim_end_matches('/').to_string(),
        };
        inst.save_inputs(&inputs)?;
    }

    loop {
        match inst.run(&mut term).await {
            Ok(RunResult::Complete) => {
                println!("\n{}", term.paint("1", "Done."));
                return Ok(());
            }
            Ok(RunResult::Waiting { step, .. }) => {
                if args.no_wait || !std::io::stdin().is_terminal() {
                    println!(
                        "\nPaused at \"{step}\". Fix the item above, then run: yunori-mail install --resume"
                    );
                    return Ok(());
                }
                println!("{}", term.paint("2", "  Checking again in 30 seconds (Ctrl-C to stop; resume later with --resume)."));
                tokio::time::sleep(Duration::from_secs(30)).await;
                app.db
                    .set_step(&step, crate::installer::state::StepStatus::Pending, None)?;
            }
            Err(e) => {
                println!(
                    "\nThe install stopped: {e:#}\nFix the problem, then run: yunori-mail install --resume"
                );
                std::process::exit(1);
            }
        }
    }
}

pub async fn serve(args: ServeArgs) -> Result<()> {
    let app = open_app(args.dev, args.mock_engine)?;
    if args.dev && app.db.admin_count_active()? == 0 {
        let token = app.db.issue_setup_token()?;
        println!(
            "Development setup token: {token}\nOpen http://{}/console/setup",
            args.listen
        );
    }
    crate::jobs::spawn_all(app.clone());
    if !args.dev {
        let a = app.clone();
        tokio::spawn(async move {
            loop {
                crate::installer::advance_gates(a.clone()).await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
    }
    edge::serve(
        app,
        edge::EdgeOptions {
            dev: args.dev,
            dev_listen: args.listen,
        },
    )
    .await
}

async fn doctor(json: bool) -> Result<()> {
    let app = open_app(false, false)?;
    let mut checks: Vec<Check> = Vec::new();
    let cfg = app.config();
    if cfg.is_none() {
        checks.push(
            Check::new(
                "config",
                "Configuration",
                Status::Fail,
                "Yunori Mail Server is not installed.",
            )
            .remedy("Run: sudo yunori-mail install"),
        );
    }
    for svc in ["stalwart", "yunori-mail", "unbound"] {
        let active = os::is_active(svc);
        checks.push(
            Check::new(
                &format!("svc:{svc}"),
                &format!("Service {svc}"),
                if active { Status::Pass } else { Status::Fail },
                if active { "running" } else { "not running" },
            )
            .remedy(format!("systemctl status {svc}")),
        );
    }
    let leaked = os::run("systemctl", &["show", "stalwart", "-p", "Environment"])
        .map(|s| s.contains("STALWART_RECOVERY"))
        .unwrap_or(false);
    checks.push(if leaked {
        Check::new(
            "recovery_env",
            "Recovery credentials",
            Status::Fail,
            "A recovery admin is configured on the engine service — that is a permanent backdoor.",
        )
        .remedy("Remove STALWART_RECOVERY_* from the stalwart unit and restart it.")
    } else {
        Check::new(
            "recovery_env",
            "Recovery credentials",
            Status::Pass,
            "No recovery admin is left configured.",
        )
    });
    let (_, engine) = ops::engine_check(app.engine().ok()).await;
    checks.push(engine);
    if let Some(cfg) = &cfg {
        if let Ok(e) = app.engine()
            && let Ok(domains) = e.list_domains().await
            && let Some(d) = domains.iter().find(|d| d.name == cfg.primary_domain)
            && let Ok(dns) = ops::domain_dns(&app, d, true).await
        {
            let worst = crate::dns::records::worst_required(&dns.records);
            checks.push(
                Check::new(
                    "dns",
                    "DNS records",
                    worst,
                    format!(
                        "{} records checked for {}.",
                        dns.records.len(),
                        cfg.primary_domain
                    ),
                )
                .remedy(format!("yunori-mail domain dns {}", cfg.primary_domain)),
            );
        }
        let host = ops::host_checks(&app, true).await;
        checks.push(host.port25);
        checks.push(host.ptr);
        checks.extend(host.blocklists);
        let cert = crate::edge::tls::disk_certificate_info(&app.paths);
        checks.push(match cert {
            None => Check::new(
                "tls",
                "Certificate",
                Status::Fail,
                "No certificate on disk.",
            ),
            Some(c) if c.self_signed => Check::new(
                "tls",
                "Certificate",
                Status::Warn,
                "Self-signed (ACME has not succeeded yet).",
            ),
            Some(c) => Check::new(
                "tls",
                "Certificate",
                if c.days_left < 14 {
                    Status::Warn
                } else {
                    Status::Pass
                },
                format!("{} — {} days left", c.hostnames.join(", "), c.days_left),
            ),
        });
        let (served, detail) = crate::edge::tls::verify_served(&app.paths, &cfg.hostname).await;
        checks.push(Check::new(
            "tls_served",
            "Certificate in use",
            if served { Status::Pass } else { Status::Warn },
            detail.join("; "),
        ));
    }
    checks.push(Check::new(
        "audit",
        "Audit ledger",
        if app.db.audit_verify()? {
            Status::Pass
        } else {
            Status::Fail
        },
        if app.db.audit_verify()? {
            "Hash chain intact."
        } else {
            "The audit chain was modified."
        },
    ));
    if json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        print_checks(&Term::new(), &checks);
    }
    if checks.iter().any(|c| c.status == Status::Fail) {
        std::process::exit(2);
    }
    Ok(())
}

async fn status() -> Result<()> {
    let app = open_app(false, false)?;
    let t = Term::new();
    let st = crate::installer::state::load(&app.db)?;
    for s in &st.steps {
        let mark = match s.status {
            crate::installer::state::StepStatus::Done => t.paint("32", "✓"),
            crate::installer::state::StepStatus::Skipped => t.paint("2", "–"),
            crate::installer::state::StepStatus::Waiting => t.paint("33", "…"),
            crate::installer::state::StepStatus::Failed => t.paint("31", "✗"),
            crate::installer::state::StepStatus::Running => t.paint("36", "›"),
            crate::installer::state::StepStatus::Pending => t.paint("2", "·"),
        };
        println!(
            "  {mark} {:<32} {}",
            s.title,
            s.detail.clone().unwrap_or_default()
        );
    }
    Ok(())
}

fn setup_token() -> Result<()> {
    let app = open_app(false, false)?;
    if app.db.admin_count_active()? > 0 {
        bail!("an owner already exists — sign in at /console/login");
    }
    let token = app.db.issue_setup_token()?;
    let host = app
        .config()
        .map(|c| c.hostname)
        .unwrap_or_else(|| "this-server".into());
    println!(
        "Open https://{host}/console/setup and enter this token (valid 24 hours):\n\n  {token}\n"
    );
    Ok(())
}

async fn find_domain_id(app: &App, name: &str) -> Result<crate::engine::EDomain> {
    let name = crate::util::normalize_domain(name).ok_or_else(|| anyhow!("invalid domain"))?;
    app.engine()?
        .list_domains()
        .await?
        .into_iter()
        .find(|d| d.name == name)
        .ok_or_else(|| anyhow!("{name} is not on this server"))
}

async fn domain_cmd(c: DomainCmd) -> Result<()> {
    let app = open_app(false, false)?;
    let e = app.engine()?;
    let t = Term::new();
    match c {
        DomainCmd::List => {
            let accounts = e.list_accounts().await?;
            for d in e.list_domains().await? {
                let (st, _) = ops::cached_dns_status(&app, &d.name);
                let n = accounts.iter().filter(|a| a.domain_id == d.id).count();
                println!("  [{}] {:<32} {n} mailbox(es)", t.status_tag(st), d.name);
            }
        }
        DomainCmd::Add { name } => {
            let name =
                crate::util::normalize_domain(&name).ok_or_else(|| anyhow!("invalid domain"))?;
            let d = e.create_domain(&name).await?;
            app.db.audit("cli", "domain.create", Some(&name), None)?;
            println!(
                "Added {}. DKIM keys are being generated; check DNS with: yunori-mail domain dns {}",
                d.name, d.name
            );
        }
        DomainCmd::Remove { name } => {
            let d = find_domain_id(&app, &name).await?;
            if e.list_accounts().await?.iter().any(|a| a.domain_id == d.id) {
                bail!("{} still has mailboxes; remove them first", d.name);
            }
            e.delete_domain(&d.id).await?;
            app.db.audit("cli", "domain.delete", Some(&d.name), None)?;
            println!("Removed {}.", d.name);
        }
        DomainCmd::Dns { name } => {
            let d = find_domain_id(&app, &name).await?;
            let dns = ops::domain_dns(&app, &d, true).await?;
            print_records(&t, &dns.records);
            println!("\nZone file:\n{}", dns.zone_file);
        }
    }
    Ok(())
}

fn split_address(address: &str) -> Result<(String, String)> {
    let a = address.trim().to_ascii_lowercase();
    let (l, d) = a
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("expected an address like name@example.com"))?;
    if !crate::util::valid_local_part(l) {
        bail!("invalid mailbox name '{l}'");
    }
    Ok((l.to_string(), d.to_string()))
}

async fn mailbox_cmd(c: MailboxCmd) -> Result<()> {
    let app = open_app(false, false)?;
    let e = app.engine()?;
    match c {
        MailboxCmd::List => {
            let domains = e.list_domains().await?;
            for a in e.list_accounts().await? {
                let aliases = crate::engine::alias_addresses(&a, &domains);
                println!(
                    "  {:<36} {}{}",
                    a.address,
                    a.display_name.clone().unwrap_or_default(),
                    if aliases.is_empty() {
                        String::new()
                    } else {
                        format!("  (aliases: {})", aliases.join(", "))
                    }
                );
            }
        }
        MailboxCmd::Add {
            address,
            name,
            quota_mb,
        } => {
            let (local, domain) = split_address(&address)?;
            let d = find_domain_id(&app, &domain).await?;
            let pw = crate::util::generate_password(20);
            let a = e
                .create_account(
                    &local,
                    &d.id,
                    name.as_deref(),
                    quota_mb.map(|m| m * 1024 * 1024),
                    &pw,
                )
                .await?;
            app.db
                .audit("cli", "mailbox.create", Some(&a.address), None)?;
            println!("Created {}.\nPassword (shown once): {pw}", a.address);
        }
        MailboxCmd::Passwd { address } => {
            let a = e
                .list_accounts()
                .await?
                .into_iter()
                .find(|a| a.address == address.trim().to_ascii_lowercase())
                .ok_or_else(|| anyhow!("no such mailbox"))?;
            let pw = crate::util::generate_password(20);
            e.set_password(&a.id, &pw).await?;
            app.db
                .audit("cli", "mailbox.password_reset", Some(&a.address), None)?;
            println!("New password for {} (shown once): {pw}", a.address);
        }
        MailboxCmd::Remove { address, yes } => {
            let a = e
                .list_accounts()
                .await?
                .into_iter()
                .find(|a| a.address == address.trim().to_ascii_lowercase())
                .ok_or_else(|| anyhow!("no such mailbox"))?;
            if a.is_admin {
                bail!(
                    "{} is the engine's administrator account and cannot be removed",
                    a.address
                );
            }
            if !yes
                && prompt(
                    &format!("Type {} to delete it and all its mail", a.address),
                    None,
                )? != a.address
            {
                bail!("not deleted");
            }
            e.delete_account(&a.id).await?;
            app.db
                .audit("cli", "mailbox.delete", Some(&a.address), None)?;
            println!("Deleted {}.", a.address);
        }
    }
    Ok(())
}

async fn alias_cmd(c: AliasCmd) -> Result<()> {
    let app = open_app(false, false)?;
    let e = app.engine()?;
    let domains = e.list_domains().await?;
    let accounts = e.list_accounts().await?;
    match c {
        AliasCmd::List => {
            for a in &accounts {
                for al in crate::engine::alias_addresses(a, &domains) {
                    println!("  {al:<36} → {}", a.address);
                }
            }
        }
        AliasCmd::Add { alias, mailbox } => {
            let (local, domain) = split_address(&alias)?;
            let d = domains
                .iter()
                .find(|d| d.name == domain)
                .ok_or_else(|| anyhow!("{domain} is not on this server"))?;
            if accounts
                .iter()
                .any(|a| a.address == alias.to_ascii_lowercase())
            {
                bail!("{alias} is a mailbox; an alias cannot shadow it");
            }
            let target = accounts
                .iter()
                .find(|a| a.address == mailbox.to_ascii_lowercase())
                .ok_or_else(|| anyhow!("no such mailbox"))?;
            e.add_alias(&target.id, &local, &d.id).await?;
            app.db.audit(
                "cli",
                "alias.create",
                Some(&alias),
                Some(&format!("→ {}", target.address)),
            )?;
            println!("{alias} now delivers to {}.", target.address);
        }
        AliasCmd::Remove { alias } => {
            let alias = alias.to_ascii_lowercase();
            for a in &accounts {
                for al in &a.aliases {
                    if domains.iter().any(|d| {
                        d.id == al.domain_id && format!("{}@{}", al.local, d.name) == alias
                    }) {
                        e.remove_alias(&a.id, &al.key).await?;
                        app.db.audit("cli", "alias.delete", Some(&alias), None)?;
                        println!("Removed {alias}.");
                        return Ok(());
                    }
                }
            }
            bail!("no such alias");
        }
    }
    Ok(())
}

async fn cert_cmd(c: CertCmd) -> Result<()> {
    let app = open_app(false, false)?;
    match c {
        CertCmd::Status => match crate::edge::tls::disk_certificate_info(&app.paths) {
            None => println!("No certificate yet."),
            Some(i) => println!(
                "{} — issuer {}, expires {} ({} days){}",
                i.hostnames.join(", "),
                i.issuer.unwrap_or_default(),
                i.not_after.unwrap_or_default(),
                i.days_left,
                if i.self_signed {
                    " — self-signed"
                } else {
                    ""
                }
            ),
        },
        CertCmd::Renew => {
            let cfg = app.config().ok_or_else(|| anyhow!("not installed"))?;
            let domains: Vec<String> = app
                .engine()?
                .list_domains()
                .await?
                .into_iter()
                .map(|d| d.name)
                .collect();
            let names = crate::edge::acme::wanted_names(
                &cfg.hostname,
                &domains,
                cfg.public_ipv4.as_deref(),
            )
            .await;
            crate::edge::acme::issue(&app.paths, &app.db, &names, &cfg.admin_email).await?;
            crate::installer::fix_permissions(&app.paths);
            crate::jobs::push_certificate(&app).await;
            println!("Renewed for {}.", names.join(", "));
        }
    }
    Ok(())
}

async fn backup(out: Option<std::path::PathBuf>) -> Result<()> {
    if !os::is_root() {
        bail!("run backups as root");
    }
    let app = open_app(false, false)?;
    let p = app.paths.clone();
    let key = match app.db.get_secret("backup.key")? {
        Some(k) => k.to_string(),
        None => {
            let k = crate::backup::new_key();
            app.db.put_secret("backup.key", &k)?;
            println!(
                "Backup recovery key (store it OFF this server — restores need it):\n\n  {k}\n"
            );
            k
        }
    };
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let work = p.backups_dir().join(format!("work-{stamp}"));
    std::fs::create_dir_all(work.join("engine"))?;
    os::chown_r("stalwart", &work.join("engine"))?;
    println!("Stopping the engine for a consistent export…");
    os::systemctl(&["stop", "stalwart"])?;
    let export = os::run(
        "runuser",
        &[
            "-u",
            "stalwart",
            "--",
            &p.stalwart_bin().to_string_lossy(),
            &format!("--config={}", p.stalwart_config().display()),
            &format!("--export={}", work.join("engine").display()),
        ],
    );
    os::systemctl(&["start", "stalwart"])?;
    export.context("engine export failed")?;
    {
        let conn = app.db.conn();
        conn.execute(
            "VACUUM INTO ?1",
            [work.join("yunori.db").to_string_lossy().to_string()],
        )?;
    }
    std::fs::copy(p.master_key(), work.join("master.key"))?;
    let tar_path = work.with_extension("tar.gz");
    os::run(
        "tar",
        &[
            "-czf",
            &tar_path.to_string_lossy(),
            "-C",
            &work.to_string_lossy(),
            ".",
        ],
    )?;
    let dest_dir = out.unwrap_or_else(|| p.backups_dir());
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(format!("yunori-backup-{stamp}.ybk"));
    crate::backup::encrypt(
        &key,
        std::fs::File::open(&tar_path)?,
        std::fs::File::create(&dest)?,
    )?;
    crate::store::vault::set_mode(&dest, 0o600);
    let _ = std::fs::remove_file(&tar_path);
    let _ = std::fs::remove_dir_all(&work);
    app.db
        .audit("cli", "backup.create", Some(&dest.to_string_lossy()), None)?;
    println!("Encrypted backup written to {}", dest.display());
    Ok(())
}

async fn restore(file: std::path::PathBuf, yes: bool) -> Result<()> {
    if !os::is_root() {
        bail!("run restores as root");
    }
    let p = Paths::from_env();
    let key = match std::env::var("YUNORI_BACKUP_KEY") {
        Ok(k) => k,
        Err(_) => rpassword::prompt_password("Backup recovery key: ")?,
    };
    if !yes
        && prompt(
            "This replaces ALL mail and settings on this server. Type restore to continue",
            None,
        )? != "restore"
    {
        bail!("not restored");
    }
    let work = p.backups_dir().join("restore-work");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let tar_path = work.join("backup.tar.gz");
    crate::backup::decrypt(
        &key,
        std::fs::File::open(&file)?,
        std::fs::File::create(&tar_path)?,
    )?;
    os::run(
        "tar",
        &[
            "-xzf",
            &tar_path.to_string_lossy(),
            "-C",
            &work.to_string_lossy(),
        ],
    )?;
    let _ = os::systemctl(&["stop", "yunori-mail"]);
    os::systemctl(&["stop", "stalwart"])?;
    os::chown_r("stalwart", &work.join("engine"))?;
    os::run(
        "runuser",
        &[
            "-u",
            "stalwart",
            "--",
            &p.stalwart_bin().to_string_lossy(),
            &format!("--config={}", p.stalwart_config().display()),
            &format!("--import={}", work.join("engine").display()),
        ],
    )
    .context("engine import failed")?;
    std::fs::copy(work.join("yunori.db"), p.db())?;
    std::fs::copy(work.join("master.key"), p.master_key())?;
    crate::installer::fix_permissions(&p);
    os::systemctl(&["start", "stalwart"])?;
    let _ = os::systemctl(&["start", "yunori-mail"]);
    let _ = std::fs::remove_dir_all(&work);
    println!(
        "Restored. Point MX, A and reverse DNS at this server if it is a new machine, then run: yunori-mail doctor"
    );
    Ok(())
}

fn uninstall(purge: bool, yes: bool) -> Result<()> {
    if !os::is_root() {
        bail!("run as root");
    }
    let p = Paths::from_env();
    if purge
        && !yes
        && prompt(
            "This deletes ALL mail and settings. Type purge to continue",
            None,
        )? != "purge"
    {
        bail!("nothing removed");
    }
    for svc in ["yunori-mail", "stalwart"] {
        let _ = os::systemctl(&["disable", "--now", svc]);
        let _ = std::fs::remove_file(p.systemd_dir().join(format!("{svc}.service")));
    }
    let _ = os::systemctl(&["daemon-reload"]);
    if purge {
        for dir in [
            p.stalwart_data(),
            p.stalwart_etc(),
            p.state_dir(),
            p.etc_dir(),
        ] {
            let _ = std::fs::remove_dir_all(&dir);
        }
        println!("Services removed and all data deleted.");
    } else {
        println!(
            "Services removed. Mail and settings were kept in {} and {}.",
            p.stalwart_data().display(),
            p.state_dir().display()
        );
    }
    Ok(())
}
