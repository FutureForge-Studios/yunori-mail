//! Tolerant parser for the BIND-style zone data Stalwart exposes as
//! `Domain.dnsZoneFile`, plus the reverse (rendering records for "copy all").

#[derive(Debug, Clone, PartialEq)]
pub struct ZoneRecord {
    /// Fully-qualified, lower-case, no trailing dot.
    pub name: String,
    pub rtype: String,
    /// Record data; TXT strings are unquoted and concatenated.
    pub value: String,
}

const TYPES: &[&str] = &[
    "A", "AAAA", "MX", "TXT", "CNAME", "SRV", "CAA", "TLSA", "NS", "PTR",
];

fn tokenize(line: &str) -> Vec<(String, bool)> {
    // (token, was_quoted)
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_q => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '"' => {
                in_q = !in_q;
                quoted = true;
                if !in_q {
                    out.push((std::mem::take(&mut cur), true));
                    quoted = false;
                }
            }
            ';' if !in_q => break,
            c if c.is_whitespace() && !in_q => {
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), quoted));
                }
                quoted = false;
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push((cur, quoted));
    }
    out
}

fn qualify(name: &str, origin: &str) -> String {
    let n = name.trim();
    if n == "@" || n.is_empty() {
        return origin.to_string();
    }
    if let Some(stripped) = n.strip_suffix('.') {
        return stripped.to_ascii_lowercase();
    }
    let lower = n.to_ascii_lowercase();
    if lower == origin || lower.ends_with(&format!(".{origin}")) {
        lower
    } else {
        format!("{lower}.{origin}")
    }
}

/// Join physical lines into logical records: a `(` … `)` group (outside
/// quotes) may span lines — Stalwart writes long RSA DKIM keys that way.
/// Comments (`;` outside quotes) are dropped.
fn logical_lines(zone: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut acc = String::new();
    let mut depth: i32 = 0;
    for line in zone.lines() {
        let mut in_q = false;
        let mut escaped = false;
        let mut kept = String::new();
        for c in line.chars() {
            if escaped {
                escaped = false;
                kept.push(c);
                continue;
            }
            match c {
                '\\' if in_q => {
                    escaped = true;
                    kept.push(c);
                }
                '"' => {
                    in_q = !in_q;
                    kept.push(c);
                }
                ';' if !in_q => break,
                '(' if !in_q => {
                    depth += 1;
                    kept.push(' ');
                }
                ')' if !in_q => {
                    depth = (depth - 1).max(0);
                    kept.push(' ');
                }
                c => kept.push(c),
            }
        }
        if !acc.is_empty() {
            acc.push(' ');
        }
        acc.push_str(&kept);
        if depth == 0 {
            if !acc.trim().is_empty() {
                out.push(std::mem::take(&mut acc));
            }
            acc.clear();
        }
    }
    if !acc.trim().is_empty() {
        out.push(acc);
    }
    out
}

pub fn parse(zone: &str, origin: &str) -> Vec<ZoneRecord> {
    let origin = origin.trim_end_matches('.').to_ascii_lowercase();
    let mut out = Vec::new();
    for line in logical_lines(zone) {
        let tokens = tokenize(&line);
        if tokens.is_empty() || tokens[0].0.starts_with('$') {
            continue;
        }
        let Some(ti) = tokens
            .iter()
            .position(|(t, q)| !q && TYPES.contains(&t.to_ascii_uppercase().as_str()))
        else {
            continue;
        };
        if ti == 0 {
            continue; // no owner name
        }
        let rtype = tokens[ti].0.to_ascii_uppercase();
        let rdata: Vec<&(String, bool)> = tokens[ti + 1..]
            .iter()
            .filter(|(t, q)| *q || (t != "(" && t != ")"))
            .collect();
        if rdata.is_empty() {
            continue;
        }
        let value = if rtype == "TXT" {
            rdata.iter().map(|(t, _)| t.as_str()).collect::<String>()
        } else {
            rdata
                .iter()
                .map(|(t, _)| t.trim_end_matches('.').to_string())
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
        };
        out.push(ZoneRecord {
            name: qualify(&tokens[0].0, &origin),
            rtype,
            value,
        });
    }
    out
}

/// Render one record as a zone line (TXT split into ≤255-byte strings).
pub fn render(name: &str, rtype: &str, value: &str) -> String {
    let data = match rtype {
        "TXT" => value
            .as_bytes()
            .chunks(255)
            .map(|c| format!("\"{}\"", String::from_utf8_lossy(c).replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(" "),
        "MX" | "CNAME" | "SRV" | "PTR" | "NS" => format!("{value}."),
        _ => value.to_string(),
    };
    format!("{name}.\t3600\tIN\t{rtype}\t{data}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONE: &str = r#"
$ORIGIN example.com.
; generated
example.com.  3600 IN MX 10 mail.example.com.
@ IN TXT "v=spf1 mx ra=postmaster -all"
v1-rsa-20261105._domainkey IN TXT ( "v=DKIM1; k=rsa; " "p=MIIBIjANBg" )
_dmarc.example.com. IN TXT "v=DMARC1; p=reject; rua=mailto:postmaster@example.com"
_imaps._tcp.example.com. IN SRV 0 1 993 mail.example.com.
autoconfig.example.com. IN CNAME mail.example.com.
"#;

    #[test]
    fn parses_stalwart_style_zone() {
        let recs = parse(ZONE, "example.com");
        let find = |t: &str, n: &str| recs.iter().find(|r| r.rtype == t && r.name == n).cloned();
        assert_eq!(
            find("MX", "example.com").unwrap().value,
            "10 mail.example.com"
        );
        assert_eq!(
            find("TXT", "example.com").unwrap().value,
            "v=spf1 mx ra=postmaster -all"
        );
        let dkim = find("TXT", "v1-rsa-20261105._domainkey.example.com").unwrap();
        assert_eq!(dkim.value, "v=DKIM1; k=rsa; p=MIIBIjANBg");
        assert_eq!(
            find("SRV", "_imaps._tcp.example.com").unwrap().value,
            "0 1 993 mail.example.com"
        );
        assert_eq!(
            find("CNAME", "autoconfig.example.com").unwrap().value,
            "mail.example.com"
        );
    }

    /// Verbatim shape of a zone produced by a live Stalwart 1.0.3 engine
    /// (keys shortened). The RSA key spans lines inside parentheses.
    const LIVE_ZONE: &str = r#"v1-ed25519-20261105._domainkey.live.test. IN TXT "v=DKIM1; k=ed25519; h=sha256; p=af28rRLiBXlY="
v1-rsa-20261105._domainkey.live.test. IN TXT (
    "v=DKIM1; k=rsa; h=sha256; p=MIIBIjANBgkqhkiG9w0BAQEFAAOC"
    "ZFe5i8kcmds/Cv8h9q216wSJvXEk1vAs9Q"
)
mail.live.test. IN TXT "v=spf1 a -all"
live.test. IN TXT "v=spf1 mx -all"
live.test. IN MX 10 mail.live.test.
_dmarc.live.test. IN TXT "v=DMARC1; p=reject; rua=mailto:postmaster@live.test"
_imaps._tcp.live.test. IN SRV 0 1 993 mail.live.test.
_pop3s._tcp.live.test. IN SRV 0 1 995 mail.live.test.
mta-sts.live.test. IN CNAME mail.live.test.
_mta-sts.live.test. IN TXT "v=STSv1; id=2881120960881205280"
ua-auto-config.live.test. IN CNAME mail.live.test.
_ua-auto-config.live.test. IN TXT "v=UAAC1; a=sha256; d=Hs1uEHsEA6g4gnK45LQqYB5rwQNQtrGfU3EH97i9M1Q="
autoconfig.live.test. IN CNAME mail.live.test.
"#;

    #[test]
    fn parses_live_stalwart_zone_with_multiline_dkim() {
        let recs = parse(LIVE_ZONE, "live.test");
        let rsa = recs
            .iter()
            .find(|r| r.name == "v1-rsa-20261105._domainkey.live.test")
            .expect("multi-line RSA DKIM parsed");
        assert_eq!(
            rsa.value,
            "v=DKIM1; k=rsa; h=sha256; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCZFe5i8kcmds/Cv8h9q216wSJvXEk1vAs9Q"
        );
        assert!(
            recs.iter()
                .any(|r| r.name == "mail.live.test" && r.value == "v=spf1 a -all")
        );
        assert_eq!(
            recs.iter()
                .filter(|r| r.name.contains("_domainkey"))
                .count(),
            2
        );
        assert_eq!(recs.len(), 13);
    }

    #[test]
    fn renders_long_txt_split() {
        let long = "p=".to_string() + &"A".repeat(400);
        let line = render("k._domainkey.example.com", "TXT", &long);
        assert_eq!(line.matches('"').count(), 4);
    }
}
