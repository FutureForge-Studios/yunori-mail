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
    Install(Box<InstallArgs>),
    /// Check whether this server can run a mail server. Changes nothing.
    Preflight {
        /// Also check that this hostname (e.g. mail.example.com) points here.
        #[arg(long)]
        hostname: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run the console, HTTPS edge and background jobs (started by systemd).
    Serve(ServeArgs),
    /// Check this server end to end and explain anything wrong.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Show install progress and the one-sentence status.
    Status,
    /// Print a new one-time link for creating the first console owner.
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
    /// Upgrade the mail engine to the version pinned by this build (with rollback),
    /// and refresh the service units.
    Upgrade,
    /// Update Yunori Mail Server to the newest release from yunori.email (the
    /// one-line installer: verified download, binary swap, engine upgrade).
    Update {
        /// Only say whether there is a newer release; change nothing.
        #[arg(long)]
        check: bool,
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
    /// Asked when not given; the suggestion follows the outbound port-25 check.
    #[arg(long, value_enum)]
    sending: Option<SendingArg>,
    #[arg(long)]
    relay_provider: Option<String>,
    /// SMTP host; derived from --relay-region for Amazon SES.
    #[arg(long)]
    relay_host: Option<String>,
    /// Provider region (Amazon SES: picks the SMTP endpoint; Resend: the bounce MX).
    #[arg(long)]
    relay_region: Option<String>,
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
    /// Publish the DNS records through this provider's API (cloudflare), so the
    /// DNS steps complete themselves. Needs --dns-token-env.
    #[arg(long, requires = "dns_token_env", value_name = "PROVIDER")]
    dns_provider: Option<String>,
    /// Name of an environment variable holding the DNS provider's API token.
    #[arg(long, requires = "dns_provider", value_name = "VAR")]
    dns_token_env: Option<String>,
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
    /// Never ask: take flags and defaults as they are.
    #[arg(long, short = 'y')]
    yes: bool,
    /// ACME directory URL of another certificate authority (default: Let's Encrypt).
    #[arg(long, env = "YUNORI_ACME_DIRECTORY")]
    acme_directory: Option<String>,
    /// Show everything the installer would download, create and change, run
    /// the read-only preflight, and stop. Changes nothing.
    #[arg(long)]
    dry_run: bool,
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
    /// Development only: use a locally running Stalwart at this URL (its admin
    /// credentials come from the YUNORI_ENGINE_ADMIN env var, `user:password`).
    #[arg(long, requires = "dev", conflicts_with = "mock_engine")]
    pub engine_url: Option<String>,
}

#[derive(Subcommand)]
pub enum DomainCmd {
    List {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    Add {
        name: String,
    },
    Remove {
        name: String,
    },
    /// Show expected vs published DNS records; with a connected DNS provider,
    /// publish the missing or wrong ones.
    Dns {
        name: String,
        /// Connect a DNS provider (cloudflare) with a token from --token-env.
        #[arg(long, value_name = "PROVIDER")]
        connect: Option<String>,
        /// Name of an environment variable holding the provider's API token.
        #[arg(long, requires = "connect", value_name = "VAR")]
        token_env: Option<String>,
        /// Forget the connected DNS provider and its token.
        #[arg(long, conflicts_with_all = ["connect", "publish"])]
        disconnect: bool,
        /// Write the records that are missing or wrong at the connected provider
        /// (never deletes), then check public DNS.
        #[arg(long)]
        publish: bool,
        /// With --publish: show what would change, change nothing.
        #[arg(long, requires = "publish")]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum MailboxCmd {
    List {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Write every mailbox as CSV (address, name, quota, usage, aliases).
    Export {
        /// File to write (default: standard output).
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Create mailboxes from CSV: address[,display_name[,quota_mb]] per line.
    /// Each person gets an invite link to choose their own password.
    Import {
        /// CSV file (`-` for standard input).
        file: std::path::PathBuf,
        /// Generate passwords instead of invite links (printed once).
        #[arg(long)]
        passwords: bool,
        /// Check every row and report; create nothing.
        #[arg(long)]
        dry_run: bool,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    Add {
        address: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        quota_mb: Option<u64>,
    },
    /// Set a new generated password.
    Passwd { address: String },
    /// Copy mail from another IMAP server (Gmail, Microsoft 365, …) into a
    /// mailbox here. Resumable: running it again copies only what is new.
    Migrate(crate::migrate::cli::MigrateArgs),
    Remove {
        address: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum AliasCmd {
    List {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    Add {
        alias: String,
        mailbox: String,
    },
    Remove {
        alias: String,
    },
}

#[derive(Subcommand)]
pub enum CertCmd {
    Status,
    Renew,
}

// ── terminal UI ──────────────────────────────────────────────────────────

struct Term {
    color: bool,
    step_started: Option<std::time::Instant>,
    setup_url: Option<String>,
    setup_note: Option<String>,
    mailbox: Option<String>,
}

impl Term {
    fn new() -> Self {
        Self {
            color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
            step_started: None,
            setup_url: None,
            setup_note: None,
            mailbox: None,
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
    fn elapsed(&mut self) -> String {
        match self.step_started.take().map(|t| t.elapsed().as_secs()) {
            Some(s) if s >= 3 => self.paint("2", &format!("  ({})", fmt_duration(s))),
            _ => String::new(),
        }
    }
}

fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

impl Ui for Term {
    fn step(&mut self, id: &str, title: &str) {
        let n = crate::installer::state::STEPS
            .iter()
            .position(|(s, _)| *s == id)
            .map(|i| i + 1)
            .unwrap_or(0);
        let total = crate::installer::state::STEPS.len();
        self.step_started = Some(std::time::Instant::now());
        println!(
            "\n{} {}",
            self.paint("2", &format!("[{n:>2}/{total}]")),
            self.paint("1", title)
        );
    }
    fn done(&mut self, _id: &str, detail: &str) {
        let t = self.elapsed();
        println!("  {} {detail}{t}", self.paint("32", "✓"));
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
    fn setup_link(&mut self, url: &str, note: Option<&str>) {
        println!(
            "\n  {}\n    {}\n  {}",
            self.paint(
                "1",
                "Continue in your browser: create the console owner (one-time link, valid 24 hours)"
            ),
            self.paint("36;1", url),
            self.paint(
                "2",
                "Anyone with this link can claim the console until you do. Don't share it."
            )
        );
        if let Some(n) = note {
            println!("  {}", self.paint("33", n));
        }
        self.setup_url = Some(url.to_string());
        self.setup_note = note.map(str::to_string);
    }
    fn mailbox(&mut self, address: &str) {
        self.mailbox = Some(address.to_string());
    }
}

fn print_checks(t: &Term, checks: &[Check]) {
    let w = checks
        .iter()
        .map(|c| c.label.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(20, 36);
    for c in checks {
        println!(
            "  [{}] {:<w$} {}",
            t.status_tag(c.status),
            c.label,
            c.detail
        );
        if let Some(r) = &c.remedy
            && c.status != Status::Pass
        {
            println!("            {:<w$} {}", "", t.paint("2", &format!("→ {r}")));
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

fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

fn read_line() -> Result<String> {
    std::io::stdout().flush()?;
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        bail!("input closed");
    }
    Ok(line.trim().to_string())
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    if !interactive() {
        bail!("{label} is required (pass it as a flag when not running interactively)");
    }
    match default {
        Some(d) => print!("{label} [{d}]: "),
        None => print!("{label}: "),
    }
    let v = read_line()?;
    Ok(if v.is_empty() {
        default.unwrap_or_default().to_string()
    } else {
        v
    })
}

/// Ask until `check` accepts the answer (it returns the cleaned value or a hint).
fn ask(
    t: &Term,
    label: &str,
    default: Option<&str>,
    check: impl Fn(&str) -> std::result::Result<String, String>,
) -> Result<String> {
    loop {
        let raw = prompt(&format!("  {label}"), default)?;
        match check(&raw) {
            Ok(v) => return Ok(v),
            Err(hint) => println!("    {}", t.paint("33", &hint)),
        }
    }
}

fn confirm(label: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    let v = prompt(&format!("  {label} ({hint})"), None)?;
    Ok(match v.to_ascii_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    })
}

fn open_app(dev: bool, mock: bool) -> Result<Arc<App>> {
    App::open(Paths::from_env(), dev, mock)
}

// ── commands ─────────────────────────────────────────────────────────────

pub async fn run(cli: Cli) -> Result<()> {
    // As root on a real install, keep new files (SQLite WAL/SHM, certs)
    // group-accessible to the unprivileged `yunori` service.
    if Paths::from_env().is_production() && os::is_root() {
        unsafe {
            libc::umask(0o007);
        }
    }
    match cli.command {
        Command::Install(args) => install(*args).await,
        Command::Preflight { hostname, json } => preflight_cmd(hostname, json).await,
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
        Command::Upgrade => upgrade().await,
        Command::Update { check } => update_cmd(check).await,
        Command::Uninstall { purge, yes } => uninstall(purge, yes),
    }
}

/// "Ready." / "Ready, with 2 warnings to review." / "Not ready: …"
fn preflight_verdict(results: &[Check]) -> (bool, String) {
    if crate::checks::has_blocking_failure(results) {
        let failed: Vec<String> = results
            .iter()
            .filter(|c| crate::checks::is_blocking(c))
            .map(|c| c.label.clone())
            .collect();
        return (
            false,
            format!("This server is not ready: fix {} first.", failed.join(", ")),
        );
    }
    let warnings = results
        .iter()
        .filter(|c| matches!(c.status, Status::Warn | Status::Fail))
        .count();
    (
        true,
        if warnings == 0 {
            "This server is ready.".into()
        } else {
            format!(
                "This server is ready, with {warnings} warning{} to review.",
                if warnings == 1 { "" } else { "s" }
            )
        },
    )
}

/// `install --dry-run`: the whole plan, concretely, then the read-only preflight.
async fn install_dry_run(args: &InstallArgs) -> Result<()> {
    let p = Paths::from_env();
    let t = Term::new();
    let domain = args
        .domain
        .as_deref()
        .and_then(crate::util::normalize_domain)
        .unwrap_or_else(|| "<your domain>".into());
    let hostname = args
        .hostname
        .clone()
        .unwrap_or_else(|| format!("mail.{domain}"));
    let (engine_url, engine_sha) = os::engine_asset()?;
    let firewall = match os::detect_firewall() {
        os::Firewall::Ufw { active: true } => "ufw is active: allow TCP {ports} (and your SSH port).",
        os::Firewall::Firewalld => "firewalld is active: add TCP {ports} permanently and reload.",
        _ => "no active host firewall: nothing to change (allow TCP {ports} in your provider's firewall).",
    }
    .replace(
        "{ports}",
        &os::PUBLIC_PORTS
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", "),
    );
    let sending = match args.sending {
        Some(SendingArg::Relay) => format!(
            "relay through {}",
            args.relay_provider.as_deref().unwrap_or("your provider")
        ),
        Some(SendingArg::Direct) => "direct (this server delivers)".into(),
        None => "asked (suggested from the outbound port-25 check)".into(),
    };
    println!(
        "{}",
        t.paint("1", "Yunori Mail Server · install plan (dry run)")
    );
    println!("{}", t.paint("2", "Nothing is changed. A real install does this, in order, and can be resumed at any step:\n"));
    let steps = [
        "Check this server: the read-only preflight below.".to_string(),
        format!(
            "Your domain: {domain}, mail hostname {hostname}, first mailbox {}@{domain}, sending {sending}.",
            args.first_mailbox
        ),
        match &args.dns_provider {
            Some(prov) => format!(
                "Check the {prov} token (read-only), publish {hostname}'s A record through it, and wait until public resolvers see it."
            ),
            None => format!(
                "Wait until {hostname} points at this server (checked against public resolvers)."
            ),
        },
        format!("Firewall: {firewall}"),
        format!(
            "Download Stalwart {} from {engine_url}\n       verify SHA-256 {engine_sha}, install as {}; apt install unbound dns-root-data; create users stalwart and yunori.",
            os::ENGINE.version,
            p.stalwart_bin().display()
        ),
        format!(
            "Configure the engine: {} and its store in {}. Its first-run management port is limited to loopback while it runs.",
            p.stalwart_config().display(),
            p.stalwart_data().display()
        ),
        format!(
            "Write {}/stalwart.service and yunori-mail.service (sandboxed), start the console and HTTPS edge on ports 80 and 443.",
            p.systemd_dir().display()
        ),
        format!(
            "Get a Let's Encrypt certificate for {hostname} (rehearsed on staging first, with backoff)."
        ),
        format!(
            "Publish DNS records for {domain} (MX, SPF, DKIM, DMARC, MTA-STS, TLS-RPT, autoconfig){}: each one is verified, never assumed.",
            if args.dns_provider.is_some() {
                " through the provider's API, only those missing or wrong, never deleting any"
            } else {
                ""
            }
        ),
        format!("Reverse DNS: this server's IP must point back to {hostname}."),
        format!("Create the first mailbox {}@{domain}.", args.first_mailbox),
        "Finish: print the one-time console setup link (it is shown, never logged).".into(),
    ];
    for (i, s) in steps.iter().enumerate() {
        println!("  {:>2}. {s}", i + 1);
    }
    println!(
        "\n  State: {} (SQLite, audit log), secrets sealed with {} (0600). Generated secrets are never printed.\n",
        p.db().display(),
        p.master_key().display()
    );
    preflight_cmd(
        args.hostname
            .clone()
            .or_else(|| args.domain.as_ref().map(|_| hostname.clone())),
        false,
    )
    .await
}

async fn preflight_cmd(hostname: Option<String>, json: bool) -> Result<()> {
    let paths = Paths::from_env();
    let t = Term::new();
    if !json {
        println!("{}", t.paint("1", "Yunori Mail Server · preflight"));
        println!(
            "{}",
            t.paint(
                "2",
                "Checks whether this machine can run a mail server. Nothing is changed.\n"
            )
        );
        if !os::is_root() {
            println!(
                "{}",
                t.paint(
                    "33",
                    "  Not running as root: port checks may be incomplete (try sudo).\n"
                )
            );
        }
    }
    let hostname = hostname
        .map(|h| {
            crate::util::normalize_domain(&h)
                .ok_or_else(|| anyhow!("'{h}' is not a valid hostname"))
        })
        .transpose()?;
    let results = crate::checks::preflight(&paths, hostname.as_deref()).await;
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        print_checks(&t, &results);
        let (ok, sentence) = preflight_verdict(&results);
        println!(
            "\n  {}",
            t.paint(if ok { "32;1" } else { "31;1" }, &sentence)
        );
        if ok {
            println!("  Next: sudo yunori-mail install");
        }
    }
    if crate::checks::has_blocking_failure(&results) {
        std::process::exit(2);
    }
    Ok(())
}

async fn install(args: InstallArgs) -> Result<()> {
    if args.dry_run {
        return install_dry_run(&args).await;
    }
    let paths = Paths::from_env();
    if paths.is_production() && !os::is_root() {
        bail!("run the installer as root: sudo yunori-mail install");
    }
    let app = App::open(paths, false, false)?;
    let mut term = Term::new();
    println!(
        "{} {}",
        term.paint("1", "Yunori Mail Server"),
        term.paint("2", crate::app::VERSION)
    );
    println!(
        "{}",
        term.paint(
            "2",
            "Installs a real mail server on this machine. Every step can be resumed: if anything stops,\nfix it and run `sudo yunori-mail install --resume`."
        )
    );

    // Re-running the one-line installer is always safe: a finished install is
    // updated in place, a paused one picks up where it stopped.
    if let Some(cfg) = app.config().filter(|c| c.install_complete) {
        println!(
            "Already installed on this machine ({}). Updating it instead.",
            cfg.hostname
        );
        return update_in_place(&app).await;
    }
    if let Some(dir) = &args.acme_directory {
        crate::edge::acme::save_directory(&app.db, dir)?;
    }
    let mut inst = Installer::new(app.clone(), Mode::Cli);
    inst.force = args.force;
    inst.skip_ptr = args.skip_ptr;
    let has_saved = app.db.get_kv("install.inputs")?.is_some();
    let resume = has_saved && (args.resume || args.domain.is_none());
    if has_saved && !args.resume && args.domain.is_none() {
        println!("\nResuming the install that was started here before.");
    }
    if !resume {
        let ask_questions = interactive() && !args.yes;
        // Preflight first: nothing is asked, or installed, on a server that can't work.
        println!("\n{}", term.paint("1", "Checking this server…"));
        let results = crate::checks::preflight(&app.paths, None).await;
        print_checks(&term, &results);
        let (ok, sentence) = preflight_verdict(&results);
        println!(
            "\n  {}",
            term.paint(if ok { "32;1" } else { "31;1" }, &sentence)
        );
        if !ok && !args.force {
            println!(
                "  Fix the problems marked above, then run the installer again (or pass --force to continue anyway)."
            );
            std::process::exit(2);
        }
        let port25_open = results
            .iter()
            .any(|c| c.id == "port25" && c.status == Status::Pass);
        let ipv4 = results
            .iter()
            .find(|c| c.id == "public_ip")
            .and_then(|c| c.detail.strip_prefix("IPv4 "))
            .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());
        inst.preflight_results = Some(results);

        let inputs = gather_inputs(&args, &term, ask_questions, port25_open, ipv4.as_deref())?;
        // A DNS provider token is checked now (read-only), before anything is installed.
        if let Some(provider) = &args.dns_provider {
            let token = read_token_env(args.dns_token_env.as_deref())?;
            let l = crate::dns_publish::connect(
                &app,
                &inputs.domain,
                provider,
                Some(&token),
                "install",
            )
            .await
            .map_err(|e| anyhow!("DNS provider: {e}"))?;
            println!(
                "  {} DNS: {} zone {} is connected; the installer publishes the records itself.",
                term.paint("32", "✓"),
                provider_label(&l.provider),
                l.zone_name
            );
        }
        if ask_questions {
            print_summary(&term, &inputs, ipv4.as_deref());
            if !confirm("Install with these settings?", true)? {
                println!("Nothing was installed. Run the installer again to start over.");
                return Ok(());
            }
        }
        inst.save_inputs(&inputs)?;
    }

    // The one-line script swapped the binary of a paused install: the running
    // console/edge must pick it up too, or it keeps the old code.
    if resume
        && app.paths.is_production()
        && std::env::var("YUNORI_BINARY_REPLACED").is_ok_and(|v| v == "1")
        && matches!(
            app.db.step_status("edge_tls")?,
            crate::installer::state::StepStatus::Done
        )
    {
        crate::installer::upgrade::refresh_units(&app)?;
        os::systemctl(&["restart", "yunori-mail"])?;
        println!(
            "  Restarted the console and HTTPS edge on yunori-mail {}.",
            crate::app::VERSION
        );
    }

    let outcome = loop {
        match inst.run(&mut term).await {
            Ok(RunResult::Complete) => break Ok(None),
            Ok(RunResult::Waiting { step, .. }) => {
                if args.no_wait || !interactive() {
                    break Ok(Some(step));
                }
                let secs = wait_seconds(&app, &step);
                if !countdown(&term, secs).await {
                    break Ok(Some(step));
                }
                app.db
                    .set_step(&step, crate::installer::state::StepStatus::Pending, None)?;
            }
            Err(e) => break Err(e),
        }
    };
    final_summary(
        &app,
        &term,
        outcome.as_ref().ok().cloned().flatten().as_deref(),
        outcome.as_ref().err(),
    );
    if outcome.is_err() {
        std::process::exit(1);
    }
    Ok(())
}

/// How long to wait before re-checking a gate.
fn wait_seconds(app: &App, step: &str) -> u64 {
    match step {
        "host_record" => 15,
        "certificate" => {
            let st = crate::edge::certs::load_state(app);
            st.next_attempt
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| (t.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds())
                .unwrap_or(30)
                .clamp(10, 600) as u64
        }
        "ptr" => 60,
        _ => 30,
    }
}

/// A one-line countdown. Returns false if the user pressed Ctrl-C.
async fn countdown(t: &Term, secs: u64) -> bool {
    let started = std::time::Instant::now();
    let total = Duration::from_secs(secs);
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        let left = total.saturating_sub(started.elapsed()).as_secs();
        if left == 0 {
            if t.color {
                print!("\r\x1b[2K");
            }
            return true;
        }
        let line = format!(
            "  Checking again in {} · Ctrl-C to pause (resume later with --resume)",
            fmt_duration(left)
        );
        if t.color {
            print!("\r\x1b[2K{}", t.paint("2", &line));
            let _ = std::io::stdout().flush();
        } else if left == secs {
            println!("{line}");
        }
        tokio::select! {
            _ = &mut ctrl_c => {
                println!();
                return false;
            }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

fn gather_inputs(
    args: &InstallArgs,
    t: &Term,
    ask_questions: bool,
    port25_open: bool,
    ipv4: Option<&str>,
) -> Result<InstallInputs> {
    if ask_questions {
        println!(
            "\n{}",
            t.paint("1", "A few questions (Enter accepts the [suggestion]):")
        );
    }
    let domain = match &args.domain {
        Some(d) => crate::util::normalize_domain(d)
            .ok_or_else(|| anyhow!("'{d}' is not a valid domain"))?,
        None => {
            let d = ask(
                t,
                "Mail domain, the part after @ (e.g. example.com)",
                None,
                |v| {
                    crate::util::normalize_domain(v).ok_or_else(|| {
                        format!("'{v}' doesn't look like a domain. Try something like example.com.")
                    })
                },
            )?;
            if let Some(parent) = d.strip_prefix("mail.").filter(|p| p.contains('.'))
                && confirm(
                    &format!("Addresses would look like you@{d}. Use {parent} instead?"),
                    true,
                )?
            {
                parent.to_string()
            } else {
                d
            }
        }
    };
    let suggested_host = format!("mail.{domain}");
    let hostname = match &args.hostname {
        Some(h) => crate::util::normalize_domain(h)
            .ok_or_else(|| anyhow!("'{h}' is not a valid hostname"))?,
        None if ask_questions => ask(t, "Mail server hostname", Some(&suggested_host), |v| {
            crate::util::normalize_domain(v)
                .ok_or_else(|| format!("'{v}' is not a valid hostname."))
        })?,
        None => suggested_host,
    };
    let check_admin = |v: &str| {
        let v = v.trim().to_ascii_lowercase();
        if !crate::util::valid_email(&v) {
            Err(format!("'{v}' is not an email address."))
        } else if v.ends_with(&format!("@{domain}")) {
            Err(format!(
                "Use an address that does not depend on this server (not @{domain}): certificate and alert mail must reach you even if this server is down."
            ))
        } else {
            Ok(v)
        }
    };
    let admin_email = match &args.admin_email {
        Some(e) => check_admin(e).map_err(|m| anyhow!(m))?,
        None => ask(
            t,
            "Your email on another provider, for certificates and alerts",
            None,
            check_admin,
        )?,
    };
    let first_mailbox = if ask_questions && args.first_mailbox == "hello" {
        ask(
            t,
            &format!("First mailbox (…@{domain})"),
            Some("hello"),
            |v| {
                let v = v.trim().to_ascii_lowercase();
                if crate::util::valid_local_part(&v) {
                    Ok(v)
                } else {
                    Err("Use letters, digits, dots, dashes or underscores.".into())
                }
            },
        )?
    } else {
        args.first_mailbox.trim().to_ascii_lowercase()
    };

    let sending = match args.sending {
        Some(SendingArg::Direct) => SendingMode::Direct,
        Some(SendingArg::Relay) => SendingMode::Relay,
        None if ask_questions => {
            if port25_open {
                println!(
                    "    This server can reach other mail servers on port 25, so it can deliver mail itself."
                );
            } else {
                println!(
                    "    {}",
                    t.paint("33", "Outbound port 25 looks blocked here, so mail should go out through a relay provider (receiving works either way).")
                );
            }
            let default = if port25_open { "1" } else { "2" };
            let pick = ask(
                t,
                "Send mail 1) directly  2) through a relay provider",
                Some(default),
                |v| match v.trim() {
                    "1" | "direct" => Ok("direct".into()),
                    "2" | "relay" => Ok("relay".into()),
                    _ => Err("Type 1 or 2.".into()),
                },
            )?;
            if pick == "relay" {
                SendingMode::Relay
            } else {
                SendingMode::Direct
            }
        }
        None => SendingMode::Direct,
    };

    let relay = if sending == SendingMode::Relay {
        Some(gather_relay(args, t, ask_questions)?)
    } else {
        None
    };
    let _ = ipv4;
    Ok(InstallInputs {
        domain,
        hostname: hostname.trim().to_ascii_lowercase(),
        admin_email,
        first_mailbox,
        sending,
        relay,
        client_origin: args.client_origin.trim_end_matches('/').to_string(),
    })
}

fn gather_relay(args: &InstallArgs, t: &Term, ask_questions: bool) -> Result<RelayInput> {
    use crate::model::{RELAY_PROVIDERS, relay_provider};
    let provider = match &args.relay_provider {
        Some(p) => p.clone(),
        None if ask_questions => {
            let list: Vec<String> = RELAY_PROVIDERS
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}) {}", i + 1, p.label))
                .collect();
            println!("    Providers: {}", list.join("  "));
            ask(t, "Relay provider", Some("1"), |v| {
                let v = v.trim().to_ascii_lowercase();
                v.parse::<usize>()
                    .ok()
                    .and_then(|i| RELAY_PROVIDERS.get(i.wrapping_sub(1)))
                    .or_else(|| relay_provider(&v))
                    .map(|p| p.key.to_string())
                    .ok_or_else(|| "Pick a number from the list.".into())
            })?
        }
        None => "custom".into(),
    };
    let preset =
        relay_provider(&provider).ok_or_else(|| anyhow!("unknown relay provider '{provider}'"))?;
    let region = match (&args.relay_region, preset.regions.is_empty()) {
        (Some(r), _) => Some(r.clone()),
        (None, false) if ask_questions => Some(ask(
            t,
            &format!("{} region", preset.label),
            preset.default_region,
            |v| {
                let v = v.trim().to_ascii_lowercase();
                if preset.regions.contains(&v.as_str()) {
                    Ok(v)
                } else {
                    Err(format!("One of: {}", preset.regions.join(", ")))
                }
            },
        )?),
        (None, false) => preset.default_region.map(str::to_string),
        (None, true) => None,
    };
    let host = match &args.relay_host {
        Some(h) => h.clone(),
        None if preset.host.is_none() && preset.host_template.is_none() => {
            if !ask_questions {
                bail!("--relay-host is required for this provider");
            }
            ask(t, "SMTP host", None, |v| {
                crate::util::normalize_domain(v)
                    .ok_or_else(|| "Enter a host name like smtp.example.net.".into())
            })?
        }
        None => String::new(),
    };
    let username = match (&args.relay_user, preset.username) {
        (Some(u), _) => u.clone(),
        (None, Some(fixed)) => fixed.to_string(),
        (None, None) if ask_questions => ask(t, "SMTP username", None, |v| {
            if v.trim().is_empty() {
                Err("The provider gives you a username.".into())
            } else {
                Ok(v.trim().to_string())
            }
        })?,
        (None, None) => bail!("--relay-user is required"),
    };
    let password = match &args.relay_password_env {
        Some(var) => {
            std::env::var(var).with_context(|| format!("environment variable {var} is not set"))?
        }
        None if interactive() => {
            rpassword::prompt_password(format!("  {} (hidden): ", preset.secret_label))?
        }
        None => bail!("pass the relay password with --relay-password-env VAR"),
    };
    Ok(RelayInput {
        provider,
        host,
        port: args.relay_port,
        username,
        region,
        password,
    })
}

fn print_summary(t: &Term, i: &InstallInputs, ipv4: Option<&str>) {
    let row = |k: &str, v: String| println!("    {:<16} {v}", t.paint("2", k));
    println!("\n  {}", t.paint("1", "Summary"));
    row("Domain", i.domain.clone());
    row(
        "Mail hostname",
        match ipv4 {
            Some(ip) => format!("{} → {ip}", i.hostname),
            None => i.hostname.clone(),
        },
    );
    row("Alerts go to", i.admin_email.clone());
    row("First mailbox", format!("{}@{}", i.first_mailbox, i.domain));
    row(
        "Sending",
        match (&i.sending, &i.relay) {
            (SendingMode::Relay, Some(r)) => format!(
                "through {}",
                crate::model::relay_provider(&r.provider)
                    .map(|p| p.label)
                    .unwrap_or("a relay")
            ),
            _ => "directly from this server (needs reverse DNS)".into(),
        },
    );
    row(
        "Certificate",
        format!(
            "{}, automatic",
            crate::edge::acme::ca_name(
                &crate::edge::acme::env_directory()
                    .unwrap_or_else(|| crate::edge::acme::LE_PRODUCTION.into())
            )
        ),
    );
    println!();
}

/// What the person gets, and what is left — printed when the run ends or pauses.
fn final_summary(app: &App, t: &Term, paused_at: Option<&str>, error: Option<&anyhow::Error>) {
    let Some(cfg) = app.config() else {
        if let Some(e) = error {
            println!(
                "\nThe install stopped: {e:#}\nFix the problem, then run: sudo yunori-mail install --resume"
            );
        }
        return;
    };
    let rule = t.paint("2", &"─".repeat(64));
    println!("\n{rule}");
    let edge_up = matches!(
        app.db.step_status("edge_tls"),
        Ok(crate::installer::state::StepStatus::Done)
    );
    let paused_title = paused_at.and_then(|id| {
        crate::installer::state::STEPS
            .iter()
            .find(|(s, _)| *s == id)
            .map(|(_, t)| *t)
    });
    let headline = match (error, paused_title) {
        (Some(_), _) => t.paint("31;1", "The install stopped."),
        (None, Some(_)) if edge_up => t.paint(
            "33;1",
            "Paused. The server finishes the remaining steps by itself once you have done the one below.",
        ),
        (None, Some(title)) => t.paint("33;1", &format!("Paused at “{title}”.")),
        (None, None) => t.paint("32;1", "Yunori Mail Server is ready."),
    };
    println!("  {headline}");
    if let Some(e) = error {
        println!("  {e:#}");
        println!("  Fix the problem, then run: sudo yunori-mail install --resume");
    }
    let row = |k: &str, v: &str| println!("  {:<14} {v}", t.paint("2", k));
    if edge_up {
        println!();
        row("Console", &format!("https://{}/console", cfg.hostname));
        if let Some(url) = &t.setup_url {
            row("Finish setup", url);
        } else if app.db.admin_count_active().unwrap_or(0) == 0 {
            row(
                "Finish setup",
                "run `sudo yunori-mail setup-token` for a new link",
            );
        }
        row(
            "Mailbox portal",
            &format!("https://{}/account", cfg.hostname),
        );
    }
    let mailbox = t.mailbox.clone().or_else(|| {
        app.db
            .get_json::<InstallInputs>("install.inputs")
            .ok()
            .flatten()
            .filter(|_| {
                matches!(
                    app.db.step_status("first_mailbox"),
                    Ok(crate::installer::state::StepStatus::Done)
                )
            })
            .map(|i| format!("{}@{}", i.first_mailbox, i.domain))
    });
    if let Some(m) = &mailbox {
        row("Mailbox", m);
        row(
            "Mail apps",
            &format!(
                "IMAP {h}:993 (TLS) · SMTP {h}:465 (TLS) or 587 (STARTTLS)",
                h = cfg.hostname
            ),
        );
        row(
            "Yunori",
            &format!("{}/welcome?connect={m}", cfg.client_origin),
        );
    }
    if let Ok(st) = crate::installer::state::load(&app.db) {
        use crate::installer::state::StepStatus as S;
        let now: Vec<_> = st
            .steps
            .iter()
            .filter(|s| matches!(s.status, S::Waiting | S::Failed))
            .collect();
        let later = st
            .steps
            .iter()
            .filter(|s| matches!(s.status, S::Pending | S::Running))
            .count();
        if !now.is_empty() && error.is_none() {
            println!("\n  {}", t.paint("1", "Your turn"));
            for s in &now {
                match &s.detail {
                    Some(d) if !d.is_empty() => println!("    · {}: {d}", s.title),
                    _ => println!("    · {}", s.title),
                }
            }
            if later > 0 {
                println!(
                    "    {}",
                    t.paint(
                        "2",
                        &format!(
                            "{later} more step{} follow{} after that.",
                            if later == 1 { "" } else { "s" },
                            if later == 1 { "s" } else { "" }
                        )
                    )
                );
            }
            if edge_up {
                println!(
                    "  {}",
                    t.paint("2", "The console shows the same list and re-checks every minute; or run: sudo yunori-mail install --resume")
                );
            } else {
                println!(
                    "  {}",
                    t.paint("2", "Then run: sudo yunori-mail install --resume")
                );
            }
        }
    }
    println!("{rule}");
}

pub async fn serve(args: ServeArgs) -> Result<()> {
    let app = open_app(args.dev, args.mock_engine)?;
    if let Some(url) = &args.engine_url {
        let admin = std::env::var("YUNORI_ENGINE_ADMIN")
            .context("set YUNORI_ENGINE_ADMIN=user:password for --engine-url")?;
        let mut cfg = app.config().unwrap_or(crate::model::ServerConfig {
            hostname: "mail.live.test".into(),
            primary_domain: "live.test".into(),
            admin_email: "owner@elsewhere.example".into(),
            client_origin: crate::app::DEFAULT_CLIENT_ORIGIN.into(),
            engine_url: url.clone(),
            sending_mode: crate::model::SendingMode::Direct,
            public_ipv4: Some("203.0.113.10".into()),
            public_ipv6: None,
            stalwart_version: Some(os::ENGINE.version.into()),
            install_complete: true,
        });
        cfg.engine_url = url.clone();
        app.save_config(&cfg)?;
        app.db.put_secret("engine.admin", &admin)?;
        app.reload_engine()?;
        println!("Development: using the engine at {url}");
    }
    if args.dev && app.db.admin_count_active()? == 0 {
        let token = app.db.issue_setup_token()?;
        println!(
            "Development setup token: {token}\nOpen http://{}/console/setup#token={token}",
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
    let (_, engine) = ops::engine_check(
        app.engine().ok(),
        cfg.as_ref().and_then(|c| c.stalwart_version.clone()),
    )
    .await;
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
        checks.push(crate::edge::certs::check(&app));
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
        "Open this one-time link to create the console owner (valid 24 hours; any earlier link stops working):\n\n  {}\n\nOr open https://{host}/console/setup and paste the token: {token}\n",
        crate::installer::setup_link(&host, &token)
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
        DomainCmd::List { json } => {
            let accounts = e.list_accounts().await?;
            if json {
                let out: Vec<_> = e
                    .list_domains()
                    .await?
                    .into_iter()
                    .map(|d| {
                        let (st, _) = ops::cached_dns_status(&app, &d.name);
                        serde_json::json!({
                            "name": d.name,
                            "enabled": d.enabled,
                            "catchAll": d.catch_all,
                            "mailboxes": accounts.iter().filter(|a| a.domain_id == d.id).count(),
                            "dnsStatus": st,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
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
            crate::dns_publish::disconnect(&app, &d.name, "cli")?;
            println!("Removed {}.", d.name);
        }
        DomainCmd::Dns {
            name,
            connect,
            token_env,
            disconnect,
            publish,
            dry_run,
        } => {
            let d = find_domain_id(&app, &name).await?;
            if disconnect {
                if crate::dns_publish::disconnect(&app, &d.name, "cli")? {
                    println!(
                        "Disconnected the DNS provider for {}. Published records stay as they are.",
                        d.name
                    );
                } else {
                    println!("No DNS provider was connected for {}.", d.name);
                }
                return Ok(());
            }
            if let Some(provider) = connect {
                let token = read_token_env(token_env.as_deref())?;
                let l = crate::dns_publish::connect(&app, &d.name, &provider, Some(&token), "cli")
                    .await
                    .map_err(|e| anyhow!("{e}"))?;
                println!(
                    "Connected {} (zone {}). The token is stored encrypted.",
                    provider_label(&l.provider),
                    l.zone_name
                );
                if !publish {
                    println!(
                        "Publish the records with: yunori-mail domain dns {} --publish",
                        d.name
                    );
                    return Ok(());
                }
            }
            if publish {
                let out = crate::dns_publish::publish(
                    &app,
                    &d,
                    crate::dns_publish::PublishRequest {
                        only: None,
                        plan_id: None,
                        dry_run,
                        check: !dry_run,
                    },
                    "cli",
                )
                .await
                .map_err(|e| anyhow!("{e}"))?;
                print_plan(&t, &out);
                println!("\n  {}", crate::dns_publish::sentence(&out));
                match &out.dns {
                    Some(dns) => {
                        println!("\nPublic DNS right now:");
                        print_records(&t, &dns.records);
                        println!(
                            "\nResolvers can take a few minutes to see new records; check again with: yunori-mail domain dns {}",
                            d.name
                        );
                    }
                    None => println!("Nothing was changed (dry run)."),
                }
                if out.failed > 0 {
                    std::process::exit(1);
                }
                return Ok(());
            }
            let dns = ops::domain_dns(&app, &d, true).await?;
            print_records(&t, &dns.records);
            println!("\nZone file:\n{}", dns.zone_file);
            if crate::dns_publish::link(&app, &d.name)?.is_some() {
                println!(
                    "A DNS provider is connected: publish with `yunori-mail domain dns {} --publish`.",
                    d.name
                );
            }
        }
    }
    Ok(())
}

fn read_token_env(var: Option<&str>) -> Result<String> {
    match var {
        Some(v) => std::env::var(v).with_context(|| format!("environment variable {v} is not set")),
        None if interactive() => Ok(rpassword::prompt_password("  API token (hidden): ")?),
        None => bail!("pass the token with --token-env VAR (never as an argument)"),
    }
}

fn provider_label(id: &str) -> &'static str {
    crate::dns::provider::info(id)
        .map(|p| p.label)
        .unwrap_or("DNS provider")
}

fn print_plan(t: &Term, out: &crate::dns_publish::PublishOutcome) {
    use crate::dns::provider::plan::Action;
    println!(
        "{} zone {}:",
        out.preview.provider_label, out.preview.zone_name
    );
    for i in &out.preview.items {
        let (tag, color) = match i.action {
            Action::Create => ("create", "32"),
            Action::Update => ("update", "33"),
            Action::Unchanged => ("ok", "2"),
            Action::Manual => ("you", "31"),
            Action::Skip => ("skip", "2"),
        };
        let result = out.results.iter().find(|r| r.id == i.id);
        let mark = match result {
            Some(r) if r.ok => " ✓".to_string(),
            Some(r) => format!(" ✗ {}", r.error.as_deref().unwrap_or_default()),
            None => String::new(),
        };
        println!(
            "  [{}] {:<5} {}{mark}",
            t.paint(color, &format!("{tag:^6}")),
            i.rtype,
            i.name
        );
        if i.is_change() {
            for c in &i.current {
                println!("           was {}", t.paint("2", c));
            }
            println!(
                "           {} {}",
                if i.current.is_empty() { "new" } else { "now" },
                i.value
            );
        }
        if let Some(n) = &i.note
            && i.action != Action::Unchanged
        {
            println!("           {}", t.paint("2", n));
        }
    }
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
    if let MailboxCmd::Migrate(args) = c {
        return crate::migrate::cli::run(&app, args).await;
    }
    let e = app.engine()?;
    match c {
        MailboxCmd::List { json } => {
            let domains = e.list_domains().await?;
            if json {
                let out: Vec<_> = e
                    .list_accounts()
                    .await?
                    .into_iter()
                    .map(|a| {
                        serde_json::json!({
                            "address": a.address,
                            "displayName": a.display_name,
                            "role": if a.is_admin { "admin" } else { "user" },
                            "quotaBytes": a.quota_bytes,
                            "usedBytes": a.used_bytes,
                            "aliases": crate::engine::alias_addresses(&a, &domains),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
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
        MailboxCmd::Export { out } => {
            let domains = e.list_domains().await?;
            let accounts = e.list_accounts().await?;
            let csv = crate::api::admin::mailboxes_csv(&accounts, &domains);
            app.db.audit(
                "cli",
                "mailbox.export",
                None,
                Some(&format!("{} mailboxes", accounts.len())),
            )?;
            match out {
                Some(path) => {
                    std::fs::write(&path, csv)?;
                    eprintln!("Wrote {} mailboxes to {}.", accounts.len(), path.display());
                }
                None => print!("{csv}"),
            }
        }
        MailboxCmd::Import {
            file,
            passwords,
            dry_run,
            json,
        } => {
            let text = if file.as_os_str() == "-" {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                s
            } else {
                std::fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?
            };
            let host = app.config().map(|c| c.hostname).unwrap_or_default();
            let report = crate::api::admin::import_csv(
                &app,
                "cli",
                &text,
                !passwords,
                dry_run,
                &format!("https://{host}"),
            )
            .await
            .map_err(|e| anyhow!(e.message))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(());
            }
            let list = |k: &str| report[k].as_array().cloned().unwrap_or_default();
            if dry_run {
                println!(
                    "Dry run: {} mailbox(es) would be created.",
                    list("ready").len()
                );
                for a in list("ready") {
                    println!("  + {}", a.as_str().unwrap_or_default());
                }
            } else {
                println!(
                    "Created {} mailbox(es) (links and passwords are shown once):",
                    list("created").len()
                );
                for c in list("created") {
                    let secret = c["inviteUrl"]
                        .as_str()
                        .or_else(|| c["password"].as_str())
                        .unwrap_or_default();
                    println!(
                        "  + {:<36} {secret}",
                        c["address"].as_str().unwrap_or_default()
                    );
                }
            }
            let skipped = list("skipped");
            if !skipped.is_empty() {
                println!("Skipped {}:", skipped.len());
                for s in skipped {
                    println!(
                        "  - line {:<4} {:<36} {}",
                        s["line"]
                            .as_u64()
                            .map(|n| n.to_string())
                            .unwrap_or_default(),
                        s["address"].as_str().unwrap_or_default(),
                        s["reason"].as_str().unwrap_or_default()
                    );
                }
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
                    quota_mb.map(|m| m.min(16 * 1024 * 1024) * 1024 * 1024),
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
            if a.is_admin {
                bail!(
                    "{} is the mail engine's administrator; Yunori holds its password, so it is not changed here",
                    a.address
                );
            }
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
        MailboxCmd::Migrate(_) => unreachable!("handled above"),
    }
    Ok(())
}

async fn alias_cmd(c: AliasCmd) -> Result<()> {
    let app = open_app(false, false)?;
    let e = app.engine()?;
    let domains = e.list_domains().await?;
    let accounts = e.list_accounts().await?;
    match c {
        AliasCmd::List { json } => {
            if json {
                let out: Vec<_> = accounts
                    .iter()
                    .flat_map(|a| {
                        crate::engine::alias_addresses(a, &domains)
                            .into_iter()
                            .map(|al| serde_json::json!({ "alias": al, "mailbox": a.address }))
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
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
    use crate::edge::certs;
    let app = open_app(false, false)?;
    let t = Term::new();
    match c {
        CertCmd::Status => {
            let s = certs::status(&app);
            let row = |k: &str, v: &str| println!("  {:<14} {v}", t.paint("2", k));
            match s.state {
                "missing" => row("Certificate", "none yet"),
                _ => {
                    row("Covers", &s.hostnames.join(", "));
                    row("Issuer", s.issuer.as_deref().unwrap_or("—"));
                    if s.state == "self_signed" {
                        row(
                            "Expires",
                            "— (temporary self-signed; replaced as soon as the real one is issued)",
                        );
                    } else {
                        row(
                            "Expires",
                            &format!(
                                "{} ({} days){}",
                                s.not_after.as_deref().unwrap_or("—"),
                                s.days_left.unwrap_or(0),
                                match s.state {
                                    "test" => " — test certificate",
                                    "expired" => " — EXPIRED",
                                    _ => "",
                                }
                            ),
                        );
                    }
                }
            }
            row("Authority", &format!("{} (automatic renewal)", s.authority));
            if let Some(r) = &s.renews_at {
                row("Renews from", r);
            }
            if let Some(p) = &s.problem {
                row("Last attempt", &p.sentence());
                if let Some(d) = &p.detail {
                    row("CA said", d);
                }
            }
            if let Some(n) = &s.next_attempt {
                row("Next attempt", &crate::util::when(n));
            }
        }
        CertCmd::Renew => {
            if app.config().is_none() {
                bail!("not installed");
            }
            println!("Requesting a certificate…");
            match certs::ensure(&app, true).await {
                certs::Outcome::Issued(names) => println!("Issued for {}.", names.join(", ")),
                certs::Outcome::Current => println!(
                    "The current certificate is valid and covers every name; nothing to do."
                ),
                certs::Outcome::Busy => {
                    println!("A certificate is being requested right now; check again in a minute.")
                }
                certs::Outcome::Failed {
                    problem,
                    next_attempt,
                }
                | certs::Outcome::Later {
                    problem: Some(problem),
                    next_attempt,
                } => {
                    println!("{}", problem.sentence());
                    if let Some(d) = &problem.detail {
                        println!("The CA said: {d}");
                    }
                    println!(
                        "Yunori tries again by itself {}.",
                        crate::util::when(&next_attempt)
                    );
                    std::process::exit(1);
                }
                certs::Outcome::Later {
                    problem: None,
                    next_attempt,
                } => {
                    println!("Next attempt {}.", crate::util::when(&next_attempt));
                }
                certs::Outcome::NotConfigured => bail!("not installed"),
            }
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
    std::fs::create_dir_all(&work)?;
    // The engine runs as `stalwart` and cannot enter Yunori's state directory
    // (root:yunori 2770), so it exports into a directory of its own.
    let io = engine_io_dir(&p, &format!("export-{stamp}"))?;
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
            &format!("--export={}", io.join("engine").display()),
        ],
    );
    os::systemctl(&["start", "stalwart"])?;
    if let Err(e) = export {
        let _ = std::fs::remove_dir_all(&io);
        let _ = std::fs::remove_dir_all(&work);
        return Err(e.context("engine export failed"));
    }
    move_dir(&io.join("engine"), &work.join("engine"))?;
    let _ = std::fs::remove_dir_all(&io);
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
    let result = restore_stopped(&p, &work);
    // Whatever happened, the server comes back up: restored, or as it was.
    let started = os::systemctl(&["start", "stalwart"]);
    let _ = os::systemctl(&["start", "yunori-mail"]);
    let _ = std::fs::remove_dir_all(&work);
    result?;
    started?;
    // `--import` brings back every message but not the search index: until it
    // is rebuilt, IMAP SEARCH and JMAP queries find nothing (seen live).
    println!("Rebuilding the search index in the background…");
    let mut reindexed = false;
    for _ in 0..30 {
        if let Ok(app) = open_app(false, false) {
            if let Ok(e) = app.engine() {
                if e.reindex_accounts().await.is_ok() {
                    reindexed = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    if !reindexed {
        println!(
            "  The engine did not answer; search may miss restored mail until it is reindexed."
        );
    }
    println!(
        "Restored. Point MX, A and reverse DNS at this server if it is a new machine, then run: yunori-mail doctor"
    );
    Ok(())
}

/// The restore itself, with both services stopped. Stalwart imports only into
/// an empty store, so the current one is moved aside first and put back if
/// anything fails; it is deleted only once the import has succeeded.
fn restore_stopped(p: &Paths, work: &std::path::Path) -> Result<()> {
    let io = engine_io_dir(p, "import")?;
    move_dir(&work.join("engine"), &io.join("engine"))?;
    os::chown_r("stalwart", &io)?;
    let data = p.stalwart_data();
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let aside = data.with_file_name(format!(
        "{}.pre-restore-{stamp}",
        data.file_name().unwrap_or_default().to_string_lossy()
    ));
    if data.exists() {
        std::fs::rename(&data, &aside).context("moving the current mail store aside")?;
    }
    let attempt = (|| -> Result<()> {
        std::fs::create_dir_all(&data)?;
        crate::store::vault::set_mode(&data, 0o770);
        os::chown_r("stalwart", &data)?;
        os::run(
            "runuser",
            &[
                "-u",
                "stalwart",
                "--",
                &p.stalwart_bin().to_string_lossy(),
                &format!("--config={}", p.stalwart_config().display()),
                &format!("--import={}", io.join("engine").display()),
            ],
        )
        .context("engine import failed")?;
        std::fs::copy(work.join("yunori.db"), p.db())?;
        std::fs::copy(work.join("master.key"), p.master_key())?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&io);
    match attempt {
        Ok(()) => {
            crate::installer::fix_permissions(p);
            let _ = std::fs::remove_dir_all(&aside);
            Ok(())
        }
        Err(e) => {
            if aside.exists() {
                let _ = std::fs::remove_dir_all(&data);
                std::fs::rename(&aside, &data).context("putting the previous mail store back")?;
            }
            Err(e.context("nothing was changed: the previous mail store is back in place"))
        }
    }
}

/// A fresh, private directory the engine (user `stalwart`) can write to.
fn engine_io_dir(p: &Paths, name: &str) -> Result<std::path::PathBuf> {
    let root = p.engine_io_dir();
    std::fs::create_dir_all(&root)?;
    crate::store::vault::set_mode(&root, 0o700);
    os::chown_r("stalwart", &root)?;
    let dir = root.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    os::chown_r("stalwart", &dir)?;
    Ok(dir)
}

/// Rename, or copy then delete when the two are on different filesystems.
fn move_dir(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    os::run(
        "cp",
        &["-a", &from.to_string_lossy(), &to.to_string_lossy()],
    )?;
    std::fs::remove_dir_all(from)?;
    Ok(())
}

async fn upgrade() -> Result<()> {
    if !os::is_root() {
        bail!("run upgrades as root: sudo yunori-mail upgrade");
    }
    let app = open_app(false, false)?;
    if crate::installer::upgrade::refresh_units(&app)? {
        println!("Service units updated.");
        let _ = os::systemctl(&["restart", "yunori-mail"]);
    }
    println!("{}", crate::installer::upgrade::upgrade_engine(&app).await?);
    println!("To update yunori-mail itself: sudo yunori-mail update");
    Ok(())
}

/// `yunori-mail update [--check]`: compare with the newest release on
/// yunori.email, then (unless `--check`) run the same one-line installer people
/// install with. It verifies the download's SHA-256, swaps the binary, and hands
/// back to `yunori-mail install`, which updates a finished install in place.
async fn update_cmd(check_only: bool) -> Result<()> {
    let latest = match open_app(false, false) {
        Ok(app) => crate::updates::check(&app).await,
        // No state directory yet (or not root): ask without remembering.
        Err(_) => crate::updates::check_now().await,
    };
    match (&latest.latest, &latest.problem) {
        (Some(l), _) if latest.available => {
            println!(
                "yunori-mail {} is installed; {l} is available.",
                latest.current
            );
            if let Some(notes) = &latest.notes_url {
                println!("What changed: {notes}");
            }
        }
        (Some(l), None) => println!(
            "yunori-mail {} is up to date (newest: {l}).",
            latest.current
        ),
        (_, Some(p)) => println!("Couldn't check for updates: {p}"),
        (None, None) => println!("No release is published yet."),
    }
    if let (Some(p), Some(_)) = (&latest.problem, &latest.latest) {
        println!("(That is the last answer; checking just now failed: {p})");
    }
    if check_only {
        return Ok(());
    }
    if !latest.available && latest.problem.is_none() {
        return Ok(());
    }
    if !os::is_root() {
        bail!("run it as root: sudo yunori-mail update");
    }
    let script = crate::updates::fetch_installer().await?;
    let dir = tempfile_dir()?;
    let path = dir.join("install.sh");
    std::fs::write(&path, script)?;
    println!(
        "Running the installer from {}/install.sh…
",
        crate::updates::feed_base()
    );
    let status = std::process::Command::new("sh")
        .arg(&path)
        .env("YUNORI_FEED", crate::updates::feed_base())
        .status()
        .context("running the installer")?;
    let _ = std::fs::remove_dir_all(&dir);
    if !status.success() {
        bail!(
            "the installer stopped (exit {}). Nothing was changed after the step it reports.",
            status.code().unwrap_or(1)
        );
    }
    Ok(())
}

/// A private directory for the downloaded installer (0700, removed afterwards).
fn tempfile_dir() -> Result<std::path::PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!("yunori-update-{}", crate::util::random_token(8)));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

/// The one-line installer ran on a machine that is already set up: pick up a
/// new yunori-mail binary (the bootstrap script sets `YUNORI_BINARY_REPLACED=1`
/// when it swapped it), refresh the service units, then bring the engine to the
/// version this build pins (snapshot, health gate, rollback).
async fn update_in_place(app: &App) -> Result<()> {
    if !os::is_root() {
        bail!("run as root: sudo yunori-mail install");
    }
    let units_changed = crate::installer::upgrade::refresh_units(app)?;
    let replaced = std::env::var("YUNORI_BINARY_REPLACED").is_ok_and(|v| v == "1");
    if units_changed || replaced {
        os::systemctl(&["restart", "yunori-mail"])?;
        println!(
            "Restarted yunori-mail {} (console and HTTPS edge).",
            env!("CARGO_PKG_VERSION")
        );
    }
    println!("{}", crate::installer::upgrade::upgrade_engine(app).await?);
    app.db.audit(
        "installer",
        "update",
        Some(env!("CARGO_PKG_VERSION")),
        Some(&format!("engine {}", os::ENGINE.version)),
    )?;
    println!(
        "Up to date: yunori-mail {} with Stalwart {}.",
        env!("CARGO_PKG_VERSION"),
        os::ENGINE.version
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
