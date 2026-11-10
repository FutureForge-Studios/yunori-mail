//! DNS automation through a DNS provider's API.
//!
//! Yunori writes **exactly** the rows of `records::expected_records` that are
//! missing or wrong at the provider, after showing a preview (`plan`). It never
//! deletes anything: the `DnsProvider` trait has no delete operation at all.
//! The public-resolver check stays the gate: publishing is followed by the
//! same `records::evaluate` run as a manual "Check now".
//!
//! Why Yunori calls the provider itself instead of handing Stalwart the token
//! (Stalwart 1.0.3 has `x:DnsServer` + `Domain.dnsManagement`): Stalwart
//! publishes *its* record set (DMARC `p=reject`, no relay `include:`, no relay
//! CNAMEs, no host A/AAAA), cannot show a preview, reports only a task status,
//! and talks to a hard-coded provider endpoint. See docs/stalwart-integration.md.

pub mod cloudflare;
#[cfg(test)]
pub mod fake;
#[cfg(test)]
mod live;
pub mod plan;

use async_trait::async_trait;
use serde::Serialize;

/// One record as the provider holds it, normalised to the presentation
/// `expected_records` uses (MX `10 host`, SRV `prio weight port target`,
/// CAA `0 issue "ca"`, TXT strings unquoted and joined, hosts without a dot).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderRecord {
    pub id: String,
    pub rtype: String,
    /// Fully qualified, lower case, no trailing dot.
    pub name: String,
    pub value: String,
    /// Cloudflare's "orange cloud" (mail must never go through it).
    pub proxied: bool,
}

/// A record to create, or the new content of one to update.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordWrite {
    pub rtype: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    pub id: String,
    pub name: String,
}

/// Every message is safe to show a person and never contains the token.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    /// The provider did not accept the token at all.
    #[error("{0}")]
    Auth(String),
    /// The token works but may not do this (e.g. read-only DNS).
    #[error("{0}")]
    Permission(String),
    /// No zone this token can see covers the domain.
    #[error("{0}")]
    NoZone(String),
    /// Timeout, connection refused, unexpected answer.
    #[error("{0}")]
    Network(String),
    /// The provider refused this one change (its own words).
    #[error("{0}")]
    Rejected(String),
}

#[async_trait]
pub trait DnsProvider: Send + Sync {
    /// Check the token (read-only) and find the zone that holds `domain`.
    async fn find_zone(&self, domain: &str) -> Result<Zone, ProviderError>;
    /// Every record in the zone (read-only).
    async fn records(&self, zone: &Zone) -> Result<Vec<ProviderRecord>, ProviderError>;
    async fn create(&self, zone: &Zone, rec: &RecordWrite) -> Result<(), ProviderError>;
    async fn update(&self, zone: &Zone, id: &str, rec: &RecordWrite) -> Result<(), ProviderError>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: &'static str,
    pub label: &'static str,
    /// How to make a token with the least privilege that works.
    pub token_help: &'static str,
    pub token_url: &'static str,
}

pub const PROVIDERS: &[ProviderInfo] = &[ProviderInfo {
    id: "cloudflare",
    label: "Cloudflare",
    token_help: "In Cloudflare, open My Profile → API Tokens → Create Token, start from \"Edit zone DNS\" and limit it to this domain's zone (Zone → DNS → Edit; Zone → Zone → Read).",
    token_url: "https://dash.cloudflare.com/profile/api-tokens",
}];

pub fn info(id: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// A client for `provider` authenticated with `token`.
pub fn client(provider: &str, token: &str) -> Option<Box<dyn DnsProvider>> {
    match provider {
        "cloudflare" => Some(Box::new(match test_base() {
            Some(base) => cloudflare::Cloudflare::with_base(&base, token),
            None => cloudflare::Cloudflare::new(token),
        })),
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    /// Tests point the Cloudflare client at a local fake (`fake.rs`).
    pub static CLOUDFLARE_BASE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn test_base() -> Option<String> {
    CLOUDFLARE_BASE.with(|b| b.borrow().clone())
}

#[cfg(not(test))]
fn test_base() -> Option<String> {
    None
}
