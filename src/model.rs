//! Types shared by checks, the installer, the API and the CLI. JSON shapes
//! mirror `console/src/api/types.ts` (camelCase).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Info,
    Unknown,
    Warn,
    Fail,
}

impl Status {
    pub fn worst(a: Status, b: Status) -> Status {
        a.max(b)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    pub label: String,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,
}

impl Check {
    pub fn new(id: &str, label: &str, status: Status, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            status,
            detail: detail.into(),
            remedy: None,
            help_url: None,
        }
    }
    pub fn remedy(mut self, r: impl Into<String>) -> Self {
        self.remedy = Some(r.into());
        self
    }
    pub fn help(mut self, url: impl Into<String>) -> Self {
        self.help_url = Some(url.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub rtype: String,
    pub name: String,
    pub expected: String,
    pub actual: Vec<String>,
    pub status: Status,
    pub required: bool,
    pub purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SendingMode {
    Direct,
    Relay,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DmarcPolicy {
    None,
    Quarantine,
    Reject,
}

impl DmarcPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            DmarcPolicy::None => "none",
            DmarcPolicy::Quarantine => "quarantine",
            DmarcPolicy::Reject => "reject",
        }
    }
}

/// Persistent server configuration (kv `config`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub hostname: String,
    pub primary_domain: String,
    /// External address for ACME + alerts (never a mailbox on this server).
    pub admin_email: String,
    pub client_origin: String,
    pub engine_url: String,
    pub sending_mode: SendingMode,
    pub public_ipv4: Option<String>,
    pub public_ipv6: Option<String>,
    pub stalwart_version: Option<String>,
    pub install_complete: bool,
}

impl ServerConfig {
    pub const KEY: &'static str = "config";
}

/// Relay settings (kv `relay`; the password lives in the vault as `relay.password`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelaySettings {
    pub provider: String,
    pub host: String,
    pub port: u16,
    pub username: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayProvider {
    pub key: &'static str,
    pub label: &'static str,
    pub host: Option<&'static str>,
    pub port: u16,
    pub spf_include: Option<&'static str>,
    pub docs_url: Option<&'static str>,
}

/// Data-driven relay registry (plan §8.2). Hosts for regional providers are user-supplied.
pub const RELAY_PROVIDERS: &[RelayProvider] = &[
    RelayProvider {
        key: "ses",
        label: "Amazon SES",
        host: None,
        port: 587,
        spf_include: Some("amazonses.com"),
        docs_url: Some("https://docs.aws.amazon.com/ses/latest/dg/smtp-connect.html"),
    },
    RelayProvider {
        key: "postmark",
        label: "Postmark",
        host: Some("smtp.postmarkapp.com"),
        port: 587,
        spf_include: None,
        docs_url: Some("https://postmarkapp.com/developer/user-guide/send-email-with-smtp"),
    },
    RelayProvider {
        key: "mailgun",
        label: "Mailgun",
        host: Some("smtp.mailgun.org"),
        port: 587,
        spf_include: Some("mailgun.org"),
        docs_url: Some(
            "https://documentation.mailgun.com/docs/mailgun/user-manual/sending-messages/send-smtp",
        ),
    },
    RelayProvider {
        key: "smtp2go",
        label: "SMTP2GO",
        host: Some("mail.smtp2go.com"),
        port: 587,
        spf_include: Some("spf.smtp2go.com"),
        docs_url: Some("https://support.smtp2go.com/"),
    },
    RelayProvider {
        key: "brevo",
        label: "Brevo",
        host: Some("smtp-relay.brevo.com"),
        port: 587,
        spf_include: Some("spf.brevo.com"),
        docs_url: Some("https://help.brevo.com/"),
    },
    RelayProvider {
        key: "resend",
        label: "Resend",
        host: Some("smtp.resend.com"),
        port: 587,
        spf_include: None,
        docs_url: Some("https://resend.com/docs/send-with-smtp"),
    },
    RelayProvider {
        key: "custom",
        label: "Other SMTP relay",
        host: None,
        port: 587,
        spf_include: None,
        docs_url: None,
    },
];

pub fn relay_provider(key: &str) -> Option<&'static RelayProvider> {
    RELAY_PROVIDERS.iter().find(|p| p.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_ordering_picks_worst() {
        assert_eq!(Status::worst(Status::Pass, Status::Warn), Status::Warn);
        assert_eq!(Status::worst(Status::Fail, Status::Warn), Status::Fail);
        assert_eq!(serde_json::to_string(&Status::Warn).unwrap(), "\"warn\"");
    }

    #[test]
    fn check_serialises_camel_case_and_skips_empty() {
        let c = Check::new("ptr", "Reverse DNS", Status::Pass, "ok").help("https://x");
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["helpUrl"], "https://x");
        assert!(v.get("remedy").is_none());
    }
}
