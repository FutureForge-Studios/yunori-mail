//! Login abuse: the addresses the engine has blocked, the allowlist that is
//! never blocked, and the automatic-ban policy for failed sign-ins.
//!
//! Stalwart bans an address on its own after repeated failed IMAP/SMTP/JMAP
//! sign-ins (`x:Security.authBanRate`); its 1.0.3 default is 100 failures a day
//! and a ban that never expires. Yunori sets a tighter, self-healing policy
//! ([`YUNORI_AUTH_BAN`]) once, at install and on the first start after an
//! upgrade, unless someone already chose their own.

use crate::app::App;
use crate::engine::{BanRate, EAllowedIp, EAuthBanPolicy, EBlockedIp, EngineError};
use anyhow::Result;
use serde::Serialize;
use std::net::IpAddr;

/// Stalwart 1.0.3's `authBanRate` default (verified on a live engine), with
/// `authBanPeriod: null` (bans are permanent).
pub const STALWART_DEFAULT_RATE: BanRate = BanRate {
    count: 100,
    period_ms: 86_400_000,
};

/// 20 failed sign-ins within an hour → blocked for a day. Slow enough to stop
/// password guessing (at most ~480 guesses a day per address), and a phone
/// stuck on an old password heals by itself the next day.
pub const YUNORI_AUTH_BAN: EAuthBanPolicy = EAuthBanPolicy {
    rate: Some(BanRate {
        count: 20,
        period_ms: 3_600_000,
    }),
    ban_for_ms: Some(86_400_000),
};

/// `kv` marker: the policy was checked (and set, if it was the stock default).
const POLICY_KEY: &str = "engine.auth_ban_policy";
const POLICY_VERSION: &str = "v1";

/// Validate an IP address or CIDR network; returns it in canonical form
/// (`10.0.0.7/24` → `10.0.0.0/24`, a full-length prefix is dropped).
pub fn normalize_network(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter an IP address or a network such as 203.0.113.0/24.".into());
    }
    let (ip_part, prefix_part) = match input.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (input, None),
    };
    let ip: IpAddr = ip_part.parse().map_err(|_| {
        format!("{input} isn't an IP address or network (like 203.0.113.7 or 203.0.113.0/24).")
    })?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    let prefix: u8 = match prefix_part {
        None => max,
        Some(p) => p
            .parse()
            .ok()
            .filter(|p| *p <= max)
            .ok_or_else(|| format!("/{p} isn't a valid prefix length (0–{max})."))?,
    };
    if ip.is_unspecified() {
        return Err("That would match every address.".into());
    }
    // Allowlisting a huge range would switch banning off for much of the internet.
    let min = if ip.is_ipv4() { 8 } else { 16 };
    if prefix < min {
        return Err(format!(
            "/{prefix} is too broad: use a network of /{min} or narrower."
        ));
    }
    let net = mask(ip, prefix);
    Ok(if prefix == max {
        net.to_string()
    } else {
        format!("{net}/{prefix}")
    })
}

fn mask(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            let m = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            IpAddr::V4((bits & m).into())
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let m = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            IpAddr::V6((bits & m).into())
        }
    }
}

/// Why the engine blocked an address, in plain words.
pub fn reason_text(reason: &str) -> &'static str {
    match reason {
        "authFailure" => "Too many failed sign-ins",
        "rcptToFailure" => "Too many attempts to send to addresses that don't exist",
        "loitering" => "Kept opening connections without doing anything",
        "portScanning" => "Probed the web server for known exploits",
        "manual" => "Blocked by an administrator",
        _ => "Blocked by the mail engine",
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedView {
    pub id: String,
    pub address: String,
    pub reason: String,
    pub reason_text: &'static str,
    pub created_at: Option<String>,
    /// `None`: until someone unblocks it.
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyView {
    pub enabled: bool,
    pub failures: Option<u64>,
    pub window_minutes: Option<u64>,
    /// `None`: bans never expire.
    pub ban_minutes: Option<u64>,
    /// Whether this is Yunori's recommended policy.
    pub recommended: bool,
}

impl From<&EAuthBanPolicy> for PolicyView {
    fn from(p: &EAuthBanPolicy) -> Self {
        PolicyView {
            enabled: p.rate.is_some(),
            failures: p.rate.map(|r| r.count),
            window_minutes: p.rate.map(|r| r.period_ms / 60_000),
            ban_minutes: p.ban_for_ms.map(|ms| ms / 60_000),
            recommended: *p == YUNORI_AUTH_BAN,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessView {
    pub blocked: Vec<BlockedView>,
    pub allowed: Vec<AllowedView>,
    pub policy: Option<PolicyView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowedView {
    pub id: String,
    pub address: String,
    pub reason: Option<String>,
    pub created_at: Option<String>,
}

fn blocked_view(b: &EBlockedIp) -> BlockedView {
    BlockedView {
        id: b.id.clone(),
        address: b.address.clone(),
        reason: b.reason.clone(),
        reason_text: reason_text(&b.reason),
        created_at: b.created_at.clone(),
        expires_at: b.expires_at.clone(),
    }
}

fn allowed_view(a: &EAllowedIp) -> AllowedView {
    AllowedView {
        id: a.id.clone(),
        address: a.address.clone(),
        reason: a.reason.clone(),
        created_at: a.created_at.clone(),
    }
}

pub async fn view(app: &App) -> Result<AccessView> {
    let e = app.engine()?;
    let mut blocked: Vec<BlockedView> = e.blocked_ips().await?.iter().map(blocked_view).collect();
    // Newest first.
    blocked.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let mut allowed: Vec<AllowedView> = e.allowed_ips().await?.iter().map(allowed_view).collect();
    allowed.sort_by(|a, b| a.address.cmp(&b.address));
    let policy = e.auth_ban_policy().await.ok().map(|p| PolicyView::from(&p));
    Ok(AccessView {
        blocked,
        allowed,
        policy,
    })
}

/// A short free-text note: one line, no control characters, at most 120 chars.
pub fn clean_note(note: Option<&str>) -> Option<String> {
    let n: String = note?
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    let n = n.trim().to_string();
    (!n.is_empty()).then_some(n)
}

/// Unblock by engine id or by address.
pub async fn unblock(app: &App, actor: &str, id_or_address: &str) -> Result<BlockedView> {
    let e = app.engine()?;
    let wanted = normalize_network(id_or_address).ok();
    let entry = e
        .blocked_ips()
        .await?
        .into_iter()
        .find(|b| {
            b.id == id_or_address
                || wanted.as_deref() == Some(b.address.as_str())
                || b.address == id_or_address
        })
        .ok_or(EngineError::NotFound)?;
    e.unblock_ip(&entry.id).await?;
    app.db.audit(
        actor,
        "security.unblock",
        Some(&entry.address),
        Some(reason_text(&entry.reason)),
    )?;
    Ok(blocked_view(&entry))
}

#[derive(Debug, thiserror::Error)]
pub enum AllowError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0} is already on the allowlist.")]
    Duplicate(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<EngineError> for AllowError {
    fn from(e: EngineError) -> Self {
        AllowError::Other(e.into())
    }
}

/// Never block this address or network. Also lifts a current block on it.
pub async fn allow(
    app: &App,
    actor: &str,
    address: &str,
    note: Option<&str>,
) -> Result<AllowedView, AllowError> {
    let net = normalize_network(address).map_err(AllowError::Invalid)?;
    let e = app.engine()?;
    if e.allowed_ips().await?.iter().any(|a| a.address == net) {
        return Err(AllowError::Duplicate(net));
    }
    let note = clean_note(note);
    let created = e.allow_ip(&net, note.as_deref()).await?;
    app.db
        .audit(actor, "security.allow", Some(&net), note.as_deref())?;
    // A block placed before the address was allowlisted is its own record:
    // lift an exact-match one too, so "allow" means "can sign in now".
    if let Some(b) = e
        .blocked_ips()
        .await?
        .into_iter()
        .find(|b| b.address == net)
    {
        e.unblock_ip(&b.id).await?;
        app.db.audit(
            actor,
            "security.unblock",
            Some(&b.address),
            Some("added to the allowlist"),
        )?;
    }
    Ok(allowed_view(&created))
}

/// Remove an allowlist entry, by engine id or by address.
pub async fn disallow(app: &App, actor: &str, id_or_address: &str) -> Result<String> {
    let e = app.engine()?;
    let wanted = normalize_network(id_or_address).ok();
    let entry = e
        .allowed_ips()
        .await?
        .into_iter()
        .find(|a| a.id == id_or_address || wanted.as_deref() == Some(a.address.as_str()))
        .ok_or(EngineError::NotFound)?;
    e.remove_allowed_ip(&entry.id).await?;
    app.db
        .audit(actor, "security.allow_remove", Some(&entry.address), None)?;
    Ok(entry.address)
}

/// Make sure failed sign-ins lead to a ban with Yunori's policy — once. If the
/// engine still has Stalwart's stock setting (or banning is off), set
/// [`YUNORI_AUTH_BAN`]; a policy someone chose themselves is left alone.
/// Idempotent; returns whether it changed anything.
pub async fn ensure_auth_ban_policy(app: &App) -> Result<bool> {
    if app.db.get_kv(POLICY_KEY)?.as_deref() == Some(POLICY_VERSION) {
        return Ok(false);
    }
    let e = app.engine()?;
    let current = e.auth_ban_policy().await?;
    let stock = current.ban_for_ms.is_none()
        && (current.rate.is_none() || current.rate == Some(STALWART_DEFAULT_RATE));
    let changed = stock && current != YUNORI_AUTH_BAN;
    if changed {
        e.set_auth_ban_policy(&YUNORI_AUTH_BAN).await?;
        app.db.audit(
            "system",
            "security.auth_ban_policy",
            Some("engine"),
            Some("20 failed sign-ins within an hour → blocked for a day"),
        )?;
    }
    app.db.set_kv(POLICY_KEY, POLICY_VERSION)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks_are_validated_and_canonical() {
        assert_eq!(normalize_network(" 203.0.113.7 ").unwrap(), "203.0.113.7");
        assert_eq!(normalize_network("203.0.113.7/32").unwrap(), "203.0.113.7");
        assert_eq!(normalize_network("10.77.0.5/24").unwrap(), "10.77.0.0/24");
        assert_eq!(
            normalize_network("2001:DB8::1/48").unwrap(),
            "2001:db8::/48"
        );
        assert_eq!(normalize_network("2001:db8::1").unwrap(), "2001:db8::1");
        for bad in [
            "",
            "not-an-ip",
            "10.0.0.0/33",
            "10.0.0.0/x",
            "999.1.1.1",
            "0.0.0.0",
            "::",
            "10.0.0.0/4",
            "2001:db8::/8",
            "1.2.3.4/24/8",
        ] {
            assert!(normalize_network(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn reasons_read_as_sentences() {
        assert_eq!(reason_text("authFailure"), "Too many failed sign-ins");
        assert_eq!(reason_text("somethingNew"), "Blocked by the mail engine");
    }

    #[test]
    fn notes_are_one_short_line() {
        assert_eq!(
            clean_note(Some("  office\nVPN ")).as_deref(),
            Some("officeVPN")
        );
        assert_eq!(clean_note(Some("   ")), None);
        assert_eq!(clean_note(Some(&"x".repeat(500))).unwrap().len(), 120);
    }

    #[test]
    fn policy_view() {
        let v = PolicyView::from(&YUNORI_AUTH_BAN);
        assert!(v.enabled && v.recommended);
        assert_eq!(
            (v.failures, v.window_minutes, v.ban_minutes),
            (Some(20), Some(60), Some(1440))
        );
        let stock = PolicyView::from(&EAuthBanPolicy {
            rate: Some(STALWART_DEFAULT_RATE),
            ban_for_ms: None,
        });
        assert!(!stock.recommended);
        assert_eq!(stock.ban_minutes, None);
    }

    #[tokio::test]
    async fn the_policy_is_set_once_and_custom_policies_are_kept() {
        let app = App::for_tests_isolated();
        assert!(ensure_auth_ban_policy(&app).await.unwrap());
        let e = app.engine().unwrap();
        assert_eq!(e.auth_ban_policy().await.unwrap(), YUNORI_AUTH_BAN);
        assert!(!ensure_auth_ban_policy(&app).await.unwrap(), "idempotent");

        // Someone chose their own policy before the upgrade: keep it.
        let app = App::for_tests_isolated();
        let custom = EAuthBanPolicy {
            rate: Some(BanRate {
                count: 5,
                period_ms: 600_000,
            }),
            ban_for_ms: None,
        };
        let e = app.engine().unwrap();
        e.set_auth_ban_policy(&custom).await.unwrap();
        assert!(!ensure_auth_ban_policy(&app).await.unwrap());
        assert_eq!(e.auth_ban_policy().await.unwrap(), custom);
    }
}
