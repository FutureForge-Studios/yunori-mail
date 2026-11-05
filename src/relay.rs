//! Relay (smarthost) providers: everything provider-specific about sending
//! through Amazon SES, Resend and the rest, kept pure so the API, the installer
//! and the DNS checks agree on one set of rules.
//!
//! - The SMTP host is derived from the region for regional providers (SES), so
//!   it cannot be mistyped.
//! - Domain verification: the records a provider issues per domain (SES Easy
//!   DKIM CNAMEs, Resend's DKIM TXT) and its bounce (MAIL FROM) domain records
//!   join that domain's DNS checklist and are verified against public resolvers
//!   like every other record. The operator pastes what the provider shows; Yunori
//!   never calls the provider's API, so no AWS or Resend admin key is stored.
//! - The engine still DKIM-signs locally, so DMARC aligns on DKIM through any relay.

use crate::dns::records::Expected;
use crate::model::{RelayIdentity, RelayProvider, RelaySettings, RelayVerify, relay_provider};
use crate::util::normalize_domain;
use std::collections::BTreeMap;

/// A validation failure tied to one form field (camelCase path, e.g. `identities.example.com.dkim`).
#[derive(Debug, Clone, PartialEq)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

fn err(field: impl Into<String>, message: impl Into<String>) -> FieldError {
    FieldError {
        field: field.into(),
        message: message.into(),
    }
}

/// `us-east-1`, `eu-central-2`, `us-gov-west-1`, …
pub fn valid_region(r: &str) -> bool {
    let parts: Vec<&str> = r.split('-').collect();
    parts.len() >= 3
        && parts.len() <= 4
        && parts[0].len() == 2
        && parts[0].chars().all(|c| c.is_ascii_lowercase())
        && parts[1..parts.len() - 1]
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase()))
        && parts
            .last()
            .is_some_and(|n| (1..=2).contains(&n.len()) && n.chars().all(|c| c.is_ascii_digit()))
}

/// The SPF include every relayed domain publishes: the operator's override, else the provider's.
pub fn spf_include(s: &RelaySettings) -> Option<String> {
    s.spf_include.clone().or_else(|| {
        relay_provider(&s.provider)
            .and_then(|p| p.spf_include)
            .map(str::to_string)
    })
}

fn normalize_spf_include(raw: &str) -> Result<Option<String>, FieldError> {
    let v = raw.trim().trim_start_matches("include:").trim();
    if v.is_empty() {
        return Ok(None);
    }
    normalize_domain(v)
        .map(Some)
        .ok_or_else(|| err("spfInclude", "Enter a host name, like spf.example.net."))
}

/// SES Easy DKIM tokens from whatever the operator pasted: the three CNAME rows
/// from the SES console (or its CSV), or just the tokens. Each token appears in
/// both the name and the value of its row, so duplicates collapse.
pub fn parse_ses_tokens(raw: &str) -> Result<Vec<String>, String> {
    let lower = raw.to_ascii_lowercase();
    let mut tokens: Vec<String> = Vec::new();
    for word in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
        if word.len() == 32 && !tokens.iter().any(|t| t == word) {
            tokens.push(word.to_string());
        }
    }
    match tokens.len() {
        3 => Ok(tokens),
        0 => Err("Paste the three DKIM records Amazon SES shows for this domain.".into()),
        n => Err(format!(
            "Found {n} DKIM token{}; Amazon SES issues exactly three. Paste all three CNAME records.",
            if n == 1 { "" } else { "s" }
        )),
    }
}

/// A provider DKIM TXT value as the provider shows it (`p=MIGf…`, possibly with
/// `v=DKIM1; k=rsa;` tags, quotes or line breaks). A bare key gets `p=` prepended.
pub fn parse_dkim_value(raw: &str) -> Result<String, String> {
    let joined = raw.replace(['"', '\''], " ");
    let compact = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return Err("Paste the DKIM value your provider shows.".into());
    }
    let mut parts: Vec<String> = Vec::new();
    let mut key = None;
    for part in compact.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(k) = part.strip_prefix("p=") {
            let k: String = k.chars().filter(|c| !c.is_whitespace()).collect();
            key = Some(k.clone());
            parts.push(format!("p={k}"));
        } else {
            parts.push(part.to_string());
        }
    }
    if key.is_none() && parts.len() == 1 {
        let k: String = parts[0].chars().filter(|c| !c.is_whitespace()).collect();
        key = Some(k.clone());
        parts = vec![format!("p={k}")];
    }
    let key = key.ok_or_else(|| {
        "That doesn't look like a DKIM value (it should contain p=…).".to_string()
    })?;
    let base64 = key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='));
    if !base64 || key.len() < 64 {
        return Err(
            "That DKIM key looks incomplete. Copy the whole value from your provider.".into(),
        );
    }
    Ok(parts.join("; "))
}

/// A bounce (MAIL FROM) domain: `bounce` or `bounce.example.com`, under `domain`.
pub fn normalize_mail_from(raw: &str, domain: &str) -> Result<Option<String>, String> {
    let v = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if v.is_empty() {
        return Ok(None);
    }
    if v == domain {
        return Err(format!(
            "Use a subdomain of {domain}, like bounce.{domain}."
        ));
    }
    let fqdn = if v.ends_with(&format!(".{domain}")) {
        v
    } else {
        format!("{v}.{domain}")
    };
    match normalize_domain(&fqdn) {
        Some(d) if d != domain && d.ends_with(&format!(".{domain}")) => Ok(Some(d)),
        _ => Err(format!(
            "Use a subdomain of {domain}, like bounce.{domain}."
        )),
    }
}

/// One domain's verification input from the console form.
#[derive(Debug, Clone, Default)]
pub struct IdentityInput {
    pub dkim: String,
    pub mail_from: String,
}

/// The relay form, before validation.
#[derive(Debug, Clone, Default)]
pub struct RelayDraft {
    pub provider: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub region: Option<String>,
    pub spf_include: Option<String>,
    /// `None` keeps the previously saved identities.
    pub identities: Option<BTreeMap<String, IdentityInput>>,
}

fn resolve_region(p: &RelayProvider, input: Option<&str>) -> Result<Option<String>, FieldError> {
    if p.regions.is_empty() {
        return Ok(None);
    }
    let r = input
        .map(|r| r.trim().to_ascii_lowercase())
        .filter(|r| !r.is_empty())
        .or_else(|| p.default_region.map(str::to_string))
        .ok_or_else(|| err("region", format!("Choose your {} region.", p.label)))?;
    if !valid_region(&r) {
        return Err(err("region", "That isn't a region name, like us-east-1."));
    }
    Ok(Some(r))
}

/// Validate the form and produce the settings to store. `domains` are the
/// domains hosted here; `previous` supplies identities when the form omits them.
pub fn build_settings(
    d: &RelayDraft,
    previous: Option<&RelaySettings>,
    domains: &[String],
) -> Result<RelaySettings, FieldError> {
    let p =
        relay_provider(&d.provider).ok_or_else(|| err("provider", "Unknown relay provider."))?;
    let region = resolve_region(p, d.region.as_deref())?;
    let host = match (p.host_template, region.as_deref()) {
        (Some(t), Some(r)) => t.replace("{region}", r),
        _ => {
            let h = if d.host.trim().is_empty() {
                p.host.unwrap_or_default()
            } else {
                d.host.trim()
            };
            normalize_domain(h).ok_or_else(|| err("host", "Enter the relay's host name."))?
        }
    };
    if d.port == 0 {
        return Err(err("port", "Enter a port."));
    }
    let username = match d.username.trim() {
        "" => p.username.unwrap_or_default().to_string(),
        u => u.to_string(),
    };
    if username.is_empty() {
        return Err(err("username", "Enter the relay username."));
    }
    let spf_include = normalize_spf_include(d.spf_include.as_deref().unwrap_or_default())?;

    let identities = match &d.identities {
        None => previous.map(|s| s.identities.clone()).unwrap_or_default(),
        Some(inputs) => {
            let mut out = BTreeMap::new();
            for (raw_domain, input) in inputs {
                let domain = raw_domain.trim().to_ascii_lowercase();
                if !domains.contains(&domain) {
                    return Err(err(
                        "identities",
                        format!("{domain} is not a domain on this server."),
                    ));
                }
                let field = |f: &str| format!("identities.{domain}.{f}");
                let mut id = RelayIdentity::default();
                match p.verify {
                    RelayVerify::SesDkim => {
                        if !input.dkim.trim().is_empty() {
                            id.dkim_tokens =
                                parse_ses_tokens(&input.dkim).map_err(|m| err(field("dkim"), m))?;
                        }
                        id.mail_from = normalize_mail_from(&input.mail_from, &domain)
                            .map_err(|m| err(field("mailFrom"), m))?;
                    }
                    RelayVerify::DkimTxt => {
                        if !input.dkim.trim().is_empty() {
                            id.dkim_value = Some(
                                parse_dkim_value(&input.dkim).map_err(|m| err(field("dkim"), m))?,
                            );
                        }
                    }
                    RelayVerify::None => {}
                }
                if id != RelayIdentity::default() {
                    out.insert(domain, id);
                }
            }
            out
        }
    };

    Ok(RelaySettings {
        provider: p.key.to_string(),
        host,
        port: d.port,
        username,
        region,
        spf_include,
        identities,
    })
}

fn row(
    id: &str,
    rtype: &str,
    name: String,
    value: String,
    required: bool,
    purpose: String,
) -> Expected {
    Expected {
        id: id.to_string(),
        rtype: rtype.to_string(),
        name,
        value,
        required,
        purpose,
    }
}

/// The provider's records for one domain (empty when the provider issues none).
/// Ids start with `relay:` so the checker treats them as provider records.
pub fn domain_records(s: &RelaySettings, domain: &str) -> Vec<Expected> {
    let Some(p) = relay_provider(&s.provider) else {
        return vec![];
    };
    let identity = s.identities.get(domain);
    let mut out = Vec::new();
    match p.verify {
        RelayVerify::SesDkim => {
            for (i, t) in identity
                .map(|i| i.dkim_tokens.as_slice())
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                out.push(row(
                    &format!("relay:dkim:{}", i + 1),
                    "CNAME",
                    format!("{t}._domainkey.{domain}"),
                    format!("{t}.dkim.amazonses.com"),
                    true,
                    format!(
                        "{}: verifies this domain so SES will send for it (Easy DKIM).",
                        p.label
                    ),
                ));
            }
        }
        RelayVerify::DkimTxt => {
            if let (Some(v), Some(sel)) =
                (identity.and_then(|i| i.dkim_value.clone()), p.dkim_selector)
            {
                out.push(row(
                    "relay:dkim",
                    "TXT",
                    format!("{sel}._domainkey.{domain}"),
                    v,
                    true,
                    format!(
                        "{}: its DKIM key, which verifies this domain with {}.",
                        p.label, p.label
                    ),
                ));
            }
        }
        RelayVerify::None => {}
    }
    if let Some(mf) = p.mail_from {
        let name = match mf.label {
            Some(l) => Some(format!("{l}.{domain}")),
            None => identity.and_then(|i| i.mail_from.clone()),
        };
        let region = s.region.as_deref().or(p.default_region);
        if let (Some(name), Some(region)) = (name, region) {
            out.push(row(
                "relay:mail-from-mx",
                "MX",
                name.clone(),
                format!("10 {}", mf.mx_template.replace("{region}", region)),
                mf.required,
                format!(
                    "{}: receives bounces for mail it sends as this domain.",
                    p.label
                ),
            ));
            out.push(row(
                "relay:mail-from-spf",
                "TXT",
                name,
                mf.spf.to_string(),
                mf.required,
                format!(
                    "{}: SPF for the bounce domain, so SPF aligns with your domain too.",
                    p.label
                ),
            ));
        }
    }
    out
}

/// The provider needs per-domain records that haven't been pasted yet.
pub fn needs_verification(s: &RelaySettings, domain: &str) -> bool {
    let Some(p) = relay_provider(&s.provider) else {
        return false;
    };
    let id = s.identities.get(domain);
    match p.verify {
        RelayVerify::SesDkim => id.is_none_or(|i| i.dkim_tokens.is_empty()),
        RelayVerify::DkimTxt => id.is_none_or(|i| i.dkim_value.is_none()),
        RelayVerify::None => false,
    }
}

/// Where the operator adds the domain at the provider, if the provider has such a page.
pub fn domains_url(s: &RelaySettings) -> Option<String> {
    let p = relay_provider(&s.provider)?;
    let region = s
        .region
        .as_deref()
        .or(p.default_region)
        .unwrap_or("us-east-1");
    p.domains_url.map(|u| u.replace("{region}", region))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T1: &str = "abcdefghijklmnopqrstuvwxyz012345";
    const T2: &str = "bcdefghijklmnopqrstuvwxyz0123456";
    const T3: &str = "cdefghijklmnopqrstuvwxyz01234567";
    const KEY: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC7xYwHmS4fXh5qk9Jh2bU0Lw8mN3pQ1rT6vY";

    fn domains() -> Vec<String> {
        vec!["example.com".into(), "other.org".into()]
    }

    fn ses_draft() -> RelayDraft {
        RelayDraft {
            provider: "ses".into(),
            port: 587,
            username: "AKIAEXAMPLESMTPUSER".into(),
            region: Some("eu-west-1".into()),
            ..Default::default()
        }
    }

    #[test]
    fn regions() {
        for ok in [
            "us-east-1",
            "eu-central-2",
            "ap-southeast-3",
            "us-gov-west-1",
        ] {
            assert!(valid_region(ok), "{ok}");
        }
        for bad in [
            "",
            "us",
            "us-east",
            "useast-1",
            "US-EAST-1",
            "us-east-123",
            "us-east-1;rm",
        ] {
            assert!(!valid_region(bad), "{bad}");
        }
    }

    #[test]
    fn ses_host_comes_from_the_region() {
        let mut d = ses_draft();
        d.host = "attacker.example".into(); // ignored: SES hosts are derived
        let s = build_settings(&d, None, &domains()).unwrap();
        assert_eq!(s.host, "email-smtp.eu-west-1.amazonaws.com");
        assert_eq!(s.region.as_deref(), Some("eu-west-1"));
        d.region = Some("not a region".into());
        assert_eq!(
            build_settings(&d, None, &domains()).unwrap_err().field,
            "region"
        );
    }

    #[test]
    fn resend_defaults_username_and_region() {
        let d = RelayDraft {
            provider: "resend".into(),
            port: 587,
            ..Default::default()
        };
        let s = build_settings(&d, None, &domains()).unwrap();
        assert_eq!(s.host, "smtp.resend.com");
        assert_eq!(s.username, "resend");
        assert_eq!(s.region.as_deref(), Some("us-east-1"));
        // Resend bounces via send.<domain>; the root SPF gets no include.
        assert_eq!(spf_include(&s), None);
    }

    #[test]
    fn ses_tokens_from_console_rows_csv_or_bare() {
        let rows = format!(
            "{T1}._domainkey.example.com CNAME {T1}.dkim.amazonses.com\n\
             {T2}._domainkey.example.com\tCNAME\t{T2}.dkim.amazonses.com\n\
             \"{T3}._domainkey.example.com\",\"CNAME\",\"{T3}.dkim.amazonses.com\""
        );
        assert_eq!(parse_ses_tokens(&rows).unwrap(), vec![T1, T2, T3]);
        assert_eq!(
            parse_ses_tokens(&format!("{T1} {T2} {T3}")).unwrap().len(),
            3
        );
        assert!(
            parse_ses_tokens(T1)
                .unwrap_err()
                .contains("Found 1 DKIM token;")
        );
        assert!(parse_ses_tokens("").is_err());
    }

    #[test]
    fn dkim_values_normalise() {
        assert_eq!(
            parse_dkim_value(&format!("p={KEY}")).unwrap(),
            format!("p={KEY}")
        );
        assert_eq!(parse_dkim_value(KEY).unwrap(), format!("p={KEY}"));
        let (a, b) = KEY.split_at(30);
        assert_eq!(
            parse_dkim_value(&format!("\"v=DKIM1; k=rsa; p={a}\"\n\"{b}\"")).unwrap(),
            format!("v=DKIM1; k=rsa; p={KEY}")
        );
        assert!(parse_dkim_value("p=short").is_err());
        assert!(parse_dkim_value("hello world").is_err());
    }

    #[test]
    fn mail_from_must_be_a_subdomain() {
        assert_eq!(
            normalize_mail_from("bounce", "example.com")
                .unwrap()
                .as_deref(),
            Some("bounce.example.com")
        );
        assert_eq!(
            normalize_mail_from("Bounce.Example.com.", "example.com")
                .unwrap()
                .as_deref(),
            Some("bounce.example.com")
        );
        assert_eq!(normalize_mail_from("", "example.com").unwrap(), None);
        assert!(normalize_mail_from("example.com", "example.com").is_err());
        assert!(normalize_mail_from("bad label", "example.com").is_err());
    }

    #[test]
    fn ses_records_for_a_verified_domain() {
        let mut d = ses_draft();
        d.identities = Some(BTreeMap::from([(
            "example.com".to_string(),
            IdentityInput {
                dkim: format!("{T1} {T2} {T3}"),
                mail_from: "bounce".into(),
            },
        )]));
        let s = build_settings(&d, None, &domains()).unwrap();
        let recs = domain_records(&s, "example.com");
        assert_eq!(recs.len(), 5);
        assert_eq!(recs[0].rtype, "CNAME");
        assert_eq!(recs[0].name, format!("{T1}._domainkey.example.com"));
        assert_eq!(recs[0].value, format!("{T1}.dkim.amazonses.com"));
        assert!(recs[0].required);
        let mx = recs.iter().find(|r| r.id == "relay:mail-from-mx").unwrap();
        assert_eq!(mx.name, "bounce.example.com");
        assert_eq!(mx.value, "10 feedback-smtp.eu-west-1.amazonses.com");
        assert!(!mx.required, "SES custom MAIL FROM is optional");
        assert!(!needs_verification(&s, "example.com"));
        assert!(needs_verification(&s, "other.org"));
        assert!(domain_records(&s, "other.org").is_empty());
        assert_eq!(spf_include(&s).as_deref(), Some("amazonses.com"));
    }

    #[test]
    fn resend_records_always_include_the_send_subdomain() {
        let mut d = RelayDraft {
            provider: "resend".into(),
            port: 587,
            region: Some("eu-west-1".into()),
            ..Default::default()
        };
        let s = build_settings(&d, None, &domains()).unwrap();
        let recs = domain_records(&s, "example.com");
        assert_eq!(
            recs.len(),
            2,
            "send.<domain> MX + SPF before the key is pasted"
        );
        assert_eq!(recs[0].name, "send.example.com");
        assert_eq!(recs[0].value, "10 feedback-smtp.eu-west-1.amazonses.com");
        assert!(recs.iter().all(|r| r.required));
        assert!(needs_verification(&s, "example.com"));

        d.identities = Some(BTreeMap::from([(
            "example.com".to_string(),
            IdentityInput {
                dkim: format!("p={KEY}"),
                mail_from: String::new(),
            },
        )]));
        let s = build_settings(&d, None, &domains()).unwrap();
        let dkim = domain_records(&s, "example.com")
            .into_iter()
            .find(|r| r.id == "relay:dkim")
            .unwrap();
        assert_eq!(dkim.name, "resend._domainkey.example.com");
        assert_eq!(dkim.value, format!("p={KEY}"));
    }

    #[test]
    fn identity_errors_name_their_field() {
        let mut d = ses_draft();
        d.identities = Some(BTreeMap::from([(
            "example.com".to_string(),
            IdentityInput {
                dkim: T1.into(),
                mail_from: String::new(),
            },
        )]));
        assert_eq!(
            build_settings(&d, None, &domains()).unwrap_err().field,
            "identities.example.com.dkim"
        );
        d.identities = Some(BTreeMap::from([(
            "nothere.net".to_string(),
            IdentityInput::default(),
        )]));
        assert_eq!(
            build_settings(&d, None, &domains()).unwrap_err().field,
            "identities"
        );
    }

    #[test]
    fn omitted_identities_are_kept_and_spf_override_wins() {
        let mut d = ses_draft();
        d.identities = Some(BTreeMap::from([(
            "example.com".to_string(),
            IdentityInput {
                dkim: format!("{T1} {T2} {T3}"),
                mail_from: String::new(),
            },
        )]));
        let first = build_settings(&d, None, &domains()).unwrap();
        d.identities = None;
        d.spf_include = Some("include:spf.example.net".into());
        let second = build_settings(&d, Some(&first), &domains()).unwrap();
        assert_eq!(second.identities, first.identities);
        assert_eq!(spf_include(&second).as_deref(), Some("spf.example.net"));
    }

    #[test]
    fn custom_needs_a_host() {
        let d = RelayDraft {
            provider: "custom".into(),
            port: 587,
            username: "u".into(),
            ..Default::default()
        };
        assert_eq!(
            build_settings(&d, None, &domains()).unwrap_err().field,
            "host"
        );
    }
}
