//! The record set Yunori asks users to publish, and its evaluation against
//! public DNS (expected vs actual, plain-language status per record).
//!
//! Source of truth for mail records is Stalwart's own zone data; Yunori then
//! applies its policy on top: DMARC follows the ladder (starts at `p=none`,
//! Stalwart defaults to `reject`), SPF gains the relay's `include:` in relay
//! mode, and the primary domain carries the host rows (A/AAAA/PTR).

use super::zone::{self, ZoneRecord};
use super::{DnsLookup, Srv, norm_host};
use crate::model::{DmarcPolicy, DnsRecord, Status};
use std::net::IpAddr;

#[derive(Debug, Clone)]
pub struct Expected {
    pub id: String,
    pub rtype: String,
    pub name: String,
    pub value: String,
    pub required: bool,
    pub purpose: String,
}

#[derive(Debug, Clone)]
pub struct RecordContext {
    pub hostname: String,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub is_primary: bool,
    pub relay_spf_include: Option<String>,
    /// The relay provider's own records for this domain (`relay::domain_records`).
    pub relay_records: Vec<Expected>,
    pub dmarc_policy: DmarcPolicy,
}

fn exp(
    id: &str,
    rtype: &str,
    name: &str,
    value: String,
    required: bool,
    purpose: &str,
) -> Expected {
    Expected {
        id: id.into(),
        rtype: rtype.into(),
        name: name.into(),
        value,
        required,
        purpose: purpose.into(),
    }
}

fn adjust_spf(value: &str, include: Option<&str>) -> String {
    let mut terms: Vec<String> = value.split_whitespace().map(str::to_string).collect();
    if let Some(inc) = include {
        let term = format!("include:{inc}");
        if !terms.contains(&term) {
            let pos = terms
                .iter()
                .position(|t| t.ends_with("all"))
                .unwrap_or(terms.len());
            terms.insert(pos, term);
        }
    }
    terms.join(" ")
}

pub(crate) fn adjust_dmarc(value: &str, policy: DmarcPolicy) -> String {
    let mut parts: Vec<String> = value
        .split(';')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let mut found = false;
    for p in parts.iter_mut() {
        if p.to_ascii_lowercase().starts_with("p=") {
            *p = format!("p={}", policy.as_str());
            found = true;
        }
    }
    if !found {
        parts.insert(1.min(parts.len()), format!("p={}", policy.as_str()));
    }
    parts.join("; ")
}

/// Build the expected record set for one domain.
pub fn expected_records(domain: &str, zone_file: &str, ctx: &RecordContext) -> Vec<Expected> {
    let domain = norm_host(domain);
    let mut out = Vec::new();

    if ctx.is_primary {
        if let Some(ip) = &ctx.ipv4 {
            out.push(exp(
                "a",
                "A",
                &ctx.hostname,
                ip.clone(),
                true,
                "Points your mail server's name at this machine. Everything else builds on it.",
            ));
        }
        if let Some(ip) = &ctx.ipv6 {
            if ctx.ipv4.is_none() {
                out.push(exp(
                    "aaaa",
                    "AAAA",
                    &ctx.hostname,
                    ip.clone(),
                    true,
                    "Points your mail server's name at this machine (it has only an IPv6 address).",
                ));
            } else {
                out.push(exp("aaaa", "AAAA", &ctx.hostname, ip.clone(), false,
                    "IPv6 address of the mail server. Only publish it if you can also set IPv6 reverse DNS."));
            }
        }
    }

    let recs: Vec<ZoneRecord> = zone::parse(zone_file, &domain);
    let mut saw_dkim = false;
    for r in &recs {
        let name = r.name.as_str();
        match r.rtype.as_str() {
            "MX" if name == domain => out.push(exp("mx", "MX", name, r.value.clone(), true,
                "Tells other mail servers to deliver mail for this domain to your server.")),
            "TXT" if name == domain && r.value.to_ascii_lowercase().starts_with("v=spf1") => out.push(exp(
                "spf", "TXT", name, adjust_spf(&r.value, ctx.relay_spf_include.as_deref()), true,
                "SPF: lists which servers may send mail as this domain.")),
            "TXT" if name == norm_host(&ctx.hostname) && r.value.to_ascii_lowercase().starts_with("v=spf1") => {
                // SPF for the HELO name (checked on bounces). One row, on the primary domain.
                if ctx.is_primary {
                    out.push(exp("spf-host", "TXT", name, r.value.clone(), false,
                        "SPF for the mail server's own name, used when receivers check the HELO greeting and bounces."));
                }
            }
            "TXT" if name.contains("._domainkey.") => {
                saw_dkim = true;
                let selector = name.split("._domainkey.").next().unwrap_or("dkim");
                out.push(exp(&format!("dkim:{selector}"), "TXT", name, r.value.clone(), true,
                    "DKIM: the public key receivers use to check your server's signature."));
            }
            "TXT" if name == format!("_dmarc.{domain}") => out.push(exp("dmarc", "TXT", name,
                adjust_dmarc(&r.value, ctx.dmarc_policy), true,
                "DMARC: tells receivers what to do with mail that fails SPF/DKIM, and where to send reports.")),
            "TXT" if name == format!("_mta-sts.{domain}") => out.push(exp("mta-sts", "TXT", name, r.value.clone(), false,
                "MTA-STS: asks senders to always use TLS when delivering to you.")),
            "TXT" if name == format!("_smtp._tls.{domain}") => out.push(exp("tls-rpt", "TXT", name, r.value.clone(), false,
                "TLS-RPT: where senders report TLS problems reaching your server.")),
            // Yunori does not open POP3, so never advertise it.
            "SRV" if name.starts_with("_pop3") => {}
            "SRV" => {
                let svc = name.strip_suffix(&format!(".{domain}")).unwrap_or(name);
                out.push(exp(&format!("srv:{svc}"), "SRV", name, r.value.clone(), false,
                    "Lets mail apps (and the Yunori client) find your server automatically."));
            }
            "CNAME" => {
                let label = name.strip_suffix(&format!(".{domain}")).unwrap_or(name);
                let purpose = match label {
                    "autoconfig" => "Thunderbird-style auto-setup for mail apps, including the Yunori client.",
                    "autodiscover" => "Outlook-style auto-setup for mail apps.",
                    "mta-sts" => "Hosts the MTA-STS policy file.",
                    _ => "Helper name for mail clients.",
                };
                // The CNAME `mta-sts.<d>` and the TXT `_mta-sts.<d>` need distinct ids.
                let id = if label == "mta-sts" { "mta-sts-host" } else { label };
                out.push(exp(id, "CNAME", name, r.value.clone(), false, purpose));
            }
            "CAA" => out.push(exp("caa", "CAA", name, r.value.clone(), false,
                "Restricts which certificate authorities may issue certificates for this domain.")),
            // DANE/TLSA is deliberately NOT offered yet: Yunori's ACME renewals
            // rotate the key, so a published TLSA (under DNSSEC) would break
            // inbound mail at the next renewal. Re-enable with key reuse (plan P3).
            "TLSA" => {}
            // Any other record the engine publishes (e.g. `_ua-auto-config`): optional.
            "TXT" => {
                let label = name.strip_suffix(&format!(".{domain}")).unwrap_or(name);
                out.push(exp(&format!("txt:{label}"), "TXT", name, r.value.clone(), false,
                    "Published by the mail engine to help mail apps verify their automatic setup."));
            }
            _ => {}
        }
    }

    if !saw_dkim {
        out.push(exp(
            "dkim",
            "TXT",
            &format!("*._domainkey.{domain}"),
            String::new(),
            true,
            "DKIM: the public key receivers use to check your server's signature.",
        ));
    }
    out.extend(ctx.relay_records.iter().cloned());
    out
}

/// Expected records as a zone file for "copy all".
pub fn render_zone(records: &[Expected]) -> String {
    records
        .iter()
        .filter(|r| !r.value.is_empty() && !r.name.starts_with('*'))
        .map(|r| zone::render(&r.name, &r.rtype, &r.value))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn squash(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub(crate) fn dkim_key(v: &str) -> Option<String> {
    v.split(';').map(str::trim).find_map(|p| {
        p.strip_prefix("p=")
            .map(|k| k.replace(char::is_whitespace, ""))
    })
}

fn record(e: &Expected, actual: Vec<String>, status: Status, remedy: Option<String>) -> DnsRecord {
    DnsRecord {
        id: e.id.clone(),
        rtype: e.rtype.clone(),
        name: e.name.clone(),
        expected: e.value.clone(),
        actual,
        status,
        required: e.required,
        purpose: e.purpose.clone(),
        remedy,
    }
}

fn article(rtype: &str) -> &'static str {
    if matches!(rtype, "A" | "AAAA" | "MX" | "SRV") {
        "an"
    } else {
        "a"
    }
}

fn missing(e: &Expected) -> (Status, Option<String>) {
    if e.required {
        (
            Status::Fail,
            Some(format!(
                "Add {} {} record named {} with the value shown.",
                article(&e.rtype),
                e.rtype,
                e.name
            )),
        )
    } else {
        (
            Status::Warn,
            Some(format!(
                "Optional — add {} {} record named {} for the best experience.",
                article(&e.rtype),
                e.rtype,
                e.name
            )),
        )
    }
}

async fn evaluate_one(e: &Expected, dns: &dyn DnsLookup, ctx: &RecordContext) -> DnsRecord {
    macro_rules! lookup {
        ($fut:expr) => {
            match $fut.await {
                Ok(v) => v,
                Err(err) => {
                    return record(
                        e,
                        vec![],
                        Status::Unknown,
                        Some(format!(
                            "Could not query public DNS ({err}). Try again in a moment."
                        )),
                    );
                }
            }
        };
    }

    match e.rtype.as_str() {
        "A" | "AAAA" => {
            let actual = if e.rtype == "A" {
                lookup!(dns.a(&e.name))
            } else {
                lookup!(dns.aaaa(&e.name))
            };
            if actual.is_empty() {
                let (s, r) = missing(e);
                record(e, actual, s, r)
            } else if actual.iter().any(|a| a == &e.value) {
                let extra = actual.len() > 1;
                record(
                    e,
                    actual,
                    if extra { Status::Warn } else { Status::Pass },
                    extra.then(|| {
                        "Remove the other addresses — they point mail somewhere else.".to_string()
                    }),
                )
            } else {
                record(
                    e,
                    actual,
                    Status::Fail,
                    Some(format!("Change the record to {} (this server).", e.value)),
                )
            }
        }
        "MX" => {
            let actual = lookup!(dns.mx(&e.name));
            let want_host = e
                .value
                .split_whitespace()
                .last()
                .map(norm_host)
                .unwrap_or_default();
            let shown: Vec<String> = actual.iter().map(|(p, h)| format!("{p} {h}")).collect();
            if actual.is_empty() {
                let (s, r) = missing(e);
                record(e, shown, s, r)
            } else if actual.iter().any(|(_, h)| *h == want_host) {
                let others = actual.iter().any(|(_, h)| *h != want_host);
                record(
                    e,
                    shown,
                    if others { Status::Warn } else { Status::Pass },
                    others.then(|| {
                        "Other MX hosts are listed too; mail may be delivered to them instead."
                            .to_string()
                    }),
                )
            } else {
                record(
                    e,
                    shown,
                    Status::Fail,
                    Some(format!(
                        "Mail for this domain currently goes elsewhere. Point the MX record at {want_host}."
                    )),
                )
            }
        }
        "TXT" if e.id == "spf" => {
            let actual: Vec<String> = lookup!(dns.txt(&e.name))
                .into_iter()
                .filter(|t| t.to_ascii_lowercase().starts_with("v=spf1"))
                .collect();
            if actual.is_empty() {
                let (s, r) = missing(e);
                record(e, actual, s, r)
            } else if actual.len() > 1 {
                record(e, actual, Status::Fail,
                    Some("There are several SPF records; receivers then ignore all of them. Merge them into one.".into()))
            } else if squash(&actual[0]) == squash(&e.value) {
                record(e, actual, Status::Pass, None)
            } else {
                let a = squash(&actual[0]);
                let authorises = a.contains(" mx")
                    || ctx
                        .ipv4
                        .as_ref()
                        .is_some_and(|ip| a.contains(&format!("ip4:{ip}")));
                let has_include = ctx
                    .relay_spf_include
                    .as_ref()
                    .is_none_or(|i| a.contains(&format!("include:{i}")));
                if authorises && has_include {
                    record(e, actual, Status::Pass, None)
                } else {
                    record(e, actual, Status::Warn,
                        Some("Your SPF record exists but may not authorise this server. Replace it with the value shown.".into()))
                }
            }
        }
        "TXT" if e.id == "dmarc" => {
            let actual: Vec<String> = lookup!(dns.txt(&e.name))
                .into_iter()
                .filter(|t| t.to_ascii_lowercase().starts_with("v=dmarc1"))
                .collect();
            match actual.len() {
                0 => {
                    let (s, r) = missing(e);
                    record(e, actual, s, r)
                }
                1 => record(e, actual, Status::Pass, None),
                _ => record(
                    e,
                    actual,
                    Status::Fail,
                    Some("Publish exactly one DMARC record.".into()),
                ),
            }
        }
        "TXT" if e.id.starts_with("dkim") || e.id == "relay:dkim" => {
            if e.value.is_empty() {
                return record(
                    e,
                    vec![],
                    Status::Unknown,
                    Some(
                        "The mail engine is still generating DKIM keys. Check again in a minute."
                            .into(),
                    ),
                );
            }
            let actual = lookup!(dns.txt(&e.name));
            let want = dkim_key(&e.value);
            if actual.is_empty() {
                let (s, r) = missing(e);
                record(e, actual, s, r)
            } else if actual.iter().any(|a| dkim_key(a) == want) {
                record(e, actual, Status::Pass, None)
            } else {
                record(e, actual, Status::Fail,
                    Some("The published key differs from this server's key — often a record split or truncated by the DNS provider. Paste the value again.".into()))
            }
        }
        "TXT" => {
            let actual = lookup!(dns.txt(&e.name));
            if actual.is_empty() {
                let (s, r) = missing(e);
                record(e, actual, s, r)
            } else if actual.iter().any(|a| squash(a) == squash(&e.value)) {
                record(e, actual, Status::Pass, None)
            } else {
                record(
                    e,
                    actual,
                    Status::Warn,
                    Some("A record exists but its value differs from the one shown.".into()),
                )
            }
        }
        "CNAME" => {
            let actual = lookup!(dns.cname(&e.name));
            let want = norm_host(&e.value);
            if actual.contains(&want) {
                return record(e, actual, Status::Pass, None);
            }
            // A provider's CNAME must be exactly that CNAME.
            if e.id.starts_with("relay:") {
                return if actual.is_empty() {
                    let (s, r) = missing(e);
                    record(e, actual, s, r)
                } else {
                    record(
                        e,
                        actual,
                        Status::Fail,
                        Some(format!("Point this name at {want}.")),
                    )
                };
            }
            // An A record pointing at this server is just as good.
            let a = dns.a(&e.name).await.unwrap_or_default();
            if ctx.ipv4.as_ref().is_some_and(|ip| a.contains(ip)) {
                return record(e, a, Status::Pass, None);
            }
            if actual.is_empty() && a.is_empty() {
                let (s, r) = missing(e);
                record(e, actual, s, r)
            } else {
                let mut shown = actual;
                shown.extend(a);
                record(
                    e,
                    shown,
                    Status::Warn,
                    Some(format!("Point this name at {want}.")),
                )
            }
        }
        "SRV" => {
            let actual: Vec<Srv> = lookup!(dns.srv(&e.name));
            let parts: Vec<&str> = e.value.split_whitespace().collect();
            let (port, target) = (
                parts.get(2).and_then(|p| p.parse::<u16>().ok()),
                parts.get(3).map(|t| norm_host(t)),
            );
            let shown: Vec<String> = actual
                .iter()
                .map(|s| format!("{} {} {} {}", s.priority, s.weight, s.port, s.target))
                .collect();
            if actual.is_empty() {
                let (s, r) = missing(e);
                record(e, shown, s, r)
            } else if actual
                .iter()
                .any(|s| Some(s.port) == port && Some(&s.target) == target.as_ref())
            {
                record(e, shown, Status::Pass, None)
            } else {
                record(
                    e,
                    shown,
                    Status::Warn,
                    Some("The SRV record points somewhere else.".into()),
                )
            }
        }
        _ => record(
            e,
            vec![],
            Status::Info,
            Some("Yunori does not verify this record type yet.".into()),
        ),
    }
}

pub async fn evaluate(
    expected: &[Expected],
    dns: &dyn DnsLookup,
    ctx: &RecordContext,
) -> Vec<DnsRecord> {
    let futs = expected.iter().map(|e| evaluate_one(e, dns, ctx));
    futures_util::future::join_all(futs).await
}

/// Reverse DNS for the server's IP, forward-confirmed (FCrDNS).
pub async fn evaluate_ptr(
    ip: &str,
    hostname: &str,
    dns: &dyn DnsLookup,
    relayed: bool,
) -> DnsRecord {
    let e = Expected {
        id: "ptr".into(),
        rtype: "PTR".into(),
        name: ip.into(),
        value: hostname.into(),
        required: !relayed,
        purpose: "Reverse DNS: proves the server's IP belongs to your mail hostname. Gmail and Outlook require it for direct sending.".into(),
    };
    let Ok(addr) = ip.parse::<IpAddr>() else {
        return record(&e, vec![], Status::Unknown, None);
    };
    let names = match dns.ptr(addr).await {
        Ok(n) => n,
        Err(err) => {
            return record(
                &e,
                vec![],
                Status::Unknown,
                Some(format!("Could not look up reverse DNS ({err}).")),
            );
        }
    };
    let want = norm_host(hostname);
    let fix = "Set it in your VPS provider's control panel (not at your DNS provider) — look for \"Reverse DNS\" or \"PTR\".";
    if names.is_empty() {
        let s = if relayed { Status::Warn } else { Status::Fail };
        return record(
            &e,
            names,
            s,
            Some(format!("No reverse DNS is set for {ip}. {fix}")),
        );
    }
    if !names.contains(&want) {
        let s = if relayed { Status::Warn } else { Status::Fail };
        return record(
            &e,
            names,
            s,
            Some(format!("Reverse DNS should be {want}. {fix}")),
        );
    }
    // Forward-confirm: the hostname must resolve back to the same IP.
    let forward = if addr.is_ipv4() {
        dns.a(&want).await
    } else {
        dns.aaaa(&want).await
    }
    .unwrap_or_default();
    if forward.iter().any(|f| f == ip) {
        record(&e, names, Status::Pass, None)
    } else {
        record(
            &e,
            names,
            Status::Warn,
            Some(format!(
                "{want} does not resolve back to {ip}; fix its A/AAAA record."
            )),
        )
    }
}

pub fn worst_required(records: &[DnsRecord]) -> Status {
    records
        .iter()
        .filter(|r| r.required)
        .map(|r| r.status)
        .fold(Status::Pass, Status::worst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::fake::FakeDns;

    const ZONE: &str = "example.com. IN MX 10 mail.example.com.\n\
        example.com. IN TXT \"v=spf1 mx ra=postmaster -all\"\n\
        v1-ed25519-20261105._domainkey.example.com. IN TXT \"v=DKIM1; k=ed25519; h=sha256; p=ABCDEF\"\n\
        _dmarc.example.com. IN TXT \"v=DMARC1; p=reject; rua=mailto:postmaster@example.com\"\n\
        autoconfig.example.com. IN CNAME mail.example.com.\n";

    fn ctx() -> RecordContext {
        RecordContext {
            hostname: "mail.example.com".into(),
            ipv4: Some("203.0.113.10".into()),
            ipv6: None,
            is_primary: true,
            relay_spf_include: None,
            relay_records: vec![],
            dmarc_policy: DmarcPolicy::None,
        }
    }

    #[test]
    fn applies_dmarc_ladder_and_relay_include() {
        let mut c = ctx();
        c.relay_spf_include = Some("amazonses.com".into());
        let recs = expected_records("example.com", ZONE, &c);
        let dmarc = recs.iter().find(|r| r.id == "dmarc").unwrap();
        assert_eq!(
            dmarc.value,
            "v=DMARC1; p=none; rua=mailto:postmaster@example.com"
        );
        let spf = recs.iter().find(|r| r.id == "spf").unwrap();
        assert_eq!(
            spf.value,
            "v=spf1 mx ra=postmaster include:amazonses.com -all"
        );
        assert!(
            recs.iter()
                .any(|r| r.id == "a" && r.value == "203.0.113.10")
        );
        assert!(
            recs.iter()
                .any(|r| r.id == "dkim:v1-ed25519-20261105" && r.required)
        );
    }

    #[tokio::test]
    async fn evaluates_expected_vs_actual() {
        let recs = expected_records("example.com", ZONE, &ctx());
        let mut dns = FakeDns::default();
        dns.a
            .insert("mail.example.com".into(), vec!["203.0.113.10".into()]);
        dns.mx
            .insert("example.com".into(), vec![(10, "mail.example.com".into())]);
        dns.txt.insert(
            "example.com".into(),
            vec!["v=spf1 mx -all".into(), "google-site-verification=x".into()],
        );
        dns.txt.insert(
            "v1-ed25519-20261105._domainkey.example.com".into(),
            vec!["v=DKIM1; k=ed25519; h=sha256; p=ABC DEF".into()],
        );
        let out = evaluate(&recs, &dns, &ctx()).await;
        let st = |id: &str| out.iter().find(|r| r.id == id).unwrap().status;
        assert_eq!(st("a"), Status::Pass);
        assert_eq!(st("mx"), Status::Pass);
        assert_eq!(
            st("spf"),
            Status::Pass,
            "an SPF that authorises mx is acceptable"
        );
        assert_eq!(
            st("dkim:v1-ed25519-20261105"),
            Status::Pass,
            "whitespace inside p= is tolerated"
        );
        assert_eq!(st("dmarc"), Status::Fail);
        assert_eq!(st("autoconfig"), Status::Warn, "optional and missing");
        assert_eq!(worst_required(&out), Status::Fail);
    }

    #[tokio::test]
    async fn duplicate_spf_and_dns_failure() {
        let recs = expected_records("example.com", ZONE, &ctx());
        let mut dns = FakeDns::default();
        dns.txt.insert(
            "example.com".into(),
            vec!["v=spf1 mx -all".into(), "v=spf1 include:x -all".into()],
        );
        dns.failing.push("example.com".into());
        let out = evaluate(&recs, &dns, &ctx()).await;
        assert_eq!(
            out.iter().find(|r| r.id == "spf").unwrap().status,
            Status::Unknown
        );
        dns.failing.clear();
        let out = evaluate(&recs, &dns, &ctx()).await;
        assert_eq!(
            out.iter().find(|r| r.id == "spf").unwrap().status,
            Status::Fail
        );
    }

    #[tokio::test]
    async fn relay_provider_rows_are_checked_strictly() {
        let t = "abcdefghijklmnopqrstuvwxyz012345";
        let key = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC7xYwHmS4fXh5qk9Jh2bU0Lw8mN3pQ1rT6vY";
        let mut c = ctx();
        c.relay_records = vec![
            Expected {
                id: "relay:dkim:1".into(),
                rtype: "CNAME".into(),
                name: format!("{t}._domainkey.example.com"),
                value: format!("{t}.dkim.amazonses.com"),
                required: true,
                purpose: String::new(),
            },
            Expected {
                id: "relay:dkim".into(),
                rtype: "TXT".into(),
                name: "resend._domainkey.example.com".into(),
                value: format!("p={key}"),
                required: true,
                purpose: String::new(),
            },
            Expected {
                id: "relay:mail-from-mx".into(),
                rtype: "MX".into(),
                name: "send.example.com".into(),
                value: "10 feedback-smtp.us-east-1.amazonses.com".into(),
                required: true,
                purpose: String::new(),
            },
        ];
        let recs = expected_records("example.com", ZONE, &c);
        assert!(
            recs.iter().any(|r| r.id == "relay:dkim:1"),
            "relay rows join the checklist"
        );
        assert!(render_zone(&recs).contains(
            "send.example.com.\t3600\tIN\tMX\t10 feedback-smtp.us-east-1.amazonses.com."
        ));

        let mut dns = FakeDns::default();
        // An A record at this server is NOT a substitute for a provider's CNAME.
        dns.a.insert(
            format!("{t}._domainkey.example.com"),
            vec!["203.0.113.10".into()],
        );
        dns.txt.insert(
            "resend._domainkey.example.com".into(),
            vec![format!("p={} {}", &key[..20], &key[20..])],
        );
        dns.mx.insert(
            "send.example.com".into(),
            vec![(10, "feedback-smtp.us-east-1.amazonses.com".into())],
        );
        let out = evaluate(&recs, &dns, &c).await;
        let st = |id: &str| out.iter().find(|r| r.id == id).unwrap().status;
        assert_eq!(st("relay:dkim:1"), Status::Fail);
        assert_eq!(
            st("relay:dkim"),
            Status::Pass,
            "provider DKIM compares keys, not text"
        );
        assert_eq!(st("relay:mail-from-mx"), Status::Pass);

        dns.cname.insert(
            format!("{t}._domainkey.example.com"),
            vec![format!("{t}.dkim.amazonses.com")],
        );
        let out = evaluate(&recs, &dns, &c).await;
        assert_eq!(
            out.iter().find(|r| r.id == "relay:dkim:1").unwrap().status,
            Status::Pass
        );
    }

    #[tokio::test]
    async fn ptr_requires_forward_confirmation() {
        let mut dns = FakeDns::default();
        dns.ptr
            .insert("203.0.113.10".into(), vec!["mail.example.com".into()]);
        let r = evaluate_ptr("203.0.113.10", "mail.example.com", &dns, false).await;
        assert_eq!(r.status, Status::Warn);
        dns.a
            .insert("mail.example.com".into(), vec!["203.0.113.10".into()]);
        let r = evaluate_ptr("203.0.113.10", "mail.example.com", &dns, false).await;
        assert_eq!(r.status, Status::Pass);
        let r = evaluate_ptr("198.51.100.1", "mail.example.com", &dns, true).await;
        assert_eq!(
            r.status,
            Status::Warn,
            "missing PTR only warns when sending is relayed"
        );
    }

    #[test]
    fn live_zone_maps_to_unique_ids_without_pop3() {
        let zone = "v1-rsa-1._domainkey.live.test. IN TXT (\n \"v=DKIM1; k=rsa; p=AB\"\n \"CD\"\n)\n\
            mail.live.test. IN TXT \"v=spf1 a -all\"\n\
            live.test. IN TXT \"v=spf1 mx -all\"\n\
            _pop3s._tcp.live.test. IN SRV 0 1 995 mail.live.test.\n\
            _imaps._tcp.live.test. IN SRV 0 1 993 mail.live.test.\n\
            mta-sts.live.test. IN CNAME mail.live.test.\n\
            _mta-sts.live.test. IN TXT \"v=STSv1; id=1\"\n\
            _ua-auto-config.live.test. IN TXT \"v=UAAC1; a=sha256; d=x\"\n";
        let mut c = ctx();
        c.hostname = "mail.live.test".into();
        let recs = expected_records("live.test", zone, &c);
        let ids: Vec<&str> = recs.iter().map(|r| r.id.as_str()).collect();
        let mut dedup = ids.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), ids.len(), "ids must be unique: {ids:?}");
        assert!(ids.contains(&"dkim:v1-rsa-1"));
        assert!(ids.contains(&"spf-host") && ids.contains(&"spf"));
        assert!(ids.contains(&"mta-sts") && ids.contains(&"mta-sts-host"));
        assert!(ids.contains(&"txt:_ua-auto-config"));
        assert!(!ids.iter().any(|i| i.contains("pop3")));
        assert_eq!(
            recs.iter().find(|r| r.id == "dkim:v1-rsa-1").unwrap().value,
            "v=DKIM1; k=rsa; p=ABCD"
        );
    }

    #[test]
    fn missing_dkim_is_flagged_pending() {
        let recs = expected_records(
            "example.com",
            "example.com. IN MX 10 mail.example.com.",
            &ctx(),
        );
        assert!(recs.iter().any(|r| r.id == "dkim" && r.value.is_empty()));
    }
}
