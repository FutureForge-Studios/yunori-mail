//! Updates. Once a day the server asks yunori.email which release is the newest
//! (`/server/latest.json`, which the Yunori website reads from this repository's
//! GitHub releases) and remembers the answer for the console. Nothing is
//! downloaded or changed by the check: `yunori-mail update` does that, when a
//! person runs it, through the same one-line installer people install with
//! (`/server/install.sh`), which verifies every binary's SHA-256.

use crate::app::{App, VERSION};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Where the installer and the update feed live.
pub const DEFAULT_FEED: &str = "https://yunori.email/server";
const CACHE_KEY: &str = "update.latest";

/// The feed's base URL: `YUNORI_UPDATE_FEED` when it is https (a mirror) or a
/// loopback address (tests), else yunori.email.
pub fn feed_base() -> String {
    std::env::var("YUNORI_UPDATE_FEED")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| allowed_feed(v))
        .unwrap_or_else(|| DEFAULT_FEED.to_string())
}

fn allowed_feed(url: &str) -> bool {
    url.starts_with("https://")
        || url.starts_with("http://127.0.0.1:")
        || url.starts_with("http://localhost:")
}

/// What the console shows about updates.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The version running now.
    pub current: String,
    /// The newest published release, once a check has got an answer.
    pub latest: Option<String>,
    /// `latest` is newer than `current`.
    pub available: bool,
    pub published_at: Option<String>,
    /// The release notes (only ever a github.com page).
    pub notes_url: Option<String>,
    pub checked_at: Option<String>,
    /// Why the last check got no answer, in a sentence; the last answer is kept.
    pub problem: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Feed {
    version: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    notes_url: Option<String>,
}

/// A version's numbers and its pre-release part, if any.
type Parsed<'a> = ((u64, u64, u64), Option<&'a str>);

/// `1.2.3` or `1.2.3-rc.1` → ((1, 2, 3), pre-release).
fn parse(v: &str) -> Option<Parsed<'_>> {
    let v = v.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) if !p.is_empty() => (c, Some(p)),
        Some(_) => return None,
        None => (v, None),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some((triple, pre))
}

/// Whether `candidate` is a later release than `current` (a release outranks its own pre-releases).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some((a, a_pre)), Some((b, b_pre))) = (parse(candidate), parse(current)) else {
        return false;
    };
    match a.cmp(&b) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (a_pre, b_pre) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
    }
}

/// The last answer, re-measured against the version running now (the binary
/// may have been updated since the check).
pub fn status(app: &App) -> UpdateStatus {
    let mut s = app
        .db
        .cache_get::<UpdateStatus>(CACHE_KEY)
        .ok()
        .flatten()
        .map(|(s, _)| s)
        .unwrap_or_default();
    s.current = VERSION.to_string();
    s.available = s.latest.as_deref().is_some_and(|l| is_newer(l, VERSION));
    s
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(format!("yunori-mail/{VERSION}"))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()?)
}

async fn fetch_latest(base: &str) -> Result<Feed> {
    let res = client()?
        .get(format!("{base}/latest.json"))
        .header("accept", "application/json")
        .send()
        .await
        .with_context(|| format!("could not reach {base}"))?;
    match res.status().as_u16() {
        200 => {}
        404 => bail!("no release is published yet"),
        code => bail!("{base} answered HTTP {code}"),
    }
    let body = res
        .text()
        .await
        .context("the update feed's answer was cut off")?;
    let feed: Feed = serde_json::from_str(&body)
        .map_err(|_| anyhow::anyhow!("{base}/latest.json didn't answer with a release"))?;
    if parse(&feed.version).is_none() {
        bail!(
            "the update feed named an unreadable version ({})",
            feed.version
        );
    }
    Ok(feed)
}

/// Ask the feed now and remember the answer. A failed check keeps the last
/// answer and says why, so a bad minute never hides an available update.
pub async fn check(app: &App) -> UpdateStatus {
    let s = ask(status(app)).await;
    if let Err(e) = app.db.cache_put(CACHE_KEY, &s) {
        tracing::warn!("could not keep the update check: {e:#}");
    }
    s
}

/// Ask the feed without remembering (the CLI, before anything is installed).
pub async fn check_now() -> UpdateStatus {
    ask(UpdateStatus {
        current: VERSION.to_string(),
        ..Default::default()
    })
    .await
}

async fn ask(mut s: UpdateStatus) -> UpdateStatus {
    s.checked_at = Some(crate::util::now_rfc3339());
    match fetch_latest(&feed_base()).await {
        Ok(feed) => {
            s.latest = Some(feed.version.trim_start_matches('v').to_string());
            s.published_at = feed.published_at.filter(|p| !p.is_empty());
            s.notes_url = feed
                .notes_url
                .filter(|u| u.starts_with("https://github.com/"));
            s.problem = None;
        }
        Err(e) => {
            tracing::info!("update check: {e:#}");
            s.problem = Some(crate::util::first_sentence(&format!("{e:#}")).to_string());
        }
    }
    s.available = s.latest.as_deref().is_some_and(|l| is_newer(l, VERSION));
    s
}

/// The one-line installer, fetched from the feed, for `yunori-mail update` to run.
pub async fn fetch_installer() -> Result<String> {
    let base = feed_base();
    let res = client()?
        .get(format!("{base}/install.sh"))
        .send()
        .await
        .with_context(|| format!("could not reach {base}"))?;
    if !res.status().is_success() {
        bail!("{base}/install.sh answered HTTP {}", res.status().as_u16());
    }
    let body = res.text().await?;
    if !body.starts_with("#!/bin/sh") || !body.contains("yunori-mail") {
        bail!("{base}/install.sh is not the Yunori Mail installer");
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.10.0", "0.9.0"), "numeric, not textual");
        assert!(
            is_newer("1.0.0", "1.0.0-rc.1"),
            "a release outranks its pre-release"
        );
        assert!(is_newer("1.0.0-rc.2", "1.0.0-rc.1"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer("latest", "0.1.0"));
        assert!(!is_newer("1.2", "1.0.0"));
        assert!(!is_newer("1.2.3.4", "1.0.0"));
    }

    #[test]
    fn feed_is_https_or_loopback() {
        assert!(allowed_feed("https://mirror.example/server"));
        assert!(allowed_feed("http://127.0.0.1:9000"));
        assert!(!allowed_feed("http://yunori.email/server"));
        assert!(!allowed_feed("file:///tmp"));
    }

    #[tokio::test]
    async fn check_reads_the_feed_and_keeps_the_last_answer() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = r#"{"version":"99.0.0","publishedAt":"2026-11-06T10:00:00Z","notesUrl":"https://github.com/FutureForge-Studios/yunori-mail/releases/tag/v99.0.0","install":"x"}"#;
        tokio::spawn(async move {
            // One answer, then the "server" goes away.
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let res = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(res.as_bytes()).await.unwrap();
        });
        // SAFETY: tests in this module are the only readers of this variable.
        unsafe { std::env::set_var("YUNORI_UPDATE_FEED", format!("http://{addr}")) };
        let app = App::for_tests();

        let s = check(&app).await;
        assert_eq!(s.latest.as_deref(), Some("99.0.0"));
        assert!(s.available);
        assert_eq!(s.current, VERSION);
        assert!(s.problem.is_none());
        assert_eq!(status(&app), s);

        // The feed is gone: the answer stays, with the reason.
        let s = check(&app).await;
        assert_eq!(s.latest.as_deref(), Some("99.0.0"));
        assert!(s.available);
        assert!(s.problem.is_some());
        unsafe { std::env::remove_var("YUNORI_UPDATE_FEED") };
    }
}
