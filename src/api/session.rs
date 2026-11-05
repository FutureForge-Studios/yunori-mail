//! First-run setup, login and logout for console admins.

use super::{ApiError, ApiResult, AppState, cookie, set_cookie};
use crate::auth::{self, SESSION_COOKIE, SESSION_HOURS};
use crate::util::valid_email;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    hostname: String,
    primary_domain: Option<String>,
    version: &'static str,
    engine_version: Option<String>,
    install_complete: bool,
    client_origin: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    authenticated: bool,
    setup_required: bool,
    admin: Option<auth::Admin>,
    server: ServerInfo,
}

pub(super) async fn server_info(app: &AppState) -> ServerInfo {
    let cfg = app.config();
    ServerInfo {
        hostname: cfg
            .as_ref()
            .map(|c| c.hostname.clone())
            .unwrap_or_else(|| "this server".into()),
        primary_domain: cfg.as_ref().map(|c| c.primary_domain.clone()),
        version: crate::app::VERSION,
        engine_version: cfg.as_ref().and_then(|c| c.stalwart_version.clone()),
        install_complete: cfg.as_ref().is_some_and(|c| c.install_complete),
        client_origin: cfg
            .map(|c| c.client_origin)
            .unwrap_or_else(|| crate::app::DEFAULT_CLIENT_ORIGIN.into()),
    }
}

async fn build_session(app: &AppState, admin: Option<auth::Admin>) -> ApiResult<SessionResponse> {
    Ok(SessionResponse {
        authenticated: admin.is_some(),
        setup_required: app.db.admin_count_active()? == 0,
        admin,
        server: server_info(app).await,
    })
}

pub async fn get_session(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<SessionResponse>> {
    let admin = match cookie(&headers, SESSION_COOKIE) {
        Some(t) => match app.db.session(&t)? {
            Some((kind, subject)) if kind == "admin" => app.db.admin_by_id(&subject)?,
            _ => None,
        },
        None => None,
    };
    Ok(Json(build_session(&app, admin).await?))
}

fn with_cookie<T: Serialize>(body: T, cookie_value: String) -> Response {
    let mut res = Json(body).into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, cookie_value.parse().unwrap());
    res
}

#[derive(Deserialize)]
pub struct SetupRequest {
    token: String,
    email: String,
    password: String,
}

pub async fn setup(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SetupRequest>,
) -> ApiResult<Response> {
    if app.throttled(
        &format!("setup:{}", super::client_ip(&headers)),
        10,
        Duration::from_secs(900),
    ) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts. Wait a few minutes.",
        ));
    }
    if app.db.admin_count_active()? > 0 {
        return Err(ApiError::conflict(
            "Setup is already complete. Sign in instead.",
        ));
    }
    if !app.db.check_setup_token(&req.token)? {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "invalid_token",
            "That setup token is not valid. Run `yunori-mail setup-token` on the server for a new one.").field("token"));
    }
    let email = req.email.trim().to_ascii_lowercase();
    if !valid_email(&email) {
        return Err(ApiError::bad_request("Enter a valid email address.").field("email"));
    }
    if req.password.chars().count() < 12 {
        return Err(ApiError::bad_request("Use at least 12 characters.").field("password"));
    }
    let admin = app
        .db
        .create_pending_admin(&email, &req.password, "owner")?;
    let secret = auth::new_totp_secret();
    app.db.put_secret(&format!("totp:{}", admin.id), &secret)?;
    let totp = auth::totp_for(&secret, &email)?;
    let token = app.db.create_session("setup", &admin.id, 1)?;
    Ok(with_cookie(
        json!({ "totpSecret": secret, "otpauthUrl": totp.get_url() }),
        set_cookie(SESSION_COOKIE, &token, 3600, !app.dev),
    ))
}

#[derive(Deserialize)]
pub struct CodeRequest {
    code: String,
}

pub async fn setup_totp(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CodeRequest>,
) -> ApiResult<Json<SessionResponse>> {
    let token = cookie(&headers, SESSION_COOKIE).ok_or_else(ApiError::unauthorized)?;
    let Some((kind, subject)) = app.db.session(&token)? else {
        return Err(ApiError::unauthorized());
    };
    if kind != "setup" {
        return Err(ApiError::unauthorized());
    }
    let admin = app
        .db
        .admin_by_id(&subject)?
        .ok_or_else(ApiError::unauthorized)?;
    let secret = app
        .db
        .get_secret(&format!("totp:{}", admin.id))?
        .ok_or_else(ApiError::unauthorized)?;
    if !auth::check_totp(&secret, &admin.email, &req.code) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_code",
            "That code did not match. Check the time on your phone and try the next code.",
        )
        .field("code"));
    }
    app.db.enable_totp(&admin.id)?;
    app.db.consume_setup_token()?;
    app.db.upgrade_session(&token, "admin")?;
    app.db.audit(
        &admin.email,
        "admin.setup",
        Some(&admin.email),
        Some("first owner created with TOTP"),
    )?;
    let admin = app.db.admin_by_id(&admin.id)?;
    Ok(Json(build_session(&app, admin).await?))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
    totp: String,
}

pub async fn login(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Response> {
    let email = req.email.trim().to_ascii_lowercase();
    let ip = super::client_ip(&headers);
    if app.throttled(&format!("login:{email}"), 8, Duration::from_secs(900))
        || app.throttled(&format!("login-ip:{ip}"), 30, Duration::from_secs(900))
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts. Wait 15 minutes.",
        ));
    }
    let invalid = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Email, password or code is incorrect.",
        )
    };
    let Some((admin, hash)) = app.db.admin_by_email(&email)? else {
        auth::dummy_verify(&req.password);
        return Err(invalid());
    };
    if !auth::verify_password(&req.password, &hash) || !admin.totp_enabled {
        return Err(invalid());
    }
    let secret = app
        .db
        .get_secret(&format!("totp:{}", admin.id))?
        .ok_or_else(invalid)?;
    if !auth::check_totp(&secret, &admin.email, &req.totp) {
        return Err(invalid());
    }
    let token = app.db.create_session("admin", &admin.id, SESSION_HOURS)?;
    app.db.audit(
        &admin.email,
        "admin.login",
        None,
        Some(&format!("from {ip}")),
    )?;
    let body = build_session(&app, Some(admin)).await?;
    Ok(with_cookie(
        body,
        set_cookie(SESSION_COOKIE, &token, SESSION_HOURS * 3600, !app.dev),
    ))
}

pub async fn logout(State(app): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(t) = cookie(&headers, SESSION_COOKIE) {
        app.db.delete_session(&t)?;
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        set_cookie(SESSION_COOKIE, "", 0, !app.dev).parse().unwrap(),
    );
    Ok(res)
}
