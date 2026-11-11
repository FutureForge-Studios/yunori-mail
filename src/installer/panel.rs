//! Where the console opens: the one thing the terminal asks.
//!
//! Either a domain name the person already pointed here (`mail.example.com`,
//! `panel.example.com`), or, with no domain yet, this server's address. For the
//! address Yunori prefers a zero-configuration name that resolves to it —
//! `203-0-113-10.sslip.io` — because a name can get a real Let's Encrypt
//! certificate and the browser shows no warning. The bare IP (with the
//! temporary self-signed certificate) is the fallback when that name does not
//! resolve here or the CA cannot reach port 80.
//!
//! Everything else (mail domain, hostname, sending, DNS, reverse DNS, the mail
//! certificate, the first mailbox) is asked in the browser.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// What the terminal decided (kv `install.panel`), applied by the `panel` step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelChoice {
    /// A domain name, an `<ip>.sslip.io` name, or a bare IP address.
    pub host: String,
    /// A domain the person typed that did not point here yet (the browser
    /// suggests it again as the mail hostname).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested: Option<String>,
}

pub const KV: &str = "install.panel";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelKind {
    /// A domain name the person owns.
    Domain,
    /// `<ip>.sslip.io`: resolves to this server with no DNS work.
    Sslip,
    /// The bare IP address (self-signed certificate).
    Ip,
}

pub const SSLIP_SUFFIX: &str = ".sslip.io";

pub fn kind(host: &str) -> PanelKind {
    if host.parse::<IpAddr>().is_ok() {
        PanelKind::Ip
    } else if host.ends_with(SSLIP_SUFFIX) {
        PanelKind::Sslip
    } else {
        PanelKind::Domain
    }
}

/// The sslip.io name of an address: dots become dashes for IPv4
/// (`203-0-113-10.sslip.io`); for IPv6 sslip.io wants dashes, never dots.
/// Every group is written out (`2a01-4f8-c17-b8f-0-0-0-2.sslip.io`, which
/// resolves like the `--` short form) so a label never starts or ends with a
/// dash, which a certificate authority would refuse.
pub fn sslip_name(ip: IpAddr) -> String {
    let label = match ip {
        IpAddr::V4(v4) => v4.to_string().replace('.', "-"),
        IpAddr::V6(v6) => v6
            .segments()
            .iter()
            .map(|s| format!("{s:x}"))
            .collect::<Vec<_>>()
            .join("-"),
    };
    format!("{label}{SSLIP_SUFFIX}")
}

/// `host` as it goes into a URL (IPv6 in brackets).
pub fn url_host(host: &str) -> String {
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V6(v6)) => format!("[{v6}]"),
        _ => host.to_string(),
    }
}

pub fn console_url(host: &str) -> String {
    format!("https://{}/console", url_host(host))
}

/// The mail domain a panel name suggests: `mail.example.com` → `example.com`.
/// Two-label names are domains themselves; sslip.io names and IPs suggest
/// nothing. (A suggestion only: the browser asks.)
pub fn suggested_domain(panel: &str) -> Option<String> {
    if kind(panel) != PanelKind::Domain {
        return None;
    }
    let labels: Vec<&str> = panel.split('.').collect();
    if labels.len() <= 2 {
        return Some(panel.to_string());
    }
    let parent = labels[1..].join(".");
    // `panel.co.uk` would suggest `co.uk`: keep the registrable-looking part.
    let second_level = ["co", "com", "net", "org", "ac", "gov", "edu", "ne", "or"];
    let p: Vec<&str> = parent.split('.').collect();
    if p.len() == 2 && second_level.contains(&p[0]) && p[1].len() == 2 {
        return Some(panel.to_string());
    }
    Some(parent)
}

/// The mail hostname to suggest for `domain`: the console's own name when it
/// is a `mail.` name of that domain (one name, one certificate), else
/// `mail.<domain>`.
pub fn suggested_hostname(domain: &str, panel: Option<&str>) -> String {
    match panel {
        Some(p) if p == format!("mail.{domain}") => p.to_string(),
        _ => format!("mail.{domain}"),
    }
}

/// Names the engine is first configured with, before the browser asks for the
/// real ones: Stalwart needs a hostname and a domain at first run, and refuses
/// a reserved `.invalid` domain (verified on 1.0.3). The onboarding renames
/// the domain and sets the hostname later, through the management API.
pub fn placeholders(host: &str, ip: Option<IpAddr>) -> (String, String) {
    match kind(host) {
        PanelKind::Domain => (
            host.to_string(),
            suggested_domain(host).unwrap_or_else(|| host.to_string()),
        ),
        PanelKind::Sslip => (host.to_string(), host.to_string()),
        PanelKind::Ip => {
            let ip = host.parse::<IpAddr>().ok().or(ip);
            let name = ip
                .map(sslip_name)
                .unwrap_or_else(|| format!("mail{SSLIP_SUFFIX}"));
            (name.clone(), name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sslip_names() {
        assert_eq!(
            sslip_name("203.0.113.10".parse().unwrap()),
            "203-0-113-10.sslip.io"
        );
        // Never a leading or trailing dash (a CA refuses such labels).
        assert_eq!(
            sslip_name("2a01:4f8:c17:b8f::2".parse().unwrap()),
            "2a01-4f8-c17-b8f-0-0-0-2.sslip.io"
        );
        assert_eq!(
            sslip_name("2a01:4ff:1f2:10d::".parse().unwrap()),
            "2a01-4ff-1f2-10d-0-0-0-0.sslip.io"
        );
        assert_eq!(kind("203-0-113-10.sslip.io"), PanelKind::Sslip);
        assert_eq!(kind("203.0.113.10"), PanelKind::Ip);
        assert_eq!(kind("2a01:4f8::2"), PanelKind::Ip);
        assert_eq!(kind("panel.example.com"), PanelKind::Domain);
        assert_eq!(url_host("2a01:4f8::2"), "[2a01:4f8::2]");
        assert_eq!(console_url("203.0.113.10"), "https://203.0.113.10/console");
    }

    #[test]
    fn suggestions_follow_the_panel_name() {
        assert_eq!(
            suggested_domain("mail.example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            suggested_domain("panel.example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            suggested_domain("example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            suggested_domain("mail.example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(
            suggested_domain("panel.co.uk").as_deref(),
            Some("panel.co.uk")
        );
        assert_eq!(suggested_domain("203-0-113-10.sslip.io"), None);
        assert_eq!(suggested_domain("203.0.113.10"), None);
        assert_eq!(
            suggested_hostname("example.com", Some("mail.example.com")),
            "mail.example.com"
        );
        assert_eq!(
            suggested_hostname("example.com", Some("panel.example.com")),
            "mail.example.com"
        );
    }

    #[test]
    fn placeholders_are_names_the_engine_accepts() {
        assert_eq!(
            placeholders("panel.example.com", None),
            ("panel.example.com".into(), "example.com".into())
        );
        assert_eq!(
            placeholders("203-0-113-10.sslip.io", None),
            (
                "203-0-113-10.sslip.io".into(),
                "203-0-113-10.sslip.io".into()
            )
        );
        assert_eq!(
            placeholders("203.0.113.10", None),
            (
                "203-0-113-10.sslip.io".into(),
                "203-0-113-10.sslip.io".into()
            )
        );
    }
}
