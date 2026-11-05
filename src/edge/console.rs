//! The console SPA, compiled by Vite into `console/dist` and embedded in the
//! binary. Served at `/console/*` and `/account/*`; assets under `/_yunori/*`.

use axum::body::Body;
use axum::http::{Response, StatusCode, header};

#[derive(rust_embed::Embed)]
#[folder = "console/dist/"]
#[allow_missing = true]
struct Assets;

const MISSING: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><title>Yunori Mail</title>
<body style="font-family:system-ui,sans-serif;max-width:36rem;margin:4rem auto;padding:0 1rem;color:#121621;background:#faf7f2">
<h1 style="font-weight:400">Yunori Mail</h1><p>The console was not built into this binary.
Build it with <code>npm --prefix console run build</code> and rebuild <code>yunori-mail</code>.</p></body></html>"#;

pub fn index() -> Response<Body> {
    match Assets::get("index.html") {
        Some(f) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(f.data.into_owned()))
            .unwrap(),
        None => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(MISSING))
            .unwrap(),
    }
}

pub fn asset(path: &str) -> Response<Body> {
    if path.contains("..") {
        return not_found();
    }
    match Assets::get(path) {
        Some(f) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache)
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .body(Body::from(f.data.into_owned()))
                .unwrap()
        }
        None => not_found(),
    }
}

fn not_found() -> Response<Body> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Not found"))
        .unwrap()
}
