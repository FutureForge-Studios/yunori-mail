//! The engine seam (plan §5.2): everything `yunori-mail` needs from the mail
//! server, behind one trait. Only the Stalwart adapter ships; the in-memory
//! mock backs `serve --dev --mock-engine` and the test-suite. A mox adapter
//! could slot in here without touching the API or installer layers.

pub mod jmap;
pub mod mock;
pub mod stalwart;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RcptState {
    Scheduled,
    Completed,
    TemporaryFailure,
    PermanentFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EQueuedRecipient {
    pub address: String,
    pub state: RcptState,
    /// Classified failure kind from the engine (e.g. "tlsError", "connectionError").
    pub error_type: Option<String>,
    pub response: Option<String>,
    pub retry_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EQueued {
    pub id: String,
    pub created_at: Option<String>,
    pub next_retry: Option<String>,
    pub recipients: Vec<EQueuedRecipient>,
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
