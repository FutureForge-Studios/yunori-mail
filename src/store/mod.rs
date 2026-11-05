//! Local state for `yunori-mail`: one SQLite file (`/var/lib/yunori-mail/yunori.db`).
//!
//! Holds the installer state machine, console admins + sessions, sealed secrets,
//! check caches and the audit ledger. Mail data never lives here — that is
//! Stalwart's store. Losing this file is recoverable (`yunori-mail adopt`).

pub mod audit;
pub mod vault;

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use vault::Vault;
use zeroize::Zeroizing;

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE IF NOT EXISTS kv (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS secrets (
  name TEXT PRIMARY KEY,
  nonce BLOB NOT NULL,
  ciphertext BLOB NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS install_steps (
  id TEXT PRIMARY KEY,
  ord INTEGER NOT NULL,
  title TEXT NOT NULL,
  status TEXT NOT NULL,
  detail TEXT,
  started_at TEXT,
  finished_at TEXT
);
CREATE TABLE IF NOT EXISTS install_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  level TEXT NOT NULL,
  step TEXT,
  message TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS admins (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL,
  totp_enabled INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  subject TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS cache (
  key TEXT PRIMARY KEY,
  json TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  target TEXT,
  detail TEXT,
  prev_hash TEXT NOT NULL,
  hash TEXT NOT NULL
);
"#];

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    vault: Option<Vault>,
}

impl Db {
    pub fn open(path: &Path, vault: Vault) -> Result<Self> {
        // Only tighten modes on files we create: the root CLI and the
        // unprivileged service share this database through group `yunori`
        // (see installer::fix_permissions), so existing modes are left alone.
        if let Some(dir) = path.parent()
            && !dir.exists()
        {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            vault::set_mode(dir, 0o770);
        }
        let created = !path.exists();
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        if created {
            vault::set_mode(path, 0o660);
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            vault: Some(vault),
        };
        db.migrate()?;
        Ok(db)
    }

    /// In-memory database with a fixed test key.
    #[cfg(test)]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            vault: Some(Vault::from_key([42u8; 32])),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", (i + 1) as i64)?;
        }
        Ok(())
    }

    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ── kv ──────────────────────────────────────────────────────────────

    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM kv WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn delete_kv(&self, key: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM kv WHERE key = ?1", params![key])?;
        Ok(())
    }

    pub fn get_json<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        match self.get_kv(key)? {
            Some(s) => Ok(Some(serde_json::from_str(&s)?)),
            None => Ok(None),
        }
    }

    pub fn set_json<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.set_kv(key, &serde_json::to_string(value)?)
    }

    // ── secrets ─────────────────────────────────────────────────────────

    fn vault(&self) -> Result<&Vault> {
        self.vault
            .as_ref()
            .ok_or_else(|| anyhow!("vault unavailable"))
    }

    pub fn put_secret(&self, name: &str, value: &str) -> Result<()> {
        let (nonce, ct) = self.vault()?.seal(name, value.as_bytes())?;
        self.conn().execute(
            "INSERT INTO secrets (name, nonce, ciphertext, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET nonce = excluded.nonce, ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
            params![name, nonce, ct, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn get_secret(&self, name: &str) -> Result<Option<Zeroizing<String>>> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .conn()
            .query_row(
                "SELECT nonce, ciphertext FROM secrets WHERE name = ?1",
                params![name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((nonce, ct)) => {
                let pt = self.vault()?.open(name, &nonce, &ct)?;
                Ok(Some(Zeroizing::new(String::from_utf8(pt.to_vec())?)))
            }
        }
    }

    pub fn has_secret(&self, name: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM secrets WHERE name = ?1",
                params![name],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    #[cfg(test)]
    pub fn delete_secret(&self, name: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM secrets WHERE name = ?1", params![name])?;
        Ok(())
    }

    // ── caches ──────────────────────────────────────────────────────────

    pub fn cache_put<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.conn().execute(
            "INSERT INTO cache (key, json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
            params![
                key,
                serde_json::to_string(value)?,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn cache_get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<(T, String)>> {
        let row: Option<(String, String)> = self
            .conn()
            .query_row(
                "SELECT json, updated_at FROM cache WHERE key = ?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((json, at)) => Some((serde_json::from_str(&json)?, at)),
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_and_secrets_roundtrip() {
        let db = Db::open_memory().unwrap();
        assert_eq!(db.get_kv("a").unwrap(), None);
        db.set_kv("a", "1").unwrap();
        db.set_kv("a", "2").unwrap();
        assert_eq!(db.get_kv("a").unwrap().as_deref(), Some("2"));

        db.put_secret("relay.password", "s3cret").unwrap();
        assert!(db.has_secret("relay.password").unwrap());
        assert_eq!(
            db.get_secret("relay.password").unwrap().unwrap().as_str(),
            "s3cret"
        );
        // Ciphertext at rest never contains the plaintext.
        let ct: Vec<u8> = db
            .conn()
            .query_row(
                "SELECT ciphertext FROM secrets WHERE name='relay.password'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!ct.windows(6).any(|w| w == b"s3cret"));
        db.delete_secret("relay.password").unwrap();
        assert!(db.get_secret("relay.password").unwrap().is_none());
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("y.db");
        let v = Vault::from_key([1u8; 32]);
        Db::open(&path, v.clone())
            .unwrap()
            .set_kv("k", "v")
            .unwrap();
        let db = Db::open(&path, v).unwrap();
        assert_eq!(db.get_kv("k").unwrap().as_deref(), Some("v"));
    }
}
