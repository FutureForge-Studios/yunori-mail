//! Where alerts go: email to the external admin address (submitted to the
//! local engine as Yunori's own sender mailbox) and a webhook (JSON POST) or
//! an ntfy topic.
//!
//! Settings live in kv `alerts.settings`; the webhook URL (a Slack, Discord
//! or ntfy URL is itself a credential) and its optional bearer token are
//! sealed in the vault and never returned: the API shows only the host.

use super::{Kind, Notice, Severity};
use crate::app::App;
use crate::engine::Engine;
use crate::store::Db;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use zeroize::Zeroizing;

pub const KV_SETTINGS: &str = "alerts.settings";
pub const SECRET_WEBHOOK_URL: &str = "alerts.webhook.url";
pub const SECRET_WEBHOOK_TOKEN: &str = "alerts.webhook.token";
pub const SECRET_SENDER: &str = "alerts.sender.password";
/// Local part of the mailbox alerts are sent from, on the primary domain.
pub const SENDER_LOCAL: &str = "yunori-alerts";
pub const SENDER_NAME: &str = "Yunori alerts";
/// The engine's implicit-TLS submission listener (see the Yunori listener set).
pub const SUBMISSION_PORT: u16 = 465;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebhookKind {
    /// JSON POST (works with most chat and incident tools).
    #[default]
    Json,
    /// An ntfy topic URL (plain-text body, Title/Priority headers).
    Ntfy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "yes")]
    pub email_enabled: bool,
    /// `None` = the install's external admin address.
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub webhook_enabled: bool,
    #[serde(default)]
    pub webhook_kind: WebhookKind,
    /// Host of the saved webhook URL, shown in the console instead of the URL.
    #[serde(default)]
    pub webhook_host: Option<String>,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            email_enabled: true,
            recipient: None,
            webhook_enabled: false,
            webhook_kind: WebhookKind::Json,
            webhook_host: None,
        }
    }
}

pub fn settings(app: &App) -> Settings {
    app.db
        .get_json::<Settings>(KV_SETTINGS)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The address email alerts go to.
pub fn recipient(app: &App, s: &Settings) -> Option<String> {
    s.recipient
        .clone()
        .or_else(|| app.config().map(|c| c.admin_email))
        .filter(|r| !r.is_empty())
}

/// A webhook URL Yunori will post to: http(s), a host, nothing else.
pub fn check_webhook_url(raw: &str) -> Result<url::Url, &'static str> {
    let u = url::Url::parse(raw.trim())
        .map_err(|_| "Enter a full URL, such as https://ntfy.sh/your-topic.")?;
    if !matches!(u.scheme(), "https" | "http") {
        return Err("The URL must start with https:// (or http:// on your own network).");
    }
    if u.host_str().is_none_or(str::is_empty) {
        return Err("The URL needs a host name.");
    }
    Ok(u)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    /// "email" | "webhook" | "ntfy"
    pub channel: String,
    pub ok: bool,
    /// What happened, in plain words ("sent to owner@…", "HTTP 404").
    pub detail: String,
}

impl Delivery {
    fn new(channel: &str, ok: bool, detail: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            ok,
            detail: detail.into(),
        }
    }
}

/// One line for the audit ledger and logs.
pub fn delivery_summary(d: &[Delivery]) -> String {
    if d.is_empty() {
        return "no channel is on; shown in the console only".into();
    }
    d.iter()
        .map(|x| {
            format!(
                "{}: {}{}",
                x.channel,
                if x.ok { "" } else { "failed, " },
                x.detail
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Send through every channel that is on.
pub async fn deliver(app: &App, n: &Notice) -> Vec<Delivery> {
    let s = settings(app);
    let mut out = Vec::new();
    if s.email_enabled {
        out.push(match send_email(app, &s, n).await {
            Ok(detail) => Delivery::new("email", true, detail),
            Err(e) => Delivery::new("email", false, format!("{e:#}")),
        });
    }
    if s.webhook_enabled {
        let channel = match s.webhook_kind {
            WebhookKind::Json => "webhook",
            WebhookKind::Ntfy => "ntfy",
        };
        out.push(match send_webhook(app, &s, n).await {
            Ok(detail) => Delivery::new(channel, true, detail),
            Err(e) => Delivery::new(channel, false, format!("{e:#}")),
        });
    }
    out
}

// ── wording ─────────────────────────────────────────────────────────────

pub fn subject(host: &str, n: &Notice) -> String {
    let t = &n.condition.title;
    match n.kind {
        Kind::Firing => format!("[{host}] {t}"),
        Kind::Worse => format!("[{host}] Worse: {t}"),
        Kind::Reminder => format!("[{host}] Still happening: {t}"),
        Kind::Resolved => format!("[{host}] Resolved: {t}"),
        Kind::Test => format!("[{host}] Test alert from Yunori"),
    }
}

fn stamp(t: chrono::DateTime<chrono::Utc>) -> String {
    t.format("%Y-%m-%d %H:%M UTC").to_string()
}

pub fn body(host: &str, n: &Notice) -> String {
    let c = &n.condition;
    let console = format!("https://{host}/console");
    match n.kind {
        Kind::Test => format!(
            "{}\n\nNothing to do: alerts from {host} reach you here.\n\nConsole: {console}\n",
            c.summary
        ),
        Kind::Resolved => format!(
            "Resolved: Yunori no longer sees this problem.\n\nIt was: {}\nFirst noticed: {}\n\nConsole: {console}\n\n{}",
            c.summary,
            stamp(n.since),
            FOOTER
        ),
        _ => format!(
            "{}\n\nWhat to do: {}\n\n{}First noticed: {}\nConsole: {console}\n\n{}",
            c.summary,
            c.remedy,
            match n.kind {
                Kind::Worse => "This has become critical.\n",
                Kind::Reminder => "This is still happening (a daily reminder).\n",
                _ => "",
            },
            stamp(n.since),
            FOOTER
        ),
    }
}

const FOOTER: &str = "Yunori sends one message when a problem starts, one when it ends, and at most a daily reminder in between. Choose where alerts go under Alerts in the console.\n";

// ── email ───────────────────────────────────────────────────────────────

/// The sender mailbox (`yunori-alerts@<primary domain>`) and its password,
/// creating the mailbox, or giving it a fresh password, when needed. The
/// password is random and sealed in the vault; nobody types it.
pub async fn ensure_sender(
    db: &Db,
    engine: &dyn Engine,
    domain: &str,
    reset: bool,
) -> Result<(String, Zeroizing<String>)> {
    let address = format!("{SENDER_LOCAL}@{domain}");
    let accounts = engine.list_accounts().await?;
    let existing = accounts
        .iter()
        .find(|a| a.address.eq_ignore_ascii_case(&address));
    if let (Some(_), false, Some(pw)) = (existing, reset, db.get_secret(SECRET_SENDER)?) {
        return Ok((address, pw));
    }
    let pw = Zeroizing::new(crate::util::generate_password(32));
    let what = match existing {
        Some(a) => {
            engine.set_password(&a.id, &pw).await?;
            "gave the alerts sender a new password"
        }
        None => {
            let d = engine
                .list_domains()
                .await?
                .into_iter()
                .find(|d| d.name.eq_ignore_ascii_case(domain))
                .ok_or_else(|| anyhow!("the primary domain {domain} is missing from the engine"))?;
            engine
                .create_account(
                    SENDER_LOCAL,
                    &d.id,
                    Some(SENDER_NAME),
                    Some(100 * 1024 * 1024),
                    &pw,
                )
                .await?;
            "created the mailbox alerts are sent from"
        }
    };
    db.put_secret(SECRET_SENDER, &pw)?;
    db.audit("system", "alerts.sender", Some(&address), Some(what))?;
    Ok((address, pw))
}

/// Compose and submit one alert email through the engine on loopback.
pub async fn submit(
    target: &super::smtp::Target<'_>,
    sender: &str,
    password: &str,
    to: &str,
    n: &Notice,
) -> Result<(), super::smtp::SmtpError> {
    let message = super::smtp::compose(
        SENDER_NAME,
        sender,
        to,
        &subject(target.name, n),
        &body(target.name, n),
        target.name,
        &[("X-Yunori-Alert", &n.condition.id)],
    );
    let env = super::smtp::Envelope {
        user: sender,
        password,
        from: sender,
        to,
    };
    super::smtp::send(target, &env, &message).await.map(|_| ())
}

async fn send_email(app: &App, s: &Settings, n: &Notice) -> Result<String> {
    let cfg = app
        .config()
        .ok_or_else(|| anyhow!("the server is not configured yet"))?;
    let to = recipient(app, s).ok_or_else(|| anyhow!("no recipient address is set"))?;
    if app.dev {
        return Err(anyhow!(
            "not sent: development mode has no mail engine to send through"
        ));
    }
    let engine = app.engine()?;
    let target = super::smtp::Target {
        host: "127.0.0.1",
        port: SUBMISSION_PORT,
        name: &cfg.hostname,
    };
    let (sender, pw) = ensure_sender(&app.db, engine.as_ref(), &cfg.primary_domain, false).await?;
    match submit(&target, &sender, &pw, &to, n).await {
        Ok(()) => {}
        // The password was changed elsewhere (or the vault restored): reset once, retry.
        Err(super::smtp::SmtpError::Auth(_)) => {
            let (sender, pw) =
                ensure_sender(&app.db, engine.as_ref(), &cfg.primary_domain, true).await?;
            submit(&target, &sender, &pw, &to, n).await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(format!("sent to {to}"))
}

// ── webhook / ntfy ──────────────────────────────────────────────────────

fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(format!("yunori-mail/{}", crate::app::VERSION))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        // A redirect could carry the token to another host.
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

/// Header-safe text (ntfy titles travel as HTTP headers).
fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '—' | '–' => '-',
            c if c.is_ascii() && !c.is_ascii_control() => c,
            _ => '?',
        })
        .collect()
}

pub fn webhook_json(host: &str, n: &Notice) -> serde_json::Value {
    let c = &n.condition;
    serde_json::json!({
        "event": n.kind.as_str(),
        "server": host,
        "at": crate::util::now_rfc3339(),
        "alert": {
            "id": c.id,
            "severity": c.severity.as_str(),
            "title": c.title,
            "summary": c.summary,
            "remedy": c.remedy,
            "since": n.since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        },
        "consoleUrl": format!("https://{host}/console"),
        // Slack-compatible one-liner.
        "text": format!("{}: {}", subject(host, n), c.summary),
    })
}

async fn send_webhook(app: &App, s: &Settings, n: &Notice) -> Result<String> {
    let url = app
        .db
        .get_secret(SECRET_WEBHOOK_URL)?
        .ok_or_else(|| anyhow!("no webhook URL is saved"))?;
    let token = app.db.get_secret(SECRET_WEBHOOK_TOKEN)?;
    let host = app
        .config()
        .map(|c| c.hostname)
        .unwrap_or_else(|| "yunori".into());
    post(
        &url,
        token.as_deref().map(|t| t.as_str()),
        s.webhook_kind,
        &host,
        n,
    )
    .await
}

/// POST one notice to a webhook or ntfy URL.
pub async fn post(
    url: &str,
    token: Option<&str>,
    kind: WebhookKind,
    host: &str,
    n: &Notice,
) -> Result<String> {
    let client = http()?;
    let mut req = client.post(url);
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        req = req.bearer_auth(t);
    }
    req = match kind {
        WebhookKind::Json => req.json(&webhook_json(host, n)),
        WebhookKind::Ntfy => {
            let (priority, tags) = match (n.kind, n.condition.severity) {
                (Kind::Resolved, _) => ("default", "white_check_mark"),
                (Kind::Test, _) => ("default", "bell"),
                (_, Severity::Critical) => ("urgent", "rotating_light"),
                (_, Severity::Warning) => ("high", "warning"),
            };
            let text = match n.kind {
                Kind::Resolved => format!("Resolved. It was: {}", n.condition.summary),
                Kind::Test => n.condition.summary.clone(),
                _ => format!(
                    "{}\n\nWhat to do: {}",
                    n.condition.summary, n.condition.remedy
                ),
            };
            req.header("Title", ascii(&subject(host, n)))
                .header("Priority", priority)
                .header("Tags", tags)
                .header("Click", ascii(&format!("https://{host}/console")))
                .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(text)
        }
    };
    let res = req.send().await.map_err(|e| {
        anyhow!(
            "could not reach {}: {}",
            url::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_default(),
            if e.is_timeout() {
                "timed out".to_string()
            } else if e.is_connect() {
                "connection refused or no route".to_string()
            } else {
                "request failed".to_string()
            }
        )
    })?;
    let status = res.status();
    if status.is_success() {
        Ok(format!("delivered (HTTP {})", status.as_u16()))
    } else {
        Err(anyhow!("the receiver answered HTTP {}", status.as_u16()))
    }
}
