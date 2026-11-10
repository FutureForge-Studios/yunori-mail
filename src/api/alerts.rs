//! Alerts settings, test send, history (console → Alerts). See `crate::alerts`.

use super::{AdminAuth, ApiError, ApiResult, AppState};
use crate::alerts::channels::{
    self, SECRET_WEBHOOK_TOKEN, SECRET_WEBHOOK_URL, SENDER_LOCAL, Settings, WebhookKind,
};
use crate::alerts::{Condition, Kind, Notice, Severity};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

fn view(app: &AppState) -> ApiResult<Value> {
    let s = channels::settings(app);
    let cfg = app.config();
    let default_recipient = cfg.as_ref().map(|c| c.admin_email.clone());
    Ok(json!({
        "active": crate::alerts::current(app),
        "history": crate::alerts::history(app, 100)?,
        "evaluatedAt": crate::alerts::evaluated_at(app),
        "settings": {
            "emailEnabled": s.email_enabled,
            "recipient": channels::recipient(app, &s),
            "recipientIsDefault": s.recipient.is_none(),
            "defaultRecipient": default_recipient,
            "sender": cfg.as_ref().map(|c| format!("{SENDER_LOCAL}@{}", c.primary_domain)),
            "webhook": {
                "enabled": s.webhook_enabled,
                "kind": s.webhook_kind,
                "host": s.webhook_host,
                "hasUrl": app.db.has_secret(SECRET_WEBHOOK_URL)?,
                "hasToken": app.db.has_secret(SECRET_WEBHOOK_TOKEN)?,
            },
        },
    }))
}

pub async fn get_alerts(State(app): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    Ok(Json(view(&app)?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookIn {
    enabled: bool,
    #[serde(default)]
    kind: WebhookKind,
    /// Omitted = keep the saved URL; "" = remove it.
    url: Option<String>,
    /// Omitted = keep the saved token (only while the URL stays the same); "" = remove it.
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsIn {
    email_enabled: bool,
    /// Empty or null = the install's admin address.
    recipient: Option<String>,
    webhook: WebhookIn,
}

pub async fn put_settings(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(req): Json<SettingsIn>,
) -> ApiResult<Json<Value>> {
    let previous = channels::settings(&app);
    let recipient = req
        .recipient
        .map(|r| r.trim().to_ascii_lowercase())
        .filter(|r| !r.is_empty());
    if let Some(r) = &recipient {
        if !crate::util::valid_email(r) {
            return Err(ApiError::bad_request("Enter a valid email address.").field("recipient"));
        }
        let hosted = match app.engine() {
            Ok(e) => e.list_domains().await.unwrap_or_default(),
            Err(_) => vec![],
        };
        let domain = r.rsplit_once('@').map(|(_, d)| d).unwrap_or_default();
        if hosted.iter().any(|d| d.name.eq_ignore_ascii_case(domain)) {
            return Err(ApiError::bad_request(
                "Use an address outside this server: if this server has a problem, mail to it may not arrive.",
            )
            .field("recipient"));
        }
    }

    // The webhook URL and token are write-only; a new URL never inherits the old token.
    let mut host = previous.webhook_host.clone();
    let mut url_changed = false;
    match req.webhook.url.as_deref().map(str::trim) {
        None => {}
        Some("") => {
            app.db.delete_secret(SECRET_WEBHOOK_URL)?;
            app.db.delete_secret(SECRET_WEBHOOK_TOKEN)?;
            host = None;
            url_changed = true;
        }
        Some(raw) => {
            let u = channels::check_webhook_url(raw)
                .map_err(|m| ApiError::bad_request(m).field("webhookUrl"))?;
            let same = app
                .db
                .get_secret(SECRET_WEBHOOK_URL)?
                .is_some_and(|old| old.as_str() == u.as_str());
            if !same {
                app.db.put_secret(SECRET_WEBHOOK_URL, u.as_str())?;
                url_changed = true;
                if req.webhook.token.is_none() {
                    app.db.delete_secret(SECRET_WEBHOOK_TOKEN)?;
                }
            }
            host = u.host_str().map(str::to_string);
        }
    }
    match req.webhook.token.as_deref().map(str::trim) {
        None => {}
        Some("") => app.db.delete_secret(SECRET_WEBHOOK_TOKEN)?,
        Some(t) => {
            if t.len() > 4096 || t.chars().any(|c| c.is_control()) {
                return Err(ApiError::bad_request("That token is not valid.").field("webhookToken"));
            }
            app.db.put_secret(SECRET_WEBHOOK_TOKEN, t)?;
        }
    }
    if req.webhook.enabled && !app.db.has_secret(SECRET_WEBHOOK_URL)? {
        return Err(ApiError::bad_request("Enter the URL to send alerts to.").field("webhookUrl"));
    }

    let next = Settings {
        email_enabled: req.email_enabled,
        recipient: recipient.clone(),
        webhook_enabled: req.webhook.enabled,
        webhook_kind: req.webhook.kind,
        webhook_host: host.clone(),
    };
    app.db.set_json(channels::KV_SETTINGS, &next)?;
    let detail = format!(
        "email {}{}; {} {}{}{}",
        if next.email_enabled { "on" } else { "off" },
        recipient
            .as_deref()
            .map(|r| format!(" to {r}"))
            .unwrap_or_else(|| " to the admin address".into()),
        match next.webhook_kind {
            WebhookKind::Json => "webhook",
            WebhookKind::Ntfy => "ntfy",
        },
        if next.webhook_enabled { "on" } else { "off" },
        host.as_deref()
            .map(|h| format!(" ({h})"))
            .unwrap_or_default(),
        match (url_changed, req.webhook.token.as_deref()) {
            (_, Some("")) => "; token removed",
            (_, Some(_)) => "; token replaced",
            (true, None) => "; new URL",
            _ => "",
        }
    );
    app.db
        .audit(&a.admin.email, "alerts.settings", None, Some(&detail))?;
    Ok(Json(view(&app)?))
}

/// "Send test alert": one message through every channel that is on.
pub async fn test_alert(State(app): State<AppState>, a: AdminAuth) -> ApiResult<Json<Value>> {
    let s = channels::settings(&app);
    if !s.email_enabled && !s.webhook_enabled {
        return Err(ApiError::conflict(
            "Turn on email or a webhook first, then send a test.",
        ));
    }
    if app.throttled(
        &format!("alerts-test:{}", a.admin.id),
        5,
        Duration::from_secs(600),
    ) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Several tests were sent already. Wait a few minutes before the next one.",
        ));
    }
    let host = app.config().map(|c| c.hostname).unwrap_or_default();
    let n = Notice {
        kind: Kind::Test,
        condition: Condition {
            id: "test".into(),
            severity: Severity::Warning,
            title: "Test alert".into(),
            summary: format!("This is a test alert from Yunori on {host}, sent from the console."),
            remedy: "Nothing to do.".into(),
        },
        since: chrono::Utc::now(),
    };
    let deliveries = crate::alerts::notify(&app, &a.admin.email, &n).await;
    Ok(Json(json!({ "deliveries": deliveries })))
}

/// "Check now": evaluate immediately (normally every 10 minutes).
pub async fn check_now(State(app): State<AppState>, a: AdminAuth) -> ApiResult<Json<Value>> {
    if app.throttled(
        &format!("alerts-check:{}", a.admin.id),
        10,
        Duration::from_secs(600),
    ) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Checked several times already. Yunori checks every 10 minutes by itself.",
        ));
    }
    let notices = crate::alerts::evaluate(&app, Duration::from_secs(3)).await;
    app.db.audit(
        &a.admin.email,
        "alerts.check",
        None,
        Some(&format!("{} change(s)", notices.len())),
    )?;
    Ok(Json(view(&app)?))
}
