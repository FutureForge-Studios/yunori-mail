//! The engine seam (plan §5.2): everything `yunori-mail` needs from the mail
//! server, behind one trait. Only the Stalwart adapter ships; the in-memory
//! mock backs `serve --dev --mock-engine` and the test-suite. A mox adapter
//! could slot in here without touching the API or installer layers.

pub mod jmap;
#[cfg(test)]
mod live_onboarding_tests;
#[cfg(test)]
mod live_queue_tests;
#[cfg(test)]
pub(crate) mod live_tests;
pub mod mail;
pub mod mock;
pub mod stalwart;
mod stalwart_ops;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("the mail engine is not reachable: {0}")]
    Unavailable(String),
    #[error("the mail engine rejected the credentials")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Rejected(String),
    #[error("engine protocol error: {0}")]
    Protocol(String),
}

pub type EngineResult<T> = Result<T, EngineError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EDomain {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub catch_all: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EAlias {
    /// Engine-side list key (Stalwart objectList index).
    pub key: String,
    pub local: String,
    pub domain_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EAccount {
    pub id: String,
    pub local: String,
    pub domain_id: String,
    pub address: String,
    pub display_name: Option<String>,
    pub is_admin: bool,
    pub quota_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
    pub aliases: Vec<EAlias>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum RcptState {
    #[default]
    Scheduled,
    Completed,
    TemporaryFailure,
    PermanentFailure,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EQueuedRecipient {
    pub address: String,
    pub state: RcptState,
    /// Classified failure kind from the engine (e.g. "tlsError", "connectionError").
    pub error_type: Option<String>,
    pub response: Option<String>,
    pub retry_count: u64,
    /// The remote host that answered (or could not be reached).
    pub remote_host: Option<String>,
    pub next_retry: Option<String>,
    /// When the engine gives up on this recipient (TTL expiry) …
    pub expires_at: Option<String>,
    /// … or after how many attempts (attempt-based expiry).
    pub expires_after_attempts: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EQueued {
    pub id: String,
    pub created_at: Option<String>,
    pub next_retry: Option<String>,
    pub recipients: Vec<EQueuedRecipient>,
    /// Envelope sender (MAIL FROM); empty for bounces.
    pub sender: Option<String>,
    pub size: Option<u64>,
}

/// An address the engine refuses connections from (Stalwart `x:BlockedIp`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EBlockedIp {
    pub id: String,
    pub address: String,
    /// `authFailure`, `rcptToFailure`, `loitering`, `portScanning`, `manual`, `other`.
    pub reason: String,
    pub created_at: Option<String>,
    /// `None`: blocked until someone unblocks it.
    pub expires_at: Option<String>,
}

/// An address or network that is never blocked (Stalwart `x:AllowedIp`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EAllowedIp {
    pub id: String,
    pub address: String,
    pub reason: Option<String>,
    pub created_at: Option<String>,
    pub expires_at: Option<String>,
}

/// "`count` events within `period_ms`".
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BanRate {
    pub count: u64,
    pub period_ms: u64,
}

/// Automatic banning after failed sign-ins (Stalwart `x:Security`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EAuthBanPolicy {
    /// `None`: failed sign-ins never lead to a ban.
    pub rate: Option<BanRate>,
    /// `None`: bans never expire.
    pub ban_for_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EAppPassword {
    pub id: String,
    pub description: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelayConfig {
    pub host: String,
    pub port: u16,
    pub implicit_tls: bool,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInfo {
    pub reachable: bool,
    pub version: Option<String>,
}

/// Credentials of a mailbox user, used only for that user's own self-service.
#[derive(Clone)]
pub struct UserCreds {
    pub email: String,
    pub password: zeroize::Zeroizing<String>,
}

#[async_trait]
pub trait Engine: Send + Sync {
    async fn info(&self) -> EngineInfo;

    // domains
    async fn list_domains(&self) -> EngineResult<Vec<EDomain>>;
    async fn create_domain(&self, name: &str) -> EngineResult<EDomain>;
    async fn update_domain(
        &self,
        id: &str,
        catch_all: Option<Option<String>>,
        enabled: Option<bool>,
    ) -> EngineResult<EDomain>;
    async fn delete_domain(&self, id: &str) -> EngineResult<()>;
    /// BIND-style zone data the engine expects to be published for the domain.
    async fn zone_file(&self, id: &str) -> EngineResult<String>;
    /// Rename a domain in place. Its DKIM keys and accounts follow it, so the
    /// administrator's sign-in becomes `admin@<new name>` (verified on 1.0.3:
    /// the old name keeps signing in only until the engine restarts).
    async fn rename_domain(&self, id: &str, name: &str) -> EngineResult<EDomain>;
    /// The engine's own name (SMTP greeting, MX and SRV targets in the zone
    /// file). Applied at once, without a restart.
    async fn set_hostname(&self, hostname: &str) -> EngineResult<()>;
    /// The engine's own name as it is configured now.
    async fn hostname(&self) -> EngineResult<String>;

    // accounts
    async fn list_accounts(&self) -> EngineResult<Vec<EAccount>>;
    async fn create_account(
        &self,
        local: &str,
        domain_id: &str,
        display_name: Option<&str>,
        quota_bytes: Option<u64>,
        password: &str,
    ) -> EngineResult<EAccount>;
    async fn update_account(
        &self,
        id: &str,
        display_name: Option<Option<String>>,
        quota_bytes: Option<Option<u64>>,
    ) -> EngineResult<EAccount>;
    async fn set_password(&self, id: &str, password: &str) -> EngineResult<()>;
    async fn delete_account(&self, id: &str) -> EngineResult<()>;
    async fn add_alias(&self, account_id: &str, local: &str, domain_id: &str) -> EngineResult<()>;
    async fn remove_alias(&self, account_id: &str, alias_key: &str) -> EngineResult<()>;

    // operations
    async fn queue(&self) -> EngineResult<Vec<EQueued>>;
    async fn reload_settings(&self) -> EngineResult<()>;
    /// Rebuild every account's search index (needed after `--import`, which
    /// restores messages but not the full-text index). Runs in the background.
    async fn reindex_accounts(&self) -> EngineResult<()>;
    async fn install_certificate(&self, cert_pem: &str, key_pem: &str) -> EngineResult<()>;
    async fn set_relay(&self, relay: Option<&RelayConfig>) -> EngineResult<()>;

    // self-service (acts AS the mailbox user, never as admin)
    async fn user_verify(&self, creds: &UserCreds) -> EngineResult<()>;
    async fn user_change_password(&self, creds: &UserCreds, next: &str) -> EngineResult<()>;
    async fn user_app_passwords(&self, creds: &UserCreds) -> EngineResult<Vec<EAppPassword>>;
    async fn user_create_app_password(
        &self,
        creds: &UserCreds,
        description: &str,
    ) -> EngineResult<(EAppPassword, String)>;
    async fn user_delete_app_password(&self, creds: &UserCreds, id: &str) -> EngineResult<()>;
    /// The text of the user's Sieve script called `name`, if there is one.
    async fn user_sieve_script(
        &self,
        creds: &UserCreds,
        name: &str,
    ) -> EngineResult<Option<String>>;
    /// Install (and activate) the user's Sieve script `name`, or with `None`
    /// deactivate and delete it. Other scripts are left alone.
    async fn user_put_sieve_script(
        &self,
        creds: &UserCreds,
        name: &str,
        script: Option<&str>,
    ) -> EngineResult<()>;
    /// The user's own mail store (JMAP Mail as the user), for importing mail.
    fn user_mail(&self, creds: &UserCreds) -> std::sync::Arc<dyn mail::UserMail>;

    // outgoing queue management
    /// Attempt delivery of these queued messages now. Returns how many were
    /// still queued (a message delivered meanwhile is not an error, unless it
    /// was the only one asked for: then `NotFound`).
    async fn queue_retry(&self, ids: &[String]) -> EngineResult<usize>;
    /// Cancel delivery: the message leaves the queue without a bounce.
    async fn queue_cancel(&self, id: &str) -> EngineResult<()>;
    /// Pause or resume all outbound delivery. The engine keeps this in memory
    /// only (an engine restart resumes), so Yunori records and re-applies it.
    async fn queue_set_paused(&self, paused: bool) -> EngineResult<()>;

    // login abuse: blocked and allowed addresses
    async fn blocked_ips(&self) -> EngineResult<Vec<EBlockedIp>>;
    async fn unblock_ip(&self, id: &str) -> EngineResult<()>;
    async fn allowed_ips(&self) -> EngineResult<Vec<EAllowedIp>>;
    /// `address` is an IP or a CIDR network, already validated by the caller.
    async fn allow_ip(&self, address: &str, reason: Option<&str>) -> EngineResult<EAllowedIp>;
    async fn remove_allowed_ip(&self, id: &str) -> EngineResult<()>;
    async fn auth_ban_policy(&self) -> EngineResult<EAuthBanPolicy>;
    async fn set_auth_ban_policy(&self, policy: &EAuthBanPolicy) -> EngineResult<()>;
}

/// Resolve an account's alias list into full addresses given the domain list.
pub fn alias_addresses(account: &EAccount, domains: &[EDomain]) -> Vec<String> {
    account
        .aliases
        .iter()
        .filter_map(|a| {
            domains
                .iter()
                .find(|d| d.id == a.domain_id)
                .map(|d| format!("{}@{}", a.local, d.name))
        })
        .collect()
}
