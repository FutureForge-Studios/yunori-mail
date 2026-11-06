//! Console endpoints (admin session required).

use super::{AdminAuth, ApiError, ApiResult, AppState, public_base};
use crate::engine::{EAccount, EDomain, RelayConfig, alias_addresses};
use crate::model::{Check, RELAY_PROVIDERS, SendingMode, Status, relay_provider};
use crate::ops;
use crate::util::{generate_password, normalize_domain, valid_email, valid_local_part};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ── shapes ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainOut {
    id: String,
    name: String,
    enabled: bool,
    is_primary: bool,
    catch_all: Option<String>,
    mailbox_count: usize,
    alias_count: usize,
    dns_status: Status,
    dns_checked_at: Option<String>,
    created_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxOut {
    id: String,
    address: String,
    local: String,
    domain_id: String,
    display_name: Option<String>,
    role: &'static str,
    quota_bytes: Option<u64>,
    used_bytes: Option<u64>,
    aliases: Vec<String>,
    created_at: Option<String>,
    /// Expiry of an unused invite link, while one is out.
    invite_expires_at: Option<String>,
}

fn domain_out(app: &AppState, d: &EDomain, accounts: &[EAccount], primary: &str) -> DomainOut {
    let (dns_status, dns_checked_at) = ops::cached_dns_status(app, &d.name);
    DomainOut {
        id: d.id.clone(),
        name: d.name.clone(),
        enabled: d.enabled,
        is_primary: d.name == primary,
        catch_all: d.catch_all.clone(),
        mailbox_count: accounts.iter().filter(|a| a.domain_id == d.id).count(),
        alias_count: accounts
            .iter()
            .flat_map(|a| a.aliases.iter())
            .filter(|al| al.domain_id == d.id)
            .count(),
        dns_status,
        dns_checked_at,
        created_at: d.created_at.clone(),
    }
}

fn mailbox_out(a: &EAccount, domains: &[EDomain]) -> MailboxOut {
    MailboxOut {
        id: a.id.clone(),
        address: a.address.clone(),
        local: a.local.clone(),
        domain_id: a.domain_id.clone(),
        display_name: a.display_name.clone(),
        role: if a.is_admin { "admin" } else { "user" },
        quota_bytes: a.quota_bytes,
        used_bytes: a.used_bytes,
        aliases: alias_addresses(a, domains),
        created_at: a.created_at.clone(),
        invite_expires_at: None,
    }
}

/// `https://<host>/account/welcome#invite=<token>`: the token rides the
/// fragment, so it never reaches a server log or a `Referer`.
fn invite_link(app: &AppState, headers: &HeaderMap, token: &str) -> String {
    format!(
        "{}/account/welcome#invite={token}",
        public_base(app, headers)
    )
}

fn primary(app: &AppState) -> String {
    app.config().map(|c| c.primary_domain).unwrap_or_default()
}

fn unknown(id: &str, label: &str) -> Check {
    Check::new(
        id,
        label,
        Status::Unknown,
        "Not checked yet — the server runs this check in the background.",
    )
}

/// Kick off a background refresh of host checks if none are cached.
fn ensure_host_checks(app: &AppState) -> Option<ops::HostChecks> {
    let cached = ops::cached_host_checks(app);
    if cached.is_none() && !app.dev {
        let app = app.clone();
        tokio::spawn(async move {
            ops::host_checks(&app, true).await;
        });
    }
    cached
}

// ── overview & install ───────────────────────────────────────────────────

pub async fn overview(State(app): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let cfg = app.config();
    let engine = app.engine().ok();
    let (engine_ok, engine_check) = ops::engine_check(
        engine.clone(),
        cfg.as_ref().and_then(|c| c.stalwart_version.clone()),
    )
    .await;
    let mut stats = json!({ "domains": 0, "mailboxes": 0, "aliases": 0, "queued": 0 });
    let mut primary_dns = Status::Unknown;
    let mut dns_signal = unknown("dns", "DNS records");
    if let (Some(e), true) = (&engine, engine_ok) {
        let domains = e.list_domains().await.unwrap_or_default();
        let accounts = e.list_accounts().await.unwrap_or_default();
        let queued = e.queue().await.map(|q| q.len()).unwrap_or(0);
        stats = json!({
            "domains": domains.len(),
            "mailboxes": accounts.len(),
            "aliases": accounts.iter().map(|a| a.aliases.len()).sum::<usize>(),
            "queued": queued,
        });
        if let Some(c) = &cfg {
            let (s, at) = ops::cached_dns_status(&app, &c.primary_domain);
            primary_dns = s;
            dns_signal = Check::new(
                "dns",
                "DNS records",
                s,
                match s {
                    Status::Pass => format!(
                        "All required records for {} are published.",
                        c.primary_domain
                    ),
                    Status::Fail => format!(
                        "Some required records for {} are missing or wrong.",
                        c.primary_domain
                    ),
                    Status::Warn => format!(
                        "Required records for {} are published; some optional ones are not.",
                        c.primary_domain
                    ),
                    _ => match at {
                        Some(_) => "Could not verify DNS on the last check.".into(),
                        None => "DNS has not been checked yet.".into(),
                    },
                },
            );
            if s == Status::Fail {
                dns_signal =
                    dns_signal.remedy("Open Domains and publish the records marked as problems.");
            }
        }
    }
    let host = ensure_host_checks(&app);
    let port25 = host
        .as_ref()
        .map(|h| h.port25.clone())
        .unwrap_or_else(|| unknown("port25", "Outbound port 25"));
    let ptr = host
        .as_ref()
        .map(|h| h.ptr.clone())
        .unwrap_or_else(|| unknown("ptr", "Reverse DNS (PTR)"));
    let listed = host
        .as_ref()
        .is_some_and(|h| h.blocklists.iter().any(|b| b.status == Status::Fail));
    let mode = cfg
        .as_ref()
        .map(|c| c.sending_mode)
        .unwrap_or(SendingMode::Direct);
    let relay_label = ops::relay_settings(&app)
        .and_then(|r| relay_provider(&r.provider).map(|p| p.label.to_string()));

    let install = crate::installer::state::load(&app.db)?;
    let install_complete = cfg.as_ref().is_some_and(|c| c.install_complete);
    let waiting = install.waiting_detail();
    let verdict = ops::verdict(&ops::VerdictInputs {
        engine_ok,
        install_complete,
        waiting_detail: waiting.as_deref(),
        primary_dns,
        port25: port25.status,
        ptr: ptr.status,
        listed,
        mode,
        relay_label: relay_label.as_deref(),
    });

    let mut signals = vec![engine_check, dns_signal, port25, ptr];
    if let Some(h) = &host {
        let worst = h
            .blocklists
            .iter()
            .map(|b| b.status)
            .fold(Status::Pass, Status::worst);
        let detail = if listed {
            "This server's IP is listed on at least one blocklist."
        } else if worst == Status::Unknown {
            "Some blocklists could not be queried."
        } else {
            "This server's IP is not on the major blocklists."
        };
        signals.push(Check::new("blocklists", "Blocklists", worst, detail));
    }
    signals.push(crate::edge::certs::check(&app));

    Ok(Json(json!({
        "verdict": verdict,
        "stats": stats,
        "sendingMode": mode,
        "signals": signals,
        "setup": if install_complete { Value::Null } else { serde_json::to_value(&install).unwrap_or(Value::Null) },
    })))
}

pub async fn install_state(State(app): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    Ok(Json(
        serde_json::to_value(crate::installer::state::load(&app.db)?).unwrap_or(Value::Null),
    ))
}

pub async fn preflight(State(app): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Vec<Check>>> {
    let hostname = app.config().map(|c| c.hostname);
    Ok(Json(
        crate::checks::preflight(&app.paths, hostname.as_deref()).await,
    ))
}

// ── certificate (automatic HTTPS) ────────────────────────────────────────

pub async fn certificate(
    State(app): State<AppState>,
    _a: AdminAuth,
) -> ApiResult<Json<crate::edge::certs::CertStatus>> {
    Ok(Json(crate::edge::certs::status(&app)))
}

/// "Try now": request a certificate immediately (ignoring the backoff, never
/// the staging rehearsal). Runs in the background; poll GET /certificate.
pub async fn certificate_renew(State(app): State<AppState>, a: AdminAuth) -> ApiResult<Response> {
    if app.dev {
        return Err(ApiError::conflict(
            "Development mode does not request certificates.",
        ));
    }
    if app.throttled(
        &format!("cert-renew:{}", a.admin.id),
        6,
        std::time::Duration::from_secs(900),
    ) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "That was tried several times already. Yunori keeps retrying by itself; give it a few minutes.",
        ));
    }
    app.db.audit(
        &a.admin.email,
        "certificate.renew_requested",
        app.config().map(|c| c.hostname).as_deref(),
        None,
    )?;
    let bg = app.clone();
    tokio::spawn(async move {
        crate::edge::certs::ensure(&bg, true).await;
        crate::installer::advance_gates(bg).await;
    });
    // Give the task a moment to take the lock so the response reads "in progress".
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    Ok((StatusCode::ACCEPTED, Json(crate::edge::certs::status(&app))).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstMailbox {
    local: String,
    display_name: Option<String>,
}

/// The onboarding checklist's "Create your first mailbox": the same mailbox
/// the terminal installer would create (with postmaster@ and abuse@).
pub async fn install_first_mailbox(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(req): Json<FirstMailbox>,
) -> ApiResult<Response> {
    let cfg = app
        .config()
        .ok_or_else(|| ApiError::conflict("The server is not configured yet."))?;
    if cfg.install_complete {
        return Err(ApiError::conflict(
            "Setup is already complete; create mailboxes under Mailboxes.",
        ));
    }
    let local = req.local.trim().to_ascii_lowercase();
    if !valid_local_part(&local) {
        return Err(
            ApiError::bad_request("Use letters, digits, dots, dashes or underscores.")
                .field("local"),
        );
    }
    let e = app.engine()?;
    let accounts = e.list_accounts().await?;
    if let Some(existing) = accounts.iter().find(|x| !x.is_admin) {
        return Err(ApiError::conflict(format!(
            "{} already exists, so this step is done.",
            existing.address
        )));
    }
    let password = generate_password(20);
    let display = req
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let acct = ops::create_first_mailbox(&app, &local, display, &password, &a.admin.email)
        .await
        .map_err(
            |err| match err.downcast_ref::<crate::engine::EngineError>() {
                Some(_) => ApiError::from(err),
                None => ApiError::bad_request(format!("{err}")).field("local"),
            },
        )?;
    let domains = e.list_domains().await?;
    let bg = app.clone();
    tokio::spawn(async move { crate::installer::advance_gates(bg).await });
    Ok((
        StatusCode::CREATED,
        Json(json!({ "mailbox": mailbox_out(&acct, &domains), "password": password })),
    )
        .into_response())
}

// ── domains ──────────────────────────────────────────────────────────────

pub async fn list_domains(
    State(app): State<AppState>,
    _a: AdminAuth,
) -> ApiResult<Json<Vec<DomainOut>>> {
    let e = app.engine()?;
    let (domains, accounts) = (e.list_domains().await?, e.list_accounts().await?);
    let p = primary(&app);
    Ok(Json(
        domains
            .iter()
            .map(|d| domain_out(&app, d, &accounts, &p))
            .collect(),
    ))
}

pub async fn get_domain(
    State(app): State<AppState>,
    _a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<Json<DomainOut>> {
    let e = app.engine()?;
    let (domains, accounts) = (e.list_domains().await?, e.list_accounts().await?);
    let d = domains
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ApiError::not_found("That domain"))?;
    Ok(Json(domain_out(&app, d, &accounts, &primary(&app))))
}

#[derive(Deserialize)]
pub struct CreateDomain {
    name: String,
}

pub async fn create_domain(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(req): Json<CreateDomain>,
) -> ApiResult<Response> {
    let name = normalize_domain(&req.name)
        .ok_or_else(|| ApiError::bad_request("Enter a domain like example.com.").field("name"))?;
    let e = app.engine()?;
    if e.list_domains().await?.iter().any(|d| d.name == name) {
        return Err(
            ApiError::conflict(format!("{name} is already set up on this server.")).field("name"),
        );
    }
    let d = e.create_domain(&name).await?;
    app.db.audit(
        &a.admin.email,
        "domain.create",
        Some(&name),
        Some("DKIM automatic, DNS manual"),
    )?;
    let bg = app.clone();
    let dc = d.clone();
    tokio::spawn(async move {
        // DKIM keys are generated asynchronously by the engine; check DNS shortly after.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let _ = ops::domain_dns(&bg, &dc, true).await;
    });
    let out = domain_out(&app, &d, &[], &primary(&app));
    Ok((StatusCode::CREATED, Json(out)).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDomain {
    #[serde(default, deserialize_with = "double_option")]
    catch_all: Option<Option<String>>,
    enabled: Option<bool>,
}

/// Distinguish "absent" (None) from "null" (Some(None)).
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

pub async fn update_domain(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
    Json(req): Json<UpdateDomain>,
) -> ApiResult<Json<DomainOut>> {
    let catch_all = match req.catch_all {
        Some(Some(addr)) if !addr.trim().is_empty() => {
            let addr = addr.trim().to_ascii_lowercase();
            if !valid_email(&addr) {
                return Err(
                    ApiError::bad_request("Enter a full email address for catch-all.")
                        .field("catchAll"),
                );
            }
            Some(Some(addr))
        }
        Some(_) => Some(None),
        None => None,
    };
    let e = app.engine()?;
    let d = e.update_domain(&id, catch_all.clone(), req.enabled).await?;
    app.db.audit(
        &a.admin.email,
        "domain.update",
        Some(&d.name),
        Some(&format!(
            "catchAll={:?} enabled={:?}",
            catch_all.map(|c| c.unwrap_or_else(|| "off".into())),
            req.enabled
        )),
    )?;
    let accounts = e.list_accounts().await?;
    Ok(Json(domain_out(&app, &d, &accounts, &primary(&app))))
}

pub async fn delete_domain(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let e = app.engine()?;
    let domains = e.list_domains().await?;
    let d = domains
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ApiError::not_found("That domain"))?;
    if d.name == primary(&app) {
        return Err(ApiError::conflict("The primary domain cannot be removed."));
    }
    let n = e
        .list_accounts()
        .await?
        .iter()
        .filter(|x| x.domain_id == id)
        .count();
    if n > 0 {
        return Err(ApiError::conflict(format!(
            "{} still has {n} mailbox{}. Remove {} first.",
            d.name,
            if n == 1 { "" } else { "es" },
            if n == 1 { "it" } else { "them" }
        )));
    }
    e.delete_domain(&id).await?;
    app.db
        .audit(&a.admin.email, "domain.delete", Some(&d.name), None)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn find_domain(app: &AppState, id: &str) -> ApiResult<EDomain> {
    app.engine()?
        .list_domains()
        .await?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ApiError::not_found("That domain"))
}

pub async fn domain_dns(
    State(app): State<AppState>,
    _a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<Json<ops::DomainDns>> {
    let d = find_domain(&app, &id).await?;
    Ok(Json(ops::domain_dns(&app, &d, false).await?))
}

pub async fn domain_dns_check(
    State(app): State<AppState>,
    _a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<Json<ops::DomainDns>> {
    let d = find_domain(&app, &id).await?;
    if app.throttled(
        &format!("dnscheck:{id}"),
        20,
        std::time::Duration::from_secs(300),
    ) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Checked very recently — wait a moment.",
        ));
    }
    Ok(Json(ops::domain_dns(&app, &d, true).await?))
}

// ── mailboxes ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxQuery {
    domain_id: Option<String>,
}

pub async fn list_mailboxes(
    State(app): State<AppState>,
    _a: AdminAuth,
    Query(q): Query<MailboxQuery>,
) -> ApiResult<Json<Vec<MailboxOut>>> {
    let e = app.engine()?;
    let (domains, accounts) = (e.list_domains().await?, e.list_accounts().await?);
    let invites = app.db.pending_invites()?;
    Ok(Json(
        accounts
            .iter()
            .filter(|a| q.domain_id.as_ref().is_none_or(|d| &a.domain_id == d))
            .map(|a| {
                let mut m = mailbox_out(a, &domains);
                m.invite_expires_at = invites
                    .iter()
                    .rev()
                    .find(|(id, _)| id == &a.id)
                    .map(|(_, exp)| exp.clone());
                m
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMailbox {
    local: String,
    domain_id: String,
    display_name: Option<String>,
    quota_mb: Option<u64>,
    password: Option<String>,
    /// Let the person choose their own password through an invite link
    /// instead of being handed one.
    #[serde(default)]
    invite: bool,
}

fn check_password(pw: &str) -> ApiResult<()> {
    if pw.chars().count() < 12 {
        return Err(ApiError::bad_request("Use at least 12 characters.").field("password"));
    }
    Ok(())
}

pub async fn create_mailbox(
    State(app): State<AppState>,
    a: AdminAuth,
    headers: HeaderMap,
    Json(req): Json<CreateMailbox>,
) -> ApiResult<Response> {
    let local = req.local.trim().to_ascii_lowercase();
    if !valid_local_part(&local) {
        return Err(
            ApiError::bad_request("Use letters, digits, dots, dashes or underscores.")
                .field("local"),
        );
    }
    let password = match req.password.filter(|p| !p.is_empty() && !req.invite) {
        Some(p) => {
            check_password(&p)?;
            p
        }
        // Invite mode: a long random password nobody sees until the person
        // replaces it with their own.
        None => generate_password(if req.invite { 32 } else { 20 }),
    };
    let e = app.engine()?;
    let domains = e.list_domains().await?;
    let domain = domains
        .iter()
        .find(|d| d.id == req.domain_id)
        .ok_or_else(|| ApiError::not_found("That domain"))?;
    let address = format!("{local}@{}", domain.name);
    let accounts = e.list_accounts().await?;
    if accounts
        .iter()
        .any(|x| x.address == address || alias_addresses(x, &domains).contains(&address))
    {
        return Err(ApiError::conflict(format!("{address} is already in use.")).field("local"));
    }
    let display = req
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let quota = req.quota_mb.filter(|q| *q > 0).map(|mb| mb * 1024 * 1024);
    let acct = e
        .create_account(&local, &domain.id, display, quota, &password)
        .await?;
    app.db.audit(
        &a.admin.email,
        "mailbox.create",
        Some(&address),
        quota
            .map(|q| format!("quota {} MB", q / 1048576))
            .as_deref(),
    )?;
    let mut out = mailbox_out(&acct, &domains);
    if req.invite {
        let (token, expires_at) = app
            .db
            .create_invite(&acct.id, &acct.address, &a.admin.email)?;
        app.db
            .audit(&a.admin.email, "mailbox.invite", Some(&acct.address), None)?;
        out.invite_expires_at = Some(expires_at.clone());
        return Ok((
            StatusCode::CREATED,
            Json(json!({
                "mailbox": out,
                "invite": { "url": invite_link(&app, &headers, &token), "expiresAt": expires_at },
            })),
        )
            .into_response());
    }
    Ok((
        StatusCode::CREATED,
        Json(json!({ "mailbox": out, "password": password })),
    )
        .into_response())
}

/// A fresh invite link for an existing mailbox (replaces an unused one). The
/// mailbox keeps working with its current password until the person sets theirs.
pub async fn invite_mailbox(
    State(app): State<AppState>,
    a: AdminAuth,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let acct = app
        .engine()?
        .list_accounts()
        .await?
        .into_iter()
        .find(|x| x.id == id)
        .ok_or_else(|| ApiError::not_found("That mailbox"))?;
    if acct.is_admin {
        return Err(ApiError::conflict(
            "This is the mail engine's administrator account; it is not for reading mail.",
        ));
    }
    let (token, expires_at) = app
        .db
        .create_invite(&acct.id, &acct.address, &a.admin.email)?;
    app.db
        .audit(&a.admin.email, "mailbox.invite", Some(&acct.address), None)?;
    Ok(Json(json!({
        "url": invite_link(&app, &headers, &token),
        "expiresAt": expires_at,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMailbox {
    #[serde(default, deserialize_with = "double_option")]
    display_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    quota_mb: Option<Option<u64>>,
}

pub async fn update_mailbox(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
    Json(req): Json<UpdateMailbox>,
) -> ApiResult<Json<MailboxOut>> {
    let e = app.engine()?;
    let display = req
        .display_name
        .map(|d| d.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    let quota = req
        .quota_mb
        .map(|q| q.filter(|v| *v > 0).map(|mb| mb * 1024 * 1024));
    let acct = e.update_account(&id, display, quota).await?;
    app.db
        .audit(&a.admin.email, "mailbox.update", Some(&acct.address), None)?;
    Ok(Json(mailbox_out(&acct, &e.list_domains().await?)))
}

#[derive(Deserialize)]
pub struct ResetPassword {
    password: Option<String>,
}

pub async fn reset_password(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
    body: Option<Json<ResetPassword>>,
) -> ApiResult<Json<Value>> {
    let password = match body.and_then(|b| b.0.password).filter(|p| !p.is_empty()) {
        Some(p) => {
            check_password(&p)?;
            p
        }
        None => generate_password(20),
    };
    let e = app.engine()?;
    let acct = e
        .list_accounts()
        .await?
        .into_iter()
        .find(|x| x.id == id)
        .ok_or_else(|| ApiError::not_found("That mailbox"))?;
    e.set_password(&id, &password).await?;
    // A password handed over directly supersedes an outstanding invite.
    app.db.revoke_invites(&id)?;
    app.db.audit(
        &a.admin.email,
        "mailbox.password_reset",
        Some(&acct.address),
        None,
    )?;
    Ok(Json(json!({ "password": password })))
}

pub async fn delete_mailbox(
    State(app): State<AppState>,
    a: AdminAuth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let e = app.engine()?;
    let acct = e
        .list_accounts()
        .await?
        .into_iter()
        .find(|x| x.id == id)
        .ok_or_else(|| ApiError::not_found("That mailbox"))?;
    if acct.is_admin {
        return Err(ApiError::conflict(
            "This is the mail engine's administrator account; Yunori needs it and it cannot be deleted here.",
        ));
    }
    e.delete_account(&id).await?;
    app.db.revoke_invites(&id)?;
    app.db
        .audit(&a.admin.email, "mailbox.delete", Some(&acct.address), None)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── aliases ──────────────────────────────────────────────────────────────

pub async fn list_aliases(
    State(app): State<AppState>,
    _a: AdminAuth,
) -> ApiResult<Json<Vec<Value>>> {
    let e = app.engine()?;
    let (domains, accounts) = (e.list_domains().await?, e.list_accounts().await?);
    let mut out = Vec::new();
    for acct in &accounts {
        for al in &acct.aliases {
            if let Some(d) = domains.iter().find(|d| d.id == al.domain_id) {
                out.push(json!({
                    "address": format!("{}@{}", al.local, d.name),
                    "mailboxId": acct.id,
                    "mailboxAddress": acct.address,
                    "enabled": al.enabled,
                }));
            }
        }
    }
    out.sort_by(|a, b| a["address"].as_str().cmp(&b["address"].as_str()));
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAlias {
    local: String,
    domain_id: String,
    mailbox_id: String,
}

pub async fn create_alias(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(req): Json<CreateAlias>,
) -> ApiResult<Response> {
    let local = req.local.trim().to_ascii_lowercase();
    if !valid_local_part(&local) {
        return Err(
            ApiError::bad_request("Use letters, digits, dots, dashes or underscores.")
                .field("local"),
        );
    }
    let e = app.engine()?;
    let (domains, accounts) = (e.list_domains().await?, e.list_accounts().await?);
    let domain = domains
        .iter()
        .find(|d| d.id == req.domain_id)
        .ok_or_else(|| ApiError::not_found("That domain"))?;
    let address = format!("{local}@{}", domain.name);
    if accounts.iter().any(|x| x.address == address) {
        return Err(ApiError::conflict(format!(
            "{address} is a mailbox; an alias cannot shadow it."
        ))
        .field("local"));
    }
    if accounts
        .iter()
        .any(|x| alias_addresses(x, &domains).contains(&address))
    {
        return Err(ApiError::conflict(format!("{address} is already an alias.")).field("local"));
    }
    let target = accounts
        .iter()
        .find(|x| x.id == req.mailbox_id)
        .ok_or_else(|| ApiError::not_found("That mailbox"))?;
    e.add_alias(&target.id, &local, &domain.id).await?;
    app.db.audit(
        &a.admin.email,
        "alias.create",
        Some(&address),
        Some(&format!("→ {}", target.address)),
    )?;
    Ok((StatusCode::CREATED, Json(json!({
        "address": address, "mailboxId": target.id, "mailboxAddress": target.address, "enabled": true
    }))).into_response())
}

#[derive(Deserialize)]
pub struct AliasQuery {
    address: String,
}

pub async fn delete_alias(
    State(app): State<AppState>,
    a: AdminAuth,
    Query(q): Query<AliasQuery>,
) -> ApiResult<StatusCode> {
    let address = q.address.trim().to_ascii_lowercase();
    let e = app.engine()?;
    let (domains, accounts) = (e.list_domains().await?, e.list_accounts().await?);
    for acct in &accounts {
        for al in &acct.aliases {
            let matches = domains
                .iter()
                .any(|d| d.id == al.domain_id && format!("{}@{}", al.local, d.name) == address);
            if matches {
                e.remove_alias(&acct.id, &al.key).await?;
                app.db
                    .audit(&a.admin.email, "alias.delete", Some(&address), None)?;
                return Ok(StatusCode::NO_CONTENT);
            }
        }
    }
    Err(ApiError::not_found("That alias"))
}

// ── sending ──────────────────────────────────────────────────────────────

pub async fn get_sending(State(app): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let cfg = app.config();
    let host = ensure_host_checks(&app);
    let settings = ops::relay_settings(&app);
    let relay = settings.as_ref().map(|r| {
        json!({
            "provider": r.provider, "host": r.host, "port": r.port, "username": r.username,
            "region": r.region, "spfInclude": r.spf_include, "identities": r.identities,
            "hasPassword": app.db.has_secret("relay.password").unwrap_or(false),
            "domainsUrl": crate::relay::domains_url(r),
        })
    });
    // Each hosted domain with the records its relay provider asks for.
    let domains = match app.engine() {
        Ok(e) => e.list_domains().await.unwrap_or_default(),
        Err(_) => vec![],
    };
    let domains: Vec<Value> = domains
        .iter()
        .map(|d| {
            let records: Vec<Value> = settings
                .as_ref()
                .map(|r| crate::relay::domain_records(r, &d.name))
                .unwrap_or_default()
                .into_iter()
                .map(|e| {
                    json!({
                        "id": e.id, "type": e.rtype, "name": e.name, "value": e.value,
                        "required": e.required, "purpose": e.purpose,
                    })
                })
                .collect();
            json!({
                "id": d.id,
                "name": d.name,
                "records": records,
                "needsVerification": settings
                    .as_ref()
                    .is_some_and(|r| crate::relay::needs_verification(r, &d.name)),
            })
        })
        .collect();
    Ok(Json(json!({
        "mode": cfg.map(|c| c.sending_mode).unwrap_or(SendingMode::Direct),
        "relay": relay,
        "providers": RELAY_PROVIDERS,
        "domains": domains,
        "port25": host.as_ref().map(|h| h.port25.clone()).unwrap_or_else(|| unknown("port25", "Outbound port 25")),
        "ptr": host.as_ref().map(|h| h.ptr.clone()).unwrap_or_else(|| unknown("ptr", "Reverse DNS (PTR)")),
    })))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IdentityIn {
    #[serde(default)]
    dkim: String,
    #[serde(default)]
    mail_from: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayIn {
    provider: String,
    #[serde(default)]
    host: String,
    port: u16,
    #[serde(default)]
    username: String,
    password: Option<String>,
    region: Option<String>,
    spf_include: Option<String>,
    /// Omitted = keep the saved per-domain records.
    identities: Option<std::collections::BTreeMap<String, IdentityIn>>,
}

#[derive(Deserialize)]
pub struct UpdateSending {
    mode: SendingMode,
    relay: Option<RelayIn>,
}

pub async fn put_sending(
    State(app): State<AppState>,
    a: AdminAuth,
    Json(req): Json<UpdateSending>,
) -> ApiResult<Json<Value>> {
    let mut cfg = app
        .config()
        .ok_or_else(|| ApiError::conflict("The server is not configured yet."))?;
    let e = app.engine()?;
    match req.mode {
        SendingMode::Direct => {
            e.set_relay(None).await?;
            cfg.sending_mode = SendingMode::Direct;
            app.db.audit(&a.admin.email, "sending.direct", None, None)?;
        }
        SendingMode::Relay => {
            let r = req
                .relay
                .ok_or_else(|| ApiError::bad_request("Relay settings are required."))?;
            let domains: Vec<String> = e
                .list_domains()
                .await?
                .into_iter()
                .map(|d| d.name)
                .collect();
            let draft = crate::relay::RelayDraft {
                provider: r.provider,
                host: r.host,
                port: r.port,
                username: r.username,
                region: r.region,
                spf_include: r.spf_include,
                identities: r.identities.map(|m| {
                    m.into_iter()
                        .map(|(k, v)| {
                            (
                                k,
                                crate::relay::IdentityInput {
                                    dkim: v.dkim,
                                    mail_from: v.mail_from,
                                },
                            )
                        })
                        .collect()
                }),
            };
            let previous = ops::relay_settings(&app);
            let settings = crate::relay::build_settings(&draft, previous.as_ref(), &domains)
                .map_err(|f| ApiError::bad_request(f.message).field(f.field))?;
            let password = match r.password.filter(|p| !p.is_empty()) {
                Some(p) => p,
                None => app
                    .db
                    .get_secret("relay.password")?
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        ApiError::bad_request("Enter the relay password.").field("password")
                    })?,
            };
            e.set_relay(Some(&RelayConfig {
                host: settings.host.clone(),
                port: settings.port,
                implicit_tls: settings.port == 465,
                username: settings.username.clone(),
                password: password.clone(),
            }))
            .await?;
            app.db.put_secret("relay.password", &password)?;
            app.db.set_json("relay", &settings)?;
            cfg.sending_mode = SendingMode::Relay;
            let verified: Vec<&str> = settings.identities.keys().map(String::as_str).collect();
            app.db.audit(
                &a.admin.email,
                "sending.relay",
                Some(&settings.host),
                Some(&format!(
                    "provider {}{}",
                    settings.provider,
                    if verified.is_empty() {
                        String::new()
                    } else {
                        format!("; provider records for {}", verified.join(", "))
                    }
                )),
            )?;
        }
    }
    app.save_config(&cfg)?;
    // SPF include, provider records and the PTR requirement changed — refresh in the background.
    let bg = app.clone();
    tokio::spawn(async move {
        crate::jobs::refresh_all_dns(&bg).await;
        ops::host_checks(&bg, true).await;
    });
    get_sending(State(app), a).await
}

// ── deliverability, system, security ─────────────────────────────────────

pub async fn deliverability(State(app): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let cfg = app.config();
    let host = ensure_host_checks(&app);
    let mut checks = match &host {
        Some(h) => {
            let mut v = vec![h.port25.clone(), h.ptr.clone()];
            v.extend(h.blocklists.clone());
            if let Some(p) = &h.provider {
                v.push(p.clone());
            }
            v
        }
        None => vec![
            unknown("port25", "Outbound port 25"),
            unknown("ptr", "Reverse DNS (PTR)"),
        ],
    };
    checks.push(crate::edge::certs::check(&app));
    let relayed = cfg
        .as_ref()
        .is_some_and(|c| c.sending_mode == SendingMode::Relay);
    let queue = match app.engine() {
        Ok(e) => ops::summarize_queue(&e.queue().await.unwrap_or_default(), relayed),
        Err(_) => ops::summarize_queue(&[], relayed),
    };
    let policy = cfg
        .as_ref()
        .map(|c| ops::dmarc_policy(&app, &c.primary_domain).as_str())
        .unwrap_or("unknown");
    Ok(Json(json!({
        "checks": checks,
        "queue": queue,
        "dmarcPolicy": policy,
        "checkedAt": host.map(|h| h.checked_at),
    })))
}

fn service_status(name: &str, dev: bool) -> Value {
    if dev {
        return json!({ "name": name, "status": "unknown", "detail": "Development mode — services are not managed." });
    }
    let out = std::process::Command::new("systemctl")
        .args(["is-active", name])
        .output();
    let state = out
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let status = match state.as_str() {
        "active" => "running",
        "inactive" => "stopped",
        "failed" => "failed",
        _ => "unknown",
    };
    json!({ "name": name, "status": status, "detail": if state.is_empty() { Value::Null } else { json!(state) } })
}

pub async fn system(State(app): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let info = match app.engine() {
        Ok(e) => e.info().await,
        Err(_) => crate::engine::EngineInfo {
            reachable: false,
            version: None,
        },
    };
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let total = crate::checks::parse_meminfo(&meminfo, "MemTotal").unwrap_or(0);
    let avail = crate::checks::parse_meminfo(&meminfo, "MemAvailable").unwrap_or(0);
    let (disk_total, disk_free) =
        crate::checks::disk_space(&app.paths.stalwart_data()).unwrap_or((0, 0));
    let load1 = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f64>().ok())
        })
        .unwrap_or(0.0);
    let status = crate::edge::certs::status(&app);
    let cert = (status.state != "missing" || status.problem.is_some())
        .then(|| serde_json::to_value(&status).unwrap_or(Value::Null));
    let services: Vec<Value> = ["stalwart", "yunori-mail", "unbound"]
        .iter()
        .map(|s| service_status(s, app.dev))
        .collect();
    Ok(Json(json!({
        "versions": {
            "yunori": crate::app::VERSION,
            "engine": info.version.or_else(|| app.config().and_then(|c| c.stalwart_version)),
        },
        "services": services,
        "resources": {
            "memTotalBytes": total,
            "memUsedBytes": total.saturating_sub(avail),
            "diskTotalBytes": disk_total,
            "diskFreeBytes": disk_free,
            "load1": load1,
        },
        "certificate": cert,
        "update": crate::updates::status(&app),
    })))
}

/// "Check now" on the System page: ask yunori.email for the newest release.
pub async fn update_check(
    State(app): State<AppState>,
    _a: AdminAuth,
) -> ApiResult<Json<crate::updates::UpdateStatus>> {
    Ok(Json(crate::updates::check(&app).await))
}

#[derive(Deserialize)]
pub struct AuditQuery {
    limit: Option<usize>,
}

pub async fn audit(
    State(app): State<AppState>,
    _a: AdminAuth,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Value>> {
    let entries = app.db.audit_list(q.limit.unwrap_or(200).min(1000))?;
    Ok(Json(
        json!({ "entries": entries, "chainOk": app.db.audit_verify()? }),
    ))
}

pub async fn admins(State(app): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    Ok(Json(json!(app.db.admins()?)))
}
