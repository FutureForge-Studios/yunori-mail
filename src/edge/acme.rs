//! ACME over HTTP-01 (Let's Encrypt unless configured otherwise). The
//! challenge files are written to a directory the edge's port-80 listener
//! serves, so issuance works whether it is driven by the installer (root) or
//! by the service (`serve`). Scheduling, retries and diagnosis live in
//! [`super::certs`]; this module only talks to the CA, and turns whatever the
//! CA says into a [`CertProblem`] a person can act on.

use crate::dns::DnsLookup;
use crate::paths::Paths;
use crate::store::Db;
use anyhow::{Result, anyhow};
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt,
    NewAccount, NewOrder, OrderStatus,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const LE_PRODUCTION: &str = LetsEncrypt::Production.url();
pub const LE_STAGING: &str = LetsEncrypt::Staging.url();

const DIRECTORY_KEY: &str = "acme.directory";

/// The CA this server uses, in order: `YUNORI_ACME_DIRECTORY` (any RFC 8555
/// CA), `YUNORI_ACME_STAGING=1` (Let's Encrypt staging), what the installer
/// saved, then Let's Encrypt production.
pub fn directory(db: &Db) -> String {
    if let Some(url) = env_directory() {
        return url;
    }
    db.get_kv(DIRECTORY_KEY)
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| LE_PRODUCTION.to_string())
}

/// A CA chosen through the environment (the installer persists it so the
/// service, which systemd starts without that environment, uses it too).
pub fn env_directory() -> Option<String> {
    if let Ok(url) = std::env::var("YUNORI_ACME_DIRECTORY")
        && !url.trim().is_empty()
    {
        return Some(url.trim().to_string());
    }
    if std::env::var("YUNORI_ACME_STAGING").is_ok_and(|v| v == "1") {
        return Some(LE_STAGING.to_string());
    }
    None
}

pub fn save_directory(db: &Db, url: &str) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1")) {
        return Err(anyhow!("the ACME directory must be an https:// URL"));
    }
    db.set_kv(DIRECTORY_KEY, url)
}

/// Plain name of a CA for messages.
pub fn ca_name(directory: &str) -> String {
    match directory {
        LE_PRODUCTION => "Let's Encrypt".into(),
        LE_STAGING => "Let's Encrypt (staging)".into(),
        other => url::Url::parse(other)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "the certificate authority".into()),
    }
}

/// The issuer domain a CA checks CAA records for, when we know it.
pub fn caa_identity(directory: &str) -> Option<&'static str> {
    match directory {
        LE_PRODUCTION | LE_STAGING => Some("letsencrypt.org"),
        _ => None,
    }
}

// ── problems a person can act on ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// The name has no A record on public DNS.
    DnsMissing,
    /// The A record points at another server.
    DnsElsewhere,
    /// The A record points at Cloudflare's proxy ("orange cloud").
    CloudflareProxy,
    /// An AAAA record points somewhere else (CAs try IPv6 first).
    Ipv6Elsewhere,
    /// A CAA record forbids this CA.
    Caa,
    /// This server's own port-80 listener is not serving challenges.
    EdgeDown,
    /// The CA could not connect to port 80 (firewall / security group).
    Unreachable,
    /// The CA connected, but something else answered.
    WrongResponse,
    /// The CA's own DNS lookup failed.
    Dns,
    /// Too many attempts; the CA asks us to wait.
    RateLimited,
    /// We could not talk to the CA at all.
    CaUnreachable,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CertProblem {
    pub kind: ProblemKind,
    /// The hostname concerned, when there is one.
    pub name: Option<String>,
    /// One sentence: what is wrong.
    pub summary: String,
    /// One sentence: what to do.
    pub remedy: String,
    /// The CA's own words, for the log and the curious.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When the CA said we may try again (rate limits).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<String>,
}

impl CertProblem {
    pub fn new(
        kind: ProblemKind,
        name: Option<&str>,
        summary: impl Into<String>,
        remedy: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            name: name.map(str::to_string),
            summary: summary.into(),
            remedy: remedy.into(),
            detail: None,
            retry_after: None,
        }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
    /// "summary remedy" — for one-line contexts (terminal, step detail).
    pub fn sentence(&self) -> String {
        format!("{} {}", self.summary, self.remedy)
            .trim()
            .to_string()
    }
}

impl std::fmt::Display for CertProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.sentence())
    }
}

/// An issuance that did not produce a certificate.
#[derive(Debug, Clone)]
pub struct Failure {
    pub problem: CertProblem,
    /// Names whose validation failed (empty when the whole order failed).
    pub failed_names: Vec<String>,
}

impl From<CertProblem> for Failure {
    fn from(problem: CertProblem) -> Self {
        Self {
            problem,
            failed_names: vec![],
        }
    }
}

/// Map an RFC 8555 problem document onto something a person can act on.
pub fn classify(
    ptype: Option<&str>,
    detail: Option<&str>,
    name: Option<&str>,
    ca: &str,
) -> CertProblem {
    let t = ptype
        .unwrap_or("")
        .trim_start_matches("urn:ietf:params:acme:error:");
    let d = detail.unwrap_or("").to_string();
    let lower = d.to_ascii_lowercase();
    let who = name.unwrap_or("this server");
    let p = match t {
        "rateLimited" => {
            let mut p = CertProblem::new(
                ProblemKind::RateLimited,
                name,
                format!("{ca} asked us to slow down after too many attempts."),
                "Nothing to do: Yunori waits and tries again by itself.",
            );
            p.retry_after = parse_retry_after(&d);
            p
        }
        "caa" => CertProblem::new(
            ProblemKind::Caa,
            name,
            format!("A CAA record on your domain does not allow {ca} to issue certificates."),
            "Add a CAA record `0 issue \"letsencrypt.org\"` next to your existing ones, or remove the CAA records.",
        ),
        "dns" => CertProblem::new(
            ProblemKind::Dns,
            name,
            format!("{ca} could not look up {who} in DNS."),
            format!(
                "Check that {who} has an A record pointing at this server; new records can take a few minutes to spread."
            ),
        ),
        "connection" => {
            if lower.contains("nxdomain") || lower.contains("no valid a records") {
                CertProblem::new(
                    ProblemKind::Dns,
                    name,
                    format!("{ca} could not find an address for {who}."),
                    format!("Add an A record for {who} pointing at this server."),
                )
            } else {
                CertProblem::new(
                    ProblemKind::Unreachable,
                    name,
                    format!("{ca} could not connect to {who} on port 80."),
                    "Allow incoming TCP 80 and 443 in your provider's firewall or security group (the server's own firewall was opened by the installer).",
                )
            }
        }
        "unauthorized" | "incorrectResponse" => CertProblem::new(
            ProblemKind::WrongResponse,
            name,
            format!("{ca} reached {who} on port 80, but another server answered."),
            format!(
                "Make sure {who} points straight at this server: no CDN or proxy in front of it, and no old A/AAAA record left behind."
            ),
        ),
        "tls" => CertProblem::new(
            ProblemKind::WrongResponse,
            name,
            format!("{ca} was redirected to HTTPS on {who} and the connection failed."),
            format!(
                "Something other than Yunori answers for {who}; point it straight at this server."
            ),
        ),
        "rejectedIdentifier" => CertProblem::new(
            ProblemKind::Other,
            name,
            format!("{ca} refuses to issue certificates for {who}."),
            "Use a hostname on a domain you own that is not on the CA's block list.",
        ),
        _ => CertProblem::new(
            ProblemKind::Other,
            name,
            format!("{ca} did not issue the certificate."),
            "Yunori will try again by itself; `yunori-mail doctor` shows the details.",
        ),
    };
    if d.is_empty() { p } else { p.detail(d) }
}

/// Let's Encrypt says "retry after 2026-11-06 12:00:00 UTC".
fn parse_retry_after(detail: &str) -> Option<String> {
    let i = detail.find("retry after ")?;
    let rest = &detail[i + "retry after ".len()..];
    let stamp: String = rest.chars().take(19).collect();
    chrono::NaiveDateTime::parse_from_str(&stamp, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|t| {
            t.and_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
}

fn ca_unreachable(ca: &str, directory: &str, e: impl std::fmt::Display) -> CertProblem {
    let host = url::Url::parse(directory)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| directory.to_string());
    CertProblem::new(
        ProblemKind::CaUnreachable,
        None,
        format!("Could not reach {ca} ({host}) over HTTPS."),
        "Check that this server can make outgoing HTTPS connections; Yunori tries again by itself.",
    )
    .detail(e.to_string())
}

fn from_acme_error(e: instant_acme::Error, ca: &str, directory: &str) -> CertProblem {
    named_acme_error(e, ca, directory, None)
}

/// Like [`from_acme_error`], for errors about the names in an order.
fn named_acme_error(
    e: instant_acme::Error,
    ca: &str,
    directory: &str,
    name: Option<&str>,
) -> CertProblem {
    match e {
        instant_acme::Error::Api(p) => classify(p.r#type.as_deref(), p.detail.as_deref(), name, ca),
        other => ca_unreachable(ca, directory, other),
    }
}

// ── the CA conversation ──────────────────────────────────────────────────

fn account_key(directory: &str) -> String {
    if directory == LE_PRODUCTION {
        // The key every earlier install used.
        "acme.account".into()
    } else {
        format!(
            "acme.account:{}",
            &crate::util::sha256_hex(directory.as_bytes())[..16]
        )
    }
}

async fn account(db: &Db, directory: &str, contact: &str) -> Result<Account, CertProblem> {
    let ca = ca_name(directory);
    let key = account_key(directory);
    if let Ok(Some(json)) = db.get_secret(&key)
        && let Ok(creds) = serde_json::from_str::<AccountCredentials>(&json)
    {
        return Account::from_credentials(creds)
            .await
            .map_err(|e| from_acme_error(e, &ca, directory));
    }
    let contact_uri = format!("mailto:{contact}");
    let contacts: Vec<&str> = if crate::util::valid_email(contact) {
        vec![contact_uri.as_str()]
    } else {
        vec![]
    };
    let (acct, creds) = Account::create(
        &NewAccount {
            contact: &contacts,
            terms_of_service_agreed: true,
            only_return_existing: false,
        },
        directory,
        None,
    )
    .await
    .map_err(|e| from_acme_error(e, &ca, directory))?;
    if let Ok(json) = serde_json::to_string(&creds) {
        let _ = db.put_secret(&key, &json);
    }
    Ok(acct)
}

/// A certificate chain and its key, as PEM.
pub struct Issued {
    pub chain_pem: String,
    pub key_pem: String,
}

/// Run one ACME order for `names` against `directory`. Nothing is written to
/// the certificate files here: the caller decides (a staging dry run throws
/// the result away).
pub async fn order(
    paths: &Paths,
    db: &Db,
    directory: &str,
    names: &[String],
    contact: &str,
) -> Result<Issued, Failure> {
    let ca = ca_name(directory);
    if names.is_empty() {
        return Err(CertProblem::new(
            ProblemKind::Other,
            None,
            "There is no hostname to certify.",
            "Run `yunori-mail install` to set the mail hostname.",
        )
        .into());
    }
    let challenges = paths.acme_challenges_dir();
    std::fs::create_dir_all(&challenges)
        .map_err(|e| Failure::from(local_problem("creating the challenge directory", e)))?;
    let acct = account(db, directory, contact).await?;
    let identifiers: Vec<Identifier> = names.iter().map(|n| Identifier::Dns(n.clone())).collect();
    let joined = names.join(", ");
    let mut order = acct
        .new_order(&NewOrder {
            identifiers: &identifiers,
        })
        .await
        .map_err(|e| Failure::from(named_acme_error(e, &ca, directory, Some(&joined))))?;

    let mut written = Vec::new();
    let result = async {
        let authorizations = order
            .authorizations()
            .await
            .map_err(|e| Failure::from(from_acme_error(e, &ca, directory)))?;
        for authz in authorizations {
            let Identifier::Dns(name) = &authz.identifier;
            match authz.status {
                AuthorizationStatus::Valid => continue,
                AuthorizationStatus::Pending => {}
                other => {
                    return Err(Failure {
                        problem: CertProblem::new(
                            ProblemKind::Other,
                            Some(name),
                            format!("{ca} reports the authorization for {name} as {other:?}."),
                            "Yunori will start a fresh order on the next attempt.",
                        ),
                        failed_names: vec![name.clone()],
                    });
                }
            }
            let ch = authz
                .challenges
                .iter()
                .find(|c| c.r#type == ChallengeType::Http01)
                .ok_or_else(|| {
                    Failure::from(CertProblem::new(
                        ProblemKind::Other,
                        Some(name),
                        format!("{ca} offered no HTTP-01 challenge for {name}."),
                        "Use a CA that supports HTTP-01 validation.",
                    ))
                })?;
            if !super::valid_token(&ch.token) {
                return Err(CertProblem::new(
                    ProblemKind::Other,
                    Some(name),
                    format!("{ca} sent a malformed challenge token."),
                    "Yunori will try again by itself.",
                )
                .into());
            }
            let ka = order.key_authorization(ch);
            let file = challenges.join(&ch.token);
            std::fs::write(&file, ka.as_str())
                .map_err(|e| Failure::from(local_problem("writing the challenge file", e)))?;
            written.push(file);
            order
                .set_challenge_ready(&ch.url)
                .await
                .map_err(|e| Failure::from(from_acme_error(e, &ca, directory)))?;
        }

        let mut delay = Duration::from_millis(500);
        let mut status = OrderStatus::Pending;
        for _ in 0..14 {
            tokio::time::sleep(delay).await;
            status = order
                .refresh()
                .await
                .map_err(|e| Failure::from(from_acme_error(e, &ca, directory)))?
                .status;
            if matches!(
                status,
                OrderStatus::Ready | OrderStatus::Invalid | OrderStatus::Valid
            ) {
                break;
            }
            delay = (delay * 2).min(Duration::from_secs(8));
        }
        if status == OrderStatus::Invalid {
            return Err(invalid_order(&mut order, &ca).await);
        }
        if status != OrderStatus::Ready && status != OrderStatus::Valid {
            return Err(CertProblem::new(
                ProblemKind::Other,
                None,
                format!("{ca} did not finish validating in time."),
                "Yunori will try again by itself.",
            )
            .into());
        }
        Ok(())
    }
    .await;
    for f in &written {
        let _ = std::fs::remove_file(f);
    }
    result?;

    let key = rcgen::KeyPair::generate()
        .map_err(|e| Failure::from(local_problem("generating a key", e)))?;
    let mut params = rcgen::CertificateParams::new(names.to_vec())
        .map_err(|e| Failure::from(local_problem("building the request", e)))?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    let csr = params
        .serialize_request(&key)
        .map_err(|e| Failure::from(local_problem("signing the request", e)))?;
    order
        .finalize(csr.der())
        .await
        .map_err(|e| Failure::from(from_acme_error(e, &ca, directory)))?;
    let mut tries = 0;
    let chain = loop {
        match order.certificate().await {
            Ok(Some(c)) => break c,
            Ok(None) if tries < 30 => {
                tries += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Ok(None) => {
                return Err(CertProblem::new(
                    ProblemKind::Other,
                    None,
                    format!("{ca} took too long to sign the certificate."),
                    "Yunori will try again by itself.",
                )
                .into());
            }
            Err(e) => return Err(from_acme_error(e, &ca, directory).into()),
        }
    };
    Ok(Issued {
        chain_pem: chain,
        key_pem: key.serialize_pem(),
    })
}

/// The order failed validation: find out which names failed and why.
async fn invalid_order(order: &mut instant_acme::Order, ca: &str) -> Failure {
    let order_error = order.state().error.clone();
    let mut failed = Vec::new();
    let mut first: Option<CertProblem> = None;
    if let Ok(authzs) = order.authorizations().await {
        for a in authzs {
            if a.status != AuthorizationStatus::Invalid {
                continue;
            }
            let Identifier::Dns(name) = &a.identifier;
            failed.push(name.clone());
            if first.is_none() {
                let err = a
                    .challenges
                    .iter()
                    .find(|c| c.r#type == ChallengeType::Http01)
                    .and_then(|c| c.error.clone());
                first = Some(match err {
                    Some(p) => classify(p.r#type.as_deref(), p.detail.as_deref(), Some(name), ca),
                    None => classify(None, None, Some(name), ca),
                });
            }
        }
    }
    let problem = first.unwrap_or_else(|| match order_error {
        Some(p) => classify(p.r#type.as_deref(), p.detail.as_deref(), None, ca),
        None => classify(None, None, None, ca),
    });
    Failure {
        problem,
        failed_names: failed,
    }
}

fn local_problem(what: &str, e: impl std::fmt::Display) -> CertProblem {
    CertProblem::new(
        ProblemKind::Other,
        None,
        format!("Failed while {what} on this server."),
        "Run `yunori-mail doctor` for details.",
    )
    .detail(e.to_string())
}

/// Hostnames to certify: the mail host plus the helper names (autoconfig,
/// autodiscover, mta-sts) of every hosted domain that already resolve here.
pub async fn wanted_names(
    hostname: &str,
    domains: &[String],
    ipv4: Option<&str>,
    dns: &dyn DnsLookup,
) -> Vec<String> {
    let mut names = vec![hostname.to_string()];
    let Some(ip) = ipv4 else { return names };
    for d in domains {
        for prefix in ["autoconfig", "autodiscover", "mta-sts"] {
            let n = format!("{prefix}.{d}");
            let points_here = dns
                .a(&n)
                .await
                .map(|a| a.iter().any(|x| x == ip))
                .unwrap_or(false);
            if points_here && !names.contains(&n) {
                names.push(n);
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_what_the_ca_says() {
        let p = classify(
            Some("urn:ietf:params:acme:error:connection"),
            Some(
                "185.1.2.3: Fetching http://mail.example.com/.well-known/acme-challenge/x: Timeout during connect (likely firewall problem)",
            ),
            Some("mail.example.com"),
            "Let's Encrypt",
        );
        assert_eq!(p.kind, ProblemKind::Unreachable);
        assert!(p.summary.contains("port 80"));
        assert!(p.remedy.contains("firewall"));
        assert!(p.detail.unwrap().contains("Timeout"));

        let p = classify(
            Some("urn:ietf:params:acme:error:unauthorized"),
            Some("Invalid response from http://mail.example.com/.well-known/acme-challenge/x: 404"),
            Some("mail.example.com"),
            "Let's Encrypt",
        );
        assert_eq!(p.kind, ProblemKind::WrongResponse);

        let p = classify(
            Some("urn:ietf:params:acme:error:caa"),
            None,
            Some("mail.example.com"),
            "Let's Encrypt",
        );
        assert_eq!(p.kind, ProblemKind::Caa);

        let p = classify(
            Some("urn:ietf:params:acme:error:rateLimited"),
            Some(
                "too many failed authorizations recently: see https://letsencrypt.org/docs/rate-limits/; retry after 2026-11-06 12:30:00 UTC",
            ),
            None,
            "Let's Encrypt",
        );
        assert_eq!(p.kind, ProblemKind::RateLimited);
        assert_eq!(p.retry_after.as_deref(), Some("2026-11-06T12:30:00Z"));

        let p = classify(
            Some("urn:ietf:params:acme:error:connection"),
            Some("DNS problem: NXDOMAIN looking up A for mail.example.com"),
            Some("mail.example.com"),
            "Let's Encrypt",
        );
        assert_eq!(p.kind, ProblemKind::Dns);
    }

    #[test]
    fn ca_names_and_caa_identities() {
        assert_eq!(ca_name(LE_PRODUCTION), "Let's Encrypt");
        assert_eq!(ca_name("https://acme.example.org/dir"), "acme.example.org");
        assert_eq!(caa_identity(LE_STAGING), Some("letsencrypt.org"));
        assert_eq!(caa_identity("https://acme.example.org/dir"), None);
        assert_eq!(account_key(LE_PRODUCTION), "acme.account");
        assert_ne!(account_key(LE_STAGING), account_key(LE_PRODUCTION));
    }

    /// Talks to Let's Encrypt's *staging* CA (made for testing ACME clients):
    /// creates a throwaway account and asks for a name the CA's policy refuses,
    /// so no validation ever runs. Checks the conversation and that the CA's
    /// problem document comes back as words a person can act on.
    /// Run with: YUNORI_LIVE_ACME=1 cargo test live_acme -- --ignored
    #[tokio::test]
    #[ignore]
    async fn live_acme_staging_problem_is_explained() {
        if std::env::var("YUNORI_LIVE_ACME").as_deref() != Ok("1") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(dir.path());
        let db = Db::open_memory().unwrap();
        let err = order(
            &paths,
            &db,
            LE_STAGING,
            &["yunori-probe.example.org".to_string()],
            "",
        )
        .await
        .err()
        .expect("the CA must refuse a reserved name");
        eprintln!("{:#?}", err.problem);
        assert_eq!(err.problem.kind, ProblemKind::Other, "{:?}", err.problem);
        assert!(
            err.problem.summary.contains("refuses")
                && err.problem.summary.contains("yunori-probe.example.org"),
            "{:?}",
            err.problem
        );
        assert!(err.problem.detail.is_some());
        // The staging account was kept for next time.
        assert!(db.get_secret(&account_key(LE_STAGING)).unwrap().is_some());
    }

    #[tokio::test]
    async fn helper_names_join_only_when_they_point_here() {
        let mut d = crate::dns::fake::FakeDns::default();
        d.a.insert("autoconfig.example.com".into(), vec!["203.0.113.10".into()]);
        d.a.insert("mta-sts.example.com".into(), vec!["198.51.100.9".into()]);
        let names = wanted_names(
            "mail.example.com",
            &["example.com".into()],
            Some("203.0.113.10"),
            &d,
        )
        .await;
        assert_eq!(names, vec!["mail.example.com", "autoconfig.example.com"]);
    }
}
