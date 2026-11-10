//! Console admin authentication: Argon2id passwords, mandatory TOTP, opaque
//! DB-backed sessions (revocable), and the one-time setup token printed by
//! the installer.

use crate::store::Db;
use crate::util::{random_token, sha256_hex};
use anyhow::{Result, anyhow};
use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use totp_rs::{Algorithm, Secret, TOTP};

pub const SESSION_COOKIE: &str = "yunori_session";
pub const PORTAL_COOKIE: &str = "yunori_portal";
pub const SESSION_HOURS: i64 = 12;
const SETUP_TOKEN_KEY: &str = "setup_token_hash";
const SETUP_TOKEN_EXPIRES: &str = "setup_token_expires";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Admin {
    pub id: String,
    pub email: String,
    pub role: String,
    pub totp_enabled: bool,
    pub created_at: String,
}

pub fn hash_password(pw: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("hashing failed: {e}"))
}

pub fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

/// Spend equivalent time when there is no real hash (no timing oracle).
pub fn dummy_verify(pw: &str) {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let hash = DUMMY.get_or_init(|| hash_password("yunori-dummy-password").unwrap_or_default());
    let _ = verify_password(pw, hash);
}

pub fn totp_for(secret_b32: &str, account: &str) -> Result<TOTP> {
    let bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|e| anyhow!("bad totp secret: {e:?}"))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("Yunori Mail".into()),
        account.to_string(),
    )
    .map_err(|e| anyhow!("totp: {e}"))
}

pub fn new_totp_secret() -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    Secret::Raw(bytes).to_encoded().to_string()
}

/// The 30-second time step a code belongs to (current, previous or next), if
/// it matches. Callers record the step so a code is accepted only once.
pub fn totp_step(secret_b32: &str, account: &str, code: &str) -> Option<u64> {
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if code.len() != 6 {
        return None;
    }
    let totp = totp_for(secret_b32, account).ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let step = now / 30;
    [step.saturating_sub(1), step, step + 1]
        .into_iter()
        .find(|s| constant_eq(totp.generate(s * 30).as_bytes(), code.as_bytes()))
}

impl Db {
    /// Accept a TOTP time step for an admin only if it is newer than the last
    /// one accepted: a code (seen over a shoulder or by a phishing proxy)
    /// cannot be replayed within its validity window.
    pub fn accept_totp_step(&self, admin_id: &str, step: u64) -> Result<bool> {
        let key = format!("totp-step:{admin_id}");
        let last: Option<u64> = self.get_json(&key)?;
        if last.is_some_and(|l| step <= l) {
            return Ok(false);
        }
        self.set_json(&key, &step)?;
        Ok(true)
    }
}

#[cfg(test)]
pub fn check_totp(secret_b32: &str, account: &str, code: &str) -> bool {
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    code.len() == 6
        && totp_for(secret_b32, account)
            .map(|t| t.check_current(&code).unwrap_or(false))
            .unwrap_or(false)
}

fn row_to_admin(r: &rusqlite::Row) -> rusqlite::Result<Admin> {
    Ok(Admin {
        id: r.get(0)?,
        email: r.get(1)?,
        role: r.get(2)?,
        totp_enabled: r.get::<_, i64>(3)? != 0,
        created_at: r.get(4)?,
    })
}

impl Db {
    pub fn admin_count_active(&self) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM admins WHERE totp_enabled = 1",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn admin_by_email(&self, email: &str) -> Result<Option<(Admin, String)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, email, role, totp_enabled, created_at, password_hash FROM admins WHERE email = ?1",
                params![email.to_ascii_lowercase()],
                |r| Ok((row_to_admin(r)?, r.get::<_, String>(5)?)),
            )
            .optional()?)
    }

    pub fn admin_by_id(&self, id: &str) -> Result<Option<Admin>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, email, role, totp_enabled, created_at FROM admins WHERE id = ?1",
                params![id],
                row_to_admin,
            )
            .optional()?)
    }

    pub fn admins(&self) -> Result<Vec<Admin>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, email, role, totp_enabled, created_at FROM admins ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], row_to_admin)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Create an admin whose second factor is not yet confirmed. Removes any
    /// earlier unconfirmed admins (an abandoned first-run setup).
    pub fn create_pending_admin(&self, email: &str, password: &str, role: &str) -> Result<Admin> {
        let id = random_token(12);
        let now = crate::util::now_rfc3339();
        let hash = hash_password(password)?;
        {
            let conn = self.conn();
            conn.execute("DELETE FROM admins WHERE totp_enabled = 0", [])?;
            conn.execute(
                "INSERT INTO admins (id, email, password_hash, role, totp_enabled, created_at) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![id, email.to_ascii_lowercase(), hash, role, now],
            )?;
        }
        Ok(self.admin_by_id(&id)?.expect("just created"))
    }

    pub fn enable_totp(&self, admin_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE admins SET totp_enabled = 1 WHERE id = ?1",
            params![admin_id],
        )?;
        Ok(())
    }

    // ── sessions ────────────────────────────────────────────────────────

    /// Returns the raw token for the cookie; only its hash is stored.
    pub fn create_session(&self, kind: &str, subject: &str, hours: i64) -> Result<String> {
        let token = random_token(32);
        let now = chrono::Utc::now();
        self.conn().execute(
            "INSERT INTO sessions (token_hash, kind, subject, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![sha256_hex(token.as_bytes()), kind, subject, now.to_rfc3339(), (now + chrono::Duration::hours(hours)).to_rfc3339()],
        )?;
        Ok(token)
    }

    /// (kind, subject) for a live session.
    pub fn session(&self, token: &str) -> Result<Option<(String, String)>> {
        let row: Option<(String, String, String)> = self
            .conn()
            .query_row(
                "SELECT kind, subject, expires_at FROM sessions WHERE token_hash = ?1",
                params![sha256_hex(token.as_bytes())],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(k, s, exp)| {
            let alive = chrono::DateTime::parse_from_rfc3339(&exp)
                .map(|e| e > chrono::Utc::now())
                .unwrap_or(false);
            alive.then_some((k, s))
        }))
    }

    pub fn delete_session(&self, token: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            params![sha256_hex(token.as_bytes())],
        )?;
        Ok(())
    }

    pub fn upgrade_session(&self, token: &str, kind: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE sessions SET kind = ?2 WHERE token_hash = ?1",
            params![sha256_hex(token.as_bytes()), kind],
        )?;
        Ok(())
    }

    pub fn purge_expired_sessions(&self) -> Result<()> {
        self.conn().execute(
            "DELETE FROM sessions WHERE expires_at < ?1",
            params![chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    // ── setup token ─────────────────────────────────────────────────────

    /// Issue a fresh one-time setup token (valid 24h). Returns the raw token.
    pub fn issue_setup_token(&self) -> Result<String> {
        let token = random_token(18);
        self.set_kv(SETUP_TOKEN_KEY, &sha256_hex(token.as_bytes()))?;
        self.set_kv(
            SETUP_TOKEN_EXPIRES,
            &(chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339(),
        )?;
        Ok(token)
    }

    pub fn check_setup_token(&self, token: &str) -> Result<bool> {
        let (Some(hash), Some(exp)) = (
            self.get_kv(SETUP_TOKEN_KEY)?,
            self.get_kv(SETUP_TOKEN_EXPIRES)?,
        ) else {
            return Ok(false);
        };
        let alive = chrono::DateTime::parse_from_rfc3339(&exp)
            .map(|e| e > chrono::Utc::now())
            .unwrap_or(false);
        let given = sha256_hex(token.trim().as_bytes());
        Ok(alive && constant_eq(given.as_bytes(), hash.as_bytes()))
    }

    pub fn consume_setup_token(&self) -> Result<()> {
        self.delete_kv(SETUP_TOKEN_KEY)?;
        self.delete_kv(SETUP_TOKEN_EXPIRES)
    }
}

pub fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_roundtrip() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong", &h));
        dummy_verify("anything");
    }

    #[test]
    fn totp_accepts_current_code_only() {
        let secret = new_totp_secret();
        let t = totp_for(&secret, "owner@x.test").unwrap();
        let code = t.generate_current().unwrap();
        assert!(check_totp(&secret, "owner@x.test", &code));
        assert!(check_totp(
            &secret,
            "owner@x.test",
            &format!("{} {}", &code[..3], &code[3..])
        ));
        assert!(!check_totp(&secret, "owner@x.test", "000000") || code == "000000");
        assert!(t.get_url().starts_with("otpauth://totp/"));
    }

    #[test]
    fn sessions_and_setup_tokens() {
        let db = Db::open_memory().unwrap();
        let tok = db.create_session("admin", "a1", 1).unwrap();
        assert_eq!(
            db.session(&tok).unwrap(),
            Some(("admin".into(), "a1".into()))
        );
        db.delete_session(&tok).unwrap();
        assert_eq!(db.session(&tok).unwrap(), None);

        let st = db.issue_setup_token().unwrap();
        assert!(db.check_setup_token(&st).unwrap());
        assert!(!db.check_setup_token("nope").unwrap());
        db.consume_setup_token().unwrap();
        assert!(!db.check_setup_token(&st).unwrap());
    }

    #[test]
    fn abandoned_setup_is_replaced() {
        let db = Db::open_memory().unwrap();
        db.create_pending_admin("a@x.test", "password-123456", "owner")
            .unwrap();
        db.create_pending_admin("b@x.test", "password-123456", "owner")
            .unwrap();
        assert_eq!(db.admins().unwrap().len(), 1);
        assert_eq!(db.admin_count_active().unwrap(), 0);
    }
}
