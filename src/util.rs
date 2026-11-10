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

/// The first sentence of a plain-language message (headlines keep it short;
/// the rest, usually the remedy, is shown where the step is).
pub fn first_sentence(s: &str) -> &str {
    match s.find(". ") {
        Some(i) => &s[..=i],
        None => s,
    }
}

/// "in 2 minutes", "in about 3 hours", "now" — for an RFC 3339 time.
pub fn when(rfc3339: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return "later".into();
    };
    let secs = (t.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds();
    match secs {
        s if s <= 5 => "now".into(),
        s if s < 90 => format!("in {s} seconds"),
        s if s < 90 * 60 => format!("in {} minutes", (s + 30) / 60),
        s => {
            let h = (s + 1800) / 3600;
            format!("in about {h} hour{}", if h == 1 { "" } else { "s" })
        }
    }
}

/// Constant-time comparison of two byte strings (length is not secret).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sentence_keeps_hostnames_whole() {
        assert_eq!(
            first_sentence(
                "Let's Encrypt could not connect to mail.example.com on port 80. Allow TCP 80."
            ),
            "Let's Encrypt could not connect to mail.example.com on port 80."
        );
        assert_eq!(first_sentence("One sentence."), "One sentence.");
    }

    #[test]
    fn when_reads_naturally() {
        let at = |secs: i64| {
            (chrono::Utc::now() + chrono::Duration::seconds(secs))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        assert_eq!(when(&at(-10)), "now");
        assert!(when(&at(45)).ends_with("seconds"));
        assert_eq!(when(&at(300)), "in 5 minutes");
        assert_eq!(when(&at(3 * 3600)), "in about 3 hours");
        assert_eq!(when("garbage"), "later");
    }

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
