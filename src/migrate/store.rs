//! Migration bookkeeping in the Yunori database: which source messages were
//! already copied (folder + UIDVALIDITY + UID, plus a hash of the Message-ID
//! for when a server renumbers a folder), the last run's progress, and a
//! lease so the portal and the CLI never run two imports into one mailbox.
//! No message content and no password is ever stored here.
//!
//! The tables are created on first use (`CREATE TABLE IF NOT EXISTS`), so
//! this feature adds no numbered migration to `store::MIGRATIONS`.

use crate::store::Db;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use std::collections::HashSet;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS migration_items (
  account TEXT NOT NULL,
  source TEXT NOT NULL,
  folder TEXT NOT NULL,
  uidvalidity INTEGER NOT NULL,
  uid INTEGER NOT NULL,
  msgid_hash TEXT,
  copied_at TEXT NOT NULL,
  PRIMARY KEY (account, source, folder, uidvalidity, uid)
);
CREATE INDEX IF NOT EXISTS migration_items_msgid
  ON migration_items (account, source, folder, msgid_hash);
CREATE TABLE IF NOT EXISTS migration_runs (
  account TEXT PRIMARY KEY,
  json TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS migration_leases (
  account TEXT PRIMARY KEY,
  owner TEXT NOT NULL,
  expires_at INTEGER NOT NULL
);
"#;

/// What has already been copied from one source folder.
#[derive(Default)]
pub struct Copied {
    pub uids: HashSet<u32>,
    pub msgids: HashSet<String>,
}

impl Copied {
    #[cfg(test)]
    pub fn contains(&self, uid: u32, msgid_hash: Option<&str>) -> bool {
        self.uids.contains(&uid) || msgid_hash.is_some_and(|h| self.msgids.contains(h))
    }
}

/// Handle on the bookkeeping tables (creates them on open).
#[derive(Clone)]
pub struct Ledger {
    db: Db,
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Ledger {
    pub fn open(db: &Db) -> Result<Self> {
        db.conn().execute_batch(SCHEMA)?;
        Ok(Self { db: db.clone() })
    }

    /// Everything copied from `folder` (any UIDVALIDITY contributes its
    /// Message-IDs; only the current one contributes UIDs).
    pub fn copied(
        &self,
        account: &str,
        source: &str,
        folder: &str,
        uidvalidity: u64,
    ) -> Result<Copied> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT uidvalidity, uid, msgid_hash FROM migration_items
             WHERE account = ?1 AND source = ?2 AND folder = ?3",
        )?;
        let mut out = Copied::default();
        let rows = stmt.query_map(params![account, source, folder], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (v, uid, h) = row?;
            if v as u64 == uidvalidity {
                out.uids.insert(uid as u32);
            }
            if let Some(h) = h {
                out.msgids.insert(h);
            }
        }
        Ok(out)
    }

    pub fn record(
        &self,
        account: &str,
        source: &str,
        folder: &str,
        uidvalidity: u64,
        uid: u32,
        msgid_hash: Option<&str>,
    ) -> Result<()> {
        self.db.conn().execute(
            "INSERT OR IGNORE INTO migration_items
               (account, source, folder, uidvalidity, uid, msgid_hash, copied_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                account,
                source,
                folder,
                uidvalidity as i64,
                uid as i64,
                msgid_hash,
                crate::util::now_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// How many messages were copied for this account (any source).
    #[cfg(test)]
    pub fn copied_total(&self, account: &str) -> Result<u64> {
        let n: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM migration_items WHERE account = ?1",
            params![account],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    pub fn save_run(&self, account: &str, json: &str) -> Result<()> {
        self.db.conn().execute(
            "INSERT INTO migration_runs (account, json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(account) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
            params![account, json, crate::util::now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn last_run(&self, account: &str) -> Result<Option<String>> {
        Ok(self
            .db
            .conn()
            .query_row(
                "SELECT json FROM migration_runs WHERE account = ?1",
                params![account],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Take the per-mailbox lease for `ttl_secs`; false if someone else holds it.
    pub fn acquire(&self, account: &str, owner: &str, ttl_secs: i64) -> Result<bool> {
        let mut conn = self.db.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM migration_leases WHERE account = ?1 AND expires_at < ?2",
            params![account, now_unix()],
        )?;
        let n = tx.execute(
            "INSERT OR IGNORE INTO migration_leases (account, owner, expires_at) VALUES (?1, ?2, ?3)",
            params![account, owner, now_unix() + ttl_secs],
        )?;
        tx.commit()?;
        Ok(n == 1)
    }

    pub fn renew(&self, account: &str, owner: &str, ttl_secs: i64) -> Result<()> {
        self.db.conn().execute(
            "UPDATE migration_leases SET expires_at = ?3 WHERE account = ?1 AND owner = ?2",
            params![account, owner, now_unix() + ttl_secs],
        )?;
        Ok(())
    }

    pub fn release(&self, account: &str, owner: &str) -> Result<()> {
        self.db.conn().execute(
            "DELETE FROM migration_leases WHERE account = ?1 AND owner = ?2",
            params![account, owner],
        )?;
        Ok(())
    }

    /// Someone (this or another process) holds an unexpired lease.
    pub fn leased(&self, account: &str) -> Result<bool> {
        let n: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM migration_leases WHERE account = ?1 AND expires_at >= ?2",
            params![account, now_unix()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_copies_by_uid_and_message_id() {
        let db = Db::open_memory().unwrap();
        let l = Ledger::open(&db).unwrap();
        let _ = Ledger::open(&db).unwrap(); // idempotent schema
        l.record("a@x", "imap.example.com|me", "INBOX", 7, 1, Some("h1"))
            .unwrap();
        l.record("a@x", "imap.example.com|me", "INBOX", 7, 2, None)
            .unwrap();
        l.record("a@x", "imap.example.com|me", "INBOX", 7, 2, None)
            .unwrap(); // duplicate ignored
        l.record("a@x", "imap.example.com|me", "Sent", 7, 1, Some("h9"))
            .unwrap();
        let c = l.copied("a@x", "imap.example.com|me", "INBOX", 7).unwrap();
        assert!(c.contains(1, None) && c.contains(2, None) && !c.contains(3, None));
        // The folder was renumbered (new UIDVALIDITY): only Message-IDs match.
        let c = l.copied("a@x", "imap.example.com|me", "INBOX", 8).unwrap();
        assert!(!c.contains(1, None));
        assert!(c.contains(42, Some("h1")));
        assert!(!c.contains(42, Some("h9")), "other folders don't count");
        // Other accounts and sources are separate.
        assert!(
            l.copied("b@x", "imap.example.com|me", "INBOX", 7)
                .unwrap()
                .uids
                .is_empty()
        );
        assert_eq!(l.copied_total("a@x").unwrap(), 3);
    }

    #[test]
    fn one_lease_per_mailbox() {
        let db = Db::open_memory().unwrap();
        let l = Ledger::open(&db).unwrap();
        assert!(l.acquire("a@x", "one", 60).unwrap());
        assert!(!l.acquire("a@x", "two", 60).unwrap());
        assert!(l.leased("a@x").unwrap());
        assert!(l.acquire("b@x", "two", 60).unwrap());
        l.release("a@x", "two").unwrap(); // not the owner: no effect
        assert!(l.leased("a@x").unwrap());
        l.release("a@x", "one").unwrap();
        assert!(l.acquire("a@x", "two", 60).unwrap());
        // An expired lease is taken over.
        assert!(l.acquire("c@x", "old", -10).unwrap());
        assert!(l.acquire("c@x", "new", 60).unwrap());
    }

    #[test]
    fn last_run_round_trip() {
        let db = Db::open_memory().unwrap();
        let l = Ledger::open(&db).unwrap();
        assert_eq!(l.last_run("a@x").unwrap(), None);
        l.save_run("a@x", "{\"state\":\"done\"}").unwrap();
        l.save_run("a@x", "{\"state\":\"failed\"}").unwrap();
        assert_eq!(
            l.last_run("a@x").unwrap().as_deref(),
            Some("{\"state\":\"failed\"}")
        );
    }
}
