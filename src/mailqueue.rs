//! The outgoing mail queue as people see it: who sent what to whom, where each
//! recipient stands, the last problem in plain words, the next attempt and when
//! the engine gives up. Plus the actions (retry now, cancel, pause/resume) with
//! an audit entry for every one. Shared by the console API and the CLI.
//!
//! Envelope and status only: message contents are never read (plan §6).

use crate::app::App;
use crate::engine::{EQueued, EQueuedRecipient, EngineError, RcptState};
use crate::model::SendingMode;
use anyhow::Result;
use serde::Serialize;

/// `kv` key holding when the queue was paused (absent: running).
pub const PAUSED_KEY: &str = "queue.paused";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecipientView {
    pub address: String,
    /// `waiting` (first attempt pending), `retrying`, `failed` (gave up),
    /// `delivered`.
    pub status: &'static str,
    /// `auth`, `tls`, `network`, `rejected`, `address`, `other` — `None` when
    /// nothing has gone wrong.
    pub kind: Option<&'static str>,
    /// The last problem as a sentence a person can act on.
    pub problem: Option<String>,
    /// What to do about it.
    pub remedy: Option<String>,
    /// The other server's own words (or the engine's error), verbatim.
    pub response: Option<String>,
    pub host: Option<String>,
    pub attempts: u64,
    pub next_retry: Option<String>,
    pub expires_at: Option<String>,
    pub expires_after_attempts: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    pub id: String,
    /// Envelope sender; empty for bounce notices (`MAIL FROM:<>`).
    pub sender: String,
    pub size: Option<u64>,
    pub created_at: Option<String>,
    pub next_retry: Option<String>,
    /// The earliest time the engine gives up on a recipient still pending.
    pub expires_at: Option<String>,
    pub recipients: Vec<RecipientView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueView {
    pub paused: bool,
    pub paused_at: Option<String>,
    pub relayed: bool,
    pub messages: Vec<MessageView>,
}

fn domain_of(address: &str) -> &str {
    address.rsplit_once('@').map(|(_, d)| d).unwrap_or(address)
}

/// SMTP reply code at the start of a response ("550 5.1.1 …" → 550).
fn reply_code(response: &str) -> Option<u16> {
    response
        .get(..3)
        .and_then(|c| c.parse().ok())
        .filter(|c| (200..600).contains(c))
}

/// Classify a recipient's last problem and say it in plain words.
/// `None` when nothing has gone wrong (delivered, or not tried yet).
pub fn explain(r: &EQueuedRecipient, relayed: bool) -> Option<(&'static str, String)> {
    if r.state == RcptState::Completed {
        return None;
    }
    let resp = r.response.as_deref().unwrap_or("");
    let lower = resp.to_ascii_lowercase();
    if r.error_type.is_none() && resp.is_empty() {
        return None;
    }
    let domain = domain_of(&r.address);
    let host = r.remote_host.as_deref().unwrap_or(domain);
    let who = if relayed {
        "The relay".to_string()
    } else {
        host.to_string()
    };
    let code = reply_code(resp);
    let permanent = r.state == RcptState::PermanentFailure;

    if code == Some(535) || lower.contains(" 5.7.8") || lower.contains("authentication failed") {
        return Some((
            "auth",
            format!("{who} refused the username or password Yunori signs in with."),
        ));
    }
    let explained = match r.error_type.as_deref() {
        Some("dnsError")
            if permanent
                || lower.contains("no mx")
                || lower.contains("nxdomain")
                || lower.contains("no record") =>
        {
            (
                "address",
                format!("{domain} doesn't accept mail: it has no mail server (MX record) in DNS."),
            )
        }
        Some("dnsError") => (
            "network",
            format!("The mail server for {domain} couldn't be looked up: the DNS lookup failed."),
        ),
        Some("connectionError") => (
            "network",
            format!("Couldn't connect to {host}: the connection was refused or timed out."),
        ),
        Some("io") => (
            "network",
            format!("The connection to {host} broke off during delivery."),
        ),
        Some("tlsError") => (
            "tls",
            format!("A secure (TLS) connection to {host} couldn't be set up."),
        ),
        Some("daneError") => (
            "tls",
            format!(
                "{host}'s certificate doesn't match its DANE (TLSA) record, so the message was held back."
            ),
        ),
        Some("mtaStsError") => (
            "tls",
            format!(
                "{domain} requires a verified secure connection (MTA-STS) that couldn't be made."
            ),
        ),
        Some("rateLimited" | "concurrencyLimited") => (
            "other",
            "Held back by a sending limit on this server; it goes out on the next attempt."
                .to_string(),
        ),
        _ => match code {
            Some(c) if c >= 500 => {
                let why = if lower.contains("5.1.1")
                    || lower.contains("user unknown")
                    || lower.contains("does not exist")
                    || lower.contains("no such user")
                {
                    "the mailbox doesn't exist"
                } else if lower.contains("5.2.2")
                    || lower.contains("quota")
                    || lower.contains("full")
                {
                    "the recipient's mailbox is full"
                } else if lower.contains("spam")
                    || lower.contains("blocked")
                    || lower.contains("block list")
                    || lower.contains("blocklist")
                    || lower.contains("reputation")
                {
                    "it looks like spam or this server's address is blocked"
                } else if lower.contains("5.7.") || lower.contains("spf") || lower.contains("dmarc")
                {
                    "it failed their sender checks (SPF, DKIM or DMARC)"
                } else {
                    "they won't accept it"
                };
                ("rejected", format!("{who} refused the message: {why}."))
            }
            Some(c) if c >= 400 => (
                "other",
                format!(
                    "{who} asked to try again later (greylisting, a rate limit or a reputation check)."
                ),
            ),
            _ if permanent => (
                "rejected",
                format!("Delivery to {domain} failed permanently."),
            ),
            _ => ("other", format!("Delivery to {domain} hasn't worked yet.")),
        },
    };
    Some(explained)
}

/// What to do about a problem of this kind.
pub fn remedy(kind: &str, relayed: bool) -> String {
    match kind {
        "auth" => "Update the relay's username and password under Sending.".into(),
        "tls" => "Usually on the receiving side. If it affects every destination, check the certificate under System.".into(),
        "network" if relayed => "Check the relay's host and port under Sending.".into(),
        "network" => "If this happens for every destination, your provider probably blocks outbound port 25: switch sending to a relay.".into(),
        "address" => "Check the address for a typo. Nothing to fix on this server.".into(),
        "rejected" => "If many recipients refuse mail, check DNS (SPF, DKIM, DMARC), reverse DNS and blocklists under Deliverability.".into(),
        _ => "Nothing to do: it is retried automatically.".into(),
    }
}

fn recipient_view(r: &EQueuedRecipient, relayed: bool) -> RecipientView {
    let status = match r.state {
        RcptState::Completed => "delivered",
        RcptState::PermanentFailure => "failed",
        RcptState::TemporaryFailure => "retrying",
        RcptState::Scheduled if r.retry_count > 0 => "retrying",
        RcptState::Scheduled => "waiting",
    };
    let explained = explain(r, relayed);
    let pending = matches!(status, "waiting" | "retrying");
    RecipientView {
        address: r.address.clone(),
        status,
        kind: explained.as_ref().map(|(k, _)| *k),
        remedy: explained.as_ref().map(|(k, _)| remedy(k, relayed)),
        problem: explained.map(|(_, p)| p),
        response: r.response.clone(),
        host: r.remote_host.clone(),
        attempts: r.retry_count,
        next_retry: r.next_retry.clone().filter(|_| pending),
        expires_at: r.expires_at.clone(),
        expires_after_attempts: r.expires_after_attempts,
    }
}

pub fn message_view(m: &EQueued, relayed: bool) -> MessageView {
    let recipients: Vec<RecipientView> = m
        .recipients
        .iter()
        .map(|r| recipient_view(r, relayed))
        .collect();
    // RFC 3339 UTC strings from one engine sort chronologically.
    let expires_at = recipients
        .iter()
        .filter(|r| matches!(r.status, "waiting" | "retrying"))
        .filter_map(|r| r.expires_at.clone())
        .min();
    MessageView {
        id: m.id.clone(),
        sender: m.sender.clone().unwrap_or_default(),
        size: m.size,
        created_at: m.created_at.clone(),
        next_retry: m.next_retry.clone(),
        expires_at,
        recipients,
    }
}

pub fn relayed(app: &App) -> bool {
    app.config()
        .is_some_and(|c| c.sending_mode == SendingMode::Relay)
}

pub fn paused_at(app: &App) -> Option<String> {
    app.db.get_kv(PAUSED_KEY).ok().flatten()
}

pub async fn view(app: &App) -> Result<QueueView> {
    let relayed = relayed(app);
    let mut messages: Vec<MessageView> = app
        .engine()?
        .queue()
        .await?
        .iter()
        .map(|m| message_view(m, relayed))
        .collect();
    messages.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let paused_at = paused_at(app);
    Ok(QueueView {
        paused: paused_at.is_some(),
        paused_at,
        relayed,
        messages,
    })
}

/// Engine ids are short alphanumerics; anything else is a typo or worse.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn summary(m: &EQueued) -> String {
    let to: Vec<&str> = m.recipients.iter().map(|r| r.address.as_str()).collect();
    let from = m
        .sender
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<>");
    format!("{from} → {}", to.join(", "))
}

/// Retry one message now (`Some(id)`) or every queued message (`None`).
/// Returns how many messages were asked to retry.
pub async fn retry(app: &App, actor: &str, id: Option<&str>) -> Result<usize> {
    let e = app.engine()?;
    let n = match id {
        Some(id) => {
            if !valid_id(id) {
                return Err(EngineError::NotFound.into());
            }
            e.queue_retry(&[id.to_string()]).await?
        }
        None => {
            let ids: Vec<String> = e.queue().await?.into_iter().map(|m| m.id).collect();
            e.queue_retry(&ids).await?
        }
    };
    app.db.audit(
        actor,
        "queue.retry",
        Some(id.unwrap_or("all")),
        Some(&format!("{n} message{}", if n == 1 { "" } else { "s" })),
    )?;
    Ok(n)
}

/// Cancel delivery of one message (it leaves the queue; no bounce is sent).
pub async fn cancel(app: &App, actor: &str, id: &str) -> Result<()> {
    if !valid_id(id) {
        return Err(EngineError::NotFound.into());
    }
    let e = app.engine()?;
    let detail = e
        .queue()
        .await?
        .iter()
        .find(|m| m.id == id)
        .map(summary)
        .ok_or(EngineError::NotFound)?;
    e.queue_cancel(id).await?;
    app.db
        .audit(actor, "queue.cancel", Some(id), Some(&detail))?;
    Ok(())
}

/// Pause or resume all outbound delivery. Stalwart keeps this in memory only,
/// so it is recorded here and re-applied by [`reassert_pause`].
pub async fn set_paused(app: &App, actor: &str, paused: bool) -> Result<()> {
    app.engine()?.queue_set_paused(paused).await?;
    if paused {
        if paused_at(app).is_none() {
            app.db.set_kv(PAUSED_KEY, &crate::util::now_rfc3339())?;
        }
    } else {
        app.db.delete_kv(PAUSED_KEY)?;
    }
    app.db.audit(
        actor,
        if paused {
            "queue.pause"
        } else {
            "queue.resume"
        },
        Some("outbound"),
        None,
    )?;
    Ok(())
}

/// An engine restart resumes its queue; keep it paused while Yunori says so.
pub async fn reassert_pause(app: &App) {
    if paused_at(app).is_some()
        && let Ok(e) = app.engine()
        && let Err(err) = e.queue_set_paused(true).await
    {
        tracing::debug!("could not re-apply the queue pause: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rcpt(
        state: RcptState,
        error_type: Option<&str>,
        response: Option<&str>,
    ) -> EQueuedRecipient {
        EQueuedRecipient {
            address: "bob@example.org".into(),
            state,
            error_type: error_type.map(str::to_string),
            response: response.map(str::to_string),
            retry_count: 1,
            remote_host: Some("mx.example.org".into()),
            ..Default::default()
        }
    }

    fn kind(r: &EQueuedRecipient) -> Option<&'static str> {
        explain(r, false).map(|(k, _)| k)
    }

    #[test]
    fn classifies_auth_tls_network_rejected() {
        use RcptState::*;
        let auth = rcpt(
            TemporaryFailure,
            Some("unexpectedResponse"),
            Some("535 5.7.8 Authentication credentials invalid"),
        );
        assert_eq!(kind(&auth), Some("auth"));
        let (_, s) = explain(&auth, true).unwrap();
        assert!(s.starts_with("The relay refused"), "{s}");

        assert_eq!(
            kind(&rcpt(
                TemporaryFailure,
                Some("tlsError"),
                Some("handshake failed")
            )),
            Some("tls")
        );
        assert_eq!(
            kind(&rcpt(TemporaryFailure, Some("daneError"), None)),
            Some("tls")
        );
        assert_eq!(
            kind(&rcpt(TemporaryFailure, Some("mtaStsError"), None)),
            Some("tls")
        );
        assert_eq!(
            kind(&rcpt(
                TemporaryFailure,
                Some("connectionError"),
                Some("Connection refused")
            )),
            Some("network")
        );
        assert_eq!(
            kind(&rcpt(
                TemporaryFailure,
                Some("dnsError"),
                Some("DNS resolution error: io error: Network is unreachable (os error 101)")
            )),
            Some("network")
        );
        let unknown = rcpt(
            PermanentFailure,
            Some("unexpectedResponse"),
            Some("550 5.1.1 User unknown"),
        );
        assert_eq!(kind(&unknown), Some("rejected"));
        assert!(
            explain(&unknown, false)
                .unwrap()
                .1
                .contains("the mailbox doesn't exist")
        );
        let spam = rcpt(
            PermanentFailure,
            Some("unexpectedResponse"),
            Some("554 5.7.1 Service unavailable; client host blocked using zen.spamhaus.org"),
        );
        assert!(explain(&spam, false).unwrap().1.contains("spam"));
    }

    #[test]
    fn a_domain_without_mx_is_an_address_problem() {
        // What a live 1.0.3 engine reports for `x@nowhere.invalid`.
        let r = rcpt(
            RcptState::PermanentFailure,
            Some("dnsError"),
            Some("no MX record found."),
        );
        let (k, s) = explain(&r, false).unwrap();
        assert_eq!(k, "address");
        assert!(s.contains("example.org doesn't accept mail"), "{s}");
    }

    #[test]
    fn deferrals_and_untouched_recipients() {
        let greylist = rcpt(
            RcptState::TemporaryFailure,
            Some("unexpectedResponse"),
            Some("451 4.7.1 Greylisted, try again later"),
        );
        let (k, s) = explain(&greylist, false).unwrap();
        assert_eq!(k, "other");
        assert!(s.contains("try again later"));
        let mut fresh = rcpt(RcptState::Scheduled, None, None);
        fresh.retry_count = 0;
        assert_eq!(explain(&fresh, false), None);
        assert_eq!(recipient_view(&fresh, false).status, "waiting");
        let done = rcpt(RcptState::Completed, None, Some("250 2.0.0 OK"));
        assert_eq!(explain(&done, false), None);
        assert_eq!(recipient_view(&done, false).status, "delivered");
    }

    #[test]
    fn message_expiry_is_the_earliest_pending_recipient() {
        let mut a = rcpt(RcptState::TemporaryFailure, Some("io"), None);
        a.expires_at = Some("2026-11-13T21:55:01Z".into());
        let mut b = rcpt(RcptState::TemporaryFailure, Some("io"), None);
        b.expires_at = Some("2026-11-12T00:00:00Z".into());
        let mut c = rcpt(RcptState::PermanentFailure, Some("dnsError"), None);
        c.expires_at = Some("2026-11-11T00:00:00Z".into());
        let m = EQueued {
            id: "q".into(),
            sender: Some("ana@example.com".into()),
            recipients: vec![a, b, c],
            ..Default::default()
        };
        let v = message_view(&m, false);
        assert_eq!(v.expires_at.as_deref(), Some("2026-11-12T00:00:00Z"));
        assert_eq!(v.recipients[2].next_retry, None);
    }

    #[test]
    fn ids_are_checked() {
        assert!(valid_id("jn93tqlaaaaa"));
        assert!(!valid_id(""));
        assert!(!valid_id("../x"));
        assert!(!valid_id(&"a".repeat(65)));
    }
}
