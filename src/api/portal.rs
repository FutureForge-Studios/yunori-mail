//! End-user self-service (`/account`). Acts AS the mailbox user against the
//! engine — the user's password is held in memory for ≤30 minutes, never
//! written to disk, and every action is authorised by Stalwart itself.

use super::{ApiError, ApiResult, AppState, cookie, set_cookie};
use crate::app::PortalSession;
use crate::auth::PORTAL_COOKIE;
use crate::engine::{EngineError, UserCreds};
use crate::util::{random_token, sha256_hex, valid_email};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const PORTAL_TTL: Duration = Duration::from_secs(30 * 60);

fn creds_from(app: &AppState, headers: &HeaderMap) -> ApiResult<UserCreds> {
    let token = cookie(headers, PORTAL_COOKIE).ok_or_else(ApiError::unauthorized)?;
    let key = sha256_hex(token.as_bytes());
    let mut sessions = app.portal_sessions.lock().unwrap();
    sessions.retain(|_, s| s.expires > Instant::now());
    let s = sessions.get_mut(&key).ok_or_else(ApiError::unauthorized)?;
    s.expires = Instant::now() + PORTAL_TTL; // sliding
    Ok(UserCreds {
        email: s.email.clone(),
        password: s.password.clone(),
    })
}

fn map_user_err(e: EngineError) -> ApiError {
    match e {
        EngineError::Unauthorized => ApiError::unauthorized(),
        other => other.into(),
    }
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

pub async fn login(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Response> {
    let email = req.email.trim().to_ascii_lowercase();
    let ip = super::client_ip(&headers);
    if app.throttled(&format!("portal:{email}"), 8, Duration::from_secs(900))
        || app.throttled(&format!("portal-ip:{ip}"), 40, Duration::from_secs(900))
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "throttled",
            "Too many attempts. Wait 15 minutes.",
        ));
    }
    let invalid = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Email or password is incorrect.",
        )
    };
    if !valid_email(&email) || req.password.is_empty() {
        return Err(invalid());
    }
    let creds = UserCreds {
        email: email.clone(),
        password: Zeroizing::new(req.password),
    };
    match app.engine()?.user_verify(&creds).await {
        Ok(()) => {}
        Err(EngineError::Unauthorized) => return Err(invalid()),
        Err(e) => return Err(e.into()),
    }
    let token = random_token(32);
    app.portal_sessions.lock().unwrap().insert(
        sha256_hex(token.as_bytes()),
        PortalSession {
            email: creds.email.clone(),
            password: creds.password.clone(),
            expires: Instant::now() + PORTAL_TTL,
        },
    );
    let body = me_body(&app, &creds).await?;
    let mut res = Json(body).into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        set_cookie(PORTAL_COOKIE, &token, PORTAL_TTL.as_secs() as i64, !app.dev)
            .parse()
            .unwrap(),
    );
    Ok(res)
}

pub async fn logout(State(app): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(t) = cookie(&headers, PORTAL_COOKIE) {
        app.portal_sessions
            .lock()
            .unwrap()
            .remove(&sha256_hex(t.as_bytes()));
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        set_cookie(PORTAL_COOKIE, "", 0, !app.dev).parse().unwrap(),
    );
    Ok(res)
}

async fn me_body(app: &AppState, creds: &UserCreds) -> ApiResult<Value> {
    let engine = app.engine()?;
    // Quota/display name come from the admin view of the same account (read-only).
    let account = engine
        .list_accounts()
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|a| a.address == creds.email));
    let app_passwords = engine
        .user_app_passwords(creds)
        .await
        .map_err(map_user_err)?;
    let cfg = app.config();
    let host = cfg.as_ref().map(|c| c.hostname.clone()).unwrap_or_default();
    Ok(json!({
        "address": creds.email,
        "displayName": account.as_ref().and_then(|a| a.display_name.clone()),
        "quotaBytes": account.as_ref().and_then(|a| a.quota_bytes),
        "usedBytes": account.as_ref().and_then(|a| a.used_bytes),
        "appPasswords": app_passwords.into_iter().map(|p| json!({
            "id": p.id, "description": p.description, "createdAt": p.created_at
        })).collect::<Vec<_>>(),
        "settings": {
            "imap": { "host": host, "port": 993, "security": "SSL/TLS" },
            "smtp": { "host": host, "port": 465, "security": "SSL/TLS" },
        },
        "clientOrigin": cfg.map(|c| c.client_origin).unwrap_or_else(|| crate::app::DEFAULT_CLIENT_ORIGIN.into()),
    }))
}

pub async fn me(State(app): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let creds = creds_from(&app, &headers)?;
    Ok(Json(me_body(&app, &creds).await?))
}

#[derive(Deserialize)]
pub struct PasswordRequest {
    current: String,
    next: String,
}

pub async fn change_password(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasswordRequest>,
) -> ApiResult<StatusCode> {
    let creds = creds_from(&app, &headers)?;
    if !crate::auth::constant_eq(req.current.as_bytes(), creds.password.as_bytes()) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Your current password is incorrect.",
        )
        .field("current"));
    }
    if req.next.chars().count() < 12 {
        return Err(ApiError::bad_request("Use at least 12 characters.").field("next"));
    }
    app.engine()?
        .user_change_password(&creds, &req.next)
        .await
        .map_err(map_user_err)?;
    // Keep the session usable with the new password.
    if let Some(t) = cookie(&headers, PORTAL_COOKIE)
        && let Some(s) = app
            .portal_sessions
            .lock()
            .unwrap()
            .get_mut(&sha256_hex(t.as_bytes()))
    {
        s.password = Zeroizing::new(req.next.clone());
    }
    app.db.audit(
        &creds.email,
        "portal.password_change",
        Some(&creds.email),
        None,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AppPasswordRequest {
    description: String,
}

pub async fn create_app_password(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AppPasswordRequest>,
) -> ApiResult<Response> {
    let creds = creds_from(&app, &headers)?;
    let description = req.description.trim();
    if description.is_empty() || description.len() > 80 {
        return Err(ApiError::bad_request(
            "Name it after the device or app (up to 80 characters).",
        )
        .field("description"));
    }
    let (p, secret) = app
        .engine()?
        .user_create_app_password(&creds, description)
        .await
        .map_err(map_user_err)?;
    app.db.audit(
        &creds.email,
        "portal.app_password_create",
        Some(&creds.email),
        Some(description),
    )?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "appPassword": { "id": p.id, "description": p.description, "createdAt": p.created_at },
            "secret": secret,
        })),
    )
        .into_response())
}

pub async fn delete_app_password(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let creds = creds_from(&app, &headers)?;
    app.engine()?
        .user_delete_app_password(&creds, &id)
        .await
        .map_err(map_user_err)?;
    app.db.audit(
        &creds.email,
        "portal.app_password_revoke",
        Some(&creds.email),
        None,
    )?;
    Ok(StatusCode::NO_CONTENT)
}
