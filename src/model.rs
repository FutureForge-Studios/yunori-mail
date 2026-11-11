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
    /// Where the console is opened (chosen in the terminal): a domain name, a
    /// `<ip>.sslip.io` name, or the bare IP address. `None` on installs made
    /// before the browser onboarding: the console lives on `hostname`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_host: Option<String>,
}

impl ServerConfig {
    pub const KEY: &'static str = "config";

    /// The address the console is opened at.
    pub fn console_host(&self) -> &str {
        self.panel_host.as_deref().unwrap_or(&self.hostname)
    }
}

/// Relay settings (kv `relay`; the password lives in the vault as `relay.password`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelaySettings {
    pub provider: String,
    /// Effective SMTP host (derived from `region` for regional providers).
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Provider region (Amazon SES: picks the SMTP endpoint and the bounce MX;
    /// Resend: picks the bounce MX of its `send.` subdomain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// SPF include override (e.g. `spf.example.net`), for providers whose include is
    /// account-specific or unknown. Without it the provider's own include is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spf_include: Option<String>,
    /// Per-domain records the provider issues (domain verification), keyed by domain.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub identities: std::collections::BTreeMap<String, RelayIdentity>,
}

/// What a provider issued for one domain. These are public DNS values, not secrets.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelayIdentity {
    /// Amazon SES Easy DKIM tokens (three, each publishes a CNAME).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dkim_tokens: Vec<String>,
    /// A provider DKIM TXT value (Resend's `resend._domainkey`), as the provider shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dkim_value: Option<String>,
    /// Custom MAIL FROM (bounce) domain, fully qualified (Amazon SES).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail_from: Option<String>,
}

/// How a provider verifies a sending domain.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RelayVerify {
    /// Nothing Yunori can publish for you (the provider's own steps apply).
    None,
    /// Amazon SES Easy DKIM: three CNAMEs `<token>._domainkey` → `<token>.dkim.amazonses.com`.
    SesDkim,
    /// One provider DKIM TXT record at `<selector>._domainkey` (Resend).
    DkimTxt,
}

/// Bounce (MAIL FROM) domain records a provider asks for.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailFromSpec {
    /// A fixed label under the domain (Resend always uses `send`); `None` = operator's choice.
    pub label: Option<&'static str>,
    /// MX target; `{region}` is substituted.
    pub mx_template: &'static str,
    pub spf: &'static str,
    /// Whether the provider needs these records before it will send.
    pub required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayProvider {
    pub key: &'static str,
    pub label: &'static str,
    /// Fixed SMTP host (pre-filled, editable). For regional providers see `host_template`.
    pub host: Option<&'static str>,
    /// SMTP host derived from the region (`{region}` substituted); the host is then not editable.
    pub host_template: Option<&'static str>,
    pub port: u16,
    /// Regions to offer (empty = the provider has none).
    pub regions: &'static [&'static str],
    pub default_region: Option<&'static str>,
    /// SMTP username the provider mandates (pre-filled).
    pub username: Option<&'static str>,
    /// What the provider calls the SMTP password.
    pub secret_label: &'static str,
    pub spf_include: Option<&'static str>,
    pub verify: RelayVerify,
    /// Selector of the provider's DKIM TXT record (`verify = DkimTxt`).
    pub dkim_selector: Option<&'static str>,
    pub mail_from: Option<MailFromSpec>,
    pub docs_url: Option<&'static str>,
    /// Where the operator adds a domain at the provider (`{region}` substituted).
    pub domains_url: Option<&'static str>,
}

const AMAZON_FEEDBACK_MX: &str = "feedback-smtp.{region}.amazonses.com";
const AMAZON_BOUNCE_SPF: &str = "v=spf1 include:amazonses.com ~all";

/// Amazon SES regions with an SMTP endpoint (any `xx-name-N` region is accepted by the API).
pub const SES_REGIONS: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "ca-central-1",
    "sa-east-1",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-central-1",
    "eu-central-2",
    "eu-south-1",
    "eu-north-1",
    "il-central-1",
    "me-south-1",
    "me-central-1",
    "af-south-1",
    "ap-south-1",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-southeast-3",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
];

/// Data-driven relay registry (plan §8.2). Amazon SES and Resend come first: they
/// are the two most common choices, and Yunori publishes their domain records.
pub const RELAY_PROVIDERS: &[RelayProvider] = &[
    RelayProvider {
        key: "ses",
        label: "Amazon SES",
        host: None,
        host_template: Some("email-smtp.{region}.amazonaws.com"),
        port: 587,
        regions: SES_REGIONS,
        default_region: Some("us-east-1"),
        username: None,
        secret_label: "SMTP password",
        spf_include: Some("amazonses.com"),
        verify: RelayVerify::SesDkim,
        dkim_selector: None,
        mail_from: Some(MailFromSpec {
            label: None,
            mx_template: AMAZON_FEEDBACK_MX,
            spf: AMAZON_BOUNCE_SPF,
            required: false,
        }),
        docs_url: Some("https://docs.aws.amazon.com/ses/latest/dg/smtp-credentials.html"),
        domains_url: Some("https://console.aws.amazon.com/ses/home?region={region}#/identities"),
    },
    RelayProvider {
        key: "resend",
        label: "Resend",
        host: Some("smtp.resend.com"),
        host_template: None,
        port: 587,
        // Resend's sending region only changes the bounce MX of `send.<domain>`.
        regions: &["us-east-1", "eu-west-1", "sa-east-1", "ap-northeast-1"],
        default_region: Some("us-east-1"),
        username: Some("resend"),
        secret_label: "API key",
        // Resend bounces through `send.<domain>`, so the root SPF needs no include.
        spf_include: None,
        verify: RelayVerify::DkimTxt,
        dkim_selector: Some("resend"),
        mail_from: Some(MailFromSpec {
            label: Some("send"),
            mx_template: AMAZON_FEEDBACK_MX,
            spf: AMAZON_BOUNCE_SPF,
            required: true,
        }),
        docs_url: Some("https://resend.com/docs/send-with-smtp"),
        domains_url: Some("https://resend.com/domains"),
    },
    RelayProvider {
        key: "postmark",
        label: "Postmark",
        host: Some("smtp.postmarkapp.com"),
        host_template: None,
        port: 587,
        regions: &[],
        default_region: None,
        username: None,
        secret_label: "Server API token",
        spf_include: None,
        verify: RelayVerify::None,
        dkim_selector: None,
        mail_from: None,
        docs_url: Some("https://postmarkapp.com/developer/user-guide/send-email-with-smtp"),
        domains_url: None,
    },
    RelayProvider {
        key: "mailgun",
        label: "Mailgun",
        host: Some("smtp.mailgun.org"),
        host_template: None,
        port: 587,
        regions: &[],
        default_region: None,
        username: None,
        secret_label: "SMTP password",
        spf_include: Some("mailgun.org"),
        verify: RelayVerify::None,
        dkim_selector: None,
        mail_from: None,
        docs_url: Some(
            "https://documentation.mailgun.com/docs/mailgun/user-manual/sending-messages/send-smtp",
        ),
        domains_url: None,
    },
    RelayProvider {
        key: "smtp2go",
        label: "SMTP2GO",
        host: Some("mail.smtp2go.com"),
        host_template: None,
        port: 587,
        regions: &[],
        default_region: None,
        username: None,
        secret_label: "SMTP password",
        spf_include: Some("spf.smtp2go.com"),
        verify: RelayVerify::None,
        dkim_selector: None,
        mail_from: None,
        docs_url: Some("https://support.smtp2go.com/"),
        domains_url: None,
    },
    RelayProvider {
        key: "brevo",
        label: "Brevo",
        host: Some("smtp-relay.brevo.com"),
        host_template: None,
        port: 587,
        regions: &[],
        default_region: None,
        username: None,
        secret_label: "SMTP key",
        spf_include: Some("spf.brevo.com"),
        verify: RelayVerify::None,
        dkim_selector: None,
        mail_from: None,
        docs_url: Some("https://help.brevo.com/"),
        domains_url: None,
    },
    RelayProvider {
        key: "custom",
        label: "Other SMTP relay",
        host: None,
        host_template: None,
        port: 587,
        regions: &[],
        default_region: None,
        username: None,
        secret_label: "Password",
        spf_include: None,
        verify: RelayVerify::None,
        dkim_selector: None,
        mail_from: None,
        docs_url: None,
        domains_url: None,
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
