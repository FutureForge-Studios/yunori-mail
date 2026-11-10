//! Append-only, hash-chained audit ledger (port of the Yunori client's
//! `server/admin/audit.ts` design). Each row's hash covers the previous row's
//! hash, so any edit or deletion breaks `verify()` from that row onward.

use super::Db;
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: i64,
    pub at: String,
    pub actor: String,
    pub action: String,
    pub target: Option<String>,
    pub detail: Option<String>,
}

const GENESIS: &str = "yunori-mail-audit-genesis";

fn row_hash(prev: &str, at: &str, actor: &str, action: &str, target: &str, detail: &str) -> String {
    let mut h = Sha256::new();
    for part in [prev, at, actor, action, target, detail] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    hex::encode(h.finalize())
}

impl Db {
    /// Record a mutation. Never pass secrets or mail content in `detail`.
    pub fn audit(
        &self,
        actor: &str,
        action: &str,
        target: Option<&str>,
        detail: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        // IMMEDIATE: take the write lock before reading the chain head, so the
        // CLI and the service appending at once wait for each other (busy
        // timeout) instead of failing with a stale snapshot.
        let tx =
            rusqlite::Transaction::new_unchecked(&conn, rusqlite::TransactionBehavior::Immediate)?;
        let prev: String = tx
            .query_row("SELECT hash FROM audit ORDER BY id DESC LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap_or_else(|_| GENESIS.to_string());
        let at = chrono::Utc::now().to_rfc3339();
        let hash = row_hash(
            &prev,
            &at,
            actor,
            action,
            target.unwrap_or(""),
            detail.unwrap_or(""),
        );
        tx.execute(
            "INSERT INTO audit (at, actor, action, target, detail, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![at, actor, action, target, detail, prev, hash],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn audit_list(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, actor, action, target, detail FROM audit ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(AuditEntry {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    actor: r.get(2)?,
                    action: r.get(3)?,
                    target: r.get(4)?,
                    detail: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Walk the whole chain; false if any row was altered, removed or reordered.
    pub fn audit_verify(&self) -> Result<bool> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT at, actor, action, COALESCE(target,''), COALESCE(detail,''), prev_hash, hash FROM audit ORDER BY id ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut prev = GENESIS.to_string();
        while let Some(r) = rows.next()? {
            let (at, actor, action, target, detail, stored_prev, stored): (
                String,
                String,
                String,
                String,
                String,
                String,
                String,
            ) = (
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            );
            if stored_prev != prev
                || row_hash(&prev, &at, &actor, &action, &target, &detail) != stored
            {
                return Ok(false);
            }
            prev = stored;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Db;

    #[test]
    fn chain_detects_tampering() {
        let db = Db::open_memory().unwrap();
        db.audit("owner@x.test", "mailbox.create", Some("a@x.test"), None)
            .unwrap();
        db.audit(
            "owner@x.test",
            "domain.create",
            Some("y.test"),
            Some("dkim automatic"),
        )
        .unwrap();
        assert!(db.audit_verify().unwrap());
        assert_eq!(db.audit_list(10).unwrap().len(), 2);

        db.conn()
            .execute("UPDATE audit SET target = 'evil' WHERE id = 1", [])
            .unwrap();
        assert!(!db.audit_verify().unwrap());
    }

    #[test]
    fn deleting_a_row_breaks_the_chain() {
        let db = Db::open_memory().unwrap();
        for i in 0..3 {
            db.audit("system", "tick", Some(&i.to_string()), None)
                .unwrap();
        }
        db.conn()
            .execute("DELETE FROM audit WHERE id = 2", [])
            .unwrap();
        assert!(!db.audit_verify().unwrap());
    }
}
