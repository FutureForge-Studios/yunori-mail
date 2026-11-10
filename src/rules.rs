//! Mailbox rules a person sets in the portal: an out-of-office auto-reply and
//! forwarding. Both become ONE Sieve script owned by Yunori (`yunori-rules`),
//! uploaded and activated as the mailbox user (the portal acts as the user).
//!
//! The settings travel inside the script, on its first line, so Yunori keeps
//! no per-user state and what the portal shows is what the engine runs.
//! Everything a person types reaches Sieve only as an escaped quoted string.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

pub const SCRIPT_NAME: &str = "yunori-rules";
const MARKER: &str = "# yunori-rules v1 ";
const MAX_SUBJECT: usize = 200;
const MAX_BODY: usize = 5000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vacation {
    pub enabled: bool,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    /// First day (inclusive, `YYYY-MM-DD`, server time). None: from now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    /// Last day (inclusive). None: until switched off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Forwarding {
    pub enabled: bool,
    #[serde(default)]
    pub address: String,
    /// Keep a copy in this mailbox too (default) or only forward.
    #[serde(default = "yes")]
    pub keep_copy: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailRules {
    #[serde(default)]
    pub vacation: Vacation,
    #[serde(default)]
    pub forwarding: Forwarding,
}

#[derive(Debug, PartialEq)]
pub struct RuleError {
    pub field: &'static str,
    pub message: String,
}

fn err(field: &'static str, message: impl Into<String>) -> RuleError {
    RuleError {
        field,
        message: message.into(),
    }
}

fn valid_date(d: &str) -> bool {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok() && d.len() == 10
}

/// Text as it may appear in a reply: no control characters but newlines/tabs,
/// CRLF normalised to LF, trimmed.
fn clean(s: &str, keep_newlines: bool) -> String {
    s.replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || (keep_newlines && (*c == '\n' || *c == '\t')))
        .collect::<String>()
        .trim()
        .to_string()
}

impl MailRules {
    /// Validate and normalise what a person submitted for mailbox `own`.
    pub fn normalised(mut self, own: &str) -> Result<Self, RuleError> {
        let v = &mut self.vacation;
        v.subject = clean(&v.subject, false);
        v.body = clean(&v.body, true);
        v.start_date = v.start_date.take().filter(|d| !d.trim().is_empty());
        v.end_date = v.end_date.take().filter(|d| !d.trim().is_empty());
        if v.enabled {
            if v.body.is_empty() {
                return Err(err("vacation.body", "Write the reply people will get."));
            }
            if v.subject.is_empty() {
                v.subject = "Out of office".into();
            }
        }
        if v.subject.chars().count() > MAX_SUBJECT {
            return Err(err(
                "vacation.subject",
                format!("Keep the subject under {MAX_SUBJECT} characters."),
            ));
        }
        if v.body.chars().count() > MAX_BODY {
            return Err(err(
                "vacation.body",
                format!("Keep the reply under {MAX_BODY} characters."),
            ));
        }
        for (field, d) in [
            ("vacation.startDate", &v.start_date),
            ("vacation.endDate", &v.end_date),
        ] {
            if let Some(d) = d {
                if !valid_date(d) {
                    return Err(err(field, "Use a date like 2026-12-24."));
                }
            }
        }
        if let (Some(a), Some(b)) = (&v.start_date, &v.end_date) {
            if b < a {
                return Err(err(
                    "vacation.endDate",
                    "The last day comes before the first day.",
                ));
            }
        }

        let f = &mut self.forwarding;
        f.address = f.address.trim().to_ascii_lowercase();
        if f.enabled {
            if !crate::util::valid_email(&f.address) {
                return Err(err("forwarding.address", "Enter a valid email address."));
            }
            if f.address == own.to_ascii_lowercase() {
                return Err(err(
                    "forwarding.address",
                    "That is this mailbox; forward to another address.",
                ));
            }
        }
        Ok(self)
    }

    pub fn is_empty(&self) -> bool {
        !self.vacation.enabled && !self.forwarding.enabled
    }

    /// The Sieve script for these (normalised) rules.
    pub fn to_sieve(&self) -> String {
        let meta = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(self).unwrap_or_default());
        let mut out = format!("{MARKER}{meta}\n");
        out.push_str(
            "# Managed by Yunori Mail Server (your account page). Changes here are overwritten.\n",
        );
        out.push_str("require [\"vacation\", \"copy\", \"date\", \"relational\"];\n");
        let v = &self.vacation;
        if v.enabled {
            let mut conds = vec![];
            if let Some(d) = &v.start_date {
                conds.push(format!("currentdate :value \"ge\" \"date\" {}", quote(d)));
            }
            if let Some(d) = &v.end_date {
                conds.push(format!("currentdate :value \"le\" \"date\" {}", quote(d)));
            }
            let action = format!(
                "vacation :days 1 :subject {} {};",
                quote(&v.subject),
                quote(&v.body)
            );
            if conds.is_empty() {
                out.push_str(&action);
                out.push('\n');
            } else {
                out.push_str(&format!(
                    "if allof({}) {{\n    {action}\n}}\n",
                    conds.join(", ")
                ));
            }
        }
        let f = &self.forwarding;
        if f.enabled {
            if f.keep_copy {
                out.push_str(&format!("redirect :copy {};\n", quote(&f.address)));
            } else {
                out.push_str(&format!("redirect {};\n", quote(&f.address)));
            }
        }
        out
    }

    /// The rules a Yunori script was generated from (None for any other text).
    pub fn from_sieve(script: &str) -> Option<Self> {
        let first = script.lines().next()?;
        let b64 = first.strip_prefix(MARKER)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

/// A Sieve quoted string (RFC 5228 §2.4.2): `\` and `"` escaped; newlines are
/// allowed inside quoted strings.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(vac: bool, fwd: bool) -> MailRules {
        MailRules {
            vacation: Vacation {
                enabled: vac,
                subject: "Away".into(),
                body: "Back on Monday.\r\nAna".into(),
                start_date: Some("2026-12-20".into()),
                end_date: Some("2026-12-31".into()),
            },
            forwarding: Forwarding {
                enabled: fwd,
                address: " Ana@Elsewhere.Example ".into(),
                keep_copy: true,
            },
        }
    }

    #[test]
    fn round_trips_through_the_script() {
        let r = rules(true, true).normalised("ana@x.test").unwrap();
        assert_eq!(r.forwarding.address, "ana@elsewhere.example");
        assert_eq!(r.vacation.body, "Back on Monday.\nAna");
        let s = r.to_sieve();
        assert!(s.contains("redirect :copy \"ana@elsewhere.example\";"));
        assert!(s.contains("currentdate :value \"ge\" \"date\" \"2026-12-20\""));
        assert!(s.contains("vacation :days 1 :subject \"Away\" \"Back on Monday.\nAna\";"));
        assert_eq!(MailRules::from_sieve(&s), Some(r));
        assert_eq!(MailRules::from_sieve("require \"fileinto\";"), None);
    }

    #[test]
    fn user_text_cannot_escape_the_string() {
        let mut r = rules(true, false);
        r.vacation.subject = "x\"; redirect \"evil@attacker.example".into();
        r.vacation.body = "a\\\";\ndiscard;\n\"".into();
        let s = r.normalised("ana@x.test").unwrap().to_sieve();
        assert!(!s.contains("\nredirect"), "{s}");
        assert!(
            s.contains(r#":subject "x\"; redirect \"evil@attacker.example""#),
            "{s}"
        );
        // Every statement line outside strings is one we wrote.
        let body_line = s.lines().find(|l| l.contains("vacation :days")).unwrap();
        assert!(body_line.contains(r#""a\\\";"#), "{s}");
    }

    #[test]
    fn validation() {
        let own = "ana@x.test";
        let mut r = rules(false, true);
        r.forwarding.address = "ana@x.test".into();
        assert_eq!(r.normalised(own).unwrap_err().field, "forwarding.address");
        let mut r = rules(false, true);
        r.forwarding.address = "not-an-address".into();
        assert_eq!(r.normalised(own).unwrap_err().field, "forwarding.address");
        let mut r = rules(true, false);
        r.vacation.body = "  ".into();
        assert_eq!(r.normalised(own).unwrap_err().field, "vacation.body");
        let mut r = rules(true, false);
        r.vacation.end_date = Some("2026-12-01".into());
        assert_eq!(r.normalised(own).unwrap_err().field, "vacation.endDate");
        let mut r = rules(true, false);
        r.vacation.start_date = Some("20-12-2026".into());
        assert_eq!(r.normalised(own).unwrap_err().field, "vacation.startDate");
        let mut r = rules(true, false);
        r.vacation.subject = String::new();
        r.vacation.start_date = None;
        r.vacation.end_date = None;
        let n = r.normalised(own).unwrap();
        assert_eq!(n.vacation.subject, "Out of office");
        assert!(!n.to_sieve().contains("if allof"));
        // Disabled rules may be half-filled.
        let mut r = rules(false, false);
        r.forwarding.address = "x".into();
        assert!(r.normalised(own).unwrap().is_empty());
    }

    #[test]
    fn forward_only_drops_the_local_copy() {
        let mut r = rules(false, true);
        r.forwarding.keep_copy = false;
        let s = r.normalised("ana@x.test").unwrap().to_sieve();
        assert!(s.contains("\nredirect \"ana@elsewhere.example\";"));
        assert!(!s.contains(":copy \"ana"));
    }
}
