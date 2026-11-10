//! `yunori-mail mailbox migrate`: the administrator moves someone's mail in
//! (for example everyone's, from a script), with the same engine as the
//! portal and the same bookkeeping, so the person can later run it again
//! from the portal and only new mail is copied.
//!
//! Writing as the user: Stalwart 1.0 lets its administrator act for a user by
//! signing in as `<user>%<admin>` with the administrator's password (checked
//! live: the session is the user's, `Email/import` lands in their mailbox).
//! So the destination user's password is not needed here. The old server's
//! password is read from an environment variable (never from the command
//! line, where `ps` and shell history would show it) and kept in memory only.

use super::*;
use anyhow::{Context, bail};
use clap::Args;

#[derive(Args)]
pub struct MigrateArgs {
    /// The mailbox here that receives the mail (name@example.com).
    pub address: String,
    /// The old IMAP server, e.g. imap.gmail.com or outlook.office365.com.
    #[arg(long)]
    pub from_host: String,
    /// 993 (SSL/TLS) or 143 (STARTTLS).
    #[arg(long, default_value_t = 993)]
    pub from_port: u16,
    /// Username on the old server (usually the full address).
    #[arg(long)]
    pub from_user: String,
    /// Environment variable holding the old server's (app) password.
    #[arg(long, value_name = "VAR")]
    pub password_env: String,
    /// Folders to copy, by name (comma-separated). Default: all but Gmail's
    /// virtual folders (All Mail, Important, Starred).
    #[arg(long, value_delimiter = ',')]
    pub folders: Option<Vec<String>>,
    /// Count what would be copied; change nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Machine-readable result.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(app: &Arc<App>, a: MigrateArgs) -> Result<()> {
    let address = a.address.trim().to_ascii_lowercase();
    let engine = app.engine()?;
    let account = engine
        .list_accounts()
        .await?
        .into_iter()
        .find(|x| x.address == address)
        .ok_or_else(|| anyhow!("no mailbox {address} on this server"))?;
    if account.is_admin {
        bail!(
            "{address} is the mail engine's administrator account; import into a person's mailbox"
        );
    }
    let password = Zeroizing::new(
        std::env::var(&a.password_env)
            .with_context(|| format!("environment variable {} is not set", a.password_env))?,
    );
    let source = SourceSpec {
        host: a.from_host.clone(),
        port: a.from_port,
        username: a.from_user.trim().to_string(),
        password,
    };
    let say = |s: String| {
        if !a.json {
            eprintln!("{s}");
        }
    };

    say(format!("Connecting to {}…", source.label()));
    let plans = inspect(source.clone(), NetPolicy::default())
        .await
        .map_err(start_error)?;
    let selection = match &a.folders {
        None => None,
        Some(names) => {
            let mut ids = Vec::new();
            for n in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
                let p = plans
                    .iter()
                    .find(|p| p.name == n || p.id == n)
                    .or_else(|| plans.iter().find(|p| p.name.eq_ignore_ascii_case(n)))
                    .ok_or_else(|| {
                        anyhow!(
                            "no folder {n:?} on the old server; it has: {}",
                            plans
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?;
                ids.push(p.id.clone());
            }
            if ids.is_empty() {
                bail!("--folders names no folder");
            }
            Some(ids)
        }
    };
    let chosen = |p: &FolderPlan| match &selection {
        Some(ids) => ids.contains(&p.id),
        None => p.selected,
    };
    for p in &plans {
        say(format!(
            "  {} {:<32} {:>7}  → {}{}",
            if chosen(p) { "+" } else { " " },
            p.name,
            p.messages,
            p.destination,
            p.note
                .as_ref()
                .filter(|_| !chosen(p))
                .map(|n| format!("  ({n})"))
                .unwrap_or_default()
        ));
    }

    // The engine admin acting for the user (see the module comment).
    let admin = app.db.get_secret("engine.admin")?.ok_or_else(|| {
        anyhow!("the engine's admin credentials are missing; run `yunori-mail doctor`")
    })?;
    let (admin_user, admin_pass) = admin
        .split_once(':')
        .ok_or_else(|| anyhow!("the engine's admin credentials are malformed"))?;
    let dest = DestAuth::Direct(UserCreds {
        email: format!("{address}%{admin_user}"),
        password: Zeroizing::new(admin_pass.to_string()),
    });
    let job = start(
        app,
        JobRequest {
            account: address.clone(),
            actor: "cli".into(),
            source,
            folders: selection,
            dry_run: a.dry_run,
        },
        dest,
        NetPolicy::default(),
    )
    .await
    .map_err(start_error)?;

    let mut last = String::new();
    loop {
        let p = job.progress();
        if p.state != JobState::Running {
            break;
        }
        let line = format!(
            "  {}/{} folders · {} of {} messages · {}{}",
            p.folders_done,
            p.folders_total,
            p.messages_copied + p.messages_skipped + p.messages_failed,
            p.messages_total,
            fmt_bytes(p.bytes_copied),
            p.current_folder
                .as_ref()
                .map(|f| format!(" · {f}"))
                .unwrap_or_default()
        );
        if line != last {
            say(line.clone());
            last = line;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    job.wait().await;
    let p = job.progress();
    if a.json {
        println!("{}", serde_json::to_string_pretty(&p)?);
    } else {
        for e in &p.errors {
            eprintln!(
                "  ! {}{}",
                e.folder
                    .as_ref()
                    .map(|f| format!("{f}: "))
                    .unwrap_or_default(),
                e.message
            );
        }
        println!("{}", p.summary.clone().unwrap_or_default());
    }
    match p.state {
        JobState::Done => Ok(()),
        _ => bail!("the import did not finish; run the same command again to continue"),
    }
}

fn start_error(e: StartError) -> anyhow::Error {
    match e {
        StartError::Input(i) => anyhow!("{} ({})", i.message, i.field),
        StartError::Busy => anyhow!("an import into this mailbox is already running"),
        StartError::ServerBusy => {
            anyhow!("too many imports are running on this server; try again later")
        }
        StartError::Source(m) => anyhow!(m),
        StartError::Engine(e) => e.into(),
        StartError::Internal(e) => e,
    }
}
