//! The browser onboarding (after the owner and their second factor exist):
//! everything the terminal no longer asks. The wizard reads one state,
//! submits answers, and asks for re-checks; every gate is still decided by the
//! installer's steps against public DNS and the CA, never by the browser.

use super::admin::ensure_host_checks;
use super::{AdminAuth, ApiError, ApiResult, AppState};
use crate::installer::panel::{self, PanelKind};
use crate::installer::state::{self, StepStatus};
use crate::installer::{InstallInputs, KV_INPUTS, KV_SKIP_PTR};
use crate::model::{Check, SendingMode, Status};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

/// Steps whose verdict depends on the mail domain or hostname: answered
/// again, they are checked again.
const NAME_GATES: &[&str] = &["host_record", "certificate", "dns", "ptr"];

pub(super) async fn state_json(app: &AppState, owner_email: &str) -> ApiResult<Value> {
    let cfg = app
        .config()
        .ok_or_else(|| ApiError::conflict("The server is not configured yet."))?;
    let install = state::load(&app.db)?;
    let inputs = app.db.get_json::<InstallInputs>(KV_INPUTS)?;
    let choice = app.db.get_json::<panel::PanelChoice>(panel::KV)?;
    let console_host = cfg.console_host().to_string();
    let kind = panel::kind(&console_host);

    // Suggestions: the name the person first typed in the terminal (even if it
    // did not point here yet), else the console's own domain name.
    let from = choice
        .as_ref()
        .and_then(|c| c.requested.clone())
        .or_else(|| (kind == PanelKind::Domain).then(|| console_host.clone()));
    let suggested_domain = from.as_deref().and_then(panel::suggested_domain);
    let suggested_hostname = suggested_domain
        .as_deref()
        .map(|d| panel::suggested_hostname(d, from.as_deref()));
    let owner_ok = suggested_domain
        .as_deref()
        .is_none_or(|d| !owner_email.ends_with(&format!("@{d}")));

    let host = ensure_host_checks(app);
    let port25 = host.as_ref().map(|h| h.port25.clone()).unwrap_or_else(|| {
        Check::new(
            "port25",
            "Outbound port 25",
            Status::Unknown,
            "Not checked yet — the server runs this check in the background.",
        )
    });
    let suggested_sending = if port25.status == Status::Pass || port25.status == Status::Unknown {
        SendingMode::Direct
    } else {
        SendingMode::Relay
    };
    let primary_domain_id = match app.engine() {
        Ok(e) => e
            .list_domains()
            .await
            .ok()
            .and_then(|ds| ds.into_iter().find(|d| d.name == cfg.primary_domain))
            .map(|d| d.id),
        Err(_) => None,
    };
    let identity = inputs.as_ref().map(
        |i| json!({ "domain": i.domain, "hostname": i.hostname, "adminEmail": i.admin_email }),
    );
    Ok(json!({
        "complete": cfg.install_complete,
        "terminalDone": app.db.terminal_done()?,
        "steps": install.steps,
        "logs": install.logs,
        "panel": {
            "host": panel::url_host(&console_host),
            "kind": kind,
            "url": panel::console_url(&console_host),
        },
        "identity": identity,
        "suggested": {
            "domain": suggested_domain,
            "hostname": suggested_hostname,
            "adminEmail": owner_ok.then_some(owner_email),
        },
        "sending": {
            "confirmed": inputs.as_ref().is_some_and(|i| i.sending_confirmed),
            "mode": cfg.sending_mode,
            "suggested": suggested_sending,
            "port25": port25,
        },
        "server": { "ipv4": cfg.public_ipv4, "ipv6": cfg.public_ipv6 },
        "mailHostname": cfg.hostname,
        "primaryDomain": cfg.primary_domain,
        "primaryDomainId": primary_domain_id,
        "ptrSkipped": app.db.get_kv(KV_SKIP_PTR)?.is_some_and(|v| v == "true"),
    }))
}

pub async fn get_state(State(app): State<AppState>, a: AdminAuth) -> ApiResult<Json<Value>> {
    Ok(Json(state_json(&app, &a.admin.email).await?))
}

fn not_after_setup(app: &AppState) -> ApiResult<()> {
    if app.config().is_some_and(|c| c.install_complete) {
        return Err(ApiError::conflict(
            "Setup is already complete; change this under Domains, Sending or System.",
        ));
    }
    Ok(())
}

fn terminal_first(app: &AppState) -> ApiResult<()> {
    if !app.db.terminal_done()? {
        return Err(ApiError::conflict(
            "The terminal part of the install has not finished: run `sudo yunori-mail install` on the server.",
        ));
    }
    Ok(())
}

/// Run the onboarding's steps in the background (the answer is already saved).
fn advance_soon(app: &AppState) {
    let bg = app.clone();
    tokio::spawn(async move { crate::installer::advance_gates(bg).await });
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityIn {
    domain: String,
    hostname: String,
    admin_email: String,
}

/// The mail domain, the mail hostname and an outside address for
/// certificates and alerts. Applied to the engine at once (the placeholder
/// domain it started with is renamed; its hostname set).
pub async fn put_identity(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(req): Json<IdentityIn>,
) -> ApiResult<Json<Value>> {
    not_after_setup(&app)?;
    terminal_first(&app)?;
    let (domain, hostname, admin_email) =
        crate::installer::check_identity(&req.domain, &req.hostname, &req.admin_email)
            .map_err(|e| ApiError::bad_request(e.message).field(e.field))?;
    let cfg = app
        .config()
        .ok_or_else(|| ApiError::conflict("The server is not configured yet."))?;
    let previous = app.db.get_json::<InstallInputs>(KV_INPUTS)?;
    let inputs = match &previous {
        Some(p) => InstallInputs {
            domain: domain.clone(),
            hostname: hostname.clone(),
            admin_email: admin_email.clone(),
            ..p.clone()
        },
        None => InstallInputs {
            domain: domain.clone(),
            hostname: hostname.clone(),
            admin_email: admin_email.clone(),
            first_mailbox: String::new(),
            sending: cfg.sending_mode,
            relay: None,
            client_origin: cfg.client_origin.clone(),
            sending_confirmed: false,
        },
    };
    {
        // One installer run at a time: the 60-second check may be mid-step.
        let _turn = app.install_run.lock().await;
        let detail = crate::installer::apply_identity(&app, &inputs, &a.admin.email)
            .await
            .map_err(|e| match e.downcast_ref::<crate::engine::EngineError>() {
                Some(_) => ApiError::from(e),
                None => ApiError::bad_request(format!("{e:#}")).field("domain"),
            })?;
        app.db.set_json(KV_INPUTS, &inputs)?;
        app.db.init_steps()?;
        app.db.set_step("inputs", StepStatus::Done, Some(&detail))?;
        let renamed = previous
            .as_ref()
            .is_some_and(|p| p.domain != domain || p.hostname != hostname);
        if renamed {
            for id in NAME_GATES {
                app.db.set_step(id, StepStatus::Pending, None)?;
            }
        }
        app.db.install_log("info", Some("inputs"), &detail)?;
    }
    app.db.audit(
        &a.admin.email,
        "onboarding.identity",
        Some(&domain),
        Some(&format!("hostname {hostname}")),
    )?;
    advance_soon(&app);
    Ok(Json(state_json(&app, &a.admin.email).await?))
}

/// "Check now": run the onboarding's gates and answer with the new state.
pub async fn check(State(app): State<AppState>, a: AdminAuth) -> ApiResult<Json<Value>> {
    if app.throttled(
        &format!("onboarding-check:{}", a.admin.id),
        20,
        std::time::Duration::from_secs(60),
    ) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Checked many times just now. The server keeps checking by itself every minute.",
        ));
    }
    crate::installer::advance_gates(app.clone()).await;
    Ok(Json(state_json(&app, &a.admin.email).await?))
}

#[derive(Deserialize)]
pub struct SkipIn {
    skip: bool,
}

/// Reverse DNS "Skip for now" (what `--skip-ptr` did in the terminal), or
/// undo it. Direct sending then likely lands in spam or is refused; the
/// overview keeps checking reverse DNS either way.
pub async fn skip_ptr(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(req): Json<SkipIn>,
) -> ApiResult<Json<Value>> {
    not_after_setup(&app)?;
    app.db
        .set_kv(KV_SKIP_PTR, if req.skip { "true" } else { "false" })?;
    if !req.skip && app.db.step_status("ptr")? == StepStatus::Skipped {
        app.db.set_step("ptr", StepStatus::Pending, None)?;
    }
    app.db.audit(
        &a.admin.email,
        if req.skip {
            "onboarding.ptr_skipped"
        } else {
            "onboarding.ptr_unskipped"
        },
        None,
        None,
    )?;
    crate::installer::advance_gates(app.clone()).await;
    Ok(Json(state_json(&app, &a.admin.email).await?))
}

/// Called by `PUT /sending` while setup is not complete: the sending step is
/// answered.
pub(super) fn sending_answered(app: &AppState, mode: SendingMode) -> ApiResult<()> {
    if app.config().is_none_or(|c| c.install_complete) {
        return Ok(());
    }
    if let Some(mut i) = app.db.get_json::<InstallInputs>(KV_INPUTS)? {
        i.sending = mode;
        i.sending_confirmed = true;
        app.db.set_json(KV_INPUTS, &i)?;
        if app.db.step_status("sending")? == StepStatus::Done {
            // Relay vs direct changes what reverse DNS needs.
            app.db.set_step("ptr", StepStatus::Pending, None)?;
        }
        app.db.set_step("sending", StepStatus::Pending, None)?;
        advance_soon(app);
    }
    Ok(())
}
