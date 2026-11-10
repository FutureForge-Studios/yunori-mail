//! End-user self-service (`/account`). Acts AS the mailbox user against the
//! engine — the user's password is held in memory for ≤30 minutes, never
//! written to disk, and every action is authorised by Stalwart itself.

use super::{ApiError, ApiResult, AppState, cookie, public_base, set_cookie};
use crate::app::{PendingProfile, PortalSession};
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
/// How long a device-profile download link works (once).
const PROFILE_TTL: Duration = Duration::from_secs(10 * 60);

fn creds_from(app: &AppState, headers: &HeaderMap) -> ApiResult<UserCreds> {
    let token = cookie(headers, PORTAL_COOKIE).ok_or_else(ApiError::unauthorized)?;
    let key = sha256_hex(token.as_bytes());
    let mut sessions = app.portal_sessions.lock().unwrap();
    sessions.retain(|_, s| s.expires > Instant::now());
    let s = sessions.get(&key).ok_or_else(ApiError::unauthorized)?;
    // Not sliding: the user's password is held for at most PORTAL_TTL after
    // sign-in, however often the session is used (the cookie expires then too).
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
    // Per address *from this IP*, so a stranger cannot lock the owner out by
    // failing on purpose; the per-address ceiling still bounds a distributed
    // guess. The IP is checked first and over-long input never becomes a key.
    let window = Duration::from_secs(900);
    if email.len() > 254
        || app.throttled(&format!("portal-ip:{ip}"), 40, window)
        || app.throttled(&format!("portal:{email}|{ip}"), 8, window)
        || app.throttled(&format!("portal:{email}"), 200, window)
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
    begin_session(&app, creds).await
}

/// Opens a portal session for credentials Stalwart has just accepted.
async fn begin_session(app: &AppState, creds: UserCreds) -> ApiResult<Response> {
    let token = random_token(32);
    app.portal_sessions.lock().unwrap().insert(
        sha256_hex(token.as_bytes()),
        PortalSession {
            email: creds.email.clone(),
            password: creds.password.clone(),
            expires: Instant::now() + PORTAL_TTL,
        },
    );
    let body = me_body(app, &creds).await?;
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
            // Calendars and contacts: apps discover the paths from the host
            // (/.well-known/caldav, /.well-known/carddav).
            "dav": { "host": host, "url": format!("https://{host}/") },
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

// ── invitations ──────────────────────────────────────────────────────────

fn invite_gone() -> ApiError {
    ApiError::new(
        StatusCode::GONE,
        "invite_invalid",
        "This invite link has expired or was already used. Ask the person who runs this mail server for a new one.",
    )
}

fn invite_throttled(app: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let ip = super::client_ip(headers);
    if app.throttled(&format!("invite-ip:{ip}"), 30, Duration::from_secs(900)) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts. Wait 15 minutes.",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct InviteCheck {
    token: String,
}

/// Who an invite is for, so the page can greet them before they choose a password.
pub async fn invite_check(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<InviteCheck>,
) -> ApiResult<Json<Value>> {
    invite_throttled(&app, &headers)?;
    let invite = app.db.invite(req.token.trim())?.ok_or_else(invite_gone)?;
    let display_name = app
        .engine()?
        .list_accounts()
        .await
        .ok()
        .and_then(|l| l.into_iter().find(|a| a.id == invite.account_id))
        .and_then(|a| a.display_name);
    Ok(Json(json!({
        "address": invite.address,
        "displayName": display_name,
        "expiresAt": invite.expires_at,
    })))
}

#[derive(Deserialize)]
pub struct InviteAccept {
    token: String,
    password: String,
}

/// Sets the person's own password and signs them in to the portal.
pub async fn invite_accept(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<InviteAccept>,
) -> ApiResult<Response> {
    invite_throttled(&app, &headers)?;
    let token = req.token.trim();
    if req.password.chars().count() < 12 {
        return Err(ApiError::bad_request("Use at least 12 characters.").field("password"));
    }
    let invite = app.db.invite(token)?.ok_or_else(invite_gone)?;
    let engine = app.engine()?;
    let Some(account) = engine
        .list_accounts()
        .await?
        .into_iter()
        .find(|a| a.id == invite.account_id && a.address == invite.address)
    else {
        // The mailbox was deleted (or its id reused) since the link was made.
        app.db.revoke_invites(&invite.account_id)?;
        return Err(invite_gone());
    };
    if !app.db.consume_invite(token)? {
        return Err(invite_gone());
    }
    if let Err(e) = engine.set_password(&account.id, &req.password).await {
        app.db.release_invite(token)?;
        return Err(match e {
            EngineError::Rejected(_) => ApiError::bad_request(
                "The mail server refused that password as too easy to guess. Choose a longer or less predictable one.",
            )
            .field("password"),
            other => other.into(),
        });
    }
    app.db.audit(
        &account.address,
        "mailbox.invite_accept",
        Some(&account.address),
        Some(&format!("invited by {}", invite.created_by)),
    )?;
    begin_session(
        &app,
        UserCreds {
            email: account.address,
            password: Zeroizing::new(req.password),
        },
    )
    .await
}

// ── devices ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleProfileRequest {
    device: String,
    #[serde(default = "yes")]
    calendars: bool,
}

fn yes() -> bool {
    true
}

/// Makes an app password for one iPhone, iPad or Mac and a one-time link to a
/// configuration profile that carries it (for a QR code or a direct download).
pub async fn apple_profile(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AppleProfileRequest>,
) -> ApiResult<Response> {
    let creds = creds_from(&app, &headers)?;
    let device = req.device.trim();
    if device.is_empty() || device.chars().count() > 40 {
        return Err(
            ApiError::bad_request("Name the device, like “iPhone” (up to 40 characters).")
                .field("device"),
        );
    }
    if app.throttled(
        &format!("profile:{}", creds.email),
        10,
        Duration::from_secs(3600),
    ) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "That's a lot of profiles in an hour. Wait a little and try again.",
        ));
    }
    let cfg = app
        .config()
        .ok_or_else(|| ApiError::conflict("The server is not configured yet."))?;
    let engine = app.engine()?;
    let description = format!("{device} (profile)");
    let (p, secret) = engine
        .user_create_app_password(&creds, &description)
        .await
        .map_err(map_user_err)?;
    let secret = Zeroizing::new(secret);
    let display_name = engine
        .list_accounts()
        .await
        .ok()
        .and_then(|l| l.into_iter().find(|a| a.address == creds.email))
        .and_then(|a| a.display_name);
    let body = Zeroizing::new(crate::edge::mobileconfig::render(
        &crate::edge::mobileconfig::Profile {
            hostname: &cfg.hostname,
            address: &creds.email,
            display_name: display_name.as_deref(),
            password: &secret,
            device,
            dav: req.calendars,
        },
    ));
    let token = random_token(24);
    {
        let mut profiles = app.profiles.lock().unwrap();
        profiles.retain(|_, p| p.expires > Instant::now());
        profiles.insert(
            sha256_hex(token.as_bytes()),
            PendingProfile {
                body,
                filename: crate::edge::mobileconfig::filename(&creds.email),
                expires: Instant::now() + PROFILE_TTL,
            },
        );
    }
    app.db.audit(
        &creds.email,
        "portal.device_profile",
        Some(&creds.email),
        Some(&description),
    )?;
    let expires_at = (chrono::Utc::now() + chrono::Duration::from_std(PROFILE_TTL).unwrap())
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "url": format!("{}/account/profile/{token}", public_base(&app, &headers)),
            "expiresAt": expires_at,
            "appPassword": { "id": p.id, "description": p.description, "createdAt": p.created_at },
        })),
    )
        .into_response())
}

/// Hands out a pending profile once; `None` when the link is unknown, used or expired.
pub fn take_profile(app: &AppState, token: &str) -> Option<PendingProfile> {
    let mut profiles = app.profiles.lock().unwrap();
    profiles.retain(|_, p| p.expires > Instant::now());
    profiles.remove(&sha256_hex(token.as_bytes()))
}
