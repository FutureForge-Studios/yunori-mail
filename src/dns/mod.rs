//! DNS verification. Install gates and the console's DNS tables query
//! **public resolvers** (1.1.1.1 / 8.8.8.8 / 9.9.9.9) with caching disabled,
//! so we see what the world sees — not /etc/hosts (Openship's 127.0.1.1
//! trap). Blocklist queries go through the local resolver instead, because
//! Spamhaus refuses queries that arrive via public resolvers.

pub mod provider;
pub mod records;
pub mod reputation;
pub mod zone;

use async_trait::async_trait;
use hickory_resolver::TokioAsyncResolver;
use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use hickory_resolver::proto::rr::{RData, RecordType};
use std::net::IpAddr;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Srv {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// `Err` = the lookup itself failed (network/servfail) → "unknown", never "fail".
pub type LookupResult<T> = Result<Vec<T>, String>;

#[async_trait]
pub trait DnsLookup: Send + Sync {
    async fn a(&self, name: &str) -> LookupResult<String>;
    async fn aaaa(&self, name: &str) -> LookupResult<String>;
    async fn mx(&self, name: &str) -> LookupResult<(u16, String)>;
    async fn txt(&self, name: &str) -> LookupResult<String>;
    async fn cname(&self, name: &str) -> LookupResult<String>;
    async fn srv(&self, name: &str) -> LookupResult<Srv>;
    async fn ptr(&self, ip: IpAddr) -> LookupResult<String>;
    /// CAA records as `flags tag "value"` (RFC 8659 presentation format).
    async fn caa(&self, name: &str) -> LookupResult<String>;
}

pub fn norm_host(s: &str) -> String {
    s.trim().trim_end_matches('.').to_ascii_lowercase()
}

pub struct Resolver {
    inner: TokioAsyncResolver,
}

fn opts() -> ResolverOpts {
    let mut o = ResolverOpts::default();
    o.cache_size = 0;
    o.timeout = Duration::from_secs(3);
    o.attempts = 2;
    o.use_hosts_file = false;
    o
}

impl Resolver {
    /// Public, uncached resolvers — what the rest of the internet sees.
    pub fn public() -> Self {
        let ips: Vec<IpAddr> = ["1.1.1.1", "8.8.8.8", "9.9.9.9"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        let group = NameServerConfigGroup::from_ips_clear(&ips, 53, true);
        let config = ResolverConfig::from_parts(None, vec![], group);
        Self {
            inner: TokioAsyncResolver::tokio(config, opts()),
        }
    }

    /// The host's own resolver (unbound on a Yunori box) — used for DNSBLs.
    pub fn system() -> Self {
        match hickory_resolver::system_conf::read_system_conf() {
            Ok((config, mut o)) => {
                o.cache_size = 0;
                o.timeout = Duration::from_secs(3);
                Self {
                    inner: TokioAsyncResolver::tokio(config, o),
                }
            }
            Err(_) => Self::public(),
        }
    }

    /// Query a specific nameserver (e.g. OpenDNS for "what is my IP").
    pub fn at(ip: IpAddr) -> Self {
        let group = NameServerConfigGroup::from_ips_clear(&[ip], 53, true);
        Self {
            inner: TokioAsyncResolver::tokio(
                ResolverConfig::from_parts(None, vec![], group),
                opts(),
            ),
        }
    }
}

/// What the zone itself says, without any cache in between: names inside the
/// zone are asked of its authoritative nameservers; anything else, or any
/// failure to reach them, falls back to the public resolvers.
///
/// Install gates use this so that a record the person has just fixed is seen
/// at once, instead of the stale answer a public resolver may keep for the old
/// record's whole TTL (seen live: Cloudflare and Google had the new A record
/// while another public resolver still served the old one, and the host-record
/// gate kept waiting on it).
pub struct Truth {
    zone: Option<(String, Resolver)>,
    public: Resolver,
}

impl Truth {
    /// The zone that holds `name` and a resolver pointed at its nameservers.
    pub async fn for_name(name: &str) -> Self {
        let public = Resolver::public();
        let zone = find_zone_servers(&public, name).await;
        Self { zone, public }
    }

    /// The zone's own nameservers are being used (else: public resolvers only).
    pub fn is_authoritative(&self) -> bool {
        self.zone.is_some()
    }

    fn pick(&self, name: &str) -> Option<&Resolver> {
        let n = norm_host(name);
        self.zone
            .as_ref()
            .filter(|(z, _)| n == *z || n.ends_with(&format!(".{z}")))
            .map(|(_, r)| r)
    }
}

/// Walk up from `name` to the first label that has NS records (via the public
/// resolvers), then resolve those nameservers' addresses (at most 4).
async fn find_zone_servers(public: &Resolver, name: &str) -> Option<(String, Resolver)> {
    let host = norm_host(name);
    let parts: Vec<&str> = host.split('.').collect();
    // Never stop at a bare TLD.
    for i in 0..parts.len().saturating_sub(1) {
        let candidate = parts[i..].join(".");
        let Ok(ns) = public.inner.ns_lookup(fqdn(&candidate)).await else {
            continue;
        };
        let names: Vec<String> = ns.iter().map(|n| n.0.to_utf8()).collect();
        if names.is_empty() {
            continue;
        }
        let mut ips: Vec<IpAddr> = vec![];
        for n in names.iter().take(4) {
            if let Ok(a) = public.inner.ipv4_lookup(fqdn(n)).await {
                ips.extend(a.iter().map(|a| IpAddr::V4(a.0)));
            }
            if let Ok(a) = public.inner.ipv6_lookup(fqdn(n)).await {
                ips.extend(a.iter().map(|a| IpAddr::V6(a.0)));
            }
        }
        ips.truncate(8);
        if ips.is_empty() {
            return None;
        }
        let group = NameServerConfigGroup::from_ips_clear(&ips, 53, true);
        let mut o = opts();
        o.recursion_desired = false;
        return Some((
            candidate,
            Resolver {
                inner: TokioAsyncResolver::tokio(
                    ResolverConfig::from_parts(None, vec![], group),
                    o,
                ),
            },
        ));
    }
    None
}

/// Ask the zone's servers when `name` is in the zone, falling back to public.
macro_rules! truth_lookup {
    ($self:ident, $name:expr, $method:ident) => {{
        // A non-empty answer from the zone's servers is the record as it is
        // now (a public cache may still hold the old one). An empty answer or
        // an error is cross-checked with the public resolvers: some
        // nameservers intermittently answer empty (seen live on sslip.io).
        if let Some(r) = $self.pick($name) {
            if let Ok(v) = r.$method($name).await {
                if !v.is_empty() {
                    return Ok(v);
                }
            }
        }
        $self.public.$method($name).await
    }};
}

#[async_trait]
impl DnsLookup for Truth {
    async fn a(&self, name: &str) -> LookupResult<String> {
        truth_lookup!(self, name, a)
    }
    async fn aaaa(&self, name: &str) -> LookupResult<String> {
        truth_lookup!(self, name, aaaa)
    }
    async fn mx(&self, name: &str) -> LookupResult<(u16, String)> {
        truth_lookup!(self, name, mx)
    }
    async fn txt(&self, name: &str) -> LookupResult<String> {
        truth_lookup!(self, name, txt)
    }
    async fn cname(&self, name: &str) -> LookupResult<String> {
        truth_lookup!(self, name, cname)
    }
    async fn srv(&self, name: &str) -> LookupResult<Srv> {
        truth_lookup!(self, name, srv)
    }
    async fn ptr(&self, ip: IpAddr) -> LookupResult<String> {
        self.public.ptr(ip).await
    }
    async fn caa(&self, name: &str) -> LookupResult<String> {
        truth_lookup!(self, name, caa)
    }
}

fn empty_or_err<T>(e: ResolveError) -> LookupResult<T> {
    match e.kind() {
        ResolveErrorKind::NoRecordsFound { .. } => Ok(vec![]),
        _ => Err(e.to_string()),
    }
}

fn fqdn(name: &str) -> String {
    let n = name.trim_end_matches('.');
    format!("{n}.")
}

#[async_trait]
impl DnsLookup for Resolver {
    async fn a(&self, name: &str) -> LookupResult<String> {
        match self.inner.ipv4_lookup(fqdn(name)).await {
            Ok(r) => Ok(r.iter().map(|a| a.0.to_string()).collect()),
            Err(e) => empty_or_err(e),
        }
    }
    async fn aaaa(&self, name: &str) -> LookupResult<String> {
        match self.inner.ipv6_lookup(fqdn(name)).await {
            Ok(r) => Ok(r.iter().map(|a| a.0.to_string()).collect()),
            Err(e) => empty_or_err(e),
        }
    }
    async fn mx(&self, name: &str) -> LookupResult<(u16, String)> {
        match self.inner.mx_lookup(fqdn(name)).await {
            Ok(r) => Ok(r
                .iter()
                .map(|m| (m.preference(), norm_host(&m.exchange().to_utf8())))
                .collect()),
            Err(e) => empty_or_err(e),
        }
    }
    async fn txt(&self, name: &str) -> LookupResult<String> {
        match self.inner.txt_lookup(fqdn(name)).await {
            Ok(r) => Ok(r
                .iter()
                .map(|t| {
                    t.txt_data()
                        .iter()
                        .map(|s| String::from_utf8_lossy(s).into_owned())
                        .collect::<String>()
                })
                .collect()),
            Err(e) => empty_or_err(e),
        }
    }
    async fn cname(&self, name: &str) -> LookupResult<String> {
        match self.inner.lookup(fqdn(name), RecordType::CNAME).await {
            Ok(r) => Ok(r
                .record_iter()
                .filter_map(|rec| match rec.data() {
                    Some(RData::CNAME(c)) => Some(norm_host(&c.0.to_utf8())),
                    _ => None,
                })
                .collect()),
            Err(e) => empty_or_err(e),
        }
    }
    async fn srv(&self, name: &str) -> LookupResult<Srv> {
        match self.inner.srv_lookup(fqdn(name)).await {
            Ok(r) => Ok(r
                .iter()
                .map(|s| Srv {
                    priority: s.priority(),
                    weight: s.weight(),
                    port: s.port(),
                    target: norm_host(&s.target().to_utf8()),
                })
                .collect()),
            Err(e) => empty_or_err(e),
        }
    }
    async fn ptr(&self, ip: IpAddr) -> LookupResult<String> {
        match self.inner.reverse_lookup(ip).await {
            Ok(r) => Ok(r.iter().map(|n| norm_host(&n.to_utf8())).collect()),
            Err(e) => empty_or_err(e),
        }
    }
    async fn caa(&self, name: &str) -> LookupResult<String> {
        match self.inner.lookup(fqdn(name), RecordType::CAA).await {
            Ok(r) => Ok(r
                .record_iter()
                .filter_map(|rec| match rec.data() {
                    Some(RData::CAA(c)) => Some(c.to_string()),
                    _ => None,
                })
                .collect()),
            Err(e) => empty_or_err(e),
        }
    }
}

#[cfg(test)]
pub mod fake {
    //! Deterministic DNS for tests.
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct FakeDns {
        pub a: HashMap<String, Vec<String>>,
        pub aaaa: HashMap<String, Vec<String>>,
        pub mx: HashMap<String, Vec<(u16, String)>>,
        pub txt: HashMap<String, Vec<String>>,
        pub cname: HashMap<String, Vec<String>>,
        pub srv: HashMap<String, Vec<Srv>>,
        pub ptr: HashMap<String, Vec<String>>,
        pub caa: HashMap<String, Vec<String>>,
        pub failing: Vec<String>,
    }

    fn get<T: Clone>(m: &HashMap<String, Vec<T>>, failing: &[String], k: &str) -> LookupResult<T> {
        if failing.iter().any(|f| f == k) {
            return Err("SERVFAIL".into());
        }
        Ok(m.get(&norm_host(k)).cloned().unwrap_or_default())
    }

    #[async_trait]
    impl DnsLookup for FakeDns {
        async fn a(&self, n: &str) -> LookupResult<String> {
            get(&self.a, &self.failing, n)
        }
        async fn aaaa(&self, n: &str) -> LookupResult<String> {
            get(&self.aaaa, &self.failing, n)
        }
        async fn mx(&self, n: &str) -> LookupResult<(u16, String)> {
            get(&self.mx, &self.failing, n)
        }
        async fn txt(&self, n: &str) -> LookupResult<String> {
            get(&self.txt, &self.failing, n)
        }
        async fn cname(&self, n: &str) -> LookupResult<String> {
            get(&self.cname, &self.failing, n)
        }
        async fn srv(&self, n: &str) -> LookupResult<Srv> {
            get(&self.srv, &self.failing, n)
        }
        async fn ptr(&self, ip: IpAddr) -> LookupResult<String> {
            get(&self.ptr, &self.failing, &ip.to_string())
        }
        async fn caa(&self, n: &str) -> LookupResult<String> {
            get(&self.caa, &self.failing, n)
        }
    }
}

#[cfg(test)]
mod network_tests {
    //! Real lookups against public resolvers. Run with `cargo test -- --ignored`.
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn public_resolver_answers_real_queries() {
        let r = Resolver::public();
        let mx = r.mx("gmail.com").await.unwrap();
        assert!(mx.iter().any(|(_, h)| h.ends_with("google.com")), "{mx:?}");
        let txt = r.txt("gmail.com").await.unwrap();
        assert!(txt.iter().any(|t| t.starts_with("v=spf1")), "{txt:?}");
        let ptr = r.ptr("8.8.8.8".parse().unwrap()).await.unwrap();
        assert!(ptr.contains(&"dns.google".to_string()), "{ptr:?}");
        let srv = r.srv("_imaps._tcp.fastmail.com").await.unwrap();
        assert!(srv.iter().any(|s| s.port == 993), "{srv:?}");
        let none = r.a("does-not-exist.example.invalid").await.unwrap();
        assert!(none.is_empty());
    }
}
