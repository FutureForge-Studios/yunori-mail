//! Business operations shared by the console API, the CLI and background jobs.

use crate::app::App;
use crate::dns::records::{self, RecordContext};
use crate::dns::{self, DnsLookup, Resolver};
use crate::engine::{EDomain, EQueued, Engine, RcptState};
use crate::model::{
    Check, DmarcPolicy, DnsRecord, RelaySettings, SendingMode, ServerConfig, Status, relay_provider,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainDns {
    pub domain: String,
    pub records: Vec<DnsRecord>,
    pub checked_at: Option<String>,
    pub zone_file: String,
}

pub fn dmarc_policy(app: &App, domain: &str) -> DmarcPolicy {
    match app
        .db
        .get_kv(&format!("dmarc_policy:{domain}"))
        .ok()
        .flatten()
        .as_deref()
    {
        Some("quarantine") => DmarcPolicy::Quarantine,
        Some("reject") => DmarcPolicy::Reject,
        _ => DmarcPolicy::None,
    }
}

pub fn relay_settings(app: &App) -> Option<RelaySettings> {
    app.db.get_json::<RelaySettings>("relay").ok().flatten()
}

pub fn record_context(app: &App, cfg: &ServerConfig, domain: &str) -> RecordContext {
    let relay_include = (cfg.sending_mode == SendingMode::Relay)
        .then(|| relay_settings(app))
        .flatten()
        .and_then(|r| {
            relay_provider(&r.provider)
                .and_then(|p| p.spf_include)
                .map(str::to_string)
        });
    RecordContext {
        hostname: cfg.hostname.clone(),
        ipv4: cfg.public_ipv4.clone(),
        ipv6: cfg.public_ipv6.clone(),
        is_primary: domain == cfg.primary_domain,
        relay_spf_include: relay_include,
        dmarc_policy: dmarc_policy(app, domain),
    }
}

/// Expected-vs-actual DNS for one domain. `fresh=false` returns the cached result when present.
pub async fn domain_dns(app: &App, domain: &EDomain, fresh: bool) -> Result<DomainDns> {
    let key = format!("dns:{}", domain.name);
    if !fresh && let Some((cached, _)) = app.db.cache_get::<DomainDns>(&key)? {
        return Ok(cached);
    }
    let cfg = app
        .config()
        .ok_or_else(|| anyhow::anyhow!("server is not configured"))?;
    let zone = app.engine()?.zone_file(&domain.id).await?;
    let ctx = record_context(app, &cfg, &domain.name);
    let expected = records::expected_records(&domain.name, &zone, &ctx);
    let resolver = Resolver::public();
    let lookup: &dyn DnsLookup = &resolver;
    let mut evaluated = records::evaluate(&expected, lookup, &ctx).await;
    if ctx.is_primary
        && let Some(ip) = &cfg.public_ipv4
    {
        let relayed = cfg.sending_mode == SendingMode::Relay;
        evaluated.push(records::evaluate_ptr(ip, &cfg.hostname, lookup, relayed).await);
    }
    let out = DomainDns {
        domain: domain.name.clone(),
        records: evaluated,
        checked_at: Some(crate::util::now_rfc3339()),
        zone_file: records::render_zone(&expected),
    };
    app.db.cache_put(&key, &out)?;
    Ok(out)
}

/// The cached DNS status (worst required record) for listing domains without a DNS round-trip.
pub fn cached_dns_status(app: &App, domain: &str) -> (Status, Option<String>) {
    match app.db.cache_get::<DomainDns>(&format!("dns:{domain}")) {
        Ok(Some((d, _))) => (records::worst_required(&d.records), d.checked_at),
        _ => (Status::Unknown, None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostChecks {
    pub port25: Check,
    pub ptr: Check,
    pub blocklists: Vec<Check>,
    pub provider: Option<Check>,
    pub checked_at: String,
}

fn ptr_check(rec: &DnsRecord) -> Check {
    let detail = if rec.actual.is_empty() {
        format!("No reverse DNS for {}.", rec.name)
    } else {
        format!("{} → {}", rec.name, rec.actual.join(", "))
    };
    let mut c = Check::new("ptr", "Reverse DNS (PTR)", rec.status, detail);
    if let Some(r) = &rec.remedy {
        c = c.remedy(r.clone());
    }
    c
}

/// Host-level deliverability checks (port 25, PTR, blocklists). Slow → cached, refreshed by jobs.
pub async fn host_checks(app: &App, fresh: bool) -> HostChecks {
    if !fresh && let Ok(Some((c, _))) = app.db.cache_get::<HostChecks>("checks:host") {
        return c;
    }
    let cfg = app.config();
    let port25 = tokio::task::spawn_blocking(crate::checks::check_outbound_25)
        .await
        .unwrap_or_else(|_| {
            Check::new(
                "port25",
                "Outbound port 25",
                Status::Unknown,
                "Check did not complete.",
            )
        });
    let (ptr, blocklists, provider) = match cfg
        .as_ref()
        .and_then(|c| c.public_ipv4.clone().map(|ip| (c, ip)))
    {
        Some((c, ip)) => {
            let public = Resolver::public();
            let relayed = c.sending_mode == SendingMode::Relay;
            let rec = records::evaluate_ptr(&ip, &c.hostname, &public, relayed).await;
            let local = Resolver::system();
            let bl = dns::reputation::check_blocklists(&ip, &local).await;
            let names = public.ptr(ip.parse().unwrap()).await.unwrap_or_default();
            let g = dns::reputation::guidance(dns::reputation::detect_provider(&names));
            let mut pc = Check::new(
                "provider",
                "Hosting provider",
                Status::Info,
                format!("{}. {} {}", g.name, g.port25, g.ptr),
            );
            if let Some(u) = g.url {
                pc = pc.help(u);
            }
            (ptr_check(&rec), bl, Some(pc))
        }
        None => (
            Check::new(
                "ptr",
                "Reverse DNS (PTR)",
                Status::Unknown,
                "The server's public IP is not known yet.",
            ),
            vec![],
            None,
        ),
    };
    let out = HostChecks {
        port25,
        ptr,
        blocklists,
        provider,
        checked_at: crate::util::now_rfc3339(),
    };
    let _ = app.db.cache_put("checks:host", &out);
    out
}

/// Cached host checks only (never blocks the request); `None` when never run.
pub fn cached_host_checks(app: &App) -> Option<HostChecks> {
    app.db
        .cache_get::<HostChecks>("checks:host")
        .ok()
        .flatten()
        .map(|(c, _)| c)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueGroup {
    pub reason: &'static str,
    pub count: usize,
    pub example: String,
    pub remedy: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSummary {
    pub total: usize,
    pub deferred: usize,
    pub groups: Vec<QueueGroup>,
}

/// Classify a failed recipient (auth / tls / network / rejected / other).
pub fn classify(
    state: &RcptState,
    error_type: Option<&str>,
    response: Option<&str>,
) -> &'static str {
    let resp = response.unwrap_or("").to_ascii_lowercase();
    if resp.starts_with("535") || resp.contains("authentication") || resp.contains("5.7.8") {
        return "auth";
    }
    match error_type {
        Some("tlsError" | "daneError" | "mtaStsError") => return "tls",
        Some("connectionError" | "dnsError" | "io" | "concurrencyLimited") => return "network",
        _ => {}
    }
    if *state == RcptState::PermanentFailure || resp.starts_with('5') {
        "rejected"
    } else {
        "other"
    }
}

fn remedy_for(reason: &str, relayed: bool) -> String {
    match reason {
        "auth" => "The relay refused the username or password. Update them under Sending.".into(),
        "tls" => "A secure connection could not be established. Check the certificate under System and the recipient's MTA-STS/DANE policy.".into(),
        "network" if relayed => "The relay host could not be reached. Check its host and port under Sending.".into(),
        "network" => "Other servers could not be reached — often a blocked outbound port 25. Switch sending to a relay.".into(),
        "rejected" => "Recipients refused the mail. Check DNS (SPF/DKIM/DMARC), reverse DNS and blocklists under Deliverability.".into(),
        _ => "Delivery is being retried automatically. If it persists, check your IP reputation and consider relay mode.".into(),
    }
}

pub fn summarize_queue(queue: &[EQueued], relayed: bool) -> QueueSummary {
    let mut groups: Vec<QueueGroup> = Vec::new();
    let mut deferred = 0;
    for m in queue {
        for r in &m.recipients {
            if matches!(r.state, RcptState::Completed | RcptState::Scheduled) && r.retry_count == 0
            {
                continue;
            }
            deferred += 1;
            let reason = classify(&r.state, r.error_type.as_deref(), r.response.as_deref());
            let example = format!(
                "{} — {}",
                r.address,
                r.response.clone().unwrap_or_else(|| "no response".into())
            );
            match groups.iter_mut().find(|g| g.reason == reason) {
                Some(g) => g.count += 1,
                None => groups.push(QueueGroup {
                    reason,
                    count: 1,
                    example,
                    remedy: remedy_for(reason, relayed),
                }),
            }
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.count));
    QueueSummary {
        total: queue.len(),
        deferred,
        groups,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub level: &'static str,
    pub sentence: String,
}

pub struct VerdictInputs<'a> {
    pub engine_ok: bool,
    pub install_complete: bool,
    pub waiting_detail: Option<&'a str>,
    pub primary_dns: Status,
    pub port25: Status,
    pub ptr: Status,
    pub listed: bool,
    pub mode: SendingMode,
    pub relay_label: Option<&'a str>,
}

/// One sentence that names the consequence (plan §9.2).
pub fn verdict(v: &VerdictInputs) -> Verdict {
    let mk = |level, s: String| Verdict { level, sentence: s };
    if !v.engine_ok {
        return mk("down", "The mail engine is not responding, so mail is neither received nor sent. Check System.".into());
    }
    if !v.install_complete {
        return mk(
            "attention",
            match v.waiting_detail {
                Some(d) => format!("Setup is not finished: {d}"),
                None => "Setup is not finished yet.".into(),
            },
        );
    }
    if v.primary_dns == Status::Fail {
        return mk("attention", "Some required DNS records are missing, so other servers may reject or misroute your mail.".into());
    }
    if v.listed {
        return mk("degraded", "This server's IP is on a blocklist, so some recipients will refuse its mail. Relay sending until it clears.".into());
    }
    if v.primary_dns == Status::Unknown {
        return mk(
            "attention",
            "DNS has not been verified yet, so Yunori cannot confirm mail can reach this server. Open Domains and press Check now.".into(),
        );
    }
    match v.mode {
        SendingMode::Relay => mk("ok", format!(
            "Mail is flowing: receiving directly, sending through {}.",
            v.relay_label.unwrap_or("your relay")
        )),
        SendingMode::Direct if v.port25 == Status::Warn || v.port25 == Status::Fail => mk("degraded",
            "Receiving works, but this server cannot send directly because outbound port 25 is blocked. Switch sending to a relay.".into()),
        SendingMode::Direct if v.ptr == Status::Fail => mk("attention",
            "Receiving works, but reverse DNS is missing, so Gmail and Outlook will likely reject mail you send.".into()),
        _ => mk("ok", "Mail is flowing normally.".into()),
    }
}

/// Engine reachability as a Check.
/// `known_version` is the pinned engine version from config (Stalwart's
/// session resource does not report its real version).
pub async fn engine_check(
    engine: Option<Arc<dyn Engine>>,
    known_version: Option<String>,
) -> (bool, Check) {
    match engine {
        None => (
            false,
            Check::new(
                "engine",
                "Mail engine",
                Status::Fail,
                "The mail engine is not installed yet.",
            ),
        ),
        Some(e) => {
            let info = e.info().await;
            if info.reachable {
                (
                    true,
                    Check::new(
                        "engine",
                        "Mail engine",
                        Status::Pass,
                        match info.version.or(known_version) {
                            Some(v) => format!("Stalwart {v} is running."),
                            None => "Stalwart is running.".to_string(),
                        },
                    ),
                )
            } else {
                (
                    false,
                    Check::new(
                        "engine",
                        "Mail engine",
                        Status::Fail,
                        "Stalwart is not responding.",
                    )
                    .remedy("Run: systemctl status stalwart — then `yunori-mail doctor`."),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EQueuedRecipient;

    #[test]
    fn classifies_deferrals() {
        assert_eq!(
            classify(
                &RcptState::TemporaryFailure,
                None,
                Some("535 5.7.8 Authentication failed")
            ),
            "auth"
        );
        assert_eq!(
            classify(&RcptState::TemporaryFailure, Some("tlsError"), None),
            "tls"
        );
        assert_eq!(
            classify(&RcptState::TemporaryFailure, Some("connectionError"), None),
            "network"
        );
        assert_eq!(
            classify(
                &RcptState::PermanentFailure,
                Some("unexpectedResponse"),
                Some("550 5.7.1 rejected")
            ),
            "rejected"
        );
        assert_eq!(
            classify(
                &RcptState::TemporaryFailure,
                Some("unexpectedResponse"),
                Some("451 try later")
            ),
            "other"
        );
    }

    #[test]
    fn queue_summary_groups() {
        let q = vec![EQueued {
            id: "1".into(),
            created_at: None,
            next_retry: None,
            recipients: vec![
                EQueuedRecipient {
                    address: "a@x".into(),
                    state: RcptState::TemporaryFailure,
                    error_type: Some("connectionError".into()),
                    response: None,
                    retry_count: 2,
                },
                EQueuedRecipient {
                    address: "b@x".into(),
                    state: RcptState::TemporaryFailure,
                    error_type: Some("connectionError".into()),
                    response: None,
                    retry_count: 1,
                },
                EQueuedRecipient {
                    address: "c@x".into(),
                    state: RcptState::Scheduled,
                    error_type: None,
                    response: None,
                    retry_count: 0,
                },
            ],
        }];
        let s = summarize_queue(&q, false);
        assert_eq!(s.deferred, 2);
        assert_eq!(s.groups[0].reason, "network");
        assert!(s.groups[0].remedy.contains("relay"));
    }

    #[test]
    fn verdict_names_the_consequence() {
        let base = VerdictInputs {
            engine_ok: true,
            install_complete: true,
            waiting_detail: None,
            primary_dns: Status::Pass,
            port25: Status::Pass,
            ptr: Status::Pass,
            listed: false,
            mode: SendingMode::Direct,
            relay_label: None,
        };
        assert_eq!(verdict(&base).level, "ok");
        let unverified = VerdictInputs {
            primary_dns: Status::Unknown,
            ..base
        };
        assert_eq!(verdict(&unverified).level, "attention");
        let blocked = VerdictInputs {
            port25: Status::Warn,
            ..base
        };
        assert!(verdict(&blocked).sentence.contains("port 25"));
        let relayed = VerdictInputs {
            port25: Status::Warn,
            mode: SendingMode::Relay,
            relay_label: Some("Amazon SES"),
            ..base
        };
        assert!(verdict(&relayed).sentence.contains("Amazon SES"));
        let down = VerdictInputs {
            engine_ok: false,
            ..base
        };
        assert_eq!(verdict(&down).level, "down");
    }
}
