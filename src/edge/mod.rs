//! The HTTPS edge (plan D7): owns :80/:443, terminates TLS with the ACME
//! certificate, serves the console + API + autoconfig, and proxies the
//! client-facing paths (JMAP, DAV, MTA-STS) to Stalwart on loopback.

pub mod acme;
pub mod autoconfig;
pub mod certs;
pub mod console;
pub mod proxy;
pub mod tls;

use crate::api::{self, AppState};
use axum::Router;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

pub fn valid_token(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 256
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn acme_challenge(State(app): State<AppState>, Path(token): Path<String>) -> Response {
    if !valid_token(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read_to_string(app.paths.acme_challenges_dir().join(&token)) {
        Ok(body) => ([(header::CONTENT_TYPE, "application/octet-stream")], body).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn autoconfig_handler(
    State(app): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(cfg) = app.config() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let requested = q.get("emailaddress").and_then(|e| autoconfig::domain_of(e));
    // Only answer for domains this server hosts.
    let domain = match requested {
        Some(d) => {
            let hosted = match app.engine() {
                Ok(e) => e
                    .list_domains()
                    .await
                    .map(|l| l.iter().any(|x| x.name == d))
                    .unwrap_or(d == cfg.primary_domain),
                Err(_) => d == cfg.primary_domain,
            };
            if !hosted {
                return StatusCode::NOT_FOUND.into_response();
            }
            d
        }
        None => cfg.primary_domain.clone(),
    };
    (
        [
            (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
            (header::CACHE_CONTROL, "max-age=3600"),
        ],
        autoconfig::render(&domain, &cfg.hostname),
    )
        .into_response()
}

async fn spa() -> Response {
    console::index()
}

async fn asset(Path(path): Path<String>) -> Response {
    console::asset(&path)
}

async fn fallback(State(app): State<AppState>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    if proxy::is_proxied(&path)
        && let Some(cfg) = app.config()
        && app.engine().is_ok()
    {
        let ip = req
            .headers()
            .get("x-yunori-client-ip")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("0.0.0.0")
            .to_string();
        return proxy::forward(&cfg.engine_url, &ip, &cfg.hostname, req).await;
    }
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

/// Stamp the real peer address (overwriting anything a client sent).
async fn client_ip(req: Request, next: Next) -> Response {
    let mut req = req;
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".into());
    req.headers_mut().insert(
        "x-yunori-client-ip",
        HeaderValue::from_str(&ip).unwrap_or(HeaderValue::from_static("unknown")),
    );
    next.run(req).await
}

async fn security_headers(State(app): State<AppState>, req: Request, next: Next) -> Response {
    let proxied = proxy::is_proxied(req.uri().path());
    let mut res = next.run(req).await;
    if proxied {
        return res;
    }
    let h = res.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, k: header::HeaderName, v: &'static str| {
        h.entry(k).or_insert(HeaderValue::from_static(v));
    };
    set(
        h,
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    );
    set(h, header::X_FRAME_OPTIONS, "DENY");
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::REFERRER_POLICY, "no-referrer");
    set(
        h,
        HeaderName::from_static("permissions-policy"),
        "camera=(), microphone=(), geolocation=()",
    );
    if !app.dev {
        set(h, header::STRICT_TRANSPORT_SECURITY, "max-age=31536000");
    }
    res
}

use axum::http::HeaderName;

pub fn router(app: AppState) -> Router {
    Router::<AppState>::new()
        .nest_service("/api/yunori", api::router(app.clone()))
        .route("/.well-known/acme-challenge/{token}", get(acme_challenge))
        .route("/mail/config-v1.1.xml", get(autoconfig_handler))
        .route(
            "/.well-known/autoconfig/mail/config-v1.1.xml",
            get(autoconfig_handler),
        )
        .route("/", get(|| async { Redirect::to("/account") }))
        .route("/console", get(spa))
        .route("/console/{*rest}", get(spa))
        .route("/account", get(spa))
        .route("/account/{*rest}", get(spa))
        .route("/_yunori/{*path}", get(asset))
        .route("/healthz", get(|| async { "ok" }))
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(
            app.clone(),
            security_headers,
        ))
        .layer(middleware::from_fn(client_ip))
        .with_state(app)
}

/// Port 80: ACME challenges + redirect everything else to HTTPS.
fn http_router(app: AppState) -> Router {
    Router::new()
        .route("/.well-known/acme-challenge/{token}", get(acme_challenge))
        .fallback(|uri: Uri, headers: axum::http::HeaderMap| async move {
            let host = headers
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .map(|h| h.split(':').next().unwrap_or(h).to_string())
                .filter(|h| crate::util::normalize_domain(h).is_some());
            match host {
                Some(h) => Redirect::permanent(&format!(
                    "https://{h}{}",
                    uri.path_and_query().map(|p| p.as_str()).unwrap_or("/")
                ))
                .into_response(),
                None => (StatusCode::BAD_REQUEST, "HTTPS only").into_response(),
            }
        })
        .with_state(app)
}

pub struct EdgeOptions {
    pub dev: bool,
    pub dev_listen: SocketAddr,
}

pub async fn serve(app: AppState, opts: EdgeOptions) -> anyhow::Result<()> {
    if opts.dev {
        tracing::info!(
            "development edge on http://{} (console at /console)",
            opts.dev_listen
        );
        let listener = tokio::net::TcpListener::bind(opts.dev_listen).await?;
        axum::serve(
            listener,
            router(app).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        return Ok(());
    }

    let hostname = app
        .config()
        .map(|c| c.hostname)
        .unwrap_or_else(|| "localhost".into());
    let hot = tls::HotCert::new(tls::load_or_self_signed(&app.paths, &hostname)?);
    let mut server_cfg = rustls::ServerConfig::builder_with_provider(tls::provider())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_cert_resolver(hot.clone());
    server_cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let rustls_cfg = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_cfg));

    // Reload the certificate when the files change (ACME issue/renewal).
    let watch_app = app.clone();
    tokio::spawn(async move {
        let mut last = std::fs::metadata(tls::cert_path(&watch_app.paths))
            .and_then(|m| m.modified())
            .ok();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let now = std::fs::metadata(tls::cert_path(&watch_app.paths))
                .and_then(|m| m.modified())
                .ok();
            if now != last {
                last = now;
                if let Some((c, k)) = tls::read_cert_files(&watch_app.paths) {
                    match tls::certified_key(&c, &k) {
                        Ok(ck) => {
                            hot.set(ck);
                            tracing::info!("edge certificate reloaded");
                        }
                        Err(e) => tracing::error!("new certificate could not be loaded: {e:#}"),
                    }
                }
            }
        }
    });

    let http_app = app.clone();
    let http = tokio::spawn(async move {
        let l = tokio::net::TcpListener::bind("[::]:80").await?;
        axum::serve(l, http_router(http_app)).await?;
        Ok::<_, anyhow::Error>(())
    });
    let https = axum_server::bind_rustls("[::]:443".parse()?, rustls_cfg)
        .serve(router(app).into_make_service_with_connect_info::<SocketAddr>());
    tracing::info!("edge listening on :80 and :443");
    tokio::select! {
        r = https => r?,
        r = http => r??,
    }
    Ok(())
}
