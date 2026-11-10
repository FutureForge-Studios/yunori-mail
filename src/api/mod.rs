//! Console + portal REST API under `/api/yunori` (contract:
//! `console/src/api/types.ts`).
//!
//! Every non-GET request must carry `X-Yunori-Request: 1` (CSRF guard on top
//! of SameSite=Strict cookies). Errors are `{ "error": { code, message, field? } }`.

pub(crate) mod admin;
mod alerts;
mod dns_provider;
mod migrate;
mod portal;
mod session;

pub use portal::take_profile;

use crate::app::App;
use crate::engine::EngineError;
use axum::extract::{FromRef, FromRequestParts, Request};
use axum::http::{HeaderMap, Method, StatusCode, header, request::Parts};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use serde_json::json;
use std::sync::Arc;

pub type AppState = Arc<App>;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub field: Option<std::borrow::Cow<'static, str>>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            field: None,
        }
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }
    pub fn field(mut self, f: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        self.field = Some(f.into());
        self
    }
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Please sign in.",
        )
    }
    pub fn not_found(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{what} was not found."),
        )
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "code": self.code, "message": self.message });
        if let Some(f) = self.field {
            body["field"] = json!(f);
        }
        (self.status, Json(json!({ "error": body }))).into_response()
    }
}

impl From<EngineError> for ApiError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Unavailable(m) => {
                tracing::warn!("engine unavailable: {m}");
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "engine_unavailable",
                    "The mail engine is not responding. Check System for details.",
                )
            }
            EngineError::Unauthorized => ApiError::new(
                StatusCode::BAD_GATEWAY,
                "engine_auth",
                "The console could not authenticate to the mail engine. Run `yunori-mail doctor`.",
            ),
            EngineError::NotFound => ApiError::not_found("That item"),
            EngineError::Rejected(m) => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "rejected", m)
            }
            EngineError::Protocol(m) => {
                tracing::error!("engine protocol error: {m}");
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "engine_error",
                    "The mail engine returned an unexpected answer.",
                )
            }
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        if let Some(engine) = e.downcast_ref::<EngineError>() {
            return match engine {
                EngineError::Unavailable(m) => EngineError::Unavailable(m.clone()).into(),
                EngineError::Unauthorized => EngineError::Unauthorized.into(),
                EngineError::NotFound => EngineError::NotFound.into(),
                EngineError::Rejected(m) => EngineError::Rejected(m.clone()).into(),
                EngineError::Protocol(m) => EngineError::Protocol(m.clone()).into(),
            };
        }
        tracing::error!("internal error: {e:#}");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Something went wrong on the server.",
        )
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Signed-in, second-factor-verified console admin.
pub struct AdminAuth {
    pub admin: crate::auth::Admin,
}

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

impl<S> FromRequestParts<S> for AdminAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let token = cookie(&parts.headers, crate::auth::SESSION_COOKIE)
            .ok_or_else(ApiError::unauthorized)?;
        match app.db.session(&token)? {
            Some((kind, subject)) if kind == "admin" => {
                let admin = app
                    .db
                    .admin_by_id(&subject)?
                    .ok_or_else(ApiError::unauthorized)?;
                Ok(AdminAuth { admin })
            }
            _ => Err(ApiError::unauthorized()),
        }
    }
}

/// Origin people reach this server at: `https://<mail hostname>` in
/// production; in `--dev` the address the browser used (plain HTTP on loopback).
pub fn public_base(app: &App, headers: &HeaderMap) -> String {
    if app.dev {
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .filter(|h| {
                h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
            })
            .unwrap_or("127.0.0.1:8787");
        return format!("http://{host}");
    }
    let host = app.config().map(|c| c.hostname).unwrap_or_default();
    format!("https://{host}")
}

/// Client IP as seen by the edge (direct peer; the edge is the TLS terminator).
pub fn client_ip(parts_headers: &HeaderMap) -> String {
    parts_headers
        .get("x-yunori-client-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

async fn csrf_guard(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe
        && req
            .headers()
            .get("x-yunori-request")
            .and_then(|v| v.to_str().ok())
            != Some("1")
    {
        return ApiError::new(StatusCode::FORBIDDEN, "csrf", "Missing request header.")
            .into_response();
    }
    let mut res = next.run(req).await;
    res.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    res
}

pub fn router(app: AppState) -> Router {
    Router::new()
        // session & auth
        .route("/session", get(session::get_session))
        .route("/setup", post(session::setup))
        .route("/setup/totp", post(session::setup_totp))
        .route("/login", post(session::login))
        .route("/logout", post(session::logout))
        // console
        .route("/overview", get(admin::overview))
        .route("/install", get(admin::install_state))
        .route("/install/first-mailbox", post(admin::install_first_mailbox))
        .route("/preflight", get(admin::preflight))
        .route("/certificate", get(admin::certificate))
        .route("/certificate/renew", post(admin::certificate_renew))
        .route(
            "/domains",
            get(admin::list_domains).post(admin::create_domain),
        )
        .route(
            "/domains/{id}",
            get(admin::get_domain)
                .patch(admin::update_domain)
                .delete(admin::delete_domain),
        )
        .route("/domains/{id}/dns", get(admin::domain_dns))
        .route("/domains/{id}/dns/check", post(admin::domain_dns_check))
        .route(
            "/domains/{id}/dns/provider",
            get(dns_provider::get_provider)
                .put(dns_provider::put_provider)
                .delete(dns_provider::delete_provider),
        )
        .route("/domains/{id}/dns/plan", get(dns_provider::plan))
        .route("/domains/{id}/dns/publish", post(dns_provider::publish))
        .route(
            "/mailboxes",
            get(admin::list_mailboxes).post(admin::create_mailbox),
        )
        .route("/mailboxes/export", get(admin::export_mailboxes))
        .route("/mailboxes/import", post(admin::import_mailboxes))
        .route(
            "/mailboxes/{id}",
            patch(admin::update_mailbox).delete(admin::delete_mailbox),
        )
        .route("/mailboxes/{id}/password", post(admin::reset_password))
        .route("/mailboxes/{id}/invite", post(admin::invite_mailbox))
        .route(
            "/aliases",
            get(admin::list_aliases)
                .post(admin::create_alias)
                .delete(admin::delete_alias),
        )
        .route("/sending", get(admin::get_sending).put(admin::put_sending))
        .route("/deliverability", get(admin::deliverability))
        .route("/system", get(admin::system))
        .route("/updates/check", post(admin::update_check))
        .route("/audit", get(admin::audit))
        .route("/admins", get(admin::admins))
        .route("/alerts", get(alerts::get_alerts))
        .route("/alerts/settings", put(alerts::put_settings))
        .route("/alerts/test", post(alerts::test_alert))
        .route("/alerts/check", post(alerts::check_now))
        // portal
        .route("/portal/login", post(portal::login))
        .route("/portal/logout", post(portal::logout))
        .route("/portal/me", get(portal::me))
        .route("/portal/password", post(portal::change_password))
        .route("/portal/app-passwords", post(portal::create_app_password))
        .route(
            "/portal/app-passwords/{id}",
            delete(portal::delete_app_password),
        )
        .route(
            "/portal/rules",
            get(portal::get_rules).put(portal::put_rules),
        )
        .route("/portal/invite", post(portal::invite_check))
        .route("/portal/invite/accept", post(portal::invite_accept))
        .route("/portal/devices/apple", post(portal::apple_profile))
        .route("/portal/migrate", get(migrate::status).post(migrate::start))
        .route("/portal/migrate/test", post(migrate::test))
        .route("/portal/migrate/cancel", post(migrate::cancel))
        .fallback(|| async { ApiError::not_found("This endpoint") })
        .layer(middleware::from_fn(csrf_guard))
        .with_state(app)
}

/// Build a `Set-Cookie` header value.
pub fn set_cookie(name: &str, value: &str, max_age_secs: i64, secure: bool) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_secs}{}",
        if secure { "; Secure" } else { "" }
    )
}

#[cfg(test)]
mod tests;
