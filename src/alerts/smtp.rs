//! A minimal SMTP submission client for alert email: implicit TLS to the
//! local engine (127.0.0.1:465), `AUTH PLAIN` as Yunori's alerts sender, one
//! message, `QUIT`. Nothing else in Yunori speaks SMTP, so this stays small
//! instead of pulling in a mail library.

use base64::Engine as _;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Debug, thiserror::Error)]
pub enum SmtpError {
    #[error("could not reach the mail engine's submission port ({0})")]
    Connect(String),
    #[error("the mail engine refused the alerts sender's password ({0})")]
    Auth(String),
    #[error("the mail engine refused the message ({0})")]
    Rejected(String),
    #[error("the mail engine's SMTP answer was unexpected ({0})")]
    Protocol(String),
}

/// Where to submit: the engine's implicit-TLS submission listener.
pub struct Target<'a> {
    pub host: &'a str,
    pub port: u16,
    /// TLS server name (the mail hostname) and the EHLO name.
    pub name: &'a str,
}

pub struct Envelope<'a> {
    pub user: &'a str,
    pub password: &'a str,
    pub from: &'a str,
    pub to: &'a str,
}

/// Submit one message over implicit TLS. Returns the engine's final reply
/// (it carries the queue id).
pub async fn send(t: &Target<'_>, env: &Envelope<'_>, message: &str) -> Result<String, SmtpError> {
    let mut attempt = 0;
    loop {
        match send_once(t, env, message).await {
            // Stalwart's inbound throttle (5 connections a second per IP by
            // default, shared with Yunori's certificate probes) closes the
            // connection before the greeting. Wait out the second and retry.
            Err(SmtpError::Protocol(m)) if m.starts_with("greeting:") && attempt < 3 => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(1200)).await;
            }
            other => return other,
        }
    }
}

async fn send_once(t: &Target<'_>, env: &Envelope<'_>, message: &str) -> Result<String, SmtpError> {
    let fut = async {
        let tcp = tokio::net::TcpStream::connect((t.host, t.port))
            .await
            .map_err(|e| SmtpError::Connect(e.to_string()))?;
        let connector = crate::edge::tls::loopback_connector()
            .map_err(|e| SmtpError::Connect(e.to_string()))?;
        let name = rustls::pki_types::ServerName::try_from(t.name.to_string())
            .map_err(|e| SmtpError::Connect(e.to_string()))?;
        let tls = connector
            .connect(name, tcp)
            .await
            .map_err(|e| SmtpError::Connect(format!("TLS: {e}")))?;
        dialogue(tls, t.name, env, message).await
    };
    tokio::time::timeout(Duration::from_secs(30), fut)
        .await
        .map_err(|_| SmtpError::Connect("timed out".into()))?
}

async fn reply<R: AsyncRead + Unpin>(r: &mut BufReader<R>) -> Result<(u16, String), SmtpError> {
    let mut text = String::new();
    loop {
        let mut line = String::new();
        let n = r
            .read_line(&mut line)
            .await
            .map_err(|e| SmtpError::Protocol(e.to_string()))?;
        if n == 0 {
            return Err(SmtpError::Protocol("connection closed".into()));
        }
        if line.len() < 4 {
            return Err(SmtpError::Protocol(line.trim().to_string()));
        }
        let code: u16 = line[..3]
            .parse()
            .map_err(|_| SmtpError::Protocol(line.trim().to_string()))?;
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line[4..].trim());
        if line.as_bytes()[3] != b'-' {
            return Ok((code, format!("{code} {text}")));
        }
    }
}

/// The SMTP conversation over an established (TLS) stream.
pub async fn dialogue<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    ehlo: &str,
    env: &Envelope<'_>,
    message: &str,
) -> Result<String, SmtpError> {
    let (r, mut w) = tokio::io::split(stream);
    let mut r = BufReader::new(r);
    async fn cmd<W: AsyncWrite + Unpin>(w: &mut W, line: &str) -> Result<(), SmtpError> {
        w.write_all(format!("{line}\r\n").as_bytes())
            .await
            .map_err(|e| SmtpError::Protocol(e.to_string()))?;
        w.flush()
            .await
            .map_err(|e| SmtpError::Protocol(e.to_string()))
    }
    let expect = |(code, text): (u16, String), ok: u16| {
        if code == ok {
            Ok(text)
        } else if code == 535 || code == 534 || code == 530 {
            Err(SmtpError::Auth(text))
        } else if (400..600).contains(&code) {
            Err(SmtpError::Rejected(text))
        } else {
            Err(SmtpError::Protocol(text))
        }
    };

    // Name the step in protocol errors, so a failure says where it happened.
    fn at<T>(stage: &str, r: Result<T, SmtpError>) -> Result<T, SmtpError> {
        r.map_err(|e| match e {
            SmtpError::Protocol(m) => SmtpError::Protocol(format!("{stage}: {m}")),
            e => e,
        })
    }

    at(
        "greeting",
        expect(at("greeting", reply(&mut r).await)?, 220),
    )?;
    at("EHLO", cmd(&mut w, &format!("EHLO {ehlo}")).await)?;
    at("EHLO", expect(at("EHLO", reply(&mut r).await)?, 250))?;
    let plain = base64::engine::general_purpose::STANDARD
        .encode(format!("\0{}\0{}", env.user, env.password));
    at("AUTH", cmd(&mut w, &format!("AUTH PLAIN {plain}")).await)?;
    // Stalwart answers a wrong password with `535 5.7.8` (verified live).
    match at("AUTH", reply(&mut r).await)? {
        (235, _) => {}
        (_, text) => return Err(SmtpError::Auth(text)),
    }
    at(
        "MAIL FROM",
        cmd(&mut w, &format!("MAIL FROM:<{}>", env.from)).await,
    )?;
    at(
        "MAIL FROM",
        expect(at("MAIL FROM", reply(&mut r).await)?, 250),
    )?;
    at(
        "RCPT TO",
        cmd(&mut w, &format!("RCPT TO:<{}>", env.to)).await,
    )?;
    at("RCPT TO", expect(at("RCPT TO", reply(&mut r).await)?, 250))?;
    at("DATA", cmd(&mut w, "DATA").await)?;
    at("DATA", expect(at("DATA", reply(&mut r).await)?, 354))?;
    let mut data = String::with_capacity(message.len() + 16);
    for line in message.split("\r\n") {
        // Dot-stuffing (RFC 5321 §4.5.2).
        if line.starts_with('.') {
            data.push('.');
        }
        data.push_str(line);
        data.push_str("\r\n");
    }
    data.push_str(".\r\n");
    w.write_all(data.as_bytes())
        .await
        .map_err(|e| SmtpError::Protocol(format!("message: {e}")))?;
    let done = at("message", expect(at("message", reply(&mut r).await)?, 250))?;
    let _ = cmd(&mut w, "QUIT").await;
    Ok(done)
}

fn header_value(v: &str) -> String {
    let clean: String = v.chars().filter(|c| *c != '\r' && *c != '\n').collect();
    if clean.is_ascii() {
        clean
    } else {
        format!(
            "=?UTF-8?B?{}?=",
            base64::engine::general_purpose::STANDARD.encode(clean.as_bytes())
        )
    }
}

/// A plain-text message (RFC 5322) with a base64 body, CRLF line endings.
pub fn compose(
    from_name: &str,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
    hostname: &str,
    extra_headers: &[(&str, &str)],
) -> String {
    let mut m = String::new();
    let mut h = |k: &str, v: &str| {
        m.push_str(k);
        m.push_str(": ");
        m.push_str(v);
        m.push_str("\r\n");
    };
    h("Date", &chrono::Utc::now().to_rfc2822());
    h("From", &format!("{} <{from}>", header_value(from_name)));
    h("To", &format!("<{}>", header_value(to)));
    h("Subject", &header_value(subject));
    h(
        "Message-ID",
        &format!(
            "<{}@{}>",
            crate::util::random_token(18).replace(['-', '_'], "x"),
            header_value(hostname)
        ),
    );
    h("MIME-Version", "1.0");
    h("Content-Type", "text/plain; charset=utf-8");
    h("Content-Transfer-Encoding", "base64");
    // RFC 3834: machine-generated, so auto-responders stay quiet.
    h("Auto-Submitted", "auto-generated");
    for (k, v) in extra_headers {
        h(k, &header_value(v));
    }
    m.push_str("\r\n");
    let normalised = body.replace("\r\n", "\n").replace('\n', "\r\n");
    let encoded = base64::engine::general_purpose::STANDARD.encode(normalised.as_bytes());
    for chunk in encoded.as_bytes().chunks(76) {
        m.push_str(std::str::from_utf8(chunk).expect("base64 is ascii"));
        m.push_str("\r\n");
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// A scripted SMTP server on one end of an in-memory pipe.
    async fn fake_server(mut s: tokio::io::DuplexStream, auth_ok: bool) -> (Vec<String>, String) {
        let mut seen = Vec::new();
        let mut buf = Vec::new();
        s.write_all(b"220 mail.test ESMTP\r\n").await.unwrap();
        let mut data_mode = false;
        let mut data = String::new();
        loop {
            let mut chunk = [0u8; 4096];
            let n = s.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            while let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
                let line = String::from_utf8(buf[..pos].to_vec()).unwrap();
                buf.drain(..pos + 2);
                if data_mode {
                    if line == "." {
                        data_mode = false;
                        s.write_all(b"250 2.0.0 Message queued as 42\r\n")
                            .await
                            .unwrap();
                    } else {
                        data.push_str(&line);
                        data.push('\n');
                    }
                    continue;
                }
                seen.push(line.clone());
                let answer: &[u8] = if line.starts_with("EHLO") {
                    b"250-mail.test\r\n250-AUTH PLAIN\r\n250 8BITMIME\r\n"
                } else if line.starts_with("AUTH PLAIN") {
                    if auth_ok {
                        b"235 2.7.0 ok\r\n"
                    } else {
                        b"535 5.7.8 Authentication credentials invalid\r\n"
                    }
                } else if line == "DATA" {
                    data_mode = true;
                    b"354 go ahead\r\n"
                } else if line == "QUIT" {
                    // The client may already have hung up.
                    let _ = s.write_all(b"221 bye\r\n").await;
                    return (seen, data);
                } else {
                    b"250 ok\r\n"
                };
                if s.write_all(answer).await.is_err() {
                    return (seen, data);
                }
            }
        }
        (seen, data)
    }

    #[tokio::test]
    async fn submits_one_message_with_dot_stuffing() {
        let (client, server) = tokio::io::duplex(1 << 16);
        let srv = tokio::spawn(fake_server(server, true));
        let env = Envelope {
            user: "yunori-alerts@example.test",
            password: "pw",
            from: "yunori-alerts@example.test",
            to: "owner@elsewhere.example",
        };
        let msg = "Subject: x\r\n\r\n.leading dot\r\nok";
        let r = dialogue(client, "mail.example.test", &env, msg)
            .await
            .unwrap();
        assert!(r.contains("queued as 42"), "{r}");
        let (seen, data) = srv.await.unwrap();
        assert_eq!(seen[0], "EHLO mail.example.test");
        let plain =
            base64::engine::general_purpose::STANDARD.encode("\0yunori-alerts@example.test\0pw");
        assert_eq!(seen[1], format!("AUTH PLAIN {plain}"));
        assert_eq!(seen[2], "MAIL FROM:<yunori-alerts@example.test>");
        assert_eq!(seen[3], "RCPT TO:<owner@elsewhere.example>");
        assert!(data.contains("\n..leading dot\n"), "{data}");
    }

    #[tokio::test]
    async fn a_refused_password_is_an_auth_error() {
        let (client, server) = tokio::io::duplex(1 << 16);
        tokio::spawn(fake_server(server, false));
        let env = Envelope {
            user: "a@b.test",
            password: "nope",
            from: "a@b.test",
            to: "c@d.test",
        };
        let e = dialogue(client, "h", &env, "x").await.unwrap_err();
        assert!(matches!(e, SmtpError::Auth(_)), "{e}");
    }

    #[test]
    fn compose_encodes_headers_and_body() {
        let m = compose(
            "Yunori alerts",
            "yunori-alerts@example.test",
            "owner@elsewhere.example",
            "Disk 95% full — café\r\nBcc: evil@x",
            "Line one\nLine two",
            "mail.example.test",
            &[("X-Yunori-Alert", "disk")],
        );
        assert!(m.contains("\r\nSubject: =?UTF-8?B?"), "{m}");
        assert!(!m.contains("\r\nBcc:"), "no header injection: {m}");
        assert!(m.contains("\r\nAuto-Submitted: auto-generated\r\n"));
        assert!(m.contains("\r\nX-Yunori-Alert: disk\r\n"));
        let body = m.split("\r\n\r\n").nth(1).unwrap().replace("\r\n", "");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "Line one\r\nLine two");
    }
}
