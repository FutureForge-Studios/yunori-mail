//! Reverse proxy from the edge to Stalwart's loopback HTTP listener — only for
//! the paths mail clients need (JMAP, DAV, discovery, MTA-STS). Stalwart's own
//! web admin is never exposed.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, Request, Response, StatusCode, header};
use std::sync::OnceLock;
use std::time::Duration;

const ALLOWED_PREFIXES: &[&str] = &[
    "/jmap",
    "/.well-known/jmap",
    "/.well-known/caldav",
    "/.well-known/carddav",
    "/.well-known/mta-sts.txt",
    "/.well-known/oauth-authorization-server",
    "/.well-known/openid-configuration",
    "/dav/",
    "/autodiscover/",
    "/Autodiscover/",
    "/auth/",
];

/// Whether a request path may reach the engine. The path is checked as the
/// engine will see it: a percent-encoded dot segment (`%2e%2e`, `.%2E`) or an
/// encoded separator (`%2f`, `%5c`) would otherwise be resolved *after* this
/// check (by the URL parser or by the engine) and escape the allowlist into
/// Stalwart's web admin and management API.
pub fn is_proxied(path: &str) -> bool {
    path.starts_with('/')
        && segments_are_plain(path)
        && ALLOWED_PREFIXES
            .iter()
            .any(|p| path == p.trim_end_matches('/') || path.starts_with(p))
}

/// Every segment, decoded as many times as anything downstream might decode
/// it, is a plain name: not `.`/`..`, and without `/`, `\` or control bytes.
fn segments_are_plain(path: &str) -> bool {
    if path.contains('\\') {
        return false;
    }
    path.split('/').all(|seg| {
        let mut cur = seg.as_bytes().to_vec();
        for _ in 0..4 {
            let next = percent_decode(&cur);
            if next.contains(&b'/')
                || next.contains(&b'\\')
                || next.iter().any(|b| *b < 0x20 || *b == 0x7f)
            {
                return false;
            }
            if next == b"." || next == b".." {
                return false;
            }
            if next == cur {
                break;
            }
            cur = next;
        }
        true
    })
}

fn percent_decode(s: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            if let (Some(h), Some(l)) = (hex(s[i + 1]), hex(s[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "x-forwarded-for",
    "x-forwarded-proto",
    "x-forwarded-host",
    "x-yunori-client-ip",
];

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("proxy client")
    })
}

fn copy_headers(from: &HeaderMap, to: &mut HeaderMap) {
    for (k, v) in from {
        if !HOP_BY_HOP.contains(&k.as_str()) {
            to.append(k.clone(), v.clone());
        }
    }
}

/// The X-Forwarded-For value for a client, if any. Only a real client address
/// is forwarded: Stalwart rejects (HTTP 400) one that claims loopback —
/// including the IPv4-mapped `::ffff:127.0.0.1` a dual-stack listener reports —
/// and an unknown address is better omitted (it then uses the loopback peer).
/// Verified on a live 1.0.3 engine.
fn forwarded_for(client_ip: &str) -> Option<axum::http::HeaderValue> {
    let ip = client_ip.parse::<std::net::IpAddr>().ok()?.to_canonical();
    if ip.is_loopback() || ip.is_unspecified() {
        return None;
    }
    ip.to_string().parse().ok()
}

pub async fn forward(
    engine_url: &str,
    client_ip: &str,
    host: &str,
    req: Request<Body>,
) -> Response<Body> {
    let path_q = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/")
        .to_string();
    // Parse once, re-check the path the URL parser normalised, and send that
    // same URL: what is checked is exactly what the engine receives.
    let url = match reqwest::Url::parse(&format!("{}{}", engine_url.trim_end_matches('/'), path_q))
    {
        Ok(u) if is_proxied(u.path()) => u,
        _ => return not_found(),
    };
    let method = reqwest::Method::from_bytes(req.method().as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut headers = HeaderMap::new();
    copy_headers(req.headers(), &mut headers);
    if let Some(v) = forwarded_for(client_ip) {
        headers.insert(HeaderName::from_static("x-forwarded-for"), v);
    }
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        "https".parse().unwrap(),
    );
    if let Ok(h) = host.parse() {
        headers.insert(HeaderName::from_static("x-forwarded-host"), h);
    }
    let body = reqwest::Body::wrap_stream(req.into_body().into_data_stream());
    let upstream = client()
        .request(method, url)
        .headers(headers)
        .body(body)
        .send()
        .await;
    match upstream {
        Ok(resp) => {
            let mut out = Response::builder().status(resp.status().as_u16());
            if let Some(h) = out.headers_mut() {
                copy_headers(resp.headers(), h);
            }
            out.body(Body::from_stream(resp.bytes_stream()))
                .unwrap_or_else(|_| bad_gateway())
        }
        Err(e) => {
            tracing::warn!("proxy to engine failed: {e}");
            bad_gateway()
        }
    }
}

fn not_found() -> Response<Body> {
    let mut r = Response::new(Body::from("Not found"));
    *r.status_mut() = StatusCode::NOT_FOUND;
    r
}

fn bad_gateway() -> Response<Body> {
    let mut r = Response::new(Body::from("The mail engine is not responding."));
    *r.status_mut() = StatusCode::BAD_GATEWAY;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        "text/plain; charset=utf-8".parse().unwrap(),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        assert!(is_proxied("/jmap/"));
        assert!(is_proxied("/jmap"));
        assert!(is_proxied("/.well-known/jmap"));
        assert!(is_proxied("/dav/cal/ana"));
        assert!(!is_proxied("/admin"));
        assert!(!is_proxied("/login"));
        assert!(!is_proxied("/dav/../admin"));
        assert!(!is_proxied("/api/schema"));
    }

    #[test]
    fn encoded_traversal_never_passes() {
        for p in [
            "/dav/%2e%2e/admin",
            "/dav/.%2E/login",
            "/dav/%2E./login",
            "/dav/%2e/admin",
            "/jmap/%2e%2e/api/settings",
            "/.well-known/jmap/%2e%2e/%2e%2e/admin",
            "/dav/%252e%252e/admin",
            "/dav/a%2fb",
            "/dav/a%2F..%2Fadmin",
            "/dav/a%5c..%5cadmin",
            "/dav/a\\..\\admin",
            "/dav/%00",
            "/dav/./x",
        ] {
            assert!(!is_proxied(p), "{p} must not be proxied");
        }
        assert!(is_proxied("/dav/cal/ana%40example.com/default"));
        assert!(is_proxied("/jmap/download/a/b/name%20with%20space.pdf"));
        assert!(is_proxied("/dav/card/x/file%25.vcf"));
    }

    #[test]
    fn decoder() {
        assert_eq!(percent_decode(b"%2e%2E"), b"..");
        assert_eq!(percent_decode(b"a%"), b"a%");
        assert_eq!(percent_decode(b"a%2"), b"a%2");
        assert_eq!(percent_decode(b"%zz"), b"%zz");
    }

    #[test]
    fn forwarded_for_is_canonical_and_never_loopback() {
        let v = |s: &str| forwarded_for(s).map(|h| h.to_str().unwrap().to_string());
        assert_eq!(v("::ffff:203.0.113.9").as_deref(), Some("203.0.113.9"));
        assert_eq!(v("203.0.113.9").as_deref(), Some("203.0.113.9"));
        assert_eq!(v("2001:db8::9").as_deref(), Some("2001:db8::9"));
        assert_eq!(v("::ffff:127.0.0.1"), None);
        assert_eq!(v("127.0.0.1"), None);
        assert_eq!(v("::1"), None);
        assert_eq!(v("unknown"), None);
    }
}
