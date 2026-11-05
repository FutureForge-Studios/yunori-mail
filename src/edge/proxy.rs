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

pub fn is_proxied(path: &str) -> bool {
    !path.contains("..")
        && ALLOWED_PREFIXES
            .iter()
            .any(|p| path == p.trim_end_matches('/') || path.starts_with(p))
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
    let url = format!("{}{}", engine_url.trim_end_matches('/'), path_q);
    let method = reqwest::Method::from_bytes(req.method().as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut headers = HeaderMap::new();
    copy_headers(req.headers(), &mut headers);
    // Only forward a real client address: Stalwart rejects (HTTP 400) an
    // X-Forwarded-For that claims loopback, and an unknown address is better
    // omitted (it then uses the loopback peer). Verified on a live 1.0.3 engine.
    if let Ok(ip) = client_ip.parse::<std::net::IpAddr>()
        && !ip.is_loopback()
        && !ip.is_unspecified()
        && let Ok(v) = ip.to_string().parse()
    {
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
        .request(method, &url)
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
}
