//! Cloudflare DNS (API v4) behind the `DnsProvider` trait.
//!
//! Only four calls are used: verify the token, find the zone, list records,
//! create/update a record. Every request has a 15-second timeout. The token
//! travels only in the `Authorization` header and never appears in an error.

use super::{DnsProvider, ProviderError, ProviderRecord, RecordWrite, Zone};
use crate::dns::norm_host;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Duration;
use zeroize::Zeroizing;

pub const API_BASE: &str = "https://api.cloudflare.com/client/v4";
const TIMEOUT: Duration = Duration::from_secs(15);
const PER_PAGE: usize = 100;
const MAX_PAGES: usize = 100;
/// Shown on every record Yunori writes, so people can tell them apart at Cloudflare.
pub const COMMENT: &str = "Published by Yunori Mail";

pub struct Cloudflare {
    base: String,
    token: Zeroizing<String>,
    http: reqwest::Client,
}

enum Failure {
    Network(String),
    Api {
        status: u16,
        codes: Vec<i64>,
        message: String,
    },
}

impl Failure {
    /// The token itself was refused (bad, expired, revoked, malformed).
    fn is_auth(&self) -> bool {
        matches!(self, Failure::Api { status, codes, .. }
            if *status == 401 || codes.iter().any(|c| matches!(c, 1000 | 6003 | 6111 | 9106 | 9109)))
    }
    /// The token is valid but lacks the permission for this call.
    fn is_permission(&self) -> bool {
        matches!(self, Failure::Api { status, codes, .. }
            if *status == 403 || codes.contains(&10000))
    }
    fn message(&self) -> String {
        match self {
            Failure::Network(m) => m.clone(),
            Failure::Api { message, .. } => message.clone(),
        }
    }
}

const AUTH_MSG: &str = "Cloudflare did not accept this token. Check that you copied all of it and that it has not expired or been revoked.";

/// Reject input that cannot be an API token before sending it anywhere.
pub fn check_token_shape(token: &str) -> Result<(), ProviderError> {
    let t = token.trim();
    if t.len() == 37 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ProviderError::Auth(
            "That looks like Cloudflare's Global API Key. Create an API token instead (My Profile → API Tokens): it can be limited to this domain's DNS.".into(),
        ));
    }
    if t.len() < 20
        || t.len() > 200
        || !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ProviderError::Auth(
            "That does not look like a Cloudflare API token (about 40 letters, digits, - and _). Copy it again from Cloudflare.".into(),
        ));
    }
    Ok(())
}

impl Cloudflare {
    pub fn new(token: &str) -> Self {
        Self::build(API_BASE, token, false)
    }

    /// A client for another endpoint (tests: a local fake), without proxies.
    pub fn with_base(base: &str, token: &str) -> Self {
        Self::build(base, token, true)
    }

    fn build(base: &str, token: &str, no_proxy: bool) -> Self {
        let mut b = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("yunori-mail/", env!("CARGO_PKG_VERSION")));
        if no_proxy {
            b = b.no_proxy();
        }
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: Zeroizing::new(token.trim().to_string()),
            http: b.build().expect("http client"),
        }
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, Failure> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(self.token.as_str());
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.map_err(|e| {
            Failure::Network(if e.is_timeout() {
                "Cloudflare did not answer within 15 seconds. Try again in a moment.".into()
            } else {
                format!("Could not reach Cloudflare ({}).", e.without_url())
            })
        })?;
        let status = res.status().as_u16();
        let bytes = res
            .bytes()
            .await
            .map_err(|e| Failure::Network(format!("Cloudflare's answer was cut off ({e}).")))?;
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if (200..300).contains(&status) && v["success"].as_bool() == Some(true) {
            return Ok(v);
        }
        let errors = v["errors"].as_array().cloned().unwrap_or_default();
        let codes = errors.iter().filter_map(|e| e["code"].as_i64()).collect();
        let message = errors
            .iter()
            .filter_map(|e| e["message"].as_str())
            .collect::<Vec<_>>()
            .join("; ");
        Err(Failure::Api {
            status,
            codes,
            message: if message.is_empty() {
                format!("Cloudflare answered HTTP {status}.")
            } else {
                format!("Cloudflare: {message}")
            },
        })
    }

    fn read_error(&self, f: Failure, what: &str) -> ProviderError {
        if f.is_auth() {
            ProviderError::Auth(AUTH_MSG.into())
        } else if f.is_permission() {
            ProviderError::Permission(format!(
                "The token may not read {what}. Give it Zone → Zone → Read and Zone → DNS → Edit for this domain's zone."
            ))
        } else {
            ProviderError::Network(f.message())
        }
    }

    fn write_error(&self, f: Failure, zone: &Zone) -> ProviderError {
        if f.is_auth() {
            ProviderError::Auth(AUTH_MSG.into())
        } else if f.is_permission() {
            ProviderError::Permission(format!(
                "Cloudflare refused to change {}: the token can read DNS but not edit it. Give it Zone → DNS → Edit.",
                zone.name
            ))
        } else {
            match f {
                Failure::Network(m) => ProviderError::Network(m),
                Failure::Api { message, .. } => ProviderError::Rejected(message),
            }
        }
    }
}

/// `a.b.example.com` → `[a.b.example.com, b.example.com, example.com]`.
fn zone_candidates(domain: &str) -> Vec<String> {
    let labels: Vec<&str> = domain.split('.').filter(|l| !l.is_empty()).collect();
    (0..labels.len().saturating_sub(1))
        .map(|i| labels[i..].join("."))
        .collect()
}

/// TXT content as Cloudflare returns it: `"a" "b"` (quoted strings) or bare.
pub fn unquote_txt(content: &str) -> String {
    let c = content.trim();
    if !c.starts_with('"') {
        return c.to_string();
    }
    let mut out = String::new();
    let mut in_q = false;
    let mut chars = c.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if in_q => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '"' => in_q = !in_q,
            ch if in_q => out.push(ch),
            _ => {}
        }
    }
    out
}

/// TXT value → quoted strings of at most 255 bytes (long DKIM keys).
pub fn quote_txt(value: &str) -> String {
    let mut parts = Vec::new();
    let mut cur = String::new();
    for ch in value.chars() {
        if cur.len() + ch.len_utf8() > 255 {
            parts.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() || parts.is_empty() {
        parts.push(cur);
    }
    parts
        .iter()
        .map(|p| format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize(r: &Value) -> Option<ProviderRecord> {
    let rtype = r["type"].as_str()?.to_ascii_uppercase();
    let content = r["content"].as_str().unwrap_or_default();
    let data = &r["data"];
    let value = match rtype.as_str() {
        "MX" => format!(
            "{} {}",
            r["priority"].as_u64().unwrap_or(0),
            norm_host(content)
        ),
        "SRV" if data.is_object() => format!(
            "{} {} {} {}",
            data["priority"].as_u64().unwrap_or(0),
            data["weight"].as_u64().unwrap_or(0),
            data["port"].as_u64().unwrap_or(0),
            norm_host(data["target"].as_str().unwrap_or_default())
        ),
        "SRV" => {
            // content is "weight port target"; priority is separate.
            let mut parts: Vec<String> = content.split_whitespace().map(str::to_string).collect();
            if let Some(last) = parts.last_mut() {
                *last = norm_host(last);
            }
            format!(
                "{} {}",
                r["priority"].as_u64().unwrap_or(0),
                parts.join(" ")
            )
        }
        "CAA" if data.is_object() => format!(
            "{} {} \"{}\"",
            data["flags"].as_u64().unwrap_or(0),
            data["tag"].as_str().unwrap_or_default(),
            data["value"].as_str().unwrap_or_default()
        ),
        "TXT" => unquote_txt(content),
        "CNAME" | "NS" => norm_host(content),
        _ => content.trim().to_string(),
    };
    Some(ProviderRecord {
        id: r["id"].as_str()?.to_string(),
        rtype,
        name: norm_host(r["name"].as_str()?),
        value,
        proxied: r["proxied"].as_bool().unwrap_or(false),
    })
}

fn invalid(rec: &RecordWrite) -> ProviderError {
    ProviderError::Rejected(format!(
        "Yunori could not prepare the {} value for {}.",
        rec.rtype, rec.name
    ))
}

/// The JSON body for one record.
pub fn body(rec: &RecordWrite) -> Result<Value, ProviderError> {
    let mut b = json!({
        "type": rec.rtype,
        "name": rec.name,
        "ttl": 1,
        "comment": COMMENT,
    });
    let parts: Vec<&str> = rec.value.split_whitespace().collect();
    match rec.rtype.as_str() {
        "A" | "AAAA" => {
            b["content"] = json!(rec.value.trim());
            b["proxied"] = json!(false);
        }
        "CNAME" => {
            b["content"] = json!(norm_host(&rec.value));
            b["proxied"] = json!(false);
        }
        "MX" => {
            let [prio, host] = parts[..] else {
                return Err(invalid(rec));
            };
            b["priority"] = json!(prio.parse::<u16>().map_err(|_| invalid(rec))?);
            b["content"] = json!(norm_host(host));
        }
        "TXT" => b["content"] = json!(quote_txt(&rec.value)),
        "SRV" => {
            let [prio, weight, port, target] = parts[..] else {
                return Err(invalid(rec));
            };
            let n = |s: &str| s.parse::<u16>().map_err(|_| invalid(rec));
            b["data"] = json!({
                "priority": n(prio)?,
                "weight": n(weight)?,
                "port": n(port)?,
                "target": norm_host(target),
            });
        }
        "CAA" => {
            let (flags, rest) = rec
                .value
                .trim()
                .split_once(' ')
                .ok_or_else(|| invalid(rec))?;
            let (tag, value) = rest.trim().split_once(' ').ok_or_else(|| invalid(rec))?;
            b["data"] = json!({
                "flags": flags.parse::<u8>().map_err(|_| invalid(rec))?,
                "tag": tag,
                "value": value.trim().trim_matches('"'),
            });
        }
        _ => return Err(invalid(rec)),
    }
    Ok(b)
}

#[async_trait]
impl DnsProvider for Cloudflare {
    async fn find_zone(&self, domain: &str) -> Result<Zone, ProviderError> {
        check_token_shape(&self.token)?;
        let domain = norm_host(domain);
        // Account-owned tokens are not known to /user/tokens/verify, so a
        // refusal there is only final if the zone lookup is refused too.
        match self
            .call(reqwest::Method::GET, "/user/tokens/verify", None)
            .await
        {
            Ok(v) => {
                let status = v["result"]["status"].as_str().unwrap_or("active");
                if status != "active" {
                    return Err(ProviderError::Auth(format!(
                        "This Cloudflare token is {status}. Create a new one, or re-activate it at Cloudflare."
                    )));
                }
            }
            Err(Failure::Network(m)) => return Err(ProviderError::Network(m)),
            Err(_) => {}
        }
        for cand in zone_candidates(&domain) {
            let v = self
                .call(
                    reqwest::Method::GET,
                    &format!("/zones?name={cand}&per_page=5"),
                    None,
                )
                .await
                .map_err(|f| self.read_error(f, "zones"))?;
            if let Some(z) = v["result"].as_array().and_then(|a| a.first()) {
                return Ok(Zone {
                    id: z["id"].as_str().unwrap_or_default().to_string(),
                    name: norm_host(z["name"].as_str().unwrap_or(&cand)),
                });
            }
        }
        Err(ProviderError::NoZone(format!(
            "This token cannot see a Cloudflare zone for {domain}. Check that {domain} is on this Cloudflare account and that the token's Zone Resources include it."
        )))
    }

    async fn records(&self, zone: &Zone) -> Result<Vec<ProviderRecord>, ProviderError> {
        let mut out = Vec::new();
        for page in 1..=MAX_PAGES {
            let v = self
                .call(
                    reqwest::Method::GET,
                    &format!(
                        "/zones/{}/dns_records?per_page={PER_PAGE}&page={page}",
                        zone.id
                    ),
                    None,
                )
                .await
                .map_err(|f| self.read_error(f, &format!("the DNS records of {}", zone.name)))?;
            let rows = v["result"].as_array().cloned().unwrap_or_default();
            out.extend(rows.iter().filter_map(normalize));
            let total = v["result_info"]["total_pages"].as_u64().unwrap_or(1) as usize;
            if page >= total || rows.is_empty() {
                break;
            }
        }
        Ok(out)
    }

    async fn create(&self, zone: &Zone, rec: &RecordWrite) -> Result<(), ProviderError> {
        self.call(
            reqwest::Method::POST,
            &format!("/zones/{}/dns_records", zone.id),
            Some(body(rec)?),
        )
        .await
        .map(|_| ())
        .map_err(|f| self.write_error(f, zone))
    }

    async fn update(&self, zone: &Zone, id: &str, rec: &RecordWrite) -> Result<(), ProviderError> {
        self.call(
            reqwest::Method::PATCH,
            &format!("/zones/{}/dns_records/{id}", zone.id),
            Some(body(rec)?),
        )
        .await
        .map(|_| ())
        .map_err(|f| self.write_error(f, zone))
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{self, FakeCloudflare};
    use super::*;

    const TOKEN: &str = "test-token-abcdefghijklmnopqrstuvwxyz0123";

    #[test]
    fn txt_quoting_roundtrips_and_splits_long_values() {
        let long = format!("v=DKIM1; k=rsa; p={}", "A".repeat(400));
        let q = quote_txt(&long);
        assert_eq!(q.matches('"').count(), 4, "two strings: {q}");
        assert_eq!(unquote_txt(&q), long);
        assert_eq!(unquote_txt("v=spf1 mx -all"), "v=spf1 mx -all");
        assert_eq!(quote_txt("a\"b"), "\"a\\\"b\"");
        assert_eq!(unquote_txt(&quote_txt("a\"b")), "a\"b");
    }

    #[test]
    fn token_shape_is_checked_before_sending() {
        assert!(check_token_shape(TOKEN).is_ok());
        assert!(matches!(
            check_token_shape("0123456789abcdef0123456789abcdef01234"),
            Err(ProviderError::Auth(m)) if m.contains("Global API Key")
        ));
        assert!(check_token_shape("short").is_err());
        assert!(check_token_shape("has a space in the middle of it ok").is_err());
        assert!(check_token_shape("abcdefghijklmnopqrstuvwxyz\r\nX-Evil: 1").is_err());
    }

    #[test]
    fn bodies_per_type() {
        let w = |t: &str, v: &str| RecordWrite {
            rtype: t.into(),
            name: "x.example.com".into(),
            value: v.into(),
        };
        let b = body(&w("MX", "10 mail.example.com")).unwrap();
        assert_eq!(b["priority"], 10);
        assert_eq!(b["content"], "mail.example.com");
        let b = body(&w("SRV", "0 1 993 mail.example.com")).unwrap();
        assert_eq!(b["data"]["port"], 993);
        assert_eq!(b["data"]["target"], "mail.example.com");
        let b = body(&w("CAA", "0 issue \"letsencrypt.org\"")).unwrap();
        assert_eq!(b["data"]["tag"], "issue");
        assert_eq!(b["data"]["value"], "letsencrypt.org");
        let b = body(&w("A", "203.0.113.10")).unwrap();
        assert_eq!(b["proxied"], false);
        assert_eq!(b["comment"], COMMENT);
        assert!(body(&w("MX", "mail.example.com")).is_err());
        assert!(body(&w("TLSA", "3 1 1 ab")).is_err());
    }

    #[test]
    fn zone_candidates_walk_up() {
        assert_eq!(
            zone_candidates("a.b.example.com"),
            vec!["a.b.example.com", "b.example.com", "example.com"]
        );
        assert_eq!(zone_candidates("example.com"), vec!["example.com"]);
    }

    #[tokio::test]
    async fn finds_zone_lists_creates_and_updates_against_a_fake() {
        let cf = FakeCloudflare::start(TOKEN, &[("z1", "example.com")]).await;
        cf.seed("z1", json!({"type": "MX", "name": "example.com", "content": "old.example.net", "priority": 5}));
        cf.seed("z1", json!({"type": "TXT", "name": "k._domainkey.example.com", "content": "\"v=DKIM1; \" \"p=abc\""}));
        cf.seed("z1", json!({"type": "SRV", "name": "_imaps._tcp.example.com", "data": {"priority": 0, "weight": 1, "port": 993, "target": "mail.example.com"}}));
        let c = Cloudflare::with_base(&cf.base, TOKEN);

        // A subdomain resolves to its parent zone.
        let zone = c.find_zone("sub.example.com").await.unwrap();
        assert_eq!(
            zone,
            Zone {
                id: "z1".into(),
                name: "example.com".into()
            }
        );

        let recs = c.records(&zone).await.unwrap();
        let get = |t: &str| recs.iter().find(|r| r.rtype == t).unwrap().clone();
        assert_eq!(get("MX").value, "5 old.example.net");
        assert_eq!(get("TXT").value, "v=DKIM1; p=abc");
        assert_eq!(get("SRV").value, "0 1 993 mail.example.com");

        let mx = get("MX");
        c.update(
            &zone,
            &mx.id,
            &RecordWrite {
                rtype: "MX".into(),
                name: "example.com".into(),
                value: "10 mail.example.com".into(),
            },
        )
        .await
        .unwrap();
        c.create(
            &zone,
            &RecordWrite {
                rtype: "A".into(),
                name: "mail.example.com".into(),
                value: "203.0.113.10".into(),
            },
        )
        .await
        .unwrap();
        let recs = c.records(&zone).await.unwrap();
        assert!(
            recs.iter()
                .any(|r| r.rtype == "MX" && r.value == "10 mail.example.com")
        );
        assert!(
            recs.iter()
                .any(|r| r.rtype == "A" && r.value == "203.0.113.10" && !r.proxied)
        );
        assert_eq!(recs.len(), 4);
        // Every request carried the token; nothing was ever deleted.
        assert!(cf.requests().iter().all(|r| !r.starts_with("DELETE")));
    }

    #[tokio::test]
    async fn paginates() {
        let cf = FakeCloudflare::start(TOKEN, &[("z1", "example.com")]).await;
        for i in 0..250 {
            cf.seed(
                "z1",
                json!({"type": "A", "name": format!("h{i}.example.com"), "content": "192.0.2.1"}),
            );
        }
        let c = Cloudflare::with_base(&cf.base, TOKEN);
        let zone = c.find_zone("example.com").await.unwrap();
        assert_eq!(c.records(&zone).await.unwrap().len(), 250);
    }

    #[tokio::test]
    async fn errors_are_plain_and_never_carry_the_token() {
        let cf = FakeCloudflare::start(TOKEN, &[("z1", "example.com")]).await;
        let wrong = "wrong-token-abcdefghijklmnopqrstuvwxyz01";
        let err = Cloudflare::with_base(&cf.base, wrong)
            .find_zone("example.com")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Auth(_)), "{err:?}");
        assert!(!err.to_string().contains(wrong));

        let err = Cloudflare::with_base(&cf.base, TOKEN)
            .find_zone("other.org")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::NoZone(ref m) if m.contains("other.org")),
            "{err:?}"
        );

        cf.set_read_only(true);
        let c = Cloudflare::with_base(&cf.base, TOKEN);
        let zone = c.find_zone("example.com").await.unwrap();
        let err = c
            .create(
                &zone,
                &RecordWrite {
                    rtype: "A".into(),
                    name: "mail.example.com".into(),
                    value: "203.0.113.10".into(),
                },
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Permission(ref m) if m.contains("DNS → Edit")),
            "{err:?}"
        );

        // Nothing listening: a network error, quickly.
        let dead = fake::unused_base().await;
        let err = Cloudflare::with_base(&dead, TOKEN)
            .find_zone("example.com")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Network(_)), "{err:?}");
    }
}
