//! Automatic HTTPS. One place decides when a certificate is needed, checks
//! what would make the CA fail before asking it, retries with backoff, and
//! remembers why the last attempt failed so the terminal, the console and
//! `doctor` can all say the same plain sentence.
//!
//! Callers: the installer's `certificate` step (as root, or from `serve`),
//! the renewal job in `serve` (every few minutes; it only acts when needed),
//! `yunori-mail certificate renew`, and the console's "Try now" button. A
//! file lock keeps two processes from ordering at the same time.
//!
//! Rate limits: Let's Encrypt allows 5 failed validations per hostname per
//! hour. After a failure, every retry first does a dry run against the
//! staging CA (much higher limits); production is only asked again once the
//! dry run passes. Helper names (autoconfig., mta-sts., …) that fail are left
//! out for a day instead of blocking the mail hostname's certificate.

use super::acme::{self, CertProblem, Failure, Issued, ProblemKind};
use super::tls;
use crate::app::App;
use crate::dns::DnsLookup;
use crate::paths::Paths;
use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

const STATE_KEY: &str = "tls.acme_state";
/// Renew when fewer days than this remain.
pub const RENEW_DAYS: i64 = 30;
/// Retry delays after consecutive failures.
const BACKOFF_SECS: &[i64] = &[
    30, 60, 120, 300, 600, 1800, 3600, 7200, 14_400, 28_800, 43_200,
];
/// Helper names that failed are left out of orders for this long.
const EXCLUDE_HOURS: i64 = 24;
/// Re-issuing only to add helper names happens at most this often.
const COVERAGE_REISSUE_HOURS: i64 = 24;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AcmeState {
    pub last_attempt: Option<String>,
    pub last_success: Option<String>,
    pub next_attempt: Option<String>,
    pub failures: u32,
    pub problem: Option<CertProblem>,
    /// Names requested on the last attempt.
    pub names: Vec<String>,
    /// Helper names left out until the given time.
    pub excluded: BTreeMap<String, String>,
    /// Last time a certificate was re-issued only to add helper names.
    pub coverage_reissue: Option<String>,
    /// Last time helper names were looked up (DNS) while the certificate was fine.
    pub coverage_checked: Option<String>,
    /// Last order sent to Let's Encrypt production (spacing while failing).
    pub last_production_attempt: Option<String>,
}

/// While failing without a rehearsal, production is asked at most this often
/// (Let's Encrypt allows 5 failed validations per hostname per hour).
const PRODUCTION_SPACING_MINUTES: i64 = 12;

/// A rehearsal failure, told as what it predicts: the same answer from the
/// real CA. Where it came from stays in the detail.
fn as_rehearsal(mut p: CertProblem) -> CertProblem {
    p.summary = p.summary.replace(
        &acme::ca_name(acme::LE_STAGING),
        &acme::ca_name(acme::LE_PRODUCTION),
    );
    p.detail = Some(match p.detail.take() {
        Some(d) => format!("{d} (a test run on Let's Encrypt's staging service, which spares the real rate limits)"),
        None => "Seen in a test run on Let's Encrypt's staging service, which spares the real rate limits.".into(),
    });
    p
}

/// Did the staging CA judge *our* setup (so production would say the same)?
/// An outage or an answer we cannot read says nothing about this server.
fn staging_conclusive(p: &CertProblem) -> bool {
    !matches!(p.kind, ProblemKind::CaUnreachable | ProblemKind::Other)
}

/// The names the certificate must cover, most important first. Installs from
/// before the browser onboarding: the mail hostname. Otherwise the console's
/// own address (unless it is a bare IP), plus the mail hostname once its host
/// record has been verified (the onboarding's `host_record` gate): until then a
/// mail hostname that does not point here yet must not hold the console's
/// certificate back. Empty while there is nothing a CA could issue for.
pub fn required_names(app: &App, cfg: &crate::model::ServerConfig) -> Vec<String> {
    let panel = cfg
        .panel_host
        .as_deref()
        .filter(|p| p.parse::<std::net::IpAddr>().is_err());
    let mail_ready = cfg.install_complete
        || cfg.panel_host.is_none()
        || panel == Some(cfg.hostname.as_str())
        || matches!(
            app.db.step_status("host_record"),
            Ok(crate::installer::state::StepStatus::Done)
        );
    let mut names = Vec::new();
    if mail_ready {
        names.push(cfg.hostname.clone());
    }
    if let Some(p) = panel
        && !names.iter().any(|n| n == p)
    {
        names.push(p.to_string());
    }
    names
}

/// CA-issued, covering every required name, not near expiry, and not a test certificate.
fn healthy(disk: Option<&tls::CertInfo>, required: &[String], directory: &str) -> bool {
    disk.is_some_and(|c| {
        !c.self_signed
            && c.days_left >= RENEW_DAYS
            && required.iter().all(|r| c.hostnames.iter().any(|h| h == r))
            && (directory == acme::LE_STAGING
                || !c
                    .issuer
                    .as_deref()
                    .is_some_and(|i| i.to_ascii_uppercase().contains("STAGING")))
    })
}

/// Would an order for `names` add nothing to a healthy certificate?
fn adds_nothing(disk: Option<&tls::CertInfo>, names: &[String]) -> bool {
    disk.is_some_and(|c| names.iter().all(|n| c.hostnames.contains(n)))
}

pub fn load_state(app: &App) -> AcmeState {
    app.db
        .get_json::<AcmeState>(STATE_KEY)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn save_state(app: &App, st: &AcmeState) {
    if let Err(e) = app.db.set_json(STATE_KEY, st) {
        tracing::warn!("could not save certificate state: {e:#}");
    }
}

fn parse_time(s: &Option<String>) -> Option<DateTime<Utc>> {
    s.as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn backoff(failures: u32) -> ChronoDuration {
    let i = (failures.max(1) as usize - 1).min(BACKOFF_SECS.len() - 1);
    ChronoDuration::seconds(BACKOFF_SECS[i])
}

// ── what to certify, and whether it is time ──────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Names the next certificate should cover (mail hostname first).
    pub names: Vec<String>,
    /// Why a certificate is needed now; `None` when the current one is fine.
    pub reason: Option<String>,
}

/// Is the certificate on disk one a browser would trust for long enough?
#[cfg(test)]
pub fn plan_for(
    hostname: &str,
    wanted: &[String],
    disk: Option<&tls::CertInfo>,
    directory: &str,
    st: &AcmeState,
    now: DateTime<Utc>,
) -> Plan {
    plan_for_names(&[hostname.to_string()], wanted, disk, directory, st, now)
}

/// [`plan_for`] with several names that must be covered (the mail hostname
/// and the console's address); helper names may be left out, these never.
pub fn plan_for_names(
    required: &[String],
    wanted: &[String],
    disk: Option<&tls::CertInfo>,
    directory: &str,
    st: &AcmeState,
    now: DateTime<Utc>,
) -> Plan {
    let mut names: Vec<String> = wanted
        .iter()
        .filter(|n| {
            required.contains(n)
                || parse_time(&st.excluded.get(n.as_str()).cloned())
                    .is_none_or(|until| until <= now)
        })
        .cloned()
        .collect();
    for (i, r) in required.iter().enumerate() {
        if !names.contains(r) {
            names.insert(i.min(names.len()), r.clone());
        }
    }
    let uncovered = disk.and_then(|c| {
        required
            .iter()
            .find(|r| !c.hostnames.iter().any(|h| h == *r))
    });
    let reason = match disk {
        None => Some("there is no certificate yet".to_string()),
        Some(c) if c.self_signed => Some("a temporary self-signed certificate is in use".into()),
        Some(_) if uncovered.is_some() => Some(format!(
            "the certificate does not cover {}",
            uncovered.map(String::as_str).unwrap_or_default()
        )),
        Some(c) if c.days_left < RENEW_DAYS => Some(format!(
            "the certificate expires in {} day{}",
            c.days_left.max(0),
            if c.days_left == 1 { "" } else { "s" }
        )),
        Some(c)
            if directory != acme::LE_STAGING
                && c.issuer
                    .as_deref()
                    .is_some_and(|i| i.to_ascii_uppercase().contains("STAGING")) =>
        {
            Some("a test (staging) certificate is in use".into())
        }
        Some(c) => {
            let missing: Vec<&String> = names
                .iter()
                .filter(|n| !c.hostnames.iter().any(|h| h == *n))
                .collect();
            let recently = parse_time(&st.coverage_reissue)
                .is_some_and(|t| now - t < ChronoDuration::hours(COVERAGE_REISSUE_HOURS));
            if !missing.is_empty() && !recently {
                Some(format!(
                    "{} now point{} here",
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    if missing.len() == 1 { "s" } else { "" }
                ))
            } else {
                None
            }
        }
    };
    if reason.is_none() {
        // Keep what is already covered; never re-issue just to drop a name.
        if let Some(c) = disk {
            names = c.hostnames.clone();
        }
    }
    Plan { names, reason }
}

// ── checks before asking the CA ──────────────────────────────────────────

/// Cloudflare's published proxy ranges (www.cloudflare.com/ips-v4).
const CLOUDFLARE_V4: &[(u32, u8)] = &[
    (0xAD_F5_30_00, 20), // 173.245.48.0/20
    (0x67_15_F4_00, 22), // 103.21.244.0/22
    (0x67_16_C8_00, 22), // 103.22.200.0/22
    (0x67_1F_04_00, 22), // 103.31.4.0/22
    (0x8D_65_40_00, 18), // 141.101.64.0/18
    (0x6C_A2_C0_00, 18), // 108.162.192.0/18
    (0xBE_5D_F0_00, 20), // 190.93.240.0/20
    (0xBC_72_60_00, 20), // 188.114.96.0/20
    (0xC5_EA_F0_00, 22), // 197.234.240.0/22
    (0xC6_29_80_00, 17), // 198.41.128.0/17
    (0xA2_9E_00_00, 15), // 162.158.0.0/15
    (0x68_10_00_00, 13), // 104.16.0.0/13
    (0x68_18_00_00, 14), // 104.24.0.0/14
    (0xAC_40_00_00, 13), // 172.64.0.0/13
    (0x83_00_48_00, 22), // 131.0.72.0/22
];

pub fn is_cloudflare(ip: &str) -> bool {
    let Ok(std::net::IpAddr::V4(v4)) = ip.parse::<std::net::IpAddr>() else {
        return false;
    };
    let n = u32::from(v4);
    CLOUDFLARE_V4
        .iter()
        .any(|(net, bits)| n & (u32::MAX << (32 - *bits as u32)) == *net)
}

/// Does `hostname` reach this server, and may the CA issue for it?
pub async fn precheck(
    name: &str,
    ipv4: Option<&str>,
    ipv6: Option<&str>,
    dns: &dyn DnsLookup,
    caa_identity: Option<&str>,
) -> Result<(), Box<CertProblem>> {
    let ipv6_only = ipv4.is_none() && ipv6.is_some();
    if ipv6_only {
        // Checked on AAAA below; an A record elsewhere doesn't matter to a CA
        // that reaches this name over IPv6, and there is no IPv4 to point it at.
        if let Ok(v6s) = dns.aaaa(name).await
            && v6s.is_empty()
        {
            return Err(Box::new(CertProblem::new(
                ProblemKind::DnsMissing,
                Some(name),
                format!("{name} has no AAAA record on public DNS yet."),
                format!(
                    "At your DNS provider, add: {name}  AAAA  {}",
                    ipv6.unwrap_or_default()
                ),
            )));
        }
    } else {
        match dns.a(name).await {
            Ok(ips) if ips.is_empty() => {
                return Err(Box::new(CertProblem::new(
                    ProblemKind::DnsMissing,
                    Some(name),
                    format!("{name} has no A record on public DNS yet."),
                    match ipv4 {
                        Some(ip) => format!("At your DNS provider, add: {name}  A  {ip}"),
                        None => format!(
                            "At your DNS provider, add an A record for {name} pointing at this server."
                        ),
                    },
                )));
            }
            Ok(ips) => {
                if let Some(ip) = ipv4
                    && !ips.iter().any(|a| a == ip)
                {
                    if ips.iter().any(|a| is_cloudflare(a)) {
                        return Err(Box::new(CertProblem::new(
                            ProblemKind::CloudflareProxy,
                            Some(name),
                            format!("{name} is proxied by Cloudflare (the orange cloud)."),
                            format!(
                                "In Cloudflare's DNS settings, set {name} to \"DNS only\" (grey cloud): mail needs a direct connection to {ip}."
                            ),
                        )));
                    }
                    return Err(Box::new(CertProblem::new(
                        ProblemKind::DnsElsewhere,
                        Some(name),
                        format!(
                            "{name} points at {}, not this server ({ip}).",
                            ips.join(", ")
                        ),
                        format!("Change the A record of {name} to {ip}."),
                    )));
                }
            }
            Err(_) => {} // The CA will say what it sees.
        }
    }
    if let Ok(v6s) = dns.aaaa(name).await
        && !v6s.is_empty()
        && ipv6.is_none_or(|mine| !v6s.iter().any(|a| a == mine))
    {
        return Err(Box::new(CertProblem::new(
            ProblemKind::Ipv6Elsewhere,
            Some(name),
            format!(
                "{name} has an IPv6 (AAAA) record, {}, that is not this server{}.",
                v6s.join(", "),
                ipv6.map(|v| format!(" ({v})")).unwrap_or_default()
            ),
            match ipv6 {
                Some(v) => format!("Certificate authorities try IPv6 first: change that AAAA record to {v}, or delete it."),
                None => "Certificate authorities try IPv6 first: delete that AAAA record (this server has no IPv6 address).".into(),
            },
        )));
    }
    if let Some(id) = caa_identity
        && let Some(blocking) = caa_blocks(name, id, dns).await
    {
        return Err(Box::new(CertProblem::new(
            ProblemKind::Caa,
            Some(name),
            format!("The CAA records on {blocking} do not allow {id} to issue certificates."),
            format!(
                "At your DNS provider, add  {blocking}  CAA  0 issue \"{id}\"  next to the existing CAA records."
            ),
        )));
    }
    Ok(())
}

/// RFC 8659: the closest name (walking up from `name`) with CAA records
/// decides. Returns that name when its `issue` records exclude `identity`.
pub async fn caa_blocks(name: &str, identity: &str, dns: &dyn DnsLookup) -> Option<String> {
    let labels: Vec<&str> = name.split('.').collect();
    for i in 0..labels.len().saturating_sub(1) {
        let candidate = labels[i..].join(".");
        let Ok(records) = dns.caa(&candidate).await else {
            return None; // Unknown: let the CA decide.
        };
        if records.is_empty() {
            continue;
        }
        let issuers: Vec<String> = records
            .iter()
            .filter_map(|r| {
                let mut parts = r.splitn(3, ' ');
                let _flags = parts.next()?;
                let tag = parts.next()?.to_ascii_lowercase();
                let value = parts.next()?.trim().trim_matches('"');
                (tag == "issue").then(|| {
                    value
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                })
            })
            .collect();
        if issuers.is_empty() {
            return None; // Only issuewild/iodef: no restriction for this name.
        }
        return (!issuers.iter().any(|i| i == identity)).then_some(candidate);
    }
    None
}

/// Is this server's own port-80 listener serving challenge files for `name`?
/// (Local only: it cannot see a cloud firewall, but it catches a stopped edge
/// or another web server holding port 80.)
pub async fn self_test(paths: &Paths, name: &str, port: u16) -> Result<(), Box<CertProblem>> {
    let dir = paths.acme_challenges_dir();
    let _ = std::fs::create_dir_all(&dir);
    let token = format!("yunori-self-test-{}", crate::util::random_token(18));
    let expected = format!("{token}.ok");
    let file = dir.join(&token);
    if std::fs::write(&file, &expected).is_err() {
        return Ok(()); // Can't stage a probe; the CA will tell.
    }
    let res = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .build()
        .ok()
        .map(|c| {
            c.get(format!(
                "http://127.0.0.1:{port}/.well-known/acme-challenge/{token}"
            ))
            .header(reqwest::header::HOST, name)
            .send()
        });
    let outcome = match res {
        None => Ok(()),
        Some(fut) => match fut.await {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(body) if body.trim() == expected => Ok(()),
                _ => Err(edge_problem(name, "another web server answers on port 80")),
            },
            Ok(r) => Err(edge_problem(
                name,
                &format!(
                    "port 80 answers with HTTP {} instead of serving the challenge",
                    r.status().as_u16()
                ),
            )),
            Err(_) => Err(edge_problem(
                name,
                "nothing on this server answers on port 80",
            )),
        },
    };
    let _ = std::fs::remove_file(&file);
    outcome
}

fn edge_problem(name: &str, what: &str) -> Box<CertProblem> {
    Box::new(CertProblem::new(
        ProblemKind::EdgeDown,
        Some(name),
        format!("The certificate check for {name} cannot work: {what}."),
        "Make sure yunori-mail owns port 80 (`systemctl status yunori-mail`, `ss -ltnp 'sport = :80'`).",
    ))
}

// ── issuing ──────────────────────────────────────────────────────────────

#[async_trait]
pub trait Issuer: Send + Sync {
    async fn issue(&self, directory: &str, names: &[String]) -> Result<Issued, Failure>;
}

pub struct AcmeIssuer<'a> {
    pub app: &'a App,
    pub contact: String,
}

#[async_trait]
impl Issuer for AcmeIssuer<'_> {
    async fn issue(&self, directory: &str, names: &[String]) -> Result<Issued, Failure> {
        acme::order(
            &self.app.paths,
            &self.app.db,
            directory,
            names,
            &self.contact,
        )
        .await
    }
}

/// What `ensure` did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The certificate on disk is fine.
    Current,
    /// A new certificate was written for these names.
    Issued(Vec<String>),
    /// This attempt failed; the next one is scheduled.
    Failed {
        problem: CertProblem,
        next_attempt: String,
    },
    /// Backing off after an earlier failure.
    Later {
        problem: Option<CertProblem>,
        next_attempt: String,
    },
    /// Another process is ordering a certificate right now.
    Busy,
    /// Not installed far enough to know the hostname.
    NotConfigured,
}

pub struct Deps<'a> {
    pub dns: &'a dyn DnsLookup,
    pub issuer: &'a dyn Issuer,
    /// Port the edge's plain-HTTP listener is on; `None` skips the local self-test.
    pub self_test_port: Option<u16>,
}

/// Bring the certificate up to date if it is time. `force` ignores the
/// backoff (an operator asked), never the staging dry run.
pub async fn ensure(app: &App, force: bool) -> Outcome {
    let Some(cfg) = app.config() else {
        return Outcome::NotConfigured;
    };
    let required = required_names(app, &cfg);
    let Some(first) = required.first() else {
        return Outcome::NotConfigured;
    };
    // The zone's own servers: a record just fixed counts at once, not after a
    // public resolver's cached copy of the old one expires.
    let dns = crate::dns::Truth::for_name(first).await;
    let issuer = AcmeIssuer {
        app,
        contact: cfg.admin_email.clone(),
    };
    let deps = Deps {
        dns: &dns,
        issuer: &issuer,
        self_test_port: (!app.dev).then_some(80),
    };
    let out = ensure_with(app, force, &deps).await;
    if let Outcome::Issued(names) = &out {
        tracing::info!("certificate issued for {}", names.join(", "));
        crate::installer::fix_permissions(&app.paths);
        crate::jobs::push_certificate(app).await;
    }
    out
}

pub async fn ensure_with(app: &App, force: bool, deps: &Deps<'_>) -> Outcome {
    let Some(cfg) = app.config() else {
        return Outcome::NotConfigured;
    };
    let required = required_names(app, &cfg);
    let Some(primary) = required.first().cloned() else {
        return Outcome::NotConfigured;
    };
    let Some(_lock) = Lock::try_acquire(&app.paths) else {
        return Outcome::Busy;
    };
    let now = Utc::now();
    let directory = acme::directory(&app.db);
    let ca = acme::ca_name(&directory);
    let mut st = load_state(app);
    let disk = tls::disk_certificate_info(&app.paths);
    let fine = healthy(disk.as_ref(), &required, &directory);
    // A healthy certificate only needs the (DNS-costly) coverage look-up hourly.
    if fine
        && !force
        && st.problem.is_none()
        && parse_time(&st.coverage_checked).is_some_and(|t| now - t < ChronoDuration::hours(1))
    {
        return Outcome::Current;
    }

    let domains: Vec<String> = match app.engine() {
        Ok(e) => e
            .list_domains()
            .await
            .map(|d| d.into_iter().map(|x| x.name).collect())
            .unwrap_or_else(|_| vec![cfg.primary_domain.clone()]),
        Err(_) => vec![cfg.primary_domain.clone()],
    };
    // Helper names (autoconfig., mta-sts., …) only once the mail hostname is
    // part of the certificate: before that the domains are still placeholders.
    let mut wanted = required.clone();
    if required.contains(&cfg.hostname) {
        for n in acme::wanted_names(
            &cfg.hostname,
            &domains,
            cfg.public_ipv4.as_deref(),
            deps.dns,
        )
        .await
        {
            if !wanted.contains(&n) {
                wanted.push(n);
            }
        }
    }
    if fine {
        st.coverage_checked = Some(stamp(now));
    }
    let plan = plan_for_names(&required, &wanted, disk.as_ref(), &directory, &st, now);
    let Some(reason) = plan.reason.clone() else {
        st.problem = None;
        st.failures = 0;
        st.next_attempt = None;
        save_state(app, &st);
        return Outcome::Current;
    };
    if !force
        && let Some(next) = parse_time(&st.next_attempt)
        && next > now
    {
        return Outcome::Later {
            problem: st.problem.clone(),
            next_attempt: stamp(next),
        };
    }
    st.last_attempt = Some(stamp(now));
    let _ = app.db.install_log(
        "info",
        Some("certificate"),
        &format!("Requesting a certificate from {ca}: {reason}."),
    );

    // 1. Checks we can make ourselves, before spending the CA's patience.
    let caa_id = acme::caa_identity(&directory);
    let mut names = Vec::new();
    for n in &plan.names {
        match precheck(
            n,
            cfg.public_ipv4.as_deref(),
            cfg.public_ipv6.as_deref(),
            deps.dns,
            caa_id,
        )
        .await
        {
            Ok(()) => names.push(n.clone()),
            Err(p) if required.contains(n) => {
                return record_failure(app, st, *p, &plan.names, now);
            }
            Err(p) => {
                exclude(&mut st, n, now);
                let _ = app.db.install_log(
                    "warn",
                    Some("certificate"),
                    &format!("Leaving {n} out for now: {}", p.summary),
                );
            }
        }
    }
    if fine && adds_nothing(disk.as_ref(), &names) {
        // Every new name was left out: re-issuing would only duplicate the certificate.
        save_state(app, &st);
        return Outcome::Current;
    }
    if let Some(port) = deps.self_test_port
        && let Err(p) = self_test(&app.paths, &primary, port).await
    {
        return record_failure(app, st, *p, &names, now);
    }

    // 2. After a failure, rehearse on staging so production's limits stay untouched.
    for attempt in 0..2 {
        if st.failures > 0 && directory == acme::LE_PRODUCTION {
            match deps.issuer.issue(acme::LE_STAGING, &names).await {
                Ok(_) => {}
                Err(f) if staging_conclusive(&f.problem) => {
                    if attempt == 0 && drop_helpers(&mut st, &mut names, &f, &required, now) {
                        if fine && adds_nothing(disk.as_ref(), &names) {
                            save_state(app, &st);
                            return Outcome::Current;
                        }
                        continue;
                    }
                    return record_failure(app, st, as_rehearsal(*f.problem), &names, now);
                }
                Err(f) => {
                    // The staging CA itself failed: that must never block
                    // production, but production is then asked sparingly.
                    let _ = app.db.install_log(
                        "warn",
                        Some("certificate"),
                        &format!(
                            "The staging rehearsal was inconclusive ({}); asking {ca} directly.",
                            f.problem.summary
                        ),
                    );
                    if let Some(last) = parse_time(&st.last_production_attempt)
                        && now - last < ChronoDuration::minutes(PRODUCTION_SPACING_MINUTES)
                    {
                        let next = last + ChronoDuration::minutes(PRODUCTION_SPACING_MINUTES);
                        st.next_attempt = Some(stamp(next));
                        save_state(app, &st);
                        return Outcome::Later {
                            problem: st.problem.clone(),
                            next_attempt: stamp(next),
                        };
                    }
                }
            }
        }
        if directory == acme::LE_PRODUCTION {
            st.last_production_attempt = Some(stamp(now));
        }
        match deps.issuer.issue(&directory, &names).await {
            Ok(issued) => {
                if let Err(e) =
                    tls::write_cert_files(&app.paths, &issued.chain_pem, &issued.key_pem)
                {
                    let p = CertProblem::new(
                        ProblemKind::Other,
                        None,
                        "The certificate was issued but could not be saved.",
                        "Check free disk space and permissions on /var/lib/yunori-mail/certs.",
                    )
                    .detail(e.to_string());
                    return record_failure(app, st, p, &names, now);
                }
                let only_coverage = fine;
                st.last_success = Some(stamp(Utc::now()));
                st.failures = 0;
                st.problem = None;
                st.next_attempt = None;
                st.names = names.clone();
                if only_coverage {
                    st.coverage_reissue = Some(stamp(now));
                }
                save_state(app, &st);
                let _ = app.db.set_kv("tls.acme_ok", "true");
                let _ = app.db.set_kv("tls.served_matches", "false");
                let _ = app.db.audit(
                    "system",
                    "certificate.issue",
                    Some(&names.join(", ")),
                    Some(&format!("{ca}: {reason}")),
                );
                let _ = app.db.install_log(
                    "info",
                    Some("certificate"),
                    &format!("{ca} issued a certificate for {}.", names.join(", ")),
                );
                return Outcome::Issued(names);
            }
            Err(f) => {
                if attempt == 0 && drop_helpers(&mut st, &mut names, &f, &required, now) {
                    if fine && adds_nothing(disk.as_ref(), &names) {
                        save_state(app, &st);
                        return Outcome::Current;
                    }
                    continue;
                }
                return record_failure(app, st, *f.problem, &names, now);
            }
        }
    }
    unreachable!("the loop returns on its second pass")
}

/// When only helper names failed, leave them out and say whether to retry.
fn drop_helpers(
    st: &mut AcmeState,
    names: &mut Vec<String>,
    f: &Failure,
    required: &[String],
    now: DateTime<Utc>,
) -> bool {
    if f.failed_names.is_empty() || f.failed_names.iter().any(|n| required.contains(n)) {
        return false;
    }
    for n in &f.failed_names {
        exclude(st, n, now);
    }
    names.retain(|n| !f.failed_names.contains(n));
    !names.is_empty()
}

fn exclude(st: &mut AcmeState, name: &str, now: DateTime<Utc>) {
    st.excluded.insert(
        name.to_string(),
        stamp(now + ChronoDuration::hours(EXCLUDE_HOURS)),
    );
}

fn record_failure(
    app: &App,
    mut st: AcmeState,
    problem: CertProblem,
    names: &[String],
    now: DateTime<Utc>,
) -> Outcome {
    st.failures = st.failures.saturating_add(1);
    let mut next = now + backoff(st.failures);
    if problem.kind == ProblemKind::RateLimited {
        let asked = parse_time(&problem.retry_after).unwrap_or(now + ChronoDuration::hours(1));
        next = next.max(asked);
    }
    st.next_attempt = Some(stamp(next));
    st.names = names.to_vec();
    st.problem = Some(problem.clone());
    save_state(app, &st);
    let _ = app
        .db
        .install_log("warn", Some("certificate"), &problem.sentence());
    if let Some(d) = &problem.detail {
        let _ = app
            .db
            .install_log("warn", Some("certificate"), &format!("The CA said: {d}"));
    }
    tracing::warn!("certificate attempt failed: {}", problem.sentence());
    Outcome::Failed {
        problem,
        next_attempt: stamp(next),
    }
}

// ── status for the console, the CLI and doctor ───────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertStatus {
    /// "valid" | "expiring" | "expired" | "self_signed" | "test" | "missing"
    pub state: &'static str,
    pub hostnames: Vec<String>,
    pub issuer: Option<String>,
    pub not_after: Option<String>,
    pub days_left: Option<i64>,
    pub authority: String,
    /// When automatic renewal will start.
    pub renews_at: Option<String>,
    pub last_attempt: Option<String>,
    pub last_success: Option<String>,
    pub next_attempt: Option<String>,
    pub failures: u32,
    pub problem: Option<CertProblem>,
    pub in_progress: bool,
    /// The engine serves this certificate on 465/587/993.
    pub served_matches: Option<bool>,
}

pub fn status(app: &App) -> CertStatus {
    let st = load_state(app);
    let directory = acme::directory(&app.db);
    let info = tls::disk_certificate_info(&app.paths);
    let state = match &info {
        None => "missing",
        Some(c) if c.self_signed => "self_signed",
        Some(c) if c.days_left < 0 => "expired",
        Some(c)
            if directory != acme::LE_STAGING
                && c.issuer
                    .as_deref()
                    .is_some_and(|i| i.to_ascii_uppercase().contains("STAGING")) =>
        {
            "test"
        }
        Some(c) if c.days_left < 14 => "expiring",
        Some(_) => "valid",
    };
    let renews_at = info
        .as_ref()
        .filter(|c| !c.self_signed)
        .and_then(|c| parse_time(&c.not_after))
        .map(|t| stamp(t - ChronoDuration::days(RENEW_DAYS)));
    CertStatus {
        state,
        hostnames: info
            .as_ref()
            .map(|c| c.hostnames.clone())
            .unwrap_or_default(),
        issuer: info.as_ref().and_then(|c| c.issuer.clone()),
        not_after: info.as_ref().and_then(|c| c.not_after.clone()),
        days_left: info.as_ref().map(|c| c.days_left),
        authority: acme::ca_name(&directory),
        renews_at,
        last_attempt: st.last_attempt,
        last_success: st.last_success,
        next_attempt: st.next_attempt,
        failures: st.failures,
        problem: st.problem,
        in_progress: Lock::try_acquire(&app.paths).is_none(),
        served_matches: app
            .db
            .get_kv("tls.served_matches")
            .ok()
            .flatten()
            .map(|v| v == "true"),
    }
}

/// The certificate as one console/doctor signal.
pub fn check(app: &App) -> crate::model::Check {
    use crate::model::{Check, Status};
    let s = status(app);
    let label = "TLS certificate";
    let problem = s.problem.as_ref();
    let with_problem = |c: Check| match problem {
        Some(p) => c.remedy(p.sentence()),
        None => c,
    };
    match s.state {
        "missing" => with_problem(Check::new(
            "tls",
            label,
            if app.dev { Status::Info } else { Status::Warn },
            "No certificate has been issued yet.",
        )),
        "self_signed" => with_problem(Check::new(
            "tls",
            label,
            if app.dev { Status::Info } else { Status::Warn },
            format!(
                "A temporary self-signed certificate is in use until {} issues the real one.",
                s.authority
            ),
        )),
        "expired" => with_problem(
            Check::new("tls", label, Status::Fail, "The certificate has expired.")
                .remedy("Run: sudo yunori-mail certificate renew"),
        ),
        "test" => with_problem(Check::new(
            "tls",
            label,
            Status::Warn,
            "A test (staging) certificate is in use; browsers and mail apps will not trust it.",
        )),
        "expiring" => with_problem(Check::new(
            "tls",
            label,
            Status::Warn,
            format!(
                "Expires in {} days; automatic renewal has not succeeded yet.",
                s.days_left.unwrap_or(0)
            ),
        )),
        _ => Check::new(
            "tls",
            label,
            Status::Pass,
            format!(
                "Valid for {} more days ({}); renews automatically.",
                s.days_left.unwrap_or(0),
                s.hostnames.join(", ")
            ),
        ),
    }
}

// ── one order at a time, across processes ────────────────────────────────

pub struct Lock {
    _file: std::fs::File,
}

impl Lock {
    pub fn try_acquire(paths: &Paths) -> Option<Lock> {
        use std::os::unix::io::AsRawFd;
        let dir = paths.state_dir();
        let _ = std::fs::create_dir_all(&dir);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("acme.lock"))
            .ok()?;
        // SAFETY: flock on a descriptor we own; released when the file closes.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        (rc == 0).then_some(Lock { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::fake::FakeDns;
    use std::sync::Mutex;

    const IP: &str = "203.0.113.10";

    fn info(names: &[&str], days: i64, self_signed: bool) -> tls::CertInfo {
        tls::CertInfo {
            hostnames: names.iter().map(|s| s.to_string()).collect(),
            issuer: Some(
                if self_signed {
                    "CN=mail.example.test"
                } else {
                    "C=US, O=Let's Encrypt, CN=R11"
                }
                .into(),
            ),
            not_after: Some(stamp(Utc::now() + ChronoDuration::days(days))),
            days_left: days,
            self_signed,
        }
    }

    #[test]
    fn plan_says_why_and_never_drops_names() {
        let now = Utc::now();
        let st = AcmeState::default();
        let host = "mail.example.test";
        let wanted = vec![host.to_string(), "autoconfig.example.test".to_string()];
        let p = plan_for(host, &wanted, None, acme::LE_PRODUCTION, &st, now);
        assert_eq!(p.reason.as_deref(), Some("there is no certificate yet"));
        assert_eq!(p.names, wanted);

        let ss = info(&[host], 3650, true);
        assert!(
            plan_for(host, &wanted, Some(&ss), acme::LE_PRODUCTION, &st, now)
                .reason
                .unwrap()
                .contains("self-signed")
        );

        let fine = info(&[host, "autoconfig.example.test"], 60, false);
        let p = plan_for(
            host,
            &[host.to_string()],
            Some(&fine),
            acme::LE_PRODUCTION,
            &st,
            now,
        );
        assert_eq!(p.reason, None);
        assert_eq!(p.names.len(), 2, "keeps covered names");

        let expiring = info(&[host], 20, false);
        assert!(
            plan_for(
                host,
                &[host.to_string()],
                Some(&expiring),
                acme::LE_PRODUCTION,
                &st,
                now
            )
            .reason
            .unwrap()
            .contains("20 days")
        );

        let missing_helper = info(&[host], 60, false);
        let p = plan_for(
            host,
            &wanted,
            Some(&missing_helper),
            acme::LE_PRODUCTION,
            &st,
            now,
        );
        assert!(
            p.reason
                .unwrap()
                .contains("autoconfig.example.test now points here")
        );

        // …but not twice a day, and not for excluded names.
        let mut recent = AcmeState {
            coverage_reissue: Some(stamp(now)),
            ..Default::default()
        };
        assert_eq!(
            plan_for(
                host,
                &wanted,
                Some(&missing_helper),
                acme::LE_PRODUCTION,
                &recent,
                now
            )
            .reason,
            None
        );
        recent.coverage_reissue = None;
        recent.excluded.insert(
            "autoconfig.example.test".into(),
            stamp(now + ChronoDuration::hours(1)),
        );
        assert_eq!(
            plan_for(
                host,
                &wanted,
                Some(&missing_helper),
                acme::LE_PRODUCTION,
                &recent,
                now
            )
            .reason,
            None
        );
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(1).num_seconds(), 30);
        assert_eq!(backoff(4).num_seconds(), 300);
        assert_eq!(backoff(99).num_seconds(), 43_200);
    }

    #[test]
    fn cloudflare_ranges() {
        assert!(is_cloudflare("104.21.3.4"));
        assert!(is_cloudflare("172.67.1.1"));
        assert!(!is_cloudflare("203.0.113.10"));
        assert!(!is_cloudflare("2606:4700::1"));
    }

    #[tokio::test]
    async fn precheck_names_the_dns_mistake() {
        let host = "mail.example.com";
        let mut d = FakeDns::default();
        let p = precheck(host, Some(IP), None, &d, Some("letsencrypt.org"))
            .await
            .unwrap_err();
        assert_eq!(p.kind, ProblemKind::DnsMissing);
        assert!(p.remedy.contains(IP));

        d.a.insert(host.into(), vec!["104.21.9.9".into()]);
        let p = precheck(host, Some(IP), None, &d, None).await.unwrap_err();
        assert_eq!(p.kind, ProblemKind::CloudflareProxy);

        d.a.insert(host.into(), vec!["198.51.100.1".into()]);
        assert_eq!(
            precheck(host, Some(IP), None, &d, None)
                .await
                .unwrap_err()
                .kind,
            ProblemKind::DnsElsewhere
        );

        d.a.insert(host.into(), vec![IP.into()]);
        d.aaaa.insert(host.into(), vec!["2001:db8::1".into()]);
        assert_eq!(
            precheck(host, Some(IP), None, &d, None)
                .await
                .unwrap_err()
                .kind,
            ProblemKind::Ipv6Elsewhere
        );
        assert!(
            precheck(host, Some(IP), Some("2001:db8::1"), &d, None)
                .await
                .is_ok()
        );

        d.aaaa.clear();
        d.caa
            .insert("example.com".into(), vec!["0 issue \"sectigo.com\"".into()]);
        let p = precheck(host, Some(IP), None, &d, Some("letsencrypt.org"))
            .await
            .unwrap_err();
        assert_eq!(p.kind, ProblemKind::Caa);
        assert!(
            p.remedy
                .contains("example.com  CAA  0 issue \"letsencrypt.org\"")
        );

        d.caa.insert(
            "example.com".into(),
            vec![
                "0 issue \"sectigo.com\"".into(),
                "0 issue \"letsencrypt.org; validationmethods=http-01\"".into(),
            ],
        );
        assert!(
            precheck(host, Some(IP), None, &d, Some("letsencrypt.org"))
                .await
                .is_ok()
        );
        // The closest name decides.
        d.caa
            .insert(host.into(), vec!["0 iodef \"mailto:x@example.com\"".into()]);
        assert!(caa_blocks(host, "letsencrypt.org", &d).await.is_none());
    }

    /// An issuer that answers from a script and records what it was asked.
    struct Scripted {
        answers: Mutex<Vec<Result<(), Failure>>>,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl Scripted {
        fn new(answers: Vec<Result<(), Failure>>) -> Self {
            Self {
                answers: Mutex::new(answers),
                calls: Mutex::new(vec![]),
            }
        }
        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Issuer for Scripted {
        async fn issue(&self, directory: &str, names: &[String]) -> Result<Issued, Failure> {
            self.calls
                .lock()
                .unwrap()
                .push((directory.to_string(), names.to_vec()));
            let next = self.answers.lock().unwrap().remove(0);
            next.map(|_| ca_signed(names))
        }
    }

    /// A leaf signed by a throwaway CA, so it reads as CA-issued (not self-signed).
    fn ca_signed(names: &[String]) -> Issued {
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Yunori Test CA");
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(names.to_vec())
            .unwrap()
            .signed_by(&key, &ca, &ca_key)
            .unwrap();
        Issued {
            chain_pem: cert.pem(),
            key_pem: key.serialize_pem(),
        }
    }

    fn test_app() -> std::sync::Arc<App> {
        let app = App::for_tests_isolated();
        let mut cfg = app.config().unwrap();
        cfg.public_ipv4 = Some(IP.into());
        cfg.public_ipv6 = None;
        app.save_config(&cfg).unwrap();
        app
    }

    fn dns_ok(app: &App) -> FakeDns {
        let cfg = app.config().unwrap();
        let mut d = FakeDns::default();
        d.a.insert(cfg.hostname.clone(), vec![IP.into()]);
        d
    }

    fn unreachable_failure(name: &str) -> Failure {
        Failure {
            problem: Box::new(acme::classify(
                Some("urn:ietf:params:acme:error:connection"),
                Some("Timeout during connect (likely firewall problem)"),
                Some(name),
                "Let's Encrypt",
            )),
            failed_names: vec![name.to_string()],
        }
    }

    #[tokio::test]
    async fn failure_backs_off_then_rehearses_on_staging() {
        let app = test_app();
        let host = app.config().unwrap().hostname;
        let dns = dns_ok(&app);

        // First attempt fails at the CA: the reason is kept, a retry is scheduled.
        let issuer = Scripted::new(vec![Err(unreachable_failure(&host))]);
        let deps = Deps {
            dns: &dns,
            issuer: &issuer,
            self_test_port: None,
        };
        let out = ensure_with(&app, false, &deps).await;
        let Outcome::Failed { problem, .. } = out else {
            panic!("{out:?}")
        };
        assert_eq!(problem.kind, ProblemKind::Unreachable);
        assert_eq!(issuer.calls()[0].0, acme::LE_PRODUCTION);
        let st = load_state(&app);
        assert_eq!(st.failures, 1);
        assert!(st.next_attempt.is_some());

        // Inside the backoff window nothing is attempted…
        let idle = Scripted::new(vec![]);
        let deps = Deps {
            dns: &dns,
            issuer: &idle,
            self_test_port: None,
        };
        assert!(matches!(
            ensure_with(&app, false, &deps).await,
            Outcome::Later { .. }
        ));
        assert!(idle.calls().is_empty());

        // …unless forced; then staging is rehearsed first, and production follows.
        let issuer = Scripted::new(vec![Ok(()), Ok(())]);
        let deps = Deps {
            dns: &dns,
            issuer: &issuer,
            self_test_port: None,
        };
        let out = ensure_with(&app, true, &deps).await;
        assert_eq!(out, Outcome::Issued(vec![host.clone()]));
        let calls = issuer.calls();
        assert_eq!(calls[0].0, acme::LE_STAGING);
        assert_eq!(calls[1].0, acme::LE_PRODUCTION);
        let st = load_state(&app);
        assert_eq!(st.failures, 0);
        assert!(st.problem.is_none());
        let disk = tls::disk_certificate_info(&app.paths).unwrap();
        assert!(!disk.self_signed);
        assert_eq!(status(&app).state, "valid");

        // Now it is current: no further orders.
        let idle = Scripted::new(vec![]);
        let deps = Deps {
            dns: &dns,
            issuer: &idle,
            self_test_port: None,
        };
        assert_eq!(ensure_with(&app, false, &deps).await, Outcome::Current);
        assert!(idle.calls().is_empty());
    }

    #[test]
    fn rehearsal_failures_name_the_real_ca() {
        let p = acme::classify(
            Some("urn:ietf:params:acme:error:connection"),
            Some("Timeout during connect (likely firewall problem)"),
            Some("mail.example.com"),
            &acme::ca_name(acme::LE_STAGING),
        );
        let p = as_rehearsal(p);
        assert!(
            p.summary.starts_with("Let's Encrypt could not connect"),
            "{}",
            p.summary
        );
        assert!(p.detail.unwrap().contains("staging"));
    }

    #[tokio::test]
    async fn a_broken_staging_ca_never_blocks_production() {
        let app = test_app();
        let host = app.config().unwrap().hostname;
        let dns = dns_ok(&app);
        let staging_broken = || {
            Failure::from(CertProblem::new(
                ProblemKind::CaUnreachable,
                None,
                "Could not reach Let's Encrypt (staging).",
                "Yunori tries again by itself.",
            ))
        };
        // An earlier failure, long enough ago.
        let st = AcmeState {
            failures: 2,
            last_production_attempt: Some(stamp(Utc::now() - ChronoDuration::minutes(30))),
            ..Default::default()
        };
        save_state(&app, &st);
        let issuer = Scripted::new(vec![Err(staging_broken()), Ok(())]);
        let deps = Deps {
            dns: &dns,
            issuer: &issuer,
            self_test_port: None,
        };
        assert_eq!(
            ensure_with(&app, true, &deps).await,
            Outcome::Issued(vec![host.clone()])
        );
        let calls = issuer.calls();
        assert_eq!(calls[0].0, acme::LE_STAGING);
        assert_eq!(calls[1].0, acme::LE_PRODUCTION);

        // Without a rehearsal, production is not asked again within 12 minutes.
        std::fs::remove_file(tls::cert_path(&app.paths)).unwrap();
        let st = AcmeState {
            failures: 3,
            last_production_attempt: Some(stamp(Utc::now() - ChronoDuration::minutes(2))),
            ..Default::default()
        };
        save_state(&app, &st);
        let issuer = Scripted::new(vec![Err(staging_broken())]);
        let deps = Deps {
            dns: &dns,
            issuer: &issuer,
            self_test_port: None,
        };
        let out = ensure_with(&app, true, &deps).await;
        assert!(matches!(out, Outcome::Later { .. }), "{out:?}");
        assert_eq!(issuer.calls().len(), 1, "staging only");
    }

    #[tokio::test]
    async fn dns_problems_never_reach_the_ca() {
        let app = test_app();
        let issuer = Scripted::new(vec![]);
        let dns = FakeDns::default(); // no A record
        let deps = Deps {
            dns: &dns,
            issuer: &issuer,
            self_test_port: None,
        };
        let out = ensure_with(&app, true, &deps).await;
        let Outcome::Failed { problem, .. } = out else {
            panic!("{out:?}")
        };
        assert_eq!(problem.kind, ProblemKind::DnsMissing);
        assert!(issuer.calls().is_empty());
    }

    #[tokio::test]
    async fn a_failing_helper_name_does_not_block_the_mail_hostname() {
        let app = test_app();
        let cfg = app.config().unwrap();
        let helper = format!("autoconfig.{}", cfg.primary_domain);
        let mut dns = dns_ok(&app);
        dns.a.insert(helper.clone(), vec![IP.into()]);
        let issuer = Scripted::new(vec![Err(unreachable_failure(&helper)), Ok(())]);
        let deps = Deps {
            dns: &dns,
            issuer: &issuer,
            self_test_port: None,
        };
        let out = ensure_with(&app, true, &deps).await;
        assert_eq!(out, Outcome::Issued(vec![cfg.hostname.clone()]));
        let calls = issuer.calls();
        assert_eq!(calls[0].1, vec![cfg.hostname.clone(), helper.clone()]);
        assert_eq!(calls[1].1, vec![cfg.hostname.clone()]);
        assert!(load_state(&app).excluded.contains_key(&helper));
    }

    #[tokio::test]
    async fn never_reissues_just_to_duplicate_a_healthy_certificate() {
        let app = test_app();
        let cfg = app.config().unwrap();
        let helper = format!("autoconfig.{}", cfg.primary_domain);
        let issued = ca_signed(std::slice::from_ref(&cfg.hostname));
        tls::write_cert_files(&app.paths, &issued.chain_pem, &issued.key_pem).unwrap();

        // The helper now points here but has a stale AAAA: it is left out, and since
        // nothing else would change, no order is placed.
        let mut dns = dns_ok(&app);
        dns.a.insert(helper.clone(), vec![IP.into()]);
        dns.aaaa.insert(helper.clone(), vec!["2001:db8::99".into()]);
        let idle = Scripted::new(vec![]);
        let deps = Deps {
            dns: &dns,
            issuer: &idle,
            self_test_port: None,
        };
        assert_eq!(ensure_with(&app, false, &deps).await, Outcome::Current);
        assert!(idle.calls().is_empty());
        let st = load_state(&app);
        assert!(st.excluded.contains_key(&helper));
        assert!(st.coverage_checked.is_some());

        // Within the hour, a healthy certificate costs no DNS at all.
        let no_dns = FakeDns {
            failing: vec![cfg.hostname.clone(), helper.clone()],
            ..Default::default()
        };
        let deps = Deps {
            dns: &no_dns,
            issuer: &idle,
            self_test_port: None,
        };
        assert_eq!(ensure_with(&app, false, &deps).await, Outcome::Current);
    }

    #[test]
    fn one_order_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(dir.path());
        let first = Lock::try_acquire(&paths).unwrap();
        assert!(Lock::try_acquire(&paths).is_none());
        drop(first);
        assert!(Lock::try_acquire(&paths).is_some());
    }
}
