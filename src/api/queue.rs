//! Outgoing queue (`/queue…`) and login-abuse controls (`/security/…`).
//! Every mutation is audit-logged by `mailqueue` / `ipaccess`.

use super::{AdminAuth, ApiError, ApiResult, AppState};
use crate::{ipaccess, mailqueue};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

pub async fn get_queue(
    State(app): State<AppState>,
    _a: AdminAuth,
) -> ApiResult<Json<mailqueue::QueueView>> {
    Ok(Json(mailqueue::view(&app).await?))
}

#[derive(Deserialize)]
pub struct RetryBody {
    /// One message; absent: every queued message.
    #[serde(default)]
    id: Option<String>,
}

pub async fn retry(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(b): Json<RetryBody>,
) -> ApiResult<Json<Value>> {
    let n = mailqueue::retry(&app, &a.admin.email, b.id.as_deref())
        .await
        .map_err(|e| not_found_as(e, "That message"))?;
    Ok(Json(json!({ "retried": n })))
}

pub async fn cancel(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    mailqueue::cancel(&app, &a.admin.email, &id)
        .await
        .map_err(|e| not_found_as(e, "That message"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pause(
    State(app): State<AppState>,
    a: AdminAuth,
) -> ApiResult<Json<mailqueue::QueueView>> {
    mailqueue::set_paused(&app, &a.admin.email, true).await?;
    Ok(Json(mailqueue::view(&app).await?))
}

pub async fn resume(
    State(app): State<AppState>,
    a: AdminAuth,
) -> ApiResult<Json<mailqueue::QueueView>> {
    mailqueue::set_paused(&app, &a.admin.email, false).await?;
    Ok(Json(mailqueue::view(&app).await?))
}

pub async fn get_access(
    State(app): State<AppState>,
    _a: AdminAuth,
) -> ApiResult<Json<ipaccess::AccessView>> {
    Ok(Json(ipaccess::view(&app).await?))
}

pub async fn unblock(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    ipaccess::unblock(&app, &a.admin.email, &id)
        .await
        .map_err(|e| not_found_as(e, "That blocked address"))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AllowBody {
    address: String,
    #[serde(default)]
    note: Option<String>,
}

pub async fn allow(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(b): Json<AllowBody>,
) -> ApiResult<(StatusCode, Json<ipaccess::AllowedView>)> {
    match ipaccess::allow(&app, &a.admin.email, &b.address, b.note.as_deref()).await {
        Ok(v) => Ok((StatusCode::CREATED, Json(v))),
        Err(ipaccess::AllowError::Invalid(m)) => {
            Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_address", m).field("address"))
        }
        Err(e @ ipaccess::AllowError::Duplicate(_)) => {
            Err(ApiError::conflict(e.to_string()).field("address"))
        }
        Err(ipaccess::AllowError::Other(e)) => Err(e.into()),
    }
}

pub async fn disallow(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    ipaccess::disallow(&app, &a.admin.email, &id)
        .await
        .map_err(|e| not_found_as(e, "That allowlist entry"))?;
    Ok(StatusCode::NO_CONTENT)
}

/// A friendlier "not found" naming the thing that was asked for.
fn not_found_as(e: anyhow::Error, what: &str) -> ApiError {
    if matches!(
        e.downcast_ref::<crate::engine::EngineError>(),
        Some(crate::engine::EngineError::NotFound)
    ) {
        return ApiError::not_found(what);
    }
    e.into()
}
