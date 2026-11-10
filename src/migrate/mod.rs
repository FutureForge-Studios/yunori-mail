//! "Move your mail here": copy an existing mailbox (Gmail, Microsoft 365,
//! iCloud, Fastmail, Yahoo or any IMAP server) into a Yunori mailbox.
//!
//! - **Source**: IMAP over verified TLS only (993, or 143 with STARTTLS), to
//!   hosts that resolve to public addresses; read-only (`EXAMINE`,
//!   `BODY.PEEK`), so nothing on the old server changes, not even \Seen.
//! - **Destination**: the user's own mail store through JMAP, as the user
//!   (`engine::mail::UserMail`): blob upload + `Email/import` with the
//!   message's flags (as keywords) and its original received date.
//! - **Resumable**: every copied message is recorded by folder, UIDVALIDITY
//!   and UID (and a hash of its Message-ID), so running again copies only
//!   what is new. No message content and no password is stored.
//! - **Secrets**: the old server's password lives in memory only, for the
//!   job's duration. The portal starts a job with a temporary app password
//!   of the user's (made with their own credentials, revoked when the job
//!   ends), so it never holds their real password past the portal's
//!   30-minute window.

pub mod cli;
pub mod imap;
#[cfg(test)]
mod live_tests;
pub mod store;

use crate::app::App;
use crate::engine::mail::{DestMailbox, UserMail};
use crate::engine::{EngineError, UserCreds};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use imap::Security;
use rustls::pki_types::CertificateDer;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use store::Ledger;
use zeroize::Zeroizing;

/// A job stops (and can be resumed) after this long.
pub const MAX_JOB_DURATION: Duration = Duration::from_secs(12 * 3600);
/// Never more than this, whatever the destination would accept.
pub const MAX_MESSAGE_BYTES: u64 = 50 * 1024 * 1024;
/// Imports running at once on this server.
pub const MAX_RUNNING_JOBS: usize = 4;
/// Description of the temporary app password a portal job runs with.
pub const TEMP_APP_PASSWORD: &str = "Mail import (temporary)";
const LEASE_SECS: i64 = 900;
const MAX_ERRORS_KEPT: usize = 50;
const FETCH_CHUNK: usize = 100;

// ── the old server ───────────────────────────────────────────────────────

/// Network rules for reaching the old server. Production uses the default;
/// tests relax it to reach a local engine.
#[derive(Clone, Default)]
pub struct NetPolicy {
    /// Allow loopback/private addresses (tests only).
    pub allow_private: bool,
    /// Connect here instead of resolving the host name (tests only).
    pub resolve_to: Option<IpAddr>,
    /// Extra trust anchors (tests only; verification itself stays on).
    pub extra_roots: Vec<CertificateDer<'static>>,
}

/// The old server and the account on it.
#[derive(Clone)]
pub struct SourceSpec {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Zeroizing<String>,
}

impl SourceSpec {
    /// Bookkeeping key: which source a copied message came from.
    pub fn key(&self) -> String {
        format!(
            "{}|{}",
            self.host.to_ascii_lowercase(),
            self.username.to_ascii_lowercase()
        )
    }
    pub fn label(&self) -> String {
        format!("{} on {}", self.username, self.host)
    }
}

/// A mistake in what the person typed, with the field it belongs to.
#[derive(Debug)]
pub struct InputError {
    pub field: &'static str,
    pub message: String,
}

fn input_err(field: &'static str, message: impl Into<String>) -> InputError {
    InputError {
        field,
        message: message.into(),
    }
}

/// Check and normalise the host, port and username.
pub fn validate_source(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<(String, Security), InputError> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let security = match port {
        993 => Security::Implicit,
        143 => Security::StartTls,
        _ => {
            return Err(input_err(
                "port",
                "Use port 993 (SSL/TLS) or 143 (STARTTLS). Mail is only fetched over an encrypted connection.",
            ));
        }
    };
    let is_ip = host.parse::<IpAddr>().is_ok();
    let valid_name = !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if !(is_ip || valid_name) {
        return Err(input_err(
            "host",
            "Enter the old server's name, like imap.example.com.",
        ));
    }
    if username.trim().is_empty() || username.len() > 320 || username.contains(['\r', '\n']) {
        return Err(input_err(
            "username",
            "Enter the username for the old mailbox (usually the full address).",
        ));
    }
    if password.is_empty() || password.len() > 1024 {
        return Err(input_err("password", "Enter the password or app password."));
    }
    Ok((host, security))
}

/// Is this an address on the public internet? (Not loopback, private,
/// link-local, shared, documentation, multicast or reserved space.)
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // shared/CGNAT
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (18..20).contains(&o[1])) // benchmarking
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x64 && s[1] == 0xff9b) // NAT64 (reaches IPv4 inside)
                || s[0] == 0) // IPv4-compatible and other reserved forms
        }
    }
}

/// Resolve the host and refuse anything that isn't on the public internet.
/// The address returned is the one connected to (no second lookup).
pub async fn resolve(host: &str, port: u16, policy: &NetPolicy) -> Result<SocketAddr> {
    if let Some(ip) = policy.resolve_to {
        return Ok(SocketAddr::new(ip, port));
    }
    let addrs: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| anyhow!("looking up {host} timed out"))?
    .map_err(|_| anyhow!("{host} was not found. Check the server name."))?
    .collect();
    if addrs.is_empty() {
        return Err(anyhow!("{host} was not found. Check the server name."));
    }
    if !policy.allow_private && addrs.iter().any(|a| !is_public(a.ip())) {
        return Err(anyhow!(
            "{host} points to a private or local address. Only mail servers on the public internet can be imported from."
        ));
    }
    Ok(addrs[0])
}

// ── source abstraction (IMAP; an in-memory one for tests) ────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct SourceFolder {
    /// Name as the server spells it (modified UTF-7).
    pub raw: Vec<u8>,
    /// Name for people (decoded).
    pub name: String,
    pub delimiter: Option<char>,
    /// LIST attributes, lower case (`\sent`, `\noselect`, …).
    pub attrs: Vec<String>,
    pub messages: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceMessage {
    pub uid: u32,
    pub flags: Vec<String>,
    pub internal_date: Option<String>,
    pub size: u64,
    pub message_id: Option<String>,
}

#[async_trait]
pub trait MailSource: Send {
    async fn folders(&mut self) -> Result<Vec<SourceFolder>>;
    /// Open a folder read-only → (messages, UIDVALIDITY).
    async fn open(&mut self, raw: &[u8]) -> Result<(u64, u64)>;
    async fn uids(&mut self) -> Result<Vec<u32>>;
    async fn metadata(&mut self, uids: &[u32]) -> Result<Vec<SourceMessage>>;
    /// The raw message, or `None` if it is gone.
    async fn body(&mut self, uid: u32) -> Result<Option<Vec<u8>>>;
    async fn close(self: Box<Self>);
}

#[async_trait]
pub trait SourceConnector: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn MailSource>>;
}

pub struct ImapConnector {
    pub spec: SourceSpec,
    pub security: Security,
    pub policy: NetPolicy,
    pub max_message: u64,
}

impl ImapConnector {
    pub fn new(spec: SourceSpec, policy: NetPolicy) -> Result<Self, InputError> {
        let (host, security) =
            validate_source(&spec.host, spec.port, &spec.username, &spec.password)?;
        Ok(Self {
            spec: SourceSpec { host, ..spec },
            security,
            policy,
            max_message: MAX_MESSAGE_BYTES,
        })
    }
}

struct ImapSource {
    c: imap::Client,
}

fn is_refusal(e: &anyhow::Error) -> bool {
    e.downcast_ref::<imap::Refused>().is_some()
}

#[async_trait]
impl SourceConnector for ImapConnector {
    async fn connect(&self) -> Result<Box<dyn MailSource>> {
        let addr = resolve(&self.spec.host, self.spec.port, &self.policy).await?;
        let mut c = imap::Client::connect(
            &imap::Target {
                host: &self.spec.host,
                addr,
                security: self.security,
                extra_roots: &self.policy.extra_roots,
            },
            (self.max_message + (1 << 20)) as usize,
        )
        .await?;
        if let Err(e) = c.login(&self.spec.username, &self.spec.password).await {
            return Err(if is_refusal(&e) {
                anyhow!(
                    "The old server refused the username or password ({}). Gmail, Microsoft 365, iCloud and Yahoo need an app password here, not your normal password.",
                    e.to_string().trim()
                )
            } else {
                e
            });
        }
        Ok(Box::new(ImapSource { c }))
    }
}

/// The Message-ID from a header block, normalised for comparison.
pub fn message_id(header: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(header);
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n')).peekable();
    while let Some(line) = lines.next() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("message-id") {
            continue;
        }
        let mut value = value.trim().to_string();
        while let Some(next) = lines.peek() {
            if next.starts_with([' ', '\t']) {
                value.push_str(next.trim());
                lines.next();
            } else {
                break;
            }
        }
        let id = match (value.find('<'), value.rfind('>')) {
            (Some(a), Some(b)) if b > a => &value[a + 1..b],
            _ => value.as_str(),
        };
        let id = id.trim();
        return (!id.is_empty()).then(|| id.to_ascii_lowercase());
    }
    None
}

#[async_trait]
impl MailSource for ImapSource {
    async fn folders(&mut self) -> Result<Vec<SourceFolder>> {
        let mut out = Vec::new();
        for (raw, delimiter, attrs) in self.c.list().await? {
            let selectable = !attrs
                .iter()
                .any(|a| a == "\\noselect" || a == "\\nonexistent");
            let messages = if selectable {
                match self.c.status(&raw).await {
                    Ok((n, _)) => n,
                    Err(e) if is_refusal(&e) => 0,
                    Err(e) => return Err(e),
                }
            } else {
                0
            };
            out.push(SourceFolder {
                name: imap::decode_mutf7(&raw),
                raw,
                delimiter,
                attrs,
                messages,
            });
        }
        Ok(out)
    }

    async fn open(&mut self, raw: &[u8]) -> Result<(u64, u64)> {
        self.c.examine(raw).await
    }

    async fn uids(&mut self) -> Result<Vec<u32>> {
        self.c.uids().await
    }

    async fn metadata(&mut self, uids: &[u32]) -> Result<Vec<SourceMessage>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let set = uids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let rows = self
            .c
            .uid_fetch(
                &set,
                "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])",
            )
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|items| {
                let mut m = SourceMessage {
                    uid: 0,
                    flags: Vec::new(),
                    internal_date: None,
                    size: 0,
                    message_id: None,
                };
                for (k, v) in items {
                    match k.as_str() {
                        "UID" => m.uid = v.num().and_then(|n| u32::try_from(n).ok())?,
                        "FLAGS" => {
                            m.flags = v
                                .list()
                                .map(|l| l.iter().filter_map(imap::V::text).collect())
                                .unwrap_or_default()
                        }
                        "INTERNALDATE" => {
                            m.internal_date = v.text().and_then(|d| imap::internal_date_utc(&d))
                        }
                        "RFC822.SIZE" => m.size = v.num().unwrap_or(0),
                        k if k.starts_with("BODY[HEADER.FIELDS") => {
                            m.message_id = v.bytes().and_then(|b| message_id(&b))
                        }
                        _ => {}
                    }
                }
                (m.uid != 0).then_some(m)
            })
            .collect())
    }

    async fn body(&mut self, uid: u32) -> Result<Option<Vec<u8>>> {
        let rows = self
            .c
            .uid_fetch(&uid.to_string(), "(UID BODY.PEEK[])")
            .await?;
        for items in rows {
            let this = items
                .iter()
                .any(|(k, v)| k == "UID" && v.num() == Some(uid as u64));
            if !this {
                continue;
            }
            for (k, v) in items {
                if k == "BODY[]" {
                    return Ok(v.bytes());
                }
            }
        }
        Ok(None)
    }

    async fn close(self: Box<Self>) {
        self.c.logout().await;
    }
}

// ── folder plan ──────────────────────────────────────────────────────────

/// Where a source folder's messages go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Target {
    /// One of the destination's special mailboxes (JMAP role).
    Role { role: String },
    /// A folder created as-is (nested), optionally under the Inbox.
    Path {
        under_inbox: bool,
        parts: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPlan {
    /// The source name as the server spells it (what a selection names).
    pub id: String,
    pub name: String,
    pub messages: u64,
    pub target: Target,
    /// Where it lands, for people ("Sent", "Projects / 2024").
    pub destination: String,
    pub selected: bool,
    /// Why it is not selected by default.
    pub note: Option<String>,
}

const ROLES: [&str; 5] = ["sent", "drafts", "trash", "junk", "archive"];

fn role_label(role: &str) -> &'static str {
    match role {
        "inbox" => "Inbox",
        "sent" => "Sent",
        "drafts" => "Drafts",
        "trash" => "Trash",
        "junk" => "Junk",
        _ => "Archive",
    }
}

fn role_by_name(leaf: &str) -> Option<&'static str> {
    Some(match leaf.to_ascii_lowercase().as_str() {
        "sent" | "sent items" | "sent messages" | "sent mail" | "sent-mail" => "sent",
        "drafts" | "draft" => "drafts",
        "trash" | "deleted items" | "deleted messages" | "deleted" | "bin" => "trash",
        "junk" | "spam" | "junk e-mail" | "junk email" | "junk mail" | "bulk mail" | "bulk" => {
            "junk"
        }
        "archive" | "archives" => "archive",
        _ => return None,
    })
}

fn is_gmail_root(s: &str) -> bool {
    s.eq_ignore_ascii_case("[Gmail]") || s.eq_ignore_ascii_case("[Google Mail]")
}

/// A name the destination accepts for one folder level.
fn clean_part(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    let s = s.trim();
    if s.is_empty() {
        "Folder".into()
    } else {
        s.chars().take(200).collect()
    }
}

/// Decide, for every selectable source folder, where it goes and whether it
/// is selected by default. INBOX → Inbox; Sent/Drafts/Trash/Junk/Archive →
/// the special mailboxes (by SPECIAL-USE attribute, else by a well-known
/// name); Gmail's virtual folders (All Mail, Important, Starred) are listed
/// but not selected, because every message in them is in another folder too.
pub fn plan_folders(folders: &[SourceFolder]) -> Vec<FolderPlan> {
    let selectable: Vec<&SourceFolder> = folders
        .iter()
        .filter(|f| {
            !f.attrs
                .iter()
                .any(|a| a == "\\noselect" || a == "\\nonexistent")
        })
        .collect();
    let parts_of = |f: &SourceFolder| -> Vec<String> {
        match f.delimiter {
            Some(d) => f.name.split(d).map(str::to_string).collect(),
            None => vec![f.name.clone()],
        }
    };
    // Roles: SPECIAL-USE attributes first, then names, each role once.
    let mut role_of: HashMap<usize, &'static str> = HashMap::new();
    let mut taken: Vec<&'static str> = Vec::new();
    for (i, f) in selectable.iter().enumerate() {
        if f.name.eq_ignore_ascii_case("INBOX") {
            role_of.insert(i, "inbox");
            taken.push("inbox");
        }
    }
    for (i, f) in selectable.iter().enumerate() {
        if role_of.contains_key(&i) {
            continue;
        }
        for role in ROLES {
            if !taken.contains(&role) && f.attrs.iter().any(|a| a == &format!("\\{role}")) {
                role_of.insert(i, role);
                taken.push(role);
                break;
            }
        }
    }
    for (i, f) in selectable.iter().enumerate() {
        if role_of.contains_key(&i) {
            continue;
        }
        let parts = parts_of(f);
        let top_level = parts.len() == 1
            || (parts.len() == 2
                && (parts[0].eq_ignore_ascii_case("INBOX") || is_gmail_root(&parts[0])));
        if !top_level {
            continue;
        }
        if let Some(role) = role_by_name(parts.last().unwrap())
            && !taken.contains(&role)
            && !f
                .attrs
                .iter()
                .any(|a| a.starts_with('\\') && is_virtual_attr(a))
        {
            role_of.insert(i, role);
            taken.push(role);
        }
    }
    selectable
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let note = virtual_note(f);
            let target = match role_of.get(&i) {
                Some(role) => Target::Role {
                    role: role.to_string(),
                },
                None => {
                    let mut parts = parts_of(f);
                    let under_inbox = parts.len() > 1 && parts[0].eq_ignore_ascii_case("INBOX");
                    if under_inbox || (parts.len() > 1 && is_gmail_root(&parts[0])) {
                        parts.remove(0);
                    }
                    Target::Path {
                        under_inbox,
                        parts: parts.iter().map(|p| clean_part(p)).collect(),
                    }
                }
            };
            let destination = match &target {
                Target::Role { role } => role_label(role).to_string(),
                Target::Path { under_inbox, parts } => {
                    let mut all = Vec::new();
                    if *under_inbox {
                        all.push("Inbox".to_string());
                    }
                    all.extend(parts.iter().cloned());
                    all.join(" / ")
                }
            };
            FolderPlan {
                id: String::from_utf8_lossy(&f.raw).into_owned(),
                name: f.name.clone(),
                messages: f.messages,
                target,
                destination,
                selected: note.is_none(),
                note,
            }
        })
        .collect()
}

fn is_virtual_attr(a: &str) -> bool {
    matches!(a, "\\all" | "\\flagged" | "\\important")
}

fn virtual_note(f: &SourceFolder) -> Option<String> {
    let leaf = f
        .delimiter
        .and_then(|d| f.name.rsplit(d).next())
        .unwrap_or(&f.name)
        .to_ascii_lowercase();
    let gmail = f
        .delimiter
        .and_then(|d| f.name.split(d).next())
        .is_some_and(is_gmail_root);
    if f.attrs.iter().any(|a| a == "\\all") || (gmail && leaf == "all mail") {
        Some("Every message again — they're already in your other folders.".into())
    } else if f.attrs.iter().any(|a| a == "\\important") || (gmail && leaf == "important") {
        Some("Gmail's Important view — the messages are in your other folders.".into())
    } else if f.attrs.iter().any(|a| a == "\\flagged") || (gmail && leaf == "starred") {
        Some("Starred messages — they're copied with their star from their own folders.".into())
    } else {
        None
    }
}

/// IMAP flags → JMAP keywords (the four system flags plus `$Forwarded` and
/// other user keywords). `\Recent` and `\Deleted` are not carried over.
pub fn keywords(flags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in flags {
        let lower = f.to_ascii_lowercase();
        let k = match lower.as_str() {
            "\\seen" => "$seen".to_string(),
            "\\flagged" => "$flagged".to_string(),
            "\\answered" => "$answered".to_string(),
            "\\draft" => "$draft".to_string(),
            l if l.starts_with('\\') => continue,
            l if !l.is_empty()
                && l.len() <= 64
                && l.chars()
                    .all(|c| c.is_ascii_graphic() && !"(){]%*\"\\".contains(c)) =>
            {
                l.to_string()
            }
            _ => continue,
        };
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

// ── destination mailboxes ────────────────────────────────────────────────

struct Destination {
    mail: Arc<dyn UserMail>,
    boxes: Vec<DestMailbox>,
    dry_run: bool,
}

impl Destination {
    fn find(&self, parent: Option<&str>, name: &str) -> Option<String> {
        self.boxes
            .iter()
            .find(|m| m.parent_id.as_deref() == parent && m.name.eq_ignore_ascii_case(name))
            .map(|m| m.id.clone())
    }

    async fn ensure(&mut self, parent: Option<&str>, name: &str) -> Result<String, EngineError> {
        if let Some(id) = self.find(parent, name) {
            return Ok(id);
        }
        let id = if self.dry_run {
            format!("dry-run:{}:{name}", parent.unwrap_or(""))
        } else {
            self.mail.create_mailbox(name, parent).await?
        };
        self.boxes.push(DestMailbox {
            id: id.clone(),
            name: name.to_string(),
            parent_id: parent.map(str::to_string),
            role: None,
        });
        Ok(id)
    }

    async fn resolve(&mut self, target: &Target) -> Result<String, EngineError> {
        match target {
            Target::Role { role } => {
                if let Some(m) = self.boxes.iter().find(|m| m.role.as_deref() == Some(role)) {
                    return Ok(m.id.clone());
                }
                // No such special mailbox yet (Stalwart has no Archive by default).
                self.ensure(None, role_label(role)).await
            }
            Target::Path { under_inbox, parts } => {
                let mut parent = if *under_inbox {
                    self.boxes
                        .iter()
                        .find(|m| m.role.as_deref() == Some("inbox"))
                        .map(|m| m.id.clone())
                } else {
                    None
                };
                for p in parts {
                    parent = Some(self.ensure(parent.as_deref(), p).await?);
                }
                parent.ok_or_else(|| EngineError::Rejected("empty folder name".into()))
            }
        }
    }
}

// ── progress ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
    /// The server restarted while it ran; run it again to continue.
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobError {
    pub folder: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub state: JobState,
    pub dry_run: bool,
    /// "me@gmail.com on imap.gmail.com"
    pub source: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub current_folder: Option<String>,
    pub folders_done: usize,
    pub folders_total: usize,
    pub messages_total: u64,
    pub messages_copied: u64,
    /// Already here from an earlier run.
    pub messages_skipped: u64,
    pub messages_failed: u64,
    pub bytes_copied: u64,
    pub errors: Vec<JobError>,
    pub summary: Option<String>,
}

impl Progress {
    fn new(source: String, dry_run: bool) -> Self {
        Self {
            state: JobState::Running,
            dry_run,
            source,
            started_at: crate::util::now_rfc3339(),
            finished_at: None,
            current_folder: None,
            folders_done: 0,
            folders_total: 0,
            messages_total: 0,
            messages_copied: 0,
            messages_skipped: 0,
            messages_failed: 0,
            bytes_copied: 0,
            errors: Vec::new(),
            summary: None,
        }
    }

    fn error(&mut self, folder: Option<&str>, message: impl Into<String>) {
        if self.errors.len() < MAX_ERRORS_KEPT {
            self.errors.push(JobError {
                folder: folder.map(str::to_string),
                message: message.into(),
            });
        }
    }

    fn sentence(&self) -> String {
        let mut s = format!(
            "{} {} message{} ({})",
            if self.dry_run { "Would copy" } else { "Copied" },
            self.messages_copied,
            if self.messages_copied == 1 { "" } else { "s" },
            fmt_bytes(self.bytes_copied)
        );
        if self.messages_skipped > 0 {
            s.push_str(&format!(", {} already here", self.messages_skipped));
        }
        if self.messages_failed > 0 {
            s.push_str(&format!(", {} could not be copied", self.messages_failed));
        }
        s.push('.');
        s
    }
}

pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// A running (or just finished) import.
pub struct Job {
    progress: Mutex<Progress>,
    cancel: AtomicBool,
    /// Everything is wrapped up (credentials revoked, lease released).
    finished: AtomicBool,
}

impl Job {
    /// Wait until the job has completely finished.
    pub async fn wait(&self) {
        while !self.finished.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    fn update(&self, f: impl FnOnce(&mut Progress)) {
        f(&mut self.progress.lock().unwrap());
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Jobs of this process, by destination address.
static JOBS: LazyLock<Mutex<HashMap<String, Arc<Job>>>> = LazyLock::new(Default::default);

/// The live job for `account`, if this process runs one.
pub fn running(account: &str) -> Option<Arc<Job>> {
    JOBS.lock()
        .unwrap()
        .get(account)
        .filter(|j| j.progress().state == JobState::Running)
        .cloned()
}

/// What to show for `account`: the live job, else the last recorded run.
pub fn status(db: &crate::store::Db, account: &str) -> Result<Option<Progress>> {
    if let Some(j) = JOBS.lock().unwrap().get(account) {
        return Ok(Some(j.progress()));
    }
    let ledger = Ledger::open(db)?;
    let Some(json) = ledger.last_run(account)? else {
        return Ok(None);
    };
    let mut p: Progress = serde_json::from_str(&json)?;
    if p.state == JobState::Running && !ledger.leased(account)? {
        p.state = JobState::Interrupted;
        p.summary = Some(
            "Stopped when the server restarted. Start it again to copy the rest — nothing is copied twice."
                .into(),
        );
    }
    Ok(Some(p))
}

// ── starting a job ───────────────────────────────────────────────────────

/// How the job signs in to the destination mailbox.
pub enum DestAuth {
    /// The user's own credentials: a temporary app password is made with them
    /// and used (and revoked) by the job.
    TemporaryAppPassword(UserCreds),
    /// Credentials used as they are for the whole job (CLI: the engine
    /// admin acting for the user, `user%admin`).
    Direct(UserCreds),
}

#[derive(Debug)]
pub enum StartError {
    Input(InputError),
    /// An import into this mailbox is already running.
    Busy,
    /// Too many imports on this server right now.
    ServerBusy,
    /// Could not reach or sign in to the old server.
    Source(String),
    Engine(EngineError),
    Internal(anyhow::Error),
}

impl From<EngineError> for StartError {
    fn from(e: EngineError) -> Self {
        StartError::Engine(e)
    }
}
impl From<anyhow::Error> for StartError {
    fn from(e: anyhow::Error) -> Self {
        StartError::Internal(e)
    }
}

pub struct JobRequest {
    /// Destination mailbox address.
    pub account: String,
    /// Who asked (audit actor).
    pub actor: String,
    pub source: SourceSpec,
    /// Source folder ids to copy; `None` = the default selection.
    pub folders: Option<Vec<String>>,
    pub dry_run: bool,
}

/// Connect to the old server and list its folders with the plan for each.
pub async fn inspect(source: SourceSpec, policy: NetPolicy) -> Result<Vec<FolderPlan>, StartError> {
    let connector = ImapConnector::new(source, policy).map_err(StartError::Input)?;
    let mut src = connector
        .connect()
        .await
        .map_err(|e| StartError::Source(friendly(&e)))?;
    let folders = src.folders().await;
    src.close().await;
    Ok(plan_folders(
        &folders.map_err(|e| StartError::Source(friendly(&e)))?,
    ))
}

/// A sentence for an error from the old server's side.
fn friendly(e: &anyhow::Error) -> String {
    let s = format!("{e:#}");
    if s.contains("invalid peer certificate") || s.contains("UnknownIssuer") {
        "The old server's certificate could not be verified, so nothing was sent to it. Check the server name.".into()
    } else {
        s
    }
}

/// Start an import in the background. Connects and signs in to the old
/// server first, so a wrong password fails here, not later.
pub async fn start(
    app: &Arc<App>,
    req: JobRequest,
    dest: DestAuth,
    policy: NetPolicy,
) -> Result<Arc<Job>, StartError> {
    let connector = ImapConnector::new(req.source.clone(), policy).map_err(StartError::Input)?;
    start_with(app, req, dest, Arc::new(connector)).await
}

async fn start_with(
    app: &Arc<App>,
    req: JobRequest,
    dest: DestAuth,
    connector: Arc<dyn SourceConnector>,
) -> Result<Arc<Job>, StartError> {
    let account = req.account.clone();
    {
        let jobs = JOBS.lock().unwrap();
        if jobs
            .get(&account)
            .is_some_and(|j| j.progress().state == JobState::Running)
        {
            return Err(StartError::Busy);
        }
        let running = jobs
            .values()
            .filter(|j| j.progress().state == JobState::Running)
            .count();
        if running >= MAX_RUNNING_JOBS {
            return Err(StartError::ServerBusy);
        }
    }
    let ledger = Ledger::open(&app.db)?;
    let owner = crate::util::random_token(12);
    if !ledger.acquire(&account, &owner, LEASE_SECS)? {
        return Err(StartError::Busy);
    }
    let release = |e: StartError| {
        let _ = ledger.release(&account, &owner);
        e
    };

    // The old server, now: a wrong password should be said straight away.
    let src = match connector.connect().await {
        Ok(s) => s,
        Err(e) => return Err(release(StartError::Source(friendly(&e)))),
    };
    let engine = match app.engine() {
        Ok(e) => e,
        Err(e) => return Err(release(e.into())),
    };

    // Destination credentials for the job's lifetime.
    let (creds, temp_id) = match dest {
        DestAuth::Direct(c) => (c, None),
        DestAuth::TemporaryAppPassword(user) => {
            // Leftovers of a run the server never finished (we hold the lease).
            if let Ok(list) = engine.user_app_passwords(&user).await {
                for p in list.iter().filter(|p| p.description == TEMP_APP_PASSWORD) {
                    let _ = engine.user_delete_app_password(&user, &p.id).await;
                }
            }
            match engine
                .user_create_app_password(&user, TEMP_APP_PASSWORD)
                .await
            {
                Ok((p, secret)) => (
                    UserCreds {
                        email: user.email.clone(),
                        password: Zeroizing::new(secret),
                    },
                    Some(p.id),
                ),
                Err(e) => {
                    src.close().await;
                    return Err(release(e.into()));
                }
            }
        }
    };

    let job = Arc::new(Job {
        progress: Mutex::new(Progress::new(req.source.label(), req.dry_run)),
        cancel: AtomicBool::new(false),
        finished: AtomicBool::new(false),
    });
    JOBS.lock().unwrap().insert(account.clone(), job.clone());
    if !req.dry_run {
        if let Ok(json) = serde_json::to_string(&job.progress()) {
            let _ = ledger.save_run(&account, &json);
        }
        app.db.audit(
            &req.actor,
            "mailbox.migrate_start",
            Some(&account),
            Some(&format!(
                "from {}{}",
                req.source.label(),
                req.folders
                    .as_ref()
                    .map(|f| format!(" ({} folders)", f.len()))
                    .unwrap_or_default()
            )),
        )?;
    }

    let app = app.clone();
    let task_job = job.clone();
    tokio::spawn(async move {
        let job = task_job;
        let mail = engine.user_mail(&creds);
        let ctx = RunCtx {
            ledger: ledger.clone(),
            account: account.clone(),
            source_key: req.source.key(),
            owner: owner.clone(),
            deadline: Instant::now() + MAX_JOB_DURATION,
        };
        let outcome = run(
            &ctx,
            &job,
            connector.as_ref(),
            Some(src),
            mail,
            req.folders,
            req.dry_run,
        )
        .await;
        job.update(|p| {
            p.current_folder = None;
            p.finished_at = Some(crate::util::now_rfc3339());
            match &outcome {
                Ok(()) if job.cancelled() => {
                    p.state = JobState::Cancelled;
                    p.summary = Some(format!("Stopped. {}", p.sentence()));
                }
                Ok(()) => {
                    p.state = JobState::Done;
                    p.summary = Some(p.sentence());
                }
                Err(e) => {
                    p.state = JobState::Failed;
                    p.summary = Some(format!("{e} {}", p.sentence()));
                }
            }
        });
        if let Some(id) = temp_id {
            // The app password removes itself: the user's password is not kept.
            if let Err(e) = engine.user_delete_app_password(&creds, &id).await {
                tracing::warn!("could not revoke the import's temporary app password: {e}");
            }
        }
        drop(creds);
        let progress = job.progress();
        if !progress.dry_run {
            if let Ok(json) = serde_json::to_string(&progress) {
                let _ = ctx.ledger.save_run(&account, &json);
            }
            let _ = app.db.audit(
                &req.actor,
                match progress.state {
                    JobState::Cancelled => "mailbox.migrate_cancelled",
                    JobState::Failed => "mailbox.migrate_failed",
                    _ => "mailbox.migrate_done",
                },
                Some(&account),
                progress.summary.as_deref(),
            );
        }
        let _ = ctx.ledger.release(&account, &owner);
        job.finished.store(true, Ordering::Release);
    });
    Ok(job)
}

// ── the copy itself ──────────────────────────────────────────────────────

struct RunCtx {
    ledger: Ledger,
    account: String,
    source_key: String,
    owner: String,
    deadline: Instant,
}

/// Why a job stopped early (shown to the person).
#[derive(Debug)]
struct Stop(String);
impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

enum FolderErr {
    /// The old server went away mid-folder: reconnect and carry on.
    Source(anyhow::Error),
    /// Give up on the whole job.
    Fatal(Stop),
}

fn dest_err(e: EngineError) -> FolderErr {
    FolderErr::Fatal(Stop(match e {
        EngineError::Unauthorized => {
            "This mailbox stopped accepting the import's sign-in. Start it again.".into()
        }
        EngineError::Rejected(m) if m.starts_with("overQuota") => {
            "Your mailbox here is full. Ask your administrator for more space, then start again to copy the rest.".into()
        }
        EngineError::Unavailable(_) => "The mail engine stopped answering.".into(),
        other => format!("The mail engine refused a change: {other}."),
    }))
}

async fn run(
    ctx: &RunCtx,
    job: &Job,
    connector: &dyn SourceConnector,
    mut src: Option<Box<dyn MailSource>>,
    mail: Arc<dyn UserMail>,
    selection: Option<Vec<String>>,
    dry_run: bool,
) -> Result<(), Stop> {
    let connect = || async {
        connector.connect().await.map_err(|e| {
            Stop(format!(
                "Could not reconnect to the old server: {}",
                friendly(&e)
            ))
        })
    };
    let mut s = match src.take() {
        Some(s) => s,
        None => connect().await?,
    };
    let folders = s.folders().await.map_err(|e| {
        Stop(format!(
            "Could not list the old server's folders: {}",
            friendly(&e)
        ))
    })?;
    let plans: Vec<FolderPlan> = plan_folders(&folders)
        .into_iter()
        .filter(|p| match &selection {
            Some(ids) => ids.iter().any(|i| i == &p.id),
            None => p.selected,
        })
        .collect();
    let max = mail
        .max_message_bytes()
        .await
        .map_err(|e| match dest_err(e) {
            FolderErr::Fatal(s) => s,
            FolderErr::Source(e) => Stop(e.to_string()),
        })?
        .min(MAX_MESSAGE_BYTES);
    let boxes = mail.mailboxes().await.map_err(|e| match dest_err(e) {
        FolderErr::Fatal(s) => s,
        FolderErr::Source(e) => Stop(e.to_string()),
    })?;
    let mut dest = Destination {
        mail: mail.clone(),
        boxes,
        dry_run,
    };
    job.update(|p| {
        p.folders_total = plans.len();
        p.messages_total = plans.iter().map(|f| f.messages).sum();
    });
    let mut reconnects = 0;
    let mut last_renew = Instant::now();
    for plan in &plans {
        job.update(|p| p.current_folder = Some(plan.name.clone()));
        let mut estimated = plan.messages;
        let mut this_run: HashSet<u32> = HashSet::new();
        let at_start = job.progress();
        loop {
            if job.cancelled() {
                s.close().await;
                return Ok(());
            }
            match copy_folder(
                ctx,
                job,
                s.as_mut(),
                &mut dest,
                plan,
                max,
                dry_run,
                &mut estimated,
                &mut last_renew,
                &mut this_run,
            )
            .await
            {
                Ok(()) => break,
                Err(FolderErr::Fatal(stop)) => {
                    s.close().await;
                    return Err(stop);
                }
                Err(FolderErr::Source(e)) if is_refusal(&e) => {
                    // The server refused this folder (e.g. it vanished): skip it.
                    job.update(|p| {
                        p.error(
                            Some(&plan.name),
                            format!("The old server refused this folder: {e}"),
                        )
                    });
                    break;
                }
                Err(FolderErr::Source(e)) => {
                    reconnects += 1;
                    if reconnects > 5 {
                        return Err(Stop(format!(
                            "The connection to the old server kept dropping ({}). Start again to continue where it stopped.",
                            friendly(&e)
                        )));
                    }
                    tracing::info!("import for {}: reconnecting after: {e:#}", ctx.account);
                    tokio::time::sleep(Duration::from_secs(2 * reconnects)).await;
                    s = connect().await?;
                    // The folder is walked again: what failed is retried.
                    job.update(|p| {
                        p.messages_failed = at_start.messages_failed;
                        p.errors.truncate(at_start.errors.len());
                    });
                }
            }
        }
        job.update(|p| p.folders_done += 1);
    }
    s.close().await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn copy_folder(
    ctx: &RunCtx,
    job: &Job,
    s: &mut dyn MailSource,
    dest: &mut Destination,
    plan: &FolderPlan,
    max: u64,
    dry_run: bool,
    estimated: &mut u64,
    last_renew: &mut Instant,
    this_run: &mut HashSet<u32>,
) -> Result<(), FolderErr> {
    let mailbox_id = dest.resolve(&plan.target).await.map_err(dest_err)?;
    let (exists, uidvalidity) = s
        .open(plan.id.as_bytes())
        .await
        .map_err(FolderErr::Source)?;
    // The folder may have changed since it was counted.
    if exists != *estimated {
        let (old, new) = (*estimated, exists);
        job.update(|p| p.messages_total = (p.messages_total + new).saturating_sub(old));
        *estimated = exists;
    }
    let uids = s.uids().await.map_err(FolderErr::Source)?;
    let copied = ctx
        .ledger
        .copied(&ctx.account, &ctx.source_key, &plan.id, uidvalidity)
        .map_err(|e| FolderErr::Fatal(Stop(format!("Bookkeeping failed: {e}"))))?;
    // Already-recorded UIDs are skipped without asking the server (those
    // copied earlier in this same run, before a reconnect, count as copied).
    let (done, todo): (Vec<u32>, Vec<u32>) = uids.iter().partition(|u| copied.uids.contains(u));
    let earlier = done.iter().filter(|u| !this_run.contains(u)).count() as u64;
    job.update(|p| p.messages_skipped += earlier);
    let mut seen_msgids = copied.msgids;
    for chunk in todo.chunks(FETCH_CHUNK) {
        let metas = s.metadata(chunk).await.map_err(FolderErr::Source)?;
        for m in metas {
            if job.cancelled() {
                return Ok(());
            }
            if Instant::now() > ctx.deadline {
                return Err(FolderErr::Fatal(Stop(format!(
                    "Stopped after {} hours. Start again to copy the rest.",
                    MAX_JOB_DURATION.as_secs() / 3600
                ))));
            }
            if last_renew.elapsed() > Duration::from_secs(30) {
                let _ = ctx.ledger.renew(&ctx.account, &ctx.owner, LEASE_SECS);
                *last_renew = Instant::now();
            }
            let msgid_hash = m
                .message_id
                .as_deref()
                .map(|id| crate::util::sha256_hex(id.as_bytes()));
            if msgid_hash.as_ref().is_some_and(|h| seen_msgids.contains(h)) {
                job.update(|p| p.messages_skipped += 1);
                continue;
            }
            if m.size > max {
                job.update(|p| {
                    p.messages_failed += 1;
                    p.error(
                        Some(&plan.name),
                        format!(
                            "A {} message is over the {} limit and was left on the old server.",
                            fmt_bytes(m.size),
                            fmt_bytes(max)
                        ),
                    );
                });
                continue;
            }
            if dry_run {
                job.update(|p| {
                    p.messages_copied += 1;
                    p.bytes_copied += m.size;
                });
                continue;
            }
            let Some(body) = s.body(m.uid).await.map_err(FolderErr::Source)? else {
                job.update(|p| {
                    p.messages_failed += 1;
                    p.error(
                        Some(&plan.name),
                        "A message disappeared from the old server while copying.",
                    );
                });
                continue;
            };
            let size = body.len() as u64;
            match dest
                .mail
                .import(
                    &mailbox_id,
                    body,
                    &keywords(&m.flags),
                    m.internal_date.as_deref(),
                )
                .await
            {
                Ok(()) => {}
                Err(EngineError::Rejected(r)) if !r.starts_with("overQuota") => {
                    job.update(|p| {
                        p.messages_failed += 1;
                        p.error(
                            Some(&plan.name),
                            format!("The engine refused a message: {r}"),
                        );
                    });
                    continue;
                }
                Err(e) => return Err(dest_err(e)),
            }
            ctx.ledger
                .record(
                    &ctx.account,
                    &ctx.source_key,
                    &plan.id,
                    uidvalidity,
                    m.uid,
                    msgid_hash.as_deref(),
                )
                .map_err(|e| FolderErr::Fatal(Stop(format!("Bookkeeping failed: {e}"))))?;
            if let Some(h) = msgid_hash {
                seen_msgids.insert(h);
            }
            this_run.insert(m.uid);
            job.update(|p| {
                p.messages_copied += 1;
                p.bytes_copied += size;
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
