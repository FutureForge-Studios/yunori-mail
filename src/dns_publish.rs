//! DNS automation for one domain: connect a provider (token sealed in the
//! vault, checked with read-only calls), preview, publish, then the same
//! public-resolver check as "Check now". Shared by the console API, the CLI
//! and the installer's DNS steps.
//!
//! The token is write-only: it is sealed under `dns_provider.<domain>.token`
//! (the vault binds the ciphertext to that name), never returned, logged or
//! written to the audit ledger.

use crate::app::App;
use crate::dns::provider::plan::{self, Action, PlanItem};
use crate::dns::provider::{self, DnsProvider, ProviderError, Zone};
use crate::dns::records::{self, Expected};
use crate::engine::EDomain;
use crate::ops::{self, DomainDns};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLink {
    pub provider: String,
    pub zone_id: String,
    pub zone_name: String,
    pub connected_at: String,
    pub connected_by: String,
    #[serde(default)]
    pub last_publish: Option<PublishSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishSummary {
    pub at: String,
    pub by: String,
    pub created: usize,
    pub updated: usize,
    pub failed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("No DNS provider is connected for {0}.")]
    NotConnected(String),
    #[error("{0}")]
    Provider(#[from] ProviderError),
    #[error(
        "The records at the DNS provider changed since the preview. Review the new preview, then publish."
    )]
    Stale,
    #[error("Unknown DNS provider '{0}'.")]
    UnknownProvider(String),
    #[error("Enter the API token.")]
    TokenRequired,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<crate::engine::EngineError> for PublishError {
    fn from(e: crate::engine::EngineError) -> Self {
        PublishError::Other(e.into())
    }
}

type Res<T> = Result<T, PublishError>;

fn link_key(domain: &str) -> String {
    format!("dns_provider:{domain}")
}

fn secret_name(domain: &str) -> String {
    format!("dns_provider.{domain}.token")
}

pub fn link(app: &App, domain: &str) -> anyhow::Result<Option<ProviderLink>> {
    app.db.get_json(&link_key(domain))
}

pub fn has_token(app: &App, domain: &str) -> bool {
    app.db.has_secret(&secret_name(domain)).unwrap_or(false)
}

fn client_for(app: &App, domain: &str) -> Res<(ProviderLink, Box<dyn DnsProvider>, Zone)> {
    let l = link(app, domain)?.ok_or_else(|| PublishError::NotConnected(domain.into()))?;
    let token = app
        .db
        .get_secret(&secret_name(domain))?
        .ok_or_else(|| PublishError::NotConnected(domain.into()))?;
    let c = provider::client(&l.provider, &token)
        .ok_or_else(|| PublishError::UnknownProvider(l.provider.clone()))?;
    let zone = Zone {
        id: l.zone_id.clone(),
        name: l.zone_name.clone(),
    };
    Ok((l, c, zone))
}

/// Check the token with read-only calls (token, zone, DNS records), then seal
/// it. `token: None` re-checks the stored one (e.g. after changing its scope).
pub async fn connect(
    app: &App,
    domain: &str,
    provider_id: &str,
    token: Option<&str>,
    actor: &str,
) -> Res<ProviderLink> {
    let info = provider::info(provider_id)
        .ok_or_else(|| PublishError::UnknownProvider(provider_id.into()))?;
    let stored;
    let token: &str = match token.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => {
            let same = link(app, domain)?.is_some_and(|l| l.provider == provider_id);
            stored = same
                .then(|| app.db.get_secret(&secret_name(domain)))
                .transpose()?
                .flatten()
                .ok_or(PublishError::TokenRequired)?;
            stored.as_str()
        }
    };
    let c = provider::client(provider_id, token)
        .ok_or_else(|| PublishError::UnknownProvider(provider_id.into()))?;
    let zone = c.find_zone(domain).await?;
    c.records(&zone).await?;
    app.db.put_secret(&secret_name(domain), token)?;
    let l = ProviderLink {
        provider: provider_id.into(),
        zone_id: zone.id,
        zone_name: zone.name.clone(),
        connected_at: crate::util::now_rfc3339(),
        connected_by: actor.into(),
        last_publish: link(app, domain)?.and_then(|l| l.last_publish),
    };
    app.db.set_json(&link_key(domain), &l)?;
    app.db.audit(
        actor,
        "dns.provider.connect",
        Some(domain),
        Some(&format!("{} zone {}", info.label, zone.name)),
    )?;
    Ok(l)
}

/// Forget the provider and its token. Records already published stay.
pub fn disconnect(app: &App, domain: &str, actor: &str) -> anyhow::Result<bool> {
    let had = link(app, domain)?.is_some() || has_token(app, domain);
    app.db.delete_kv(&link_key(domain))?;
    app.db.delete_secret(&secret_name(domain))?;
    if had {
        app.db
            .audit(actor, "dns.provider.disconnect", Some(domain), None)?;
    }
    Ok(had)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub create: usize,
    pub update: usize,
    pub unchanged: usize,
    pub manual: usize,
    pub skip: usize,
}

fn counts(items: &[PlanItem]) -> Counts {
    let n = |a: Action| items.iter().filter(|i| i.action == a).count();
    Counts {
        create: n(Action::Create),
        update: n(Action::Update),
        unchanged: n(Action::Unchanged),
        manual: n(Action::Manual),
        skip: n(Action::Skip),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub domain: String,
    pub provider: String,
    pub provider_label: String,
    pub zone_name: String,
    pub items: Vec<PlanItem>,
    pub counts: Counts,
    /// Pass back to publish: it refuses if the provider changed meanwhile.
    pub plan_id: String,
}

async fn expected_for(app: &App, domain: &EDomain) -> Res<(Vec<Expected>, records::RecordContext)> {
    let cfg = app
        .config()
        .ok_or_else(|| anyhow::anyhow!("server is not configured"))?;
    let zone = app.engine()?.zone_file(&domain.id).await?;
    let ctx = ops::record_context(app, &cfg, &domain.name);
    Ok((records::expected_records(&domain.name, &zone, &ctx), ctx))
}

async fn build_preview(
    domain: &str,
    l: &ProviderLink,
    c: &dyn DnsProvider,
    zone: &Zone,
    expected: &[Expected],
    ctx: &records::RecordContext,
) -> Res<Preview> {
    let existing = c.records(zone).await?;
    let items = plan::plan(expected, &existing, &zone.name, ctx);
    Ok(Preview {
        domain: domain.into(),
        provider: l.provider.clone(),
        provider_label: provider::info(&l.provider)
            .map(|p| p.label)
            .unwrap_or("DNS provider")
            .into(),
        zone_name: zone.name.clone(),
        counts: counts(&items),
        plan_id: plan::fingerprint(&items),
        items,
    })
}

/// What publishing would do. Read-only at the provider.
pub async fn preview(app: &App, domain: &EDomain) -> Res<Preview> {
    let (l, c, zone) = client_for(app, &domain.name)?;
    let (expected, ctx) = expected_for(app, domain).await?;
    build_preview(&domain.name, &l, c.as_ref(), &zone, &expected, &ctx).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemResult {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishOutcome {
    #[serde(flatten)]
    pub preview: Preview,
    pub dry_run: bool,
    pub results: Vec<ItemResult>,
    pub created: usize,
    pub updated: usize,
    pub failed: usize,
    /// The public-resolver check run right after publishing (absent on a dry run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns: Option<DomainDns>,
}

pub struct PublishRequest<'a> {
    /// Only these item ids (default: every create/update).
    pub only: Option<&'a [String]>,
    /// Refuse if the plan no longer matches this preview.
    pub plan_id: Option<&'a str>,
    pub dry_run: bool,
    /// Re-run the public-resolver check afterwards.
    pub check: bool,
}

/// Write the selected changes, one at a time, then (optionally) re-check
/// public DNS. A failed record does not stop the others; a network failure does.
async fn apply(
    c: &dyn DnsProvider,
    zone: &Zone,
    items: &[PlanItem],
    only: Option<&[String]>,
) -> Vec<ItemResult> {
    let mut results = Vec::new();
    let mut unreachable: Option<String> = None;
    for i in items.iter().filter(|i| i.is_change()) {
        if only.is_some_and(|o| !o.contains(&i.id)) {
            continue;
        }
        if let Some(m) = &unreachable {
            results.push(ItemResult {
                id: i.id.clone(),
                ok: false,
                error: Some(format!("Not attempted: {m}")),
            });
            continue;
        }
        let w = provider::RecordWrite {
            rtype: i.rtype.clone(),
            name: i.name.clone(),
            value: i.value.clone(),
        };
        let r = match (i.action, &i.target) {
            (Action::Update, Some(t)) => c.update(zone, t, &w).await,
            _ => c.create(zone, &w).await,
        };
        if let Err(ProviderError::Network(m) | ProviderError::Auth(m)) = &r {
            unreachable = Some(m.clone());
        }
        results.push(ItemResult {
            id: i.id.clone(),
            ok: r.is_ok(),
            error: r.err().map(|e| e.to_string()),
        });
    }
    results
}

pub async fn publish(
    app: &App,
    domain: &EDomain,
    req: PublishRequest<'_>,
    actor: &str,
) -> Res<PublishOutcome> {
    let (mut l, c, zone) = client_for(app, &domain.name)?;
    let (expected, ctx) = expected_for(app, domain).await?;
    let preview = build_preview(&domain.name, &l, c.as_ref(), &zone, &expected, &ctx).await?;
    if req.plan_id.is_some_and(|p| p != preview.plan_id) {
        return Err(PublishError::Stale);
    }
    let results = if req.dry_run {
        Vec::new()
    } else {
        apply(c.as_ref(), &zone, &preview.items, req.only).await
    };
    let action_of = |id: &str| preview.items.iter().find(|i| i.id == id).map(|i| i.action);
    let ok_with = |a: Action| {
        results
            .iter()
            .filter(|r| r.ok && action_of(&r.id) == Some(a))
            .count()
    };
    let (created, updated) = (ok_with(Action::Create), ok_with(Action::Update));
    let failed = results.iter().filter(|r| !r.ok).count();
    if !req.dry_run && !results.is_empty() {
        let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
        app.db.audit(
            actor,
            "dns.publish",
            Some(&domain.name),
            Some(&format!(
                "{}: created {created}, updated {updated}, failed {failed} ({})",
                preview.provider_label,
                ids.join(", ")
            )),
        )?;
        l.last_publish = Some(PublishSummary {
            at: crate::util::now_rfc3339(),
            by: actor.into(),
            created,
            updated,
            failed,
        });
        app.db.set_json(&link_key(&domain.name), &l)?;
    }
    let dns = if req.check && !req.dry_run {
        Some(ops::domain_dns(app, domain, true).await?)
    } else {
        None
    };
    Ok(PublishOutcome {
        preview,
        dry_run: req.dry_run,
        results,
        created,
        updated,
        failed,
        dns,
    })
}

/// Installer, before the engine exists: publish only the mail host's A
/// (or, IPv6-only, AAAA) record on the primary domain, if a provider is
/// connected. `Ok(None)` when there is nothing to do.
pub async fn publish_host_record(app: &App, actor: &str) -> Res<Option<String>> {
    let cfg = app
        .config()
        .ok_or_else(|| anyhow::anyhow!("server is not configured"))?;
    if link(app, &cfg.primary_domain)?.is_none() {
        return Ok(None);
    }
    let (mut l, c, zone) = client_for(app, &cfg.primary_domain)?;
    let ctx = ops::record_context(app, &cfg, &cfg.primary_domain);
    let expected: Vec<Expected> = records::expected_records(&cfg.primary_domain, "", &ctx)
        .into_iter()
        .filter(|e| (e.id == "a" || e.id == "aaaa") && e.required)
        .collect();
    let existing = c.records(&zone).await?;
    let items = plan::plan(&expected, &existing, &zone.name, &ctx);
    if items.iter().all(|i| i.action == Action::Skip) {
        return Ok(items.first().and_then(|i| i.note.clone()));
    }
    if let Some(m) = items.iter().find(|i| i.action == Action::Manual) {
        return Ok(Some(m.note.clone().unwrap_or_default()));
    }
    let results = apply(c.as_ref(), &zone, &items, None).await;
    if results.is_empty() {
        return Ok(Some(format!(
            "{} already points at this server.",
            cfg.hostname
        )));
    }
    if let Some(err) = results.iter().find_map(|r| r.error.clone()) {
        return Err(PublishError::Provider(ProviderError::Rejected(err)));
    }
    app.db.audit(
        actor,
        "dns.publish",
        Some(&cfg.primary_domain),
        Some(&format!(
            "{}: host record for {}",
            provider::info(&l.provider)
                .map(|p| p.label)
                .unwrap_or("DNS provider"),
            cfg.hostname
        )),
    )?;
    l.last_publish = Some(PublishSummary {
        at: crate::util::now_rfc3339(),
        by: actor.into(),
        created: results.len(),
        updated: 0,
        failed: 0,
    });
    app.db.set_json(&link_key(&cfg.primary_domain), &l)?;
    Ok(Some(format!(
        "Published {} → {} at {}.",
        cfg.hostname,
        items
            .iter()
            .find(|i| i.is_change())
            .map(|i| i.value.as_str())
            .unwrap_or_default(),
        zone.name
    )))
}

/// One line for logs and the terminal.
pub fn sentence(o: &PublishOutcome) -> String {
    let c = &o.preview.counts;
    if o.dry_run {
        return format!(
            "Would create {}, update {}; {} already correct, {} need you.",
            c.create, c.update, c.unchanged, c.manual
        );
    }
    let mut s = format!(
        "{}: created {}, updated {}, {} already correct",
        o.preview.provider_label, o.created, o.updated, c.unchanged
    );
    if o.failed > 0 {
        s.push_str(&format!(", {} failed", o.failed));
    }
    if c.manual > 0 {
        s.push_str(&format!(", {} need you", c.manual));
    }
    s.push('.');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::provider::CLOUDFLARE_BASE;
    use crate::dns::provider::fake::FakeCloudflare;

    const TOKEN: &str = "cf-test-token-0123456789abcdefghijklmnop";

    #[tokio::test]
    async fn installer_publishes_the_host_record_once() {
        let app = App::for_tests();
        // Nothing connected: nothing to do.
        assert!(
            publish_host_record(&app, "install")
                .await
                .unwrap()
                .is_none()
        );

        let cf = FakeCloudflare::start(TOKEN, &[("z", "example.test")]).await;
        CLOUDFLARE_BASE.with(|b| *b.borrow_mut() = Some(cf.base.clone()));
        connect(&app, "example.test", "cloudflare", Some(TOKEN), "install")
            .await
            .unwrap();
        let msg = publish_host_record(&app, "install").await.unwrap().unwrap();
        assert!(msg.contains("mail.example.test"), "{msg}");
        let recs = cf.records("z");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["type"], "A");
        assert_eq!(recs[0]["content"], "203.0.113.10");
        // Idempotent: a second run changes nothing.
        let msg = publish_host_record(&app, "install").await.unwrap().unwrap();
        assert!(msg.contains("already points"), "{msg}");
        assert_eq!(cf.records("z").len(), 1);
    }

    #[tokio::test]
    async fn unknown_provider_and_missing_token_are_plain_errors() {
        let app = App::for_tests();
        let e = connect(&app, "example.test", "nope", Some(TOKEN), "t")
            .await
            .unwrap_err();
        assert!(matches!(e, PublishError::UnknownProvider(_)));
        let e = connect(&app, "example.test", "cloudflare", None, "t")
            .await
            .unwrap_err();
        assert!(matches!(e, PublishError::TokenRequired));
    }
}
