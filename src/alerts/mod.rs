//! Proactive alerts (plan §8.3 and P2 "Alerts"): Yunori notices problems
//! before the people using the server do, and tells the admin.
//!
//! Every ten minutes a background job gathers what the console already knows
//! (engine reachability, the certificate, the outbound queue, the cached
//! blocklist and DNS results, disk space, the last backup), turns it into
//! *conditions* (stable id, severity, one-sentence summary, a remedy a person
//! can act on) and runs them through a small state machine:
//!
//! - a condition that starts firing → one message;
//! - it gets worse (warning → critical) → one message;
//! - it keeps firing → at most one reminder per 24 hours;
//! - it has been clear for two evaluations in a row → one "resolved" message.
//!
//! Messages go to the external admin address by email (submitted to the local
//! engine as a dedicated, Yunori-managed sender) and/or to a webhook or ntfy
//! topic (see `channels`). Every message is kept in a short history
//! (`alert_events`) and recorded in the audit ledger.

pub mod channels;
pub mod smtp;
#[cfg(test)]
mod tests;

use crate::app::App;
use crate::engine::{EQueued, RcptState};
use crate::model::{Check, Status};
use crate::ops::DomainDns;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// How often the background job evaluates.
pub const EVERY: Duration = Duration::from_secs(600);
/// At most one reminder per alert in this window while it keeps firing.
pub const REMIND_AFTER_HOURS: i64 = 24;
/// Evaluations in a row a condition must be clear before it counts as resolved.
pub const CLEAR_TICKS: u32 = 2;
/// Deferred outbound mail older than this is "stuck".
pub const QUEUE_HOURS: i64 = 6;
/// A certificate closer than this to expiry is alerted on.
pub const CERT_DAYS: i64 = 14;
/// Consecutive failed renewal attempts before renewal counts as failing.
pub const CERT_FAILURES: u32 = 3;
/// Disk usage thresholds (percent of the mail store's filesystem).
pub const DISK_WARN: f64 = 90.0;
pub const DISK_CRITICAL: f64 = 97.0;
/// A recorded backup older than this is alerted on (never-run backups are not).
pub const BACKUP_DAYS: i64 = 7;

const KV_ACTIVE: &str = "alerts.active";
const KV_DNS_PASSED: &str = "alerts.dns_passed";
const KV_EVALUATED: &str = "alerts.evaluated_at";
const HISTORY_KEEP: i64 = 500;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
}

/// One problem Yunori can see right now.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    /// Stable id (`engine`, `certificate.expiry`, `dns:example.com`, …).
    pub id: String,
    pub severity: Severity,
    /// A short name for subjects and lists ("Certificate expiring").
    pub title: String,
    /// One sentence: what is wrong and what it means.
    pub summary: String,
    /// One sentence: what to do.
    pub remedy: String,
}

impl Condition {
    fn new(
        id: impl Into<String>,
        severity: Severity,
        title: impl Into<String>,
        summary: impl Into<String>,
        remedy: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            severity,
            title: title.into(),
            summary: summary.into(),
            remedy: remedy.into(),
        }
    }
}

/// An alert that is firing (kv `alerts.active`, keyed by condition id).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Active {
    #[serde(flatten)]
    pub condition: Condition,
    pub since: DateTime<Utc>,
    pub last_notified: DateTime<Utc>,
    pub notified_severity: Severity,
    /// Evaluations in a row the condition has been clear (resolves at `CLEAR_TICKS`).
    #[serde(default)]
    pub clear_ticks: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Firing,
    Worse,
    Reminder,
    Resolved,
    Test,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Firing => "firing",
            Kind::Worse => "worse",
            Kind::Reminder => "reminder",
            Kind::Resolved => "resolved",
            Kind::Test => "test",
        }
    }
}

/// A message to send.
#[derive(Debug, Clone)]
pub struct Notice {
    pub kind: Kind,
    pub condition: Condition,
    pub since: DateTime<Utc>,
}

// ── the state machine ───────────────────────────────────────────────────

/// Advance the alert state by one evaluation. Pure: `now` is injected.
pub fn step(
    active: &mut BTreeMap<String, Active>,
    firing: Vec<Condition>,
    now: DateTime<Utc>,
) -> Vec<Notice> {
    let mut notices = Vec::new();
    let ids: BTreeSet<String> = firing.iter().map(|c| c.id.clone()).collect();
    for c in firing {
        match active.get_mut(&c.id) {
            None => {
                notices.push(Notice {
                    kind: Kind::Firing,
                    condition: c.clone(),
                    since: now,
                });
                active.insert(
                    c.id.clone(),
                    Active {
                        notified_severity: c.severity,
                        condition: c,
                        since: now,
                        last_notified: now,
                        clear_ticks: 0,
                    },
                );
            }
            Some(a) => {
                a.clear_ticks = 0;
                let kind = if c.severity > a.notified_severity {
                    Some(Kind::Worse)
                } else if now - a.last_notified >= ChronoDuration::hours(REMIND_AFTER_HOURS) {
                    Some(Kind::Reminder)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    a.last_notified = now;
                    a.notified_severity = c.severity;
                    notices.push(Notice {
                        kind,
                        condition: c.clone(),
                        since: a.since,
                    });
                }
                a.condition = c;
            }
        }
    }
    let gone: Vec<String> = active
        .keys()
        .filter(|k| !ids.contains(*k))
        .cloned()
        .collect();
    for id in gone {
        let a = active.get_mut(&id).expect("present");
        a.clear_ticks += 1;
        if a.clear_ticks >= CLEAR_TICKS {
            let a = active.remove(&id).expect("present");
            notices.push(Notice {
                kind: Kind::Resolved,
                condition: a.condition,
                since: a.since,
            });
        }
    }
    notices
}

// ── conditions ──────────────────────────────────────────────────────────

/// The certificate facts alerts care about (from `edge::certs::status`).
#[derive(Debug, Clone, Default)]
pub struct CertFacts {
    pub state: String,
    pub days_left: Option<i64>,
    pub failures: u32,
    /// (summary, remedy) of the last failure.
    pub problem: Option<(String, String)>,
}

/// Everything one evaluation looks at. Built by `gather`; pure from here on.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub now: DateTime<Utc>,
    pub hostname: String,
    pub relayed: bool,
    pub engine_reachable: bool,
    pub cert: Option<CertFacts>,
    pub queue: Vec<EQueued>,
    pub ip: Option<String>,
    /// Blocklist checks (cached host checks); only `Fail` ones matter.
    pub blocklists: Vec<Check>,
    /// Cached DNS results per hosted domain; `None` when the domain list is unknown.
    pub dns: Option<Vec<DomainDns>>,
    /// (total, free) bytes of the mail store's filesystem.
    pub disk: Option<(u64, u64)>,
    pub last_backup: Option<DateTime<Utc>>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Turn what the server knows into the conditions that are firing now.
/// `dns_passed` remembers every required record that has passed once
/// (`domain|record id`), so a later failure reads as drift, not as a record
/// that was never published.
pub fn conditions(i: &Inputs, dns_passed: &mut BTreeSet<String>) -> Vec<Condition> {
    let mut out = Vec::new();
    let host = &i.hostname;

    if !i.engine_reachable {
        out.push(Condition::new(
            "engine",
            Severity::Critical,
            "Mail engine not responding",
            "The mail engine is not responding, so mail is neither received nor sent.",
            "On the server run: sudo systemctl status stalwart, then sudo yunori-mail doctor.",
        ));
    }

    if let Some(c) = &i.cert {
        let remedy = |fallback: &str| {
            c.problem
                .as_ref()
                .map(|(_, r)| r.clone())
                .unwrap_or_else(|| fallback.to_string())
        };
        match c.state.as_str() {
            "expired" => out.push(Condition::new(
                "certificate.expiry",
                Severity::Critical,
                "Certificate expired",
                format!(
                    "The HTTPS certificate for {host} has expired, so mail apps and browsers refuse to connect."
                ),
                remedy("Run: sudo yunori-mail certificate renew"),
            )),
            _ if c.state != "self_signed"
                && c.state != "missing"
                && c.days_left.is_some_and(|d| d < CERT_DAYS) =>
            {
                let d = c.days_left.unwrap_or(0);
                out.push(Condition::new(
                    "certificate.expiry",
                    if d <= 3 {
                        Severity::Critical
                    } else {
                        Severity::Warning
                    },
                    "Certificate expiring",
                    format!(
                        "The HTTPS certificate for {host} expires in {}, and automatic renewal has not succeeded yet.",
                        plural(d.max(0) as usize, "day", "days")
                    ),
                    remedy(
                        "Open System, Certificate, and press Try now; the reason is shown there if it fails.",
                    ),
                ))
            }
            _ => {
                if c.failures >= CERT_FAILURES
                    && let Some((summary, remedy)) = &c.problem
                {
                    out.push(Condition::new(
                        "certificate.renewal",
                        Severity::Warning,
                        "Certificate renewal failing",
                        format!(
                            "Automatic certificate renewal has failed {} times in a row: {}",
                            c.failures,
                            summary.trim_end_matches('.').to_string() + "."
                        ),
                        remedy.clone(),
                    ));
                }
            }
        }
    }

    // Outbound mail waiting too long.
    let cutoff = i.now - ChronoDuration::hours(QUEUE_HOURS);
    let stuck: Vec<EQueued> = i
        .queue
        .iter()
        .filter(|m| {
            m.created_at
                .as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .is_some_and(|t| t.with_timezone(&Utc) < cutoff)
        })
        .map(|m| EQueued {
            recipients: m
                .recipients
                .iter()
                .filter(|r| {
                    r.state == RcptState::TemporaryFailure
                        || (r.state == RcptState::Scheduled && r.retry_count > 0)
                })
                .cloned()
                .collect(),
            ..m.clone()
        })
        .filter(|m| !m.recipients.is_empty())
        .collect();
    if !stuck.is_empty() {
        let summary = crate::ops::summarize_queue(&stuck, i.relayed);
        let remedy = summary
            .groups
            .first()
            .map(|g| g.remedy.clone())
            .unwrap_or_else(|| "Open Deliverability to see why delivery is being retried.".into());
        out.push(Condition::new(
            "queue",
            Severity::Warning,
            "Outgoing mail is stuck",
            format!(
                "{} been waiting more than {QUEUE_HOURS} hours to be delivered.",
                if stuck.len() == 1 {
                    "1 outgoing message has".to_string()
                } else {
                    format!("{} outgoing messages have", stuck.len())
                }
            ),
            remedy,
        ));
    }

    // Blocklists (cached host checks, refreshed every 6 hours).
    let listed: Vec<&Check> = i
        .blocklists
        .iter()
        .filter(|b| b.status == Status::Fail)
        .collect();
    if !listed.is_empty() {
        let ip = i.ip.as_deref().unwrap_or("This server's IP");
        let names: Vec<String> = listed
            .iter()
            .map(|b| b.label.trim_end_matches(" blocklist").to_string())
            .collect();
        let urls: Vec<String> = listed.iter().filter_map(|b| b.help_url.clone()).collect();
        let lists = format!(
            "the {} {}",
            join_and(&names),
            if names.len() == 1 {
                "blocklist"
            } else {
                "blocklists"
            }
        );
        let ask = if urls.is_empty() {
            "Ask the blocklist to remove the address".to_string()
        } else {
            format!("Ask for removal at {}", join_and(&urls))
        };
        out.push(if i.relayed {
            Condition::new(
                "blocklist",
                Severity::Warning,
                "Server IP on a blocklist",
                format!(
                    "{ip} is on {lists}; outgoing mail goes through your relay, so it is not affected for now."
                ),
                format!("{ask}, so sending directly stays possible."),
            )
        } else {
            Condition::new(
                "blocklist",
                Severity::Critical,
                "Server IP on a blocklist",
                format!("{ip} is on {lists}, so some recipients will refuse mail from this server."),
                format!("{ask}, and send through a relay under Sending until the listing clears."),
            )
        });
    }

    // DNS drift: a required record that passed before and fails now.
    if let Some(domains) = &i.dns {
        let present: BTreeSet<&str> = domains.iter().map(|d| d.domain.as_str()).collect();
        dns_passed.retain(|k| k.split_once('|').is_some_and(|(d, _)| present.contains(d)));
        for d in domains {
            let mut drifted: Vec<&crate::model::DnsRecord> = Vec::new();
            for r in d.records.iter().filter(|r| r.required) {
                let key = format!("{}|{}", d.domain, r.id);
                match r.status {
                    Status::Pass => {
                        dns_passed.insert(key);
                    }
                    Status::Fail if dns_passed.contains(&key) => drifted.push(r),
                    _ => {}
                }
            }
            if drifted.is_empty() {
                continue;
            }
            let names: Vec<String> = drifted
                .iter()
                .map(|r| format!("{} {}", r.rtype, r.name))
                .collect();
            let mx = drifted.iter().any(|r| r.rtype == "MX");
            out.push(Condition::new(
                format!("dns:{}", d.domain),
                if mx {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                format!("DNS changed for {}", d.domain),
                format!(
                    "The {} {} for {} used to be correct but now {} on public resolvers, so {}.",
                    join_and(&names),
                    if names.len() == 1 {
                        "record"
                    } else {
                        "records"
                    },
                    d.domain,
                    if names.len() == 1 { "fails" } else { "fail" },
                    if mx {
                        "mail for it may not arrive"
                    } else {
                        "other servers may reject or misfile its mail"
                    }
                ),
                format!(
                    "Check the records at your DNS provider; Domains, {}, shows what is expected.",
                    d.domain
                ),
            ));
        }
    }

    if let Some((total, free)) = i.disk
        && total > 0
    {
        let used = 100.0 * (total.saturating_sub(free)) as f64 / total as f64;
        if used > DISK_WARN {
            out.push(Condition::new(
                "disk",
                if used > DISK_CRITICAL {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                "Disk almost full",
                format!(
                    "The mail disk is {:.0}% full ({} free); once it fills up, the server stops accepting mail.",
                    used.floor(),
                    gb(free)
                ),
                "Free space or grow the disk; Mailboxes shows which mailboxes use the most.",
            ));
        }
    }

    if let Some(last) = i.last_backup {
        let days = (i.now - last).num_days();
        if days >= BACKUP_DAYS {
            out.push(Condition::new(
                "backup",
                Severity::Warning,
                "Backups are out of date",
                format!("The last backup of this server was made {days} days ago."),
                "Run: sudo yunori-mail backup, then copy the file somewhere off this server.",
            ));
        }
    }

    out
}

// ── gathering (impure) ──────────────────────────────────────────────────

fn last_backup(app: &App) -> Option<DateTime<Utc>> {
    let conn = app.db.conn();
    let at: Option<String> = conn
        .query_row(
            "SELECT at FROM audit WHERE action LIKE 'backup.create%' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok();
    at.and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// Collect the inputs. `None` until the install is complete: during setup
/// the checklist already says what is missing.
pub async fn gather(app: &App, confirm_delay: Duration) -> Option<Inputs> {
    let cfg = app.config().filter(|c| c.install_complete)?;
    let engine = app.engine().ok();
    let mut reachable = match &engine {
        Some(e) => e.info().await.reachable,
        None => false,
    };
    if !reachable && let Some(e) = &engine {
        // One more try, so an engine restart (upgrade, certificate reload) is not an alert.
        tokio::time::sleep(confirm_delay).await;
        reachable = e.info().await.reachable;
    }
    let (queue, dns) = match (&engine, reachable) {
        (Some(e), true) => {
            let queue = e.queue().await.unwrap_or_default();
            let dns = e.list_domains().await.ok().map(|domains| {
                domains
                    .iter()
                    .filter_map(|d| {
                        app.db
                            .cache_get::<DomainDns>(&format!("dns:{}", d.name))
                            .ok()
                            .flatten()
                            .map(|(v, _)| v)
                    })
                    .collect()
            });
            (queue, dns)
        }
        _ => (vec![], None),
    };
    let cert = crate::edge::certs::status(app);
    let host = crate::ops::cached_host_checks(app);
    Some(Inputs {
        now: Utc::now(),
        hostname: cfg.hostname.clone(),
        relayed: cfg.sending_mode == crate::model::SendingMode::Relay,
        engine_reachable: reachable,
        cert: Some(CertFacts {
            state: cert.state.to_string(),
            days_left: cert.days_left,
            failures: cert.failures,
            problem: cert.problem.map(|p| (p.summary, p.remedy)),
        }),
        queue,
        ip: cfg.public_ipv4.clone().or(cfg.public_ipv6.clone()),
        blocklists: host.map(|h| h.blocklists).unwrap_or_default(),
        dns,
        disk: crate::checks::disk_space(&app.paths.stalwart_data()),
        last_backup: last_backup(app),
    })
}

// ── state, history ──────────────────────────────────────────────────────

pub fn load_active(app: &App) -> BTreeMap<String, Active> {
    app.db
        .get_json::<BTreeMap<String, Active>>(KV_ACTIVE)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Alerts firing right now (worst first), for the Overview and the Alerts page.
pub fn current(app: &App) -> Vec<Active> {
    let mut v: Vec<Active> = load_active(app)
        .into_values()
        .filter(|a| a.clear_ticks == 0)
        .collect();
    v.sort_by(|a, b| {
        b.condition
            .severity
            .cmp(&a.condition.severity)
            .then(a.since.cmp(&b.since))
    });
    v
}

pub fn evaluated_at(app: &App) -> Option<String> {
    app.db.get_kv(KV_EVALUATED).ok().flatten()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub at: String,
    pub alert_id: String,
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub summary: String,
    pub deliveries: Vec<channels::Delivery>,
}

fn record(app: &App, n: &Notice, deliveries: &[channels::Delivery]) -> anyhow::Result<()> {
    let conn = app.db.conn();
    conn.execute(
        "INSERT INTO alert_events (at, alert_id, kind, severity, title, summary, delivery)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            crate::util::now_rfc3339(),
            n.condition.id,
            n.kind.as_str(),
            n.condition.severity.as_str(),
            n.condition.title,
            n.condition.summary,
            serde_json::to_string(deliveries)?,
        ],
    )?;
    conn.execute(
        "DELETE FROM alert_events WHERE id <= (SELECT MAX(id) FROM alert_events) - ?1",
        [HISTORY_KEEP],
    )?;
    Ok(())
}

pub fn history(app: &App, limit: usize) -> anyhow::Result<Vec<HistoryEntry>> {
    let conn = app.db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, at, alert_id, kind, severity, title, summary, delivery
         FROM alert_events ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |r| {
            let delivery: String = r.get(7)?;
            Ok(HistoryEntry {
                id: r.get(0)?,
                at: r.get(1)?,
                alert_id: r.get(2)?,
                kind: r.get(3)?,
                severity: r.get(4)?,
                title: r.get(5)?,
                summary: r.get(6)?,
                deliveries: serde_json::from_str(&delivery).unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Send a notice through every channel that is on, then keep it in the
/// history and the audit ledger.
pub async fn notify(app: &App, actor: &str, n: &Notice) -> Vec<channels::Delivery> {
    let deliveries = channels::deliver(app, n).await;
    if let Err(e) = record(app, n, &deliveries) {
        tracing::warn!("could not record alert {}: {e:#}", n.condition.id);
    }
    let summary = channels::delivery_summary(&deliveries);
    let action = format!("alert.{}", n.kind.as_str());
    if let Err(e) = app
        .db
        .audit(actor, &action, Some(&n.condition.id), Some(&summary))
    {
        tracing::warn!("could not audit alert {}: {e:#}", n.condition.id);
    }
    deliveries
}

// ── the job ─────────────────────────────────────────────────────────────

static RUN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One evaluation: gather, compare with the stored state, send what changed.
pub async fn evaluate(app: &App, confirm_delay: Duration) -> Vec<Notice> {
    let _one_at_a_time = RUN.lock().await;
    let Some(inputs) = gather(app, confirm_delay).await else {
        return vec![];
    };
    let mut passed: BTreeSet<String> = app
        .db
        .get_json(KV_DNS_PASSED)
        .ok()
        .flatten()
        .unwrap_or_default();
    let firing = conditions(&inputs, &mut passed);
    let _ = app.db.set_json(KV_DNS_PASSED, &passed);
    let mut active = load_active(app);
    let notices = step(&mut active, firing, inputs.now);
    // Saved before sending, so a crash mid-delivery never repeats a message.
    if let Err(e) = app.db.set_json(KV_ACTIVE, &active) {
        tracing::error!("could not save alert state: {e:#}");
        return vec![];
    }
    let _ = app.db.set_kv(KV_EVALUATED, &crate::util::now_rfc3339());
    for n in &notices {
        let d = notify(app, "system", n).await;
        tracing::info!(
            "alert {} {}: {}",
            n.condition.id,
            n.kind.as_str(),
            channels::delivery_summary(&d)
        );
    }
    notices
}

/// The background loop (started by `jobs::spawn_all`).
pub async fn run_forever(app: std::sync::Arc<App>) {
    // Let the first host and DNS checks land before judging them.
    tokio::time::sleep(Duration::from_secs(300)).await;
    loop {
        evaluate(&app, Duration::from_secs(15)).await;
        tokio::time::sleep(EVERY).await;
    }
}
