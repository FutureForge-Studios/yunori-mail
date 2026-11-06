//! Mailbox invitations: a link the administrator hands to a person so they
//! choose their own password. The token is shown once (it travels in the URL
//! fragment, `/account/welcome#invite=…`), only its SHA-256 is stored, it works
//! once, and it expires after [`INVITE_DAYS`]. A new invite for the same
//! mailbox replaces any unused one.

use super::Db;
use crate::util::{random_token, sha256_hex};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};

pub const INVITE_DAYS: i64 = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub account_id: String,
    pub address: String,
    pub created_by: String,
    pub expires_at: String,
}

fn alive(expires_at: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(expires_at)
        .map(|e| e > now)
        .unwrap_or(false)
}

impl Db {
    /// A fresh invite for `account_id`; returns `(token, expires_at)`.
    pub fn create_invite(
        &self,
        account_id: &str,
        address: &str,
        created_by: &str,
    ) -> Result<(String, String)> {
        let token = random_token(32);
        let now = Utc::now();
        let expires = (now + chrono::Duration::days(INVITE_DAYS))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM invites WHERE account_id = ?1 AND used_at IS NULL",
            params![account_id],
        )?;
        tx.execute(
            "INSERT INTO invites (token_hash, account_id, address, created_by, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                sha256_hex(token.as_bytes()),
                account_id,
                address,
                created_by,
                now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                expires
            ],
        )?;
        tx.commit()?;
        Ok((token, expires))
    }

    /// The invite behind `token`, if it is unused and not expired.
    pub fn invite(&self, token: &str) -> Result<Option<Invite>> {
        let row = self
            .conn()
            .query_row(
                "SELECT account_id, address, created_by, expires_at FROM invites
                 WHERE token_hash = ?1 AND used_at IS NULL",
                params![sha256_hex(token.as_bytes())],
                |r| {
                    Ok(Invite {
                        account_id: r.get(0)?,
                        address: r.get(1)?,
                        created_by: r.get(2)?,
                        expires_at: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row.filter(|i| alive(&i.expires_at, Utc::now())))
    }

    /// Marks the invite used. `false` when it was already used or has expired,
    /// so two tabs racing on one link cannot both set a password.
    pub fn consume_invite(&self, token: &str) -> Result<bool> {
        let Some(invite) = self.invite(token)? else {
            return Ok(false);
        };
        let n = self.conn().execute(
            "UPDATE invites SET used_at = ?2 WHERE token_hash = ?1 AND used_at IS NULL",
            params![sha256_hex(token.as_bytes()), crate::util::now_rfc3339()],
        )?;
        Ok(n == 1 && alive(&invite.expires_at, Utc::now()))
    }

    /// Undo [`Db::consume_invite`] when setting the password then failed, so
    /// the person can try again with the same link.
    pub fn release_invite(&self, token: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE invites SET used_at = NULL WHERE token_hash = ?1",
            params![sha256_hex(token.as_bytes())],
        )?;
        Ok(())
    }

    /// Drops every unused invite for a mailbox (password reset, deletion).
    pub fn revoke_invites(&self, account_id: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM invites WHERE account_id = ?1 AND used_at IS NULL",
            params![account_id],
        )?)
    }

    /// Expiry of the live invite per mailbox, for the console's mailbox list.
    pub fn pending_invites(&self) -> Result<Vec<(String, String)>> {
        let now = Utc::now();
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT account_id, expires_at FROM invites WHERE used_at IS NULL ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter(|(_, e)| alive(e, now)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_works_once_and_replaces_older_ones() {
        let db = Db::open_memory().unwrap();
        let (first, _) = db
            .create_invite("a1", "ana@example.com", "owner@example.com")
            .unwrap();
        let (second, _) = db
            .create_invite("a1", "ana@example.com", "owner@example.com")
            .unwrap();
        assert!(
            db.invite(&first).unwrap().is_none(),
            "older invite replaced"
        );
        let i = db.invite(&second).unwrap().expect("live invite");
        assert_eq!(i.address, "ana@example.com");
        assert_eq!(db.pending_invites().unwrap().len(), 1);

        assert!(db.consume_invite(&second).unwrap());
        assert!(!db.consume_invite(&second).unwrap(), "single use");
        assert!(db.invite(&second).unwrap().is_none());
        assert!(db.pending_invites().unwrap().is_empty());

        db.release_invite(&second).unwrap();
        assert!(
            db.invite(&second).unwrap().is_some(),
            "released after a failure"
        );
    }

    #[test]
    fn expired_and_revoked_invites_do_not_work() {
        let db = Db::open_memory().unwrap();
        let (token, _) = db
            .create_invite("a2", "bo@example.com", "owner@example.com")
            .unwrap();
        db.conn()
            .execute("UPDATE invites SET expires_at = '2020-01-01T00:00:00Z'", [])
            .unwrap();
        assert!(db.invite(&token).unwrap().is_none());
        assert!(!db.consume_invite(&token).unwrap());

        let (token, _) = db
            .create_invite("a2", "bo@example.com", "owner@example.com")
            .unwrap();
        assert_eq!(db.revoke_invites("a2").unwrap(), 1);
        assert!(db.invite(&token).unwrap().is_none());
        assert!(db.invite("not-a-token").unwrap().is_none());
    }
}
