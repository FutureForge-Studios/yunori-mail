//! A mailbox user's own mail store, for bringing messages in ("Move your mail
//! here"). Acts AS the user (their credentials or an app password of theirs),
//! never as the engine admin: standard JMAP Mail (RFC 8621) — upload the raw
//! message as a blob, then `Email/import` it with its keywords and received
//! date. The in-memory store backs the mock engine and the unit tests.

use super::jmap::{Auth, Call, JmapClient, get_list};
use super::{EngineError, EngineResult, UserCreds};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const MAIL_USING: [&str; 2] = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"];

#[derive(Debug, Clone, PartialEq)]
pub struct DestMailbox {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    /// JMAP role: inbox, sent, drafts, trash, junk, archive, …
    pub role: Option<String>,
}

#[async_trait]
pub trait UserMail: Send + Sync {
    /// The largest message the store accepts, in bytes.
    async fn max_message_bytes(&self) -> EngineResult<u64>;
    async fn mailboxes(&self) -> EngineResult<Vec<DestMailbox>>;
    /// Create (subscribed) and return the new id.
    async fn create_mailbox(&self, name: &str, parent_id: Option<&str>) -> EngineResult<String>;
    /// Store one raw RFC 5322 message in `mailbox_id`.
    async fn import(
        &self,
        mailbox_id: &str,
        raw: Vec<u8>,
        keywords: &[String],
        received_at: Option<&str>,
    ) -> EngineResult<()>;
}

// ── Stalwart (JMAP Mail) ─────────────────────────────────────────────────

pub struct JmapUserMail {
    client: JmapClient,
    auth: Auth,
    session: tokio::sync::OnceCell<(String, u64)>,
}

impl JmapUserMail {
    pub fn new(client: JmapClient, creds: &UserCreds) -> Self {
        Self {
            client,
            auth: Auth::basic(&creds.email, &creds.password),
            session: tokio::sync::OnceCell::new(),
        }
    }

    /// (mail account id, maxSizeUpload) from the user's session.
    async fn session(&self) -> EngineResult<&(String, u64)> {
        self.session
            .get_or_try_init(|| async {
                let s = self.client.session(&self.auth).await?;
                let account = s
                    .pointer("/primaryAccounts/urn:ietf:params:jmap:mail")
                    .and_then(Value::as_str)
                    .ok_or_else(|| EngineError::Protocol("no mail account in the session".into()))?
                    .to_string();
                let max = s
                    .pointer("/capabilities/urn:ietf:params:jmap:core/maxSizeUpload")
                    .and_then(Value::as_u64)
                    .unwrap_or(50_000_000);
                Ok((account, max))
            })
            .await
    }

    async fn call(&self, calls: Vec<Call>) -> EngineResult<Vec<Value>> {
        self.client.call_using(&self.auth, &MAIL_USING, calls).await
    }
}

fn not_created(resp: &Value, key: &str) -> Option<EngineError> {
    let e = resp.get("notCreated")?.get(key)?;
    let kind = e.get("type").and_then(Value::as_str).unwrap_or("refused");
    let desc = e.get("description").and_then(Value::as_str).unwrap_or("");
    Some(match kind {
        "overQuota" => EngineError::Rejected("overQuota: the mailbox is full".into()),
        "forbidden" => EngineError::Unauthorized,
        _ if desc.is_empty() => EngineError::Rejected(kind.to_string()),
        _ => EngineError::Rejected(format!("{kind}: {desc}")),
    })
}

#[async_trait]
impl UserMail for JmapUserMail {
    async fn max_message_bytes(&self) -> EngineResult<u64> {
        Ok(self.session().await?.1)
    }

    async fn mailboxes(&self) -> EngineResult<Vec<DestMailbox>> {
        let account = self.session().await?.0.clone();
        let r = self
            .call(vec![Call::new(
                "Mailbox/get",
                json!({ "accountId": account, "ids": null, "properties": ["name", "parentId", "role"] }),
            )])
            .await?;
        Ok(get_list(&r[0])
            .into_iter()
            .filter_map(|m| {
                Some(DestMailbox {
                    id: m.get("id")?.as_str()?.to_string(),
                    name: m.get("name")?.as_str()?.to_string(),
                    parent_id: m
                        .get("parentId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    role: m.get("role").and_then(Value::as_str).map(str::to_string),
                })
            })
            .collect())
    }

    async fn create_mailbox(&self, name: &str, parent_id: Option<&str>) -> EngineResult<String> {
        let account = self.session().await?.0.clone();
        let r = self
            .call(vec![Call::new(
                "Mailbox/set",
                // Subscribed, or IMAP apps that list subscriptions won't show it.
                json!({ "accountId": account, "create": { "m": {
                    "name": name, "parentId": parent_id, "isSubscribed": true
                } } }),
            )])
            .await?;
        if let Some(e) = not_created(&r[0], "m") {
            return Err(e);
        }
        r[0].pointer("/created/m/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| EngineError::Protocol("Mailbox/set returned no id".into()))
    }

    async fn import(
        &self,
        mailbox_id: &str,
        raw: Vec<u8>,
        keywords: &[String],
        received_at: Option<&str>,
    ) -> EngineResult<()> {
        let account = self.session().await?.0.clone();
        let blob = self
            .client
            .upload(&self.auth, &account, "message/rfc822", raw)
            .await?;
        let mut email = Map::new();
        email.insert("blobId".into(), json!(blob));
        email.insert("mailboxIds".into(), json!({ mailbox_id: true }));
        email.insert(
            "keywords".into(),
            Value::Object(keywords.iter().map(|k| (k.clone(), json!(true))).collect()),
        );
        if let Some(at) = received_at {
            email.insert("receivedAt".into(), json!(at));
        }
        let r = self
            .call(vec![Call::new(
                "Email/import",
                json!({ "accountId": account, "emails": { "m": email } }),
            )])
            .await?;
        if let Some(e) = not_created(&r[0], "m") {
            return Err(e);
        }
        if r[0].pointer("/created/m").is_none() {
            return Err(EngineError::Protocol(format!(
                "unexpected Email/import answer: {}",
                r[0]
            )));
        }
        Ok(())
    }
}

// ── In memory (mock engine, tests) ───────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct StoredMessage {
    pub mailbox_id: String,
    pub size: usize,
    pub keywords: Vec<String>,
    pub received_at: Option<String>,
}

#[derive(Default)]
struct MemoryAccount {
    seq: u64,
    mailboxes: Vec<DestMailbox>,
    messages: Vec<StoredMessage>,
}

/// Every user's mail, shared by the per-user handles the mock engine gives out.
#[derive(Clone, Default)]
pub struct MemoryMail {
    accounts: Arc<Mutex<HashMap<String, MemoryAccount>>>,
    /// Largest accepted message (tests lower it).
    pub max_bytes: u64,
}

impl MemoryMail {
    pub fn new() -> Self {
        Self {
            accounts: Arc::default(),
            max_bytes: 50_000_000,
        }
    }

    pub fn for_user(&self, email: &str) -> Arc<dyn UserMail> {
        Arc::new(MemoryUserMail {
            store: self.clone(),
            email: email.to_string(),
        })
    }

    #[cfg(test)]
    pub fn messages(&self, email: &str) -> Vec<StoredMessage> {
        self.with(email, |a| a.messages.clone())
    }

    pub fn mailbox_list(&self, email: &str) -> Vec<DestMailbox> {
        self.with(email, |a| a.mailboxes.clone())
    }

    fn with<T>(&self, email: &str, f: impl FnOnce(&mut MemoryAccount) -> T) -> T {
        let mut all = self.accounts.lock().unwrap();
        let a = all
            .entry(email.to_string())
            .or_insert_with(|| MemoryAccount {
                seq: 10,
                mailboxes: [
                    ("a", "Inbox", "inbox"),
                    ("b", "Deleted Items", "trash"),
                    ("c", "Junk Mail", "junk"),
                    ("d", "Drafts", "drafts"),
                    ("e", "Sent Items", "sent"),
                ]
                .into_iter()
                .map(|(id, name, role)| DestMailbox {
                    id: id.into(),
                    name: name.into(),
                    parent_id: None,
                    role: Some(role.into()),
                })
                .collect(),
                messages: Vec::new(),
            });
        f(a)
    }
}

struct MemoryUserMail {
    store: MemoryMail,
    email: String,
}

#[async_trait]
impl UserMail for MemoryUserMail {
    async fn max_message_bytes(&self) -> EngineResult<u64> {
        Ok(self.store.max_bytes)
    }

    async fn mailboxes(&self) -> EngineResult<Vec<DestMailbox>> {
        Ok(self.store.mailbox_list(&self.email))
    }

    async fn create_mailbox(&self, name: &str, parent_id: Option<&str>) -> EngineResult<String> {
        self.store.with(&self.email, |a| {
            if a.mailboxes
                .iter()
                .any(|m| m.parent_id.as_deref() == parent_id && m.name.eq_ignore_ascii_case(name))
            {
                return Err(EngineError::Rejected(
                    "invalidProperties: a mailbox with this name exists".into(),
                ));
            }
            if parent_id.is_some_and(|p| !a.mailboxes.iter().any(|m| m.id == p)) {
                return Err(EngineError::Rejected("invalidProperties (parentId)".into()));
            }
            a.seq += 1;
            let id = format!("m{}", a.seq);
            a.mailboxes.push(DestMailbox {
                id: id.clone(),
                name: name.to_string(),
                parent_id: parent_id.map(str::to_string),
                role: None,
            });
            Ok(id)
        })
    }

    async fn import(
        &self,
        mailbox_id: &str,
        raw: Vec<u8>,
        keywords: &[String],
        received_at: Option<&str>,
    ) -> EngineResult<()> {
        if raw.len() as u64 > self.store.max_bytes {
            return Err(EngineError::Rejected("tooLarge".into()));
        }
        self.store.with(&self.email, |a| {
            if !a.mailboxes.iter().any(|m| m.id == mailbox_id) {
                return Err(EngineError::NotFound);
            }
            a.messages.push(StoredMessage {
                mailbox_id: mailbox_id.to_string(),
                size: raw.len(),
                keywords: keywords.to_vec(),
                received_at: received_at.map(str::to_string),
            });
            Ok(())
        })
    }
}
