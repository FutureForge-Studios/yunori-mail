//! Small shared helpers.

use rand::Rng;
use rand::seq::SliceRandom;

/// A strong random password: mixed classes, no ambiguous characters.
pub fn generate_password(len: usize) -> String {
    const LOWER: &[u8] = b"abcdefghijkmnopqrstuvwxyz";
    const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGIT: &[u8] = b"23456789";
    const SYM: &[u8] = b"-_.!@#%+=";
    let len = len.max(16);
    let mut rng = rand::rngs::OsRng;
    let mut out: Vec<u8> = vec![
        LOWER[rng.gen_range(0..LOWER.len())],
        UPPER[rng.gen_range(0..UPPER.len())],
        DIGIT[rng.gen_range(0..DIGIT.len())],
        SYM[rng.gen_range(0..SYM.len())],
    ];
    let all: Vec<u8> = [LOWER, UPPER, DIGIT, SYM].concat();
    while out.len() < len {
        out.push(all[rng.gen_range(0..all.len())]);
    }
    out.shuffle(&mut rng);
    String::from_utf8(out).expect("ascii")
}

/// URL-safe random token (base64url, no padding).
pub fn random_token(bytes: usize) -> String {
    use base64::Engine as _;
    let mut buf = vec![0u8; bytes];
    rand::rngs::OsRng.fill(&mut buf[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Lower-case, trimmed, trailing-dot-free DNS name; None if not a plausible hostname.
pub fn normalize_domain(input: &str) -> Option<String> {
    let d = input.trim().trim_end_matches('.').to_ascii_lowercase();
    if d.len() < 3 || d.len() > 253 || !d.contains('.') {
        return None;
    }
    let ok = d.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    if ok
        && !d
            .split('.')
            .next_back()
            .unwrap_or("")
            .chars()
            .all(|c| c.is_ascii_digit())
    {
        Some(d)
    } else {
        None
    }
}

/// Validate an email local part we are willing to create.
pub fn valid_local_part(local: &str) -> bool {
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

pub fn valid_email(email: &str) -> bool {
    match email.rsplit_once('@') {
        Some((l, d)) => valid_local_part(l) && normalize_domain(d).is_some(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_strong() {
        for _ in 0..50 {
            let p = generate_password(20);
            assert_eq!(p.len(), 20);
            assert!(p.chars().any(|c| c.is_ascii_lowercase()));
            assert!(p.chars().any(|c| c.is_ascii_uppercase()));
            assert!(p.chars().any(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn domain_normalisation() {
        assert_eq!(
            normalize_domain(" Example.COM. ").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            normalize_domain("mail.example.co.jp").as_deref(),
            Some("mail.example.co.jp")
        );
        assert!(normalize_domain("localhost").is_none());
        assert!(normalize_domain("-bad.com").is_none());
        assert!(normalize_domain("1.2.3.4").is_none());
        assert!(normalize_domain("a..b").is_none());
    }

    #[test]
    fn local_parts_and_emails() {
        assert!(valid_local_part("ana.ito+news"));
        assert!(!valid_local_part(".ana"));
        assert!(!valid_local_part("a b"));
        assert!(valid_email("ana@example.com"));
        assert!(!valid_email("ana@localhost"));
    }
}
