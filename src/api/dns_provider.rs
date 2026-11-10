//! DNS automation for one domain (`/domains/{id}/dns/provider|plan|publish`).
//! The provider token is write-only: responses carry `hasSecret` only.

use super::{AdminAuth, ApiError, ApiResult, AppState};
use crate::dns::provider::{self, ProviderError, ProviderInfo};
use crate::dns_publish::{
    self, Preview, PublishError, PublishOutcome, PublishRequest, PublishSummary,
};
use crate::engine::EDomain;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

impl From<PublishError> for ApiError {
    fn from(e: PublishError) -> Self {
        let msg = e.to_string();
        match e {
            PublishError::NotConnected(_) => ApiError::conflict(msg),
            PublishError::Stale => ApiError::new(StatusCode::CONFLICT, "plan_changed", msg),
            PublishError::UnknownProvider(_) => ApiError::bad_request(msg).field("provider"),
            PublishError::TokenRequired => ApiError::bad_request(msg).field("token"),
            PublishError::Provider(p) => match p {
                ProviderError::Auth(_) => {
                    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "provider_auth", msg)
                        .field("token")
                }
                ProviderError::Permission(_) => {
                    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "provider_permission", msg)
                        .field("token")
                }
                ProviderError::NoZone(_) => {
                    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "provider_zone", msg)
                        .field("token")
                }
                ProviderError::Network(_) => {
                    ApiError::new(StatusCode::BAD_GATEWAY, "provider_unreachable", msg)
                }
                ProviderError::Rejected(_) => {
                    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "provider_rejected", msg)
                }
            },
            PublishError::Other(e) => e.into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connected {
    provider: String,
    label: &'static str,
    zone_name: String,
    has_secret: bool,
    connected_at: String,
    connected_by: String,
    last_publish: Option<PublishSummary>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    domain: String,
    connected: Option<Connected>,
    providers: &'static [ProviderInfo],
}

async fn find(app: &AppState, id: &str) -> ApiResult<EDomain> {
    app.engine()?
        .list_domains()
        .await?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ApiError::not_found("That domain"))
}

fn status(app: &AppState, domain: &str) -> ApiResult<ProviderStatus> {
    let connected = dns_publish::link(app, domain)?.map(|l| Connected {
        label: provider::info(&l.provider)
            .map(|p| p.label)
            .unwrap_or("DNS provider"),
        has_secret: dns_publish::has_token(app, domain),
        provider: l.provider,
        zone_name: l.zone_name,
        connected_at: l.connected_at,
        connected_by: l.connected_by,
        last_publish: l.last_publish,
    });
    Ok(ProviderStatus {
        domain: domain.into(),
        connected,
        providers: provider::PROVIDERS,
    })
}

fn throttle(app: &AppState, key: String, max: usize) -> ApiResult<()> {
    if app.throttled(&key, max, std::time::Duration::from_secs(300)) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts in a short time — wait a few minutes.",
        ));
    }
    Ok(())
}

pub async fn get_provider(
    State(app): State<AppState>,
    _a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<Json<ProviderStatus>> {
    let d = find(&app, &id).await?;
    Ok(Json(status(&app, &d.name)?))
}

#[derive(Deserialize)]
pub struct ConnectIn {
    provider: String,
    /// Leave empty to re-check the stored token.
    #[serde(default)]
    token: Option<String>,
}

pub async fn put_provider(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
    Json(req): Json<ConnectIn>,
) -> ApiResult<Json<ProviderStatus>> {
    let d = find(&app, &id).await?;
    throttle(&app, format!("dnsconnect:{id}"), 10)?;
    dns_publish::connect(
        &app,
        &d.name,
        &req.provider,
        req.token.as_deref(),
        &a.admin.email,
    )
    .await?;
    Ok(Json(status(&app, &d.name)?))
}

pub async fn delete_provider(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<Json<ProviderStatus>> {
    let d = find(&app, &id).await?;
    dns_publish::disconnect(&app, &d.name, &a.admin.email)?;
    Ok(Json(status(&app, &d.name)?))
}

pub async fn plan(
    State(app): State<AppState>,
    _a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<Json<Preview>> {
    let d = find(&app, &id).await?;
    throttle(&app, format!("dnsplan:{id}"), 30)?;
    Ok(Json(dns_publish::preview(&app, &d).await?))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PublishIn {
    #[serde(default)]
    only: Option<Vec<String>>,
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

pub async fn publish(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
    Json(req): Json<PublishIn>,
) -> ApiResult<Json<PublishOutcome>> {
    let d = find(&app, &id).await?;
    throttle(&app, format!("dnspublish:{id}"), 10)?;
    let out = dns_publish::publish(
        &app,
        &d,
        PublishRequest {
            only: req.only.as_deref(),
            plan_id: req.plan_id.as_deref(),
            dry_run: req.dry_run,
            check: true,
        },
        &a.admin.email,
    )
    .await?;
    Ok(Json(out))
}
