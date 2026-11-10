//! A small IMAP4rev1 client: just what moving a mailbox needs (LOGIN, LIST,
//! STATUS, EXAMINE, UID SEARCH/FETCH, plus CREATE/APPEND for the live tests).
//!
//! Always TLS, always verified: implicit TLS on 993, or STARTTLS on 143 (the
//! upgrade happens before anything else is sent, and LOGIN never goes in the
//! clear). Every read has a timeout, lines and literals are capped, and the
//! password is never logged or kept beyond the connection.

use anyhow::{Context, Result, anyhow, bail};
use rustls::pki_types::{CertificateDer, ServerName};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// Longest response line we accept outside literals (a `UID SEARCH` answer
/// for a very large folder is the longest legitimate one).
const MAX_LINE: usize = 8 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(90);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How to reach the old server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// Port 993: TLS from the first byte.
    Implicit,
    /// Port 143: plain greeting, then STARTTLS before anything else.
    StartTls,
}

/// One parsed IMAP value.
#[derive(Debug, Clone, PartialEq)]
pub enum V {
    Atom(String),
    Str(Vec<u8>),
    List(Vec<V>),
    Nil,
}

impl V {
    pub fn atom(&self) -> Option<&str> {
        match self {
            V::Atom(a) => Some(a),
            _ => None,
        }
    }
    /// Atom or string, as text (lossy).
    pub fn text(&self) -> Option<String> {
        match self {
            V::Atom(a) => Some(a.clone()),
            V::Str(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }
    pub fn bytes(self) -> Option<Vec<u8>> {
        match self {
            V::Str(b) => Some(b),
            V::Atom(a) => Some(a.into_bytes()),
            _ => None,
        }
    }
    pub fn list(&self) -> Option<&[V]> {
        match self {
            V::List(l) => Some(l),
            _ => None,
        }
    }
    pub fn num(&self) -> Option<u64> {
        self.atom().and_then(|a| a.parse().ok())
    }
}

/// A server response.
#[derive(Debug)]
pub enum Resp {
    /// `* …` data. Status-type data (`OK`/`NO`/`BYE`/…) keeps its text raw in
    /// the second element.
    Untagged(Vec<V>),
    Tagged {
        tag: String,
        ok: bool,
        text: String,
    },
    Continue,
}

/// Read one logical response: a line plus any literals it announces.
/// Returns the text (with each literal replaced by `{n}`) and the literals.
async fn read_logical<R: AsyncRead + Unpin>(
    r: &mut BufReader<R>,
    max_literal: usize,
) -> Result<(Vec<u8>, Vec<Vec<u8>>)> {
    let mut text = Vec::new();
    let mut lits = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = tokio::time::timeout(READ_TIMEOUT, read_line_capped(r, &mut line))
            .await
            .map_err(|_| anyhow!("the server stopped answering"))??;
        if n == 0 {
            bail!("the server closed the connection");
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        // A literal announcement ends the line: `{123}` (or `{123+}`).
        let lit = literal_len(&line);
        text.extend_from_slice(&line);
        if text.len() > MAX_LINE {
            bail!("the server sent an over-long response");
        }
        match lit {
            Some(len) => {
                if len > max_literal {
                    bail!("the server sent a {len}-byte item, over the {max_literal}-byte limit");
                }
                let mut buf = vec![0u8; len];
                tokio::time::timeout(
                    READ_TIMEOUT + Duration::from_secs((len / (64 * 1024)) as u64),
                    r.read_exact(&mut buf),
                )
                .await
                .map_err(|_| anyhow!("the server stopped sending a message"))??;
                lits.push(buf);
            }
            None => return Ok((text, lits)),
        }
    }
}

async fn read_line_capped<R: AsyncRead + Unpin>(
    r: &mut BufReader<R>,
    out: &mut Vec<u8>,
) -> Result<usize> {
    let mut total = 0;
    loop {
        let buf = r.fill_buf().await?;
        if buf.is_empty() {
            return Ok(total);
        }
        let (take, done) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (buf.len(), false),
        };
        out.extend_from_slice(&buf[..take]);
        r.consume(take);
        total += take;
        if done {
            return Ok(total);
        }
        if out.len() > MAX_LINE {
            bail!("the server sent an over-long line");
        }
    }
}

fn literal_len(line: &[u8]) -> Option<usize> {
    let rest = line.strip_suffix(b"}")?;
    let open = rest.iter().rposition(|&b| b == b'{')?;
    let digits = &rest[open + 1..];
    let digits = digits.strip_suffix(b"+").unwrap_or(digits);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

/// Tokenise response text (after `* `), pulling literals in order.
pub fn parse_values(text: &[u8], lits: Vec<Vec<u8>>) -> Result<Vec<V>> {
    let mut lits = lits.into_iter();
    let mut pos = 0;
    let mut stack: Vec<Vec<V>> = vec![Vec::new()];
    while pos < text.len() {
        match text[pos] {
            b' ' => pos += 1,
            b'(' => {
                stack.push(Vec::new());
                pos += 1;
            }
            b')' => {
                if stack.len() < 2 {
                    bail!("unbalanced ')' in a server response");
                }
                let l = stack.pop().unwrap();
                stack.last_mut().unwrap().push(V::List(l));
                pos += 1;
            }
            b'"' => {
                let mut s = Vec::new();
                pos += 1;
                loop {
                    match text.get(pos) {
                        None => bail!("unterminated string in a server response"),
                        Some(b'\\') => {
                            if let Some(&c) = text.get(pos + 1) {
                                s.push(c);
                            }
                            pos += 2;
                        }
                        Some(b'"') => {
                            pos += 1;
                            break;
                        }
                        Some(&c) => {
                            s.push(c);
                            pos += 1;
                        }
                    }
                }
                stack.last_mut().unwrap().push(V::Str(s));
            }
            b'{' if literal_len_at(text, pos).is_some() => {
                let end = text[pos..].iter().position(|&b| b == b'}').unwrap() + pos;
                let lit = lits
                    .next()
                    .ok_or_else(|| anyhow!("missing literal in a server response"))?;
                stack.last_mut().unwrap().push(V::Str(lit));
                pos = end + 1;
            }
            _ => {
                // Atom; a `[` inside it runs to the matching `]`, spaces and
                // parentheses included (`BODY[HEADER.FIELDS (MESSAGE-ID)]`).
                let start = pos;
                let mut depth = 0usize;
                while pos < text.len() {
                    let c = text[pos];
                    if c == b'[' {
                        depth += 1;
                    } else if c == b']' {
                        depth = depth.saturating_sub(1);
                    } else if depth == 0 && matches!(c, b' ' | b'(' | b')') {
                        break;
                    }
                    pos += 1;
                }
                let a = String::from_utf8_lossy(&text[start..pos]).into_owned();
                stack
                    .last_mut()
                    .unwrap()
                    .push(if a.eq_ignore_ascii_case("NIL") {
                        V::Nil
                    } else {
                        V::Atom(a)
                    });
            }
        }
    }
    if stack.len() != 1 {
        bail!("unbalanced '(' in a server response");
    }
    Ok(stack.pop().unwrap())
}

fn literal_len_at(text: &[u8], pos: usize) -> Option<usize> {
    let end = text[pos..].iter().position(|&b| b == b'}')? + pos;
    literal_len(&text[pos..=end])
}

fn parse_resp(text: Vec<u8>, lits: Vec<Vec<u8>>) -> Result<Resp> {
    if text.starts_with(b"+") {
        return Ok(Resp::Continue);
    }
    if let Some(rest) = text.strip_prefix(b"* ") {
        let word_end = rest.iter().position(|&b| b == b' ').unwrap_or(rest.len());
        let word = String::from_utf8_lossy(&rest[..word_end]).to_ascii_uppercase();
        if matches!(
            word.as_str(),
            "OK" | "NO" | "BAD" | "BYE" | "PREAUTH" | "CAPABILITY"
        ) {
            let tail =
                String::from_utf8_lossy(rest.get(word_end + 1..).unwrap_or_default()).into_owned();
            return Ok(Resp::Untagged(vec![
                V::Atom(word),
                V::Str(tail.into_bytes()),
            ]));
        }
        return Ok(Resp::Untagged(parse_values(rest, lits)?));
    }
    let s = String::from_utf8_lossy(&text);
    let mut it = s.splitn(3, ' ');
    let tag = it.next().unwrap_or_default().to_string();
    let status = it.next().unwrap_or_default().to_ascii_uppercase();
    let text = it.next().unwrap_or_default().to_string();
    Ok(Resp::Tagged {
        tag,
        ok: status == "OK",
        text,
    })
}

/// An argument for a command: raw protocol text, or a string sent safely
/// (quoted when it can be, as a literal otherwise).
pub enum Arg<'a> {
    Raw(&'a str),
    Str(&'a [u8]),
}

fn quotable(s: &[u8]) -> bool {
    s.len() < 1000 && s.iter().all(|&c| (0x20..0x7f).contains(&c))
}

fn quote(s: &[u8]) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for &c in s {
        if c == b'"' || c == b'\\' {
            out.push('\\');
        }
        out.push(c as char);
    }
    out.push('"');
    out
}

/// The server rejected a command (`NO` / `BAD`).
#[derive(Debug)]
pub struct Refused(pub String);
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Refused {}

/// Where and how to connect. `addr` is already resolved and checked.
pub struct Target<'a> {
    pub host: &'a str,
    pub addr: SocketAddr,
    pub security: Security,
    /// Extra trust anchors (tests only: a self-signed engine certificate).
    pub extra_roots: &'a [CertificateDer<'static>],
}

pub struct Client {
    io: BufReader<TlsStream<TcpStream>>,
    tag: u32,
    pub capabilities: Vec<String>,
    max_literal: usize,
}

fn tls_config(extra: &[CertificateDer<'static>]) -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for c in extra {
        roots.add(c.clone())?;
    }
    Ok(Arc::new(
        rustls::ClientConfig::builder_with_provider(crate::edge::tls::provider())
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

/// Read a CRLF line from a plain stream one byte at a time, so nothing past
/// it is consumed before the TLS upgrade.
async fn plain_line(tcp: &mut TcpStream) -> Result<String> {
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = tokio::time::timeout(READ_TIMEOUT, tcp.read(&mut b))
            .await
            .map_err(|_| anyhow!("the server stopped answering"))??;
        if n == 0 {
            bail!("the server closed the connection");
        }
        line.push(b[0]);
        if b[0] == b'\n' {
            return Ok(String::from_utf8_lossy(&line).trim_end().to_string());
        }
        if line.len() > 8192 {
            bail!("the server sent an over-long greeting");
        }
    }
}

impl Client {
    pub async fn connect(t: &Target<'_>, max_literal: usize) -> Result<Self> {
        let mut tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(t.addr))
            .await
            .map_err(|_| anyhow!("could not reach {} (timed out)", t.host))?
            .with_context(|| format!("could not reach {}", t.host))?;
        let mut greeted = false;
        if t.security == Security::StartTls {
            let greeting = plain_line(&mut tcp).await?;
            if !greeting.starts_with("* OK") {
                bail!("the server did not greet as an IMAP server");
            }
            tcp.write_all(b"S0 STARTTLS\r\n").await?;
            loop {
                let l = plain_line(&mut tcp).await?;
                if l.starts_with("S0 ") {
                    if !l.to_ascii_uppercase().starts_with("S0 OK") {
                        bail!("the server does not offer STARTTLS on port 143; use port 993");
                    }
                    break;
                }
            }
            greeted = true;
        }
        let name = ServerName::try_from(t.host.to_string())
            .map_err(|_| anyhow!("{} is not a valid server name", t.host))?;
        let tls = tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio_rustls::TlsConnector::from(tls_config(t.extra_roots)?).connect(name, tcp),
        )
        .await
        .map_err(|_| anyhow!("secure connection to {} timed out", t.host))?
        .map_err(|e| anyhow!("secure connection to {} failed: {e}", t.host))?;
        let mut c = Client {
            io: BufReader::new(tls),
            tag: 0,
            capabilities: Vec::new(),
            max_literal,
        };
        if !greeted {
            match c.read().await? {
                Resp::Untagged(v) if v.first().and_then(V::atom) == Some("OK") => {}
                Resp::Untagged(v) if v.first().and_then(V::atom) == Some("PREAUTH") => {}
                _ => bail!("the server did not greet as an IMAP server"),
            }
        }
        c.refresh_capabilities().await?;
        Ok(c)
    }

    async fn read(&mut self) -> Result<Resp> {
        let (text, lits) = read_logical(&mut self.io, self.max_literal).await?;
        parse_resp(text, lits)
    }

    async fn refresh_capabilities(&mut self) -> Result<()> {
        let data = self.run(&[Arg::Raw("CAPABILITY")]).await?;
        self.capabilities = data
            .iter()
            .filter(|v| v.first().and_then(V::atom) == Some("CAPABILITY"))
            .filter_map(|v| v.get(1).and_then(V::text))
            .flat_map(|s| {
                s.split_whitespace()
                    .map(|w| w.to_ascii_uppercase())
                    .collect::<Vec<_>>()
            })
            .collect();
        Ok(())
    }

    pub fn has(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }

    /// Send one command, collect untagged data until its tagged answer.
    pub async fn run(&mut self, args: &[Arg<'_>]) -> Result<Vec<Vec<V>>> {
        self.tag += 1;
        let tag = format!("Y{}", self.tag);
        let mut buf = format!("{tag} ").into_bytes();
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                buf.push(b' ');
            }
            match a {
                Arg::Raw(s) => buf.extend_from_slice(s.as_bytes()),
                Arg::Str(s) if quotable(s) => buf.extend_from_slice(quote(s).as_bytes()),
                Arg::Str(s) => {
                    buf.extend_from_slice(format!("{{{}}}\r\n", s.len()).as_bytes());
                    self.io.get_mut().write_all(&buf).await?;
                    buf.clear();
                    self.wait_continue(&tag).await?;
                    buf.extend_from_slice(s);
                }
            }
        }
        buf.extend_from_slice(b"\r\n");
        self.io.get_mut().write_all(&buf).await?;
        self.io.get_mut().flush().await?;
        let mut data = Vec::new();
        loop {
            match self.read().await? {
                Resp::Untagged(v) => {
                    if v.first().and_then(V::atom) == Some("BYE") {
                        // Data still arrives; the tagged answer (or EOF) ends it.
                    }
                    data.push(v);
                }
                Resp::Continue => bail!("unexpected continuation from the server"),
                Resp::Tagged { tag: t, ok, text } if t == tag => {
                    if ok {
                        return Ok(data);
                    }
                    return Err(Refused(text).into());
                }
                Resp::Tagged { .. } => {}
            }
        }
    }

    async fn wait_continue(&mut self, tag: &str) -> Result<()> {
        loop {
            match self.read().await? {
                Resp::Continue => return Ok(()),
                Resp::Untagged(_) => {}
                Resp::Tagged { tag: t, text, .. } if t == tag => {
                    return Err(Refused(text).into());
                }
                Resp::Tagged { .. } => {}
            }
        }
    }

    pub async fn login(&mut self, user: &str, pass: &str) -> Result<()> {
        if self.has("LOGINDISABLED") {
            bail!("the server refuses password sign-in on this connection");
        }
        self.run(&[
            Arg::Raw("LOGIN"),
            Arg::Str(user.as_bytes()),
            Arg::Str(pass.as_bytes()),
        ])
        .await?;
        // Servers often announce more capabilities once signed in.
        self.refresh_capabilities().await
    }

    /// `LIST "" "*"` → (raw name, delimiter, attributes in lower case).
    pub async fn list(&mut self) -> Result<Vec<(Vec<u8>, Option<char>, Vec<String>)>> {
        let data = self
            .run(&[Arg::Raw("LIST"), Arg::Raw("\"\""), Arg::Raw("\"*\"")])
            .await?;
        let mut out = Vec::new();
        for v in data {
            if v.first().and_then(V::atom).map(str::to_ascii_uppercase) != Some("LIST".into()) {
                continue;
            }
            let mut it = v.into_iter().skip(1);
            let attrs: Vec<String> = it
                .next()
                .and_then(|a| {
                    a.list().map(|l| {
                        l.iter()
                            .filter_map(V::text)
                            .map(|s| s.to_ascii_lowercase())
                            .collect()
                    })
                })
                .unwrap_or_default();
            let delim = it
                .next()
                .and_then(|d| d.text())
                .and_then(|d| d.chars().next());
            let Some(name) = it.next().and_then(V::bytes) else {
                continue;
            };
            out.push((name, delim, attrs));
        }
        Ok(out)
    }

    /// `STATUS` → (messages, uidvalidity).
    pub async fn status(&mut self, mailbox: &[u8]) -> Result<(u64, Option<u64>)> {
        let data = self
            .run(&[
                Arg::Raw("STATUS"),
                Arg::Str(mailbox),
                Arg::Raw("(MESSAGES UIDVALIDITY)"),
            ])
            .await?;
        for v in data {
            if v.first().and_then(V::atom).map(str::to_ascii_uppercase) == Some("STATUS".into())
                && let Some(items) = v.get(2).and_then(V::list)
            {
                let get = |k: &str| {
                    items
                        .chunks(2)
                        .find(|p| p[0].atom().is_some_and(|a| a.eq_ignore_ascii_case(k)))
                        .and_then(|p| p.get(1).and_then(V::num))
                };
                return Ok((get("MESSAGES").unwrap_or(0), get("UIDVALIDITY")));
            }
        }
        Ok((0, None))
    }

    /// `EXAMINE` (read-only, so nothing is marked read) → (exists, uidvalidity).
    pub async fn examine(&mut self, mailbox: &[u8]) -> Result<(u64, u64)> {
        let data = self.run(&[Arg::Raw("EXAMINE"), Arg::Str(mailbox)]).await?;
        let mut exists = 0;
        let mut validity = 0;
        for v in data {
            if v.get(1)
                .and_then(V::atom)
                .is_some_and(|a| a.eq_ignore_ascii_case("EXISTS"))
            {
                exists = v.first().and_then(V::num).unwrap_or(0);
            }
            if v.first().and_then(V::atom) == Some("OK")
                && let Some(t) = v.get(1).and_then(V::text)
                && let Some(rest) = t.trim_start().strip_prefix("[UIDVALIDITY ")
            {
                validity = rest
                    .split(']')
                    .next()
                    .and_then(|n| n.trim().parse().ok())
                    .unwrap_or(0);
            }
        }
        Ok((exists, validity))
    }

    /// Every UID in the selected mailbox.
    pub async fn uids(&mut self) -> Result<Vec<u32>> {
        let data = self
            .run(&[Arg::Raw("UID"), Arg::Raw("SEARCH"), Arg::Raw("ALL")])
            .await?;
        let mut out: Vec<u32> = data
            .into_iter()
            .filter(|v| {
                v.first()
                    .and_then(V::atom)
                    .is_some_and(|a| a.eq_ignore_ascii_case("SEARCH"))
            })
            .flat_map(|v| v.into_iter().skip(1).filter_map(|x| x.num()))
            .filter_map(|n| u32::try_from(n).ok())
            .collect();
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// `UID FETCH <set> (<items>)` → each message's item map.
    pub async fn uid_fetch(&mut self, set: &str, items: &str) -> Result<Vec<Vec<(String, V)>>> {
        let data = self
            .run(&[
                Arg::Raw("UID"),
                Arg::Raw("FETCH"),
                Arg::Raw(set),
                Arg::Raw(items),
            ])
            .await?;
        let mut out = Vec::new();
        for v in data {
            if v.get(1)
                .and_then(V::atom)
                .is_some_and(|a| a.eq_ignore_ascii_case("FETCH"))
                && let Some(V::List(l)) = v.into_iter().nth(2)
            {
                let mut pairs = Vec::new();
                let mut it = l.into_iter();
                while let (Some(k), Some(val)) = (it.next(), it.next()) {
                    if let Some(k) = k.text() {
                        pairs.push((k.to_ascii_uppercase(), val));
                    }
                }
                out.push(pairs);
            }
        }
        Ok(out)
    }

    #[cfg(test)]
    pub async fn create(&mut self, mailbox: &[u8]) -> Result<()> {
        self.run(&[Arg::Raw("CREATE"), Arg::Str(mailbox)])
            .await
            .map(|_| ())
    }

    #[cfg(test)]
    /// `APPEND` with flags and an internal date (live tests seed with it).
    pub async fn append(
        &mut self,
        mailbox: &[u8],
        flags: &str,
        internal_date: &str,
        message: &[u8],
    ) -> Result<()> {
        let flags = format!("({flags})");
        let date = quote(internal_date.as_bytes());
        // A message always goes as a literal, whatever it contains.
        self.tag += 1;
        let tag = format!("Y{}", self.tag);
        let mut head = format!("{tag} APPEND ").into_bytes();
        head.extend_from_slice(
            (if quotable(mailbox) {
                quote(mailbox)
            } else {
                bail!("test folder names must be ASCII")
            })
            .as_bytes(),
        );
        head.extend_from_slice(format!(" {flags} {date} {{{}}}\r\n", message.len()).as_bytes());
        self.io.get_mut().write_all(&head).await?;
        self.wait_continue(&tag).await?;
        self.io.get_mut().write_all(message).await?;
        self.io.get_mut().write_all(b"\r\n").await?;
        loop {
            match self.read().await? {
                Resp::Tagged { tag: t, ok, text } if t == tag => {
                    return if ok {
                        Ok(())
                    } else {
                        Err(Refused(text).into())
                    };
                }
                _ => {}
            }
        }
    }

    pub async fn logout(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.run(&[Arg::Raw("LOGOUT")])).await;
    }
}

// ── modified UTF-7 (RFC 3501 §5.1.3) ─────────────────────────────────────

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

/// Decode a mailbox name; invalid sequences come through as written.
pub fn decode_mutf7(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut rest = s.as_ref();
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let chunk = &after[..end];
        if chunk.is_empty() {
            out.push('&');
        } else {
            match decode_b64_utf16(chunk) {
                Some(t) => out.push_str(&t),
                None => {
                    out.push('&');
                    out.push_str(chunk);
                    out.push('-');
                }
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn decode_b64_utf16(chunk: &str) -> Option<String> {
    let mut bits: u32 = 0;
    let mut nbits = 0;
    let mut bytes = Vec::new();
    for c in chunk.bytes() {
        let v = B64.iter().position(|&b| b == c)? as u32;
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            bytes.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
    }
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|p| u16::from_be_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// Encode a mailbox name for the wire.
#[cfg(test)]
pub fn encode_mutf7(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut Vec<u8>| {
        if pending.is_empty() {
            return;
        }
        out.push(b'&');
        let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
        let mut bits: u32 = 0;
        let mut nbits = 0;
        for b in bytes {
            bits = (bits << 8) | b as u32;
            nbits += 8;
            while nbits >= 6 {
                nbits -= 6;
                out.push(B64[((bits >> nbits) & 63) as usize]);
            }
        }
        if nbits > 0 {
            out.push(B64[((bits << (6 - nbits)) & 63) as usize]);
        }
        out.push(b'-');
        pending.clear();
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.extend_from_slice(b"&-");
            } else {
                out.push(c as u8);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// `17-Jul-1996 02:44:25 -0700` → UTC RFC 3339.
pub fn internal_date_utc(s: &str) -> Option<String> {
    chrono::DateTime::parse_from_str(s.trim(), "%d-%b-%Y %H:%M:%S %z")
        .ok()
        .map(|d| {
            d.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_fetch_and_status() {
        let v = parse_values(
            br#"LIST (\HasNoChildren \Sent) "/" "[Gmail]/Sent Mail""#,
            vec![],
        )
        .unwrap();
        assert_eq!(v[0], V::Atom("LIST".into()));
        assert_eq!(
            v[1],
            V::List(vec![
                V::Atom("\\HasNoChildren".into()),
                V::Atom("\\Sent".into())
            ])
        );
        assert_eq!(v[3], V::Str(b"[Gmail]/Sent Mail".to_vec()));

        let v = parse_values(
            br#"12 FETCH (UID 40 FLAGS (\Seen $Forwarded) INTERNALDATE "17-Jul-1996 02:44:25 -0700" RFC822.SIZE 4286 BODY[HEADER.FIELDS (MESSAGE-ID)] {30})"#,
            vec![b"Message-ID: <a@b>\r\n\r\n".to_vec()],
        )
        .unwrap();
        let items = v[2].list().unwrap();
        assert_eq!(items[0], V::Atom("UID".into()));
        assert_eq!(items[1].num(), Some(40));
        assert_eq!(items[8], V::Atom("BODY[HEADER.FIELDS (MESSAGE-ID)]".into()));
        assert_eq!(items[9], V::Str(b"Message-ID: <a@b>\r\n\r\n".to_vec()));

        let v = parse_values(br#"STATUS "a \"q\" b" (MESSAGES 3 UIDVALIDITY 99)"#, vec![]).unwrap();
        assert_eq!(v[1], V::Str(b"a \"q\" b".to_vec()));
        assert_eq!(v[2].list().unwrap()[3].num(), Some(99));
        assert!(parse_values(b"LIST (a", vec![]).is_err());
        assert!(parse_values(b"LIST a)", vec![]).is_err());
        assert_eq!(parse_values(b"X NIL", vec![]).unwrap()[1], V::Nil);
    }

    #[test]
    fn literal_announcements() {
        assert_eq!(literal_len(b"* 1 FETCH (BODY[] {123}"), Some(123));
        assert_eq!(literal_len(b"A {5+}"), Some(5));
        assert_eq!(literal_len(b"* OK {abc}"), None);
        assert_eq!(literal_len(b"* OK done"), None);
    }

    #[tokio::test]
    async fn reads_lines_with_literals_and_caps_them() {
        let data = b"* 1 FETCH (UID 7 BODY[] {5}\r\nhello)\r\nY1 OK done\r\n".to_vec();
        let mut r = BufReader::new(&data[..]);
        let (text, lits) = read_logical(&mut r, 100).await.unwrap();
        assert_eq!(lits, vec![b"hello".to_vec()]);
        match parse_resp(text, lits).unwrap() {
            Resp::Untagged(v) => {
                let l = v[2].list().unwrap();
                assert_eq!(l[3], V::Str(b"hello".to_vec()));
            }
            other => panic!("{other:?}"),
        }
        let (text, lits) = read_logical(&mut r, 100).await.unwrap();
        assert!(matches!(
            parse_resp(text, lits).unwrap(),
            Resp::Tagged { ok: true, .. }
        ));
        let big = b"* 1 FETCH (BODY[] {500}\r\n".to_vec();
        let mut r = BufReader::new(&big[..]);
        assert!(read_logical(&mut r, 100).await.is_err());
    }

    #[test]
    fn status_responses_keep_their_text() {
        match parse_resp(
            b"* OK [UIDVALIDITY 3857529045] UIDs valid (".to_vec(),
            vec![],
        )
        .unwrap()
        {
            Resp::Untagged(v) => {
                assert_eq!(v[0], V::Atom("OK".into()));
                assert!(v[1].text().unwrap().starts_with("[UIDVALIDITY 3857529045]"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn modified_utf7_round_trip() {
        assert_eq!(decode_mutf7(b"&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_mutf7(b"R&AOk-sum&AOk-s"), "Résumés");
        assert_eq!(decode_mutf7(b"Tom &- Jerry"), "Tom & Jerry");
        for s in [
            "日本語",
            "Résumés",
            "Tom & Jerry",
            "Projekte/Übersicht",
            "plain",
        ] {
            assert_eq!(decode_mutf7(&encode_mutf7(s)), s);
        }
        assert_eq!(encode_mutf7("Tom & Jerry"), b"Tom &- Jerry");
        assert_eq!(decode_mutf7(b"broken &***-"), "broken &***-");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote(br#"pa"ss\"#), r#""pa\"ss\\""#);
        assert!(quotable(b"hello world"));
        assert!(!quotable("pässword".as_bytes()));
        assert!(!quotable(b"line\r\nbreak"));
    }

    #[test]
    fn internal_dates() {
        assert_eq!(
            internal_date_utc("17-Jul-1996 02:44:25 -0700").as_deref(),
            Some("1996-07-17T09:44:25Z")
        );
        assert_eq!(
            internal_date_utc(" 7-Jul-2020 00:00:00 +0200").as_deref(),
            Some("2020-07-06T22:00:00Z")
        );
        assert_eq!(internal_date_utc("garbage"), None);
    }
}
