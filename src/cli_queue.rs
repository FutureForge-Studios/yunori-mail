//! `yunori-mail queue …` and `yunori-mail access …`: the console's queue
//! manager and blocked-address view, for a terminal (and scripts: `--json`).
//! Every change is audit-logged with the actor `cli`.

use crate::app::App;
use crate::paths::Paths;
use crate::{ipaccess, mailqueue};
use anyhow::{Result, bail};
use clap::Subcommand;
use std::sync::Arc;

#[derive(Subcommand)]
pub enum QueueCmd {
    /// List queued messages, each recipient's status and the last problem.
    List {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Try delivering a message now (or every queued message with --all).
    Retry {
        /// Message id (from `queue list`).
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        id: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Cancel delivery of a message: it leaves the queue, no bounce is sent.
    #[command(alias = "cancel")]
    Delete { id: String },
    /// Hold all outgoing mail in the queue (incoming mail is unaffected).
    Pause,
    /// Deliver outgoing mail again.
    Resume,
}

#[derive(Subcommand)]
pub enum AccessCmd {
    /// Addresses blocked after failed sign-ins or abuse, the allowlist, and the ban policy.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Lift a block (by IP address or id).
    Unblock { address: String },
    /// Never block this IP address or network (CIDR, e.g. 203.0.113.0/24).
    Allow {
        address: String,
        /// A short note, e.g. "office".
        #[arg(long)]
        note: Option<String>,
    },
    /// Remove an allowlist entry (by address or id).
    Remove { address: String },
}

fn open() -> Result<Arc<App>> {
    App::open(Paths::from_env(), false, false)
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

fn day(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| {
            t.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d %H:%M UTC")
                .to_string()
        })
        .unwrap_or_else(|_| rfc3339.to_string())
}

pub async fn queue_cmd(c: QueueCmd) -> Result<()> {
    let app = open()?;
    match c {
        QueueCmd::List { json } => {
            let v = mailqueue::view(&app).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(());
            }
            print!("{}", render_queue(&v));
        }
        QueueCmd::Retry { id, all } => {
            let n = mailqueue::retry(&app, "cli", if all { None } else { id.as_deref() })
                .await
                .map_err(|e| {
                    not_found(
                        e,
                        "no queued message with that id (see `yunori-mail queue list`)",
                    )
                })?;
            if mailqueue::paused_at(&app).is_some() {
                println!(
                    "{} will be retried once the queue is resumed (`yunori-mail queue resume`).",
                    plural(n, "message")
                );
            } else {
                println!("Retrying {} now.", plural(n, "message"));
            }
        }
        QueueCmd::Delete { id } => {
            mailqueue::cancel(&app, "cli", &id).await.map_err(|e| {
                not_found(
                    e,
                    "no queued message with that id (see `yunori-mail queue list`)",
                )
            })?;
            println!("Cancelled {id}. It will not be delivered, and no bounce is sent.");
        }
        QueueCmd::Pause => {
            mailqueue::set_paused(&app, "cli", true).await?;
            println!(
                "Outgoing mail is paused: new mail waits in the queue. Incoming mail is unaffected. Resume with `yunori-mail queue resume`."
            );
        }
        QueueCmd::Resume => {
            mailqueue::set_paused(&app, "cli", false).await?;
            println!("Outgoing mail is flowing again.");
        }
    }
    Ok(())
}

pub fn render_queue(v: &mailqueue::QueueView) -> String {
    let mut out = String::new();
    let head = match &v.paused_at {
        Some(at) => format!(" · PAUSED since {}", day(at)),
        None => String::new(),
    };
    out.push_str(&format!(
        "Outgoing queue: {}{head}\n",
        plural(v.messages.len(), "message")
    ));
    if v.messages.is_empty() {
        out.push_str("  Nothing is waiting to be delivered.\n");
        return out;
    }
    for m in &v.messages {
        let from = if m.sender.is_empty() {
            "<> (bounce notice)"
        } else {
            &m.sender
        };
        out.push_str(&format!(
            "\n  {}  from {from}{}\n",
            m.id,
            m.created_at
                .as_deref()
                .map(|t| format!(", queued {}", day(t)))
                .unwrap_or_default()
        ));
        for r in &m.recipients {
            let mut line = format!("    {}  {}", r.address, r.status);
            if r.attempts > 0 {
                line.push_str(&format!(
                    " after {}",
                    plural(r.attempts as usize, "attempt")
                ));
            }
            if let Some(n) = &r.next_retry {
                line.push_str(&format!(" · next try {}", crate::util::when(n)));
            }
            if matches!(r.status, "waiting" | "retrying") {
                if let Some(e) = &r.expires_at {
                    line.push_str(&format!(" · gives up {}", day(e)));
                } else if let Some(n) = r.expires_after_attempts {
                    line.push_str(&format!(" · gives up after {n} attempts"));
                }
            }
            out.push_str(&line);
            out.push('\n');
            if let Some(p) = &r.problem {
                out.push_str(&format!("      {p}\n"));
            }
            if let Some(resp) = &r.response {
                out.push_str(&format!("      Server said: {resp}\n"));
            }
            if let Some(rem) = &r.remedy {
                out.push_str(&format!("      What to do: {rem}\n"));
            }
        }
    }
    out
}

pub async fn access_cmd(c: AccessCmd) -> Result<()> {
    let app = open()?;
    match c {
        AccessCmd::List { json } => {
            let v = ipaccess::view(&app).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(());
            }
            match &v.policy {
                Some(p) if p.enabled => println!(
                    "Sign-in protection: {} failed sign-ins within {} minutes block the address {}.",
                    p.failures.unwrap_or(0),
                    p.window_minutes.unwrap_or(0),
                    match p.ban_minutes {
                        Some(m) if m % 60 == 0 =>
                            format!("for {}", plural((m / 60) as usize, "hour")),
                        Some(m) => format!("for {}", plural(m as usize, "minute")),
                        None => "until someone unblocks it".into(),
                    }
                ),
                Some(_) => {
                    println!("Sign-in protection: OFF (failed sign-ins never block an address).")
                }
                None => {}
            }
            println!("\nBlocked: {}", v.blocked.len());
            for b in &v.blocked {
                println!(
                    "  {:<40} {} · {}",
                    b.address,
                    b.reason_text,
                    b.expires_at
                        .as_deref()
                        .map(|e| format!("until {}", day(e)))
                        .unwrap_or_else(|| "until unblocked".into())
                );
            }
            println!("\nAllowlist (never blocked): {}", v.allowed.len());
            for a in &v.allowed {
                println!("  {:<40} {}", a.address, a.reason.as_deref().unwrap_or(""));
            }
        }
        AccessCmd::Unblock { address } => {
            let b = ipaccess::unblock(&app, "cli", &address)
                .await
                .map_err(|e| {
                    not_found(
                        e,
                        "that address is not blocked (see `yunori-mail access list`)",
                    )
                })?;
            println!("Unblocked {}.", b.address);
        }
        AccessCmd::Allow { address, note } => {
            match ipaccess::allow(&app, "cli", &address, note.as_deref()).await {
                Ok(a) => println!("{} will never be blocked.", a.address),
                Err(ipaccess::AllowError::Other(e)) => return Err(e),
                Err(e) => bail!("{e}"),
            }
        }
        AccessCmd::Remove { address } => {
            let a = ipaccess::disallow(&app, "cli", &address)
                .await
                .map_err(|e| not_found(e, "that address is not on the allowlist"))?;
            println!("Removed {a} from the allowlist.");
        }
    }
    Ok(())
}

fn not_found(e: anyhow::Error, msg: &str) -> anyhow::Error {
    if matches!(
        e.downcast_ref::<crate::engine::EngineError>(),
        Some(crate::engine::EngineError::NotFound)
    ) {
        return anyhow::anyhow!("{msg}");
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailqueue::{MessageView, QueueView, RecipientView};

    #[test]
    fn renders_a_queue_in_plain_words() {
        let v = QueueView {
            paused: true,
            paused_at: Some("2026-11-10T21:00:00Z".into()),
            relayed: false,
            messages: vec![MessageView {
                id: "jn93tqlaaaaa".into(),
                sender: "bob@live.test".into(),
                size: Some(84),
                created_at: Some("2026-11-10T21:55:01Z".into()),
                next_retry: None,
                expires_at: Some("2026-11-13T21:55:01Z".into()),
                recipients: vec![RecipientView {
                    address: "b@example.com".into(),
                    status: "retrying",
                    kind: Some("network"),
                    problem: Some("The mail server for example.com couldn't be looked up.".into()),
                    remedy: Some("Check DNS.".into()),
                    response: Some("DNS resolution error".into()),
                    host: Some("example.com".into()),
                    attempts: 2,
                    next_retry: None,
                    expires_at: Some("2026-11-13T21:55:01Z".into()),
                    expires_after_attempts: None,
                }],
            }],
        };
        let s = render_queue(&v);
        assert!(s.contains("PAUSED since 2026-11-10 21:00 UTC"), "{s}");
        assert!(
            s.contains("b@example.com  retrying after 2 attempts · gives up 2026-11-13 21:55 UTC"),
            "{s}"
        );
        assert!(s.contains("What to do: Check DNS."));
    }
}
