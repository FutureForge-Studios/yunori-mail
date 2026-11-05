//! Stalwart 1.0.x adapter over its JMAP management API.
//!
//! Admin operations authenticate as the permanent admin created at bootstrap
//! (credentials sealed in the vault, used only over loopback). Self-service
//! operations authenticate as the mailbox user themself — the console never
//! acts on a user's own credentials with admin rights.

use super::jmap::{
    Auth, Call, JmapClient, SetOp, get_list, ids_ref, object_list, set_of, set_outcome,
};
use super::*;
use serde_json::{Map, Value, json};

const ACCOUNT_PROPS: &[&str] = &[
    "@type",
    "name",
    "domainId",
    "emailAddress",
    "description",
    "roles",
    "quotas",
    "usedDiskQuota",
    "aliases",
    "createdAt",
];

/// The Yunori relay route name inside Stalwart's `MtaRoute` objects.
pub const RELAY_ROUTE: &str = "yunori-relay";

pub struct StalwartEngine {
    client: JmapClient,
    admin: Auth,
}

impl StalwartEngine {
    pub fn new(base_url: &str, admin_user: &str, admin_pass: &str) -> Self {
        Self {
            client: JmapClient::new(base_url),
            admin: Auth::basic(admin_user, admin_pass),
        }
    }

    async fn admin_call(&self, calls: Vec<Call>) -> EngineResult<Vec<Value>> {
        self.client.call(&self.admin, calls).await
    }

    async fn get_account(&self, id: &str) -> EngineResult<Value> {
        let r = self
            .admin_call(vec![Call::new(
                "x:Account/get",
                json!({ "ids": [id], "properties": ACCOUNT_PROPS }),
            )])
            .await?;
        get_list(&r[0])
            .into_iter()
            .next()
            .ok_or(EngineError::NotFound)
    }

    async fn singleton_id(&self, auth: &Auth, object: &str) -> EngineResult<String> {
        let r = self
            .client
            .call(
                auth,
                vec![Call::new(format!("{object}/get"), json!({ "ids": null }))],
            )
            .await?;
        get_list(&r[0])
            .into_iter()
            .next()
            .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
            .ok_or_else(|| EngineError::Protocol(format!("{object} singleton not found")))
    }

    async fn update_singleton(&self, object: &str, patch: Value) -> EngineResult<()> {
        let id = self.singleton_id(&self.admin, object).await?;
        let r = self
            .admin_call(vec![Call::new(
                format!("{object}/set"),
                json!({ "update": { id.clone(): patch } }),
            )])
            .await?;
        set_outcome(&r[0], SetOp::Update, &id).map(|_| ())
    }

    async fn create_action(&self, action: Value) -> EngineResult<Value> {
        let r = self
            .admin_call(vec![Call::new(
                "x:Action/set",
                json!({ "create": { "a": action } }),
            )])
            .await?;
        set_outcome(&r[0], SetOp::Create, "a")
    }
}

fn parse_domain(v: &Value) -> Option<EDomain> {
    Some(EDomain {
        id: v.get("id")?.as_str()?.to_string(),
        name: v.get("name")?.as_str()?.to_string(),
        enabled: v.get("enable").and_then(Value::as_bool).unwrap_or(true),
        catch_all: v
            .get("catchAllAddress")
            .and_then(Value::as_str)
            .map(str::to_string),
        created_at: v
            .get("createdAt")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Iterate an objectList (`{"0": {...}}`) as (key, value) pairs in key order.
fn list_entries(v: Option<&Value>) -> Vec<(String, Value)> {
    let mut entries: Vec<(String, Value)> = v
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    entries.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or(u64::MAX));
    entries
}

fn next_list_key(v: Option<&Value>) -> String {
    list_entries(v)
        .iter()
        .filter_map(|(k, _)| k.parse::<u64>().ok())
        .max()
        .map(|m| (m + 1).to_string())
        .unwrap_or_else(|| "0".to_string())
}

pub(crate) fn parse_account(v: &Value) -> Option<EAccount> {
    if v.get("@type")
        .and_then(Value::as_str)
        .is_some_and(|t| t != "User")
    {
        return None; // groups are not mailboxes in the console
    }
    let local = v.get("name")?.as_str()?.to_string();
    let address = v
        .get("emailAddress")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| local.clone());
    let aliases = list_entries(v.get("aliases"))
        .into_iter()
        .filter_map(|(key, a)| {
            Some(EAlias {
                key,
                local: a.get("name")?.as_str()?.to_string(),
                domain_id: a.get("domainId")?.as_str()?.to_string(),
                enabled: a.get("enable").and_then(Value::as_bool).unwrap_or(true),
            })
        })
        .collect();
    Some(EAccount {
        id: v.get("id")?.as_str()?.to_string(),
        local,
        domain_id: v
            .get("domainId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        address,
        display_name: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        is_admin: v
            .get("roles")
            .and_then(|r| r.get("@type"))
            .and_then(Value::as_str)
            == Some("Admin"),
        quota_bytes: v
            .get("quotas")
            .and_then(|q| q.get("maxDiskQuota"))
            .and_then(Value::as_u64),
        used_bytes: v.get("usedDiskQuota").and_then(Value::as_u64),
        aliases,
        created_at: v
            .get("createdAt")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn parse_queued(v: &Value) -> Option<EQueued> {
    let recipients = v
        .get("recipients")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(addr, r)| {
                    let status = r.get("status");
                    let kind = status
                        .and_then(|s| s.get("@type"))
                        .and_then(Value::as_str)
                        .unwrap_or("Scheduled");
                    let state = match kind {
                        "Completed" => RcptState::Completed,
                        "TemporaryFailure" => RcptState::TemporaryFailure,
                        "PermanentFailure" => RcptState::PermanentFailure,
                        _ => RcptState::Scheduled,
                    };
                    let response = status.and_then(|s| {
                        let code = s.get("responseCode").and_then(Value::as_u64);
                        let msg = s
                            .get("responseMessage")
                            .or_else(|| s.get("errorMessage"))
                            .and_then(Value::as_str);
                        match (code, msg) {
                            (Some(c), Some(m)) => Some(format!("{c} {m}")),
                            (None, Some(m)) => Some(m.to_string()),
                            (Some(c), None) => Some(c.to_string()),
                            _ => None,
                        }
                    });
                    EQueuedRecipient {
                        address: addr.clone(),
                        state,
                        error_type: status
                            .and_then(|s| s.get("errorType"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        response,
                        retry_count: r.get("retryCount").and_then(Value::as_u64).unwrap_or(0),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some(EQueued {
        id: v.get("id")?.as_str()?.to_string(),
        created_at: v
            .get("createdAt")
            .and_then(Value::as_str)
            .map(str::to_string),
        next_retry: v
            .get("nextRetry")
            .and_then(Value::as_str)
            .map(str::to_string),
        recipients,
    })
}

#[async_trait]
impl Engine for StalwartEngine {
    async fn info(&self) -> EngineInfo {
        match self.client.session(&self.admin).await {
            Ok(session) => EngineInfo {
                reachable: true,
                version: session
                    .pointer("/capabilities/urn:stalwart:jmap/version")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            Err(_) => EngineInfo {
                reachable: false,
                version: None,
            },
        }
    }

    async fn list_domains(&self) -> EngineResult<Vec<EDomain>> {
        let r = self
            .admin_call(vec![
                Call::new("x:Domain/query", json!({})),
                Call::new(
                    "x:Domain/get",
                    json!({ "#ids": ids_ref(0, "x:Domain/query"), "properties": ["name", "enable", "catchAllAddress", "createdAt"] }),
                ),
            ])
            .await?;
        let mut out: Vec<EDomain> = get_list(&r[1]).iter().filter_map(parse_domain).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn create_domain(&self, name: &str) -> EngineResult<EDomain> {
        let r = self
            .admin_call(vec![Call::new(
                "x:Domain/set",
                json!({ "create": { "d": {
                    "name": name,
                    "certificateManagement": { "@type": "Manual" },
                    "dkimManagement": { "@type": "Automatic" },
                    "dnsManagement": { "@type": "Manual" },
                    "subAddressing": { "@type": "Enabled" }
                }}}),
            )])
            .await?;
        let created = set_outcome(&r[0], SetOp::Create, "d")?;
        let id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Protocol("no id".into()))?;
        Ok(EDomain {
            id: id.to_string(),
            name: name.to_string(),
            enabled: true,
            catch_all: None,
            created_at: None,
        })
    }

    async fn update_domain(
        &self,
        id: &str,
        catch_all: Option<Option<String>>,
        enabled: Option<bool>,
    ) -> EngineResult<EDomain> {
        let mut patch = Map::new();
        if let Some(c) = catch_all {
            patch.insert(
                "catchAllAddress".into(),
                c.map(Value::String).unwrap_or(Value::Null),
            );
        }
        if let Some(e) = enabled {
            patch.insert("enable".into(), Value::Bool(e));
        }
        if !patch.is_empty() {
            let r = self
                .admin_call(vec![Call::new(
                    "x:Domain/set",
                    json!({ "update": { id: Value::Object(patch) } }),
                )])
                .await?;
            set_outcome(&r[0], SetOp::Update, id)?;
        }
        self.list_domains()
            .await?
            .into_iter()
            .find(|d| d.id == id)
            .ok_or(EngineError::NotFound)
    }

    async fn delete_domain(&self, id: &str) -> EngineResult<()> {
        let r = self
            .admin_call(vec![Call::new("x:Domain/set", json!({ "destroy": [id] }))])
            .await?;
        set_outcome(&r[0], SetOp::Destroy, id).map(|_| ())
    }

    async fn zone_file(&self, id: &str) -> EngineResult<String> {
        let r = self
            .admin_call(vec![Call::new(
                "x:Domain/get",
                json!({ "ids": [id], "properties": ["dnsZoneFile"] }),
            )])
            .await?;
        get_list(&r[0])
            .into_iter()
            .next()
            .ok_or(EngineError::NotFound)
            .map(|v| {
                v.get("dnsZoneFile")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
    }

    async fn list_accounts(&self) -> EngineResult<Vec<EAccount>> {
        let r = self
            .admin_call(vec![
                Call::new("x:Account/query", json!({})),
                Call::new(
                    "x:Account/get",
                    json!({ "#ids": ids_ref(0, "x:Account/query"), "properties": ACCOUNT_PROPS }),
                ),
            ])
            .await?;
        let mut out: Vec<EAccount> = get_list(&r[1]).iter().filter_map(parse_account).collect();
        out.sort_by(|a, b| a.address.cmp(&b.address));
        Ok(out)
    }

    async fn create_account(
        &self,
        local: &str,
        domain_id: &str,
        display_name: Option<&str>,
        quota_bytes: Option<u64>,
        password: &str,
    ) -> EngineResult<EAccount> {
        let quotas = match quota_bytes {
            Some(q) => json!({ "maxDiskQuota": q }),
            None => json!({}),
        };
        let r = self
            .admin_call(vec![Call::new(
                "x:Account/set",
                json!({ "create": { "a": {
                    "@type": "User",
                    "name": local,
                    "domainId": domain_id,
                    "description": display_name,
                    "credentials": object_list(vec![json!({ "@type": "Password", "secret": password })]),
                    "quotas": quotas,
                    "roles": { "@type": "User" },
                    "permissions": { "@type": "Inherit" },
                    "aliases": {}
                }}}),
            )])
            .await?;
        let created = set_outcome(&r[0], SetOp::Create, "a")?;
        let id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Protocol("no id".into()))?;
        parse_account(&self.get_account(id).await?)
            .ok_or_else(|| EngineError::Protocol("bad account".into()))
    }

    async fn update_account(
        &self,
        id: &str,
        display_name: Option<Option<String>>,
        quota_bytes: Option<Option<u64>>,
    ) -> EngineResult<EAccount> {
        let mut patch = Map::new();
        if let Some(d) = display_name {
            patch.insert(
                "description".into(),
                d.map(Value::String).unwrap_or(Value::Null),
            );
        }
        if let Some(q) = quota_bytes {
            patch.insert(
                "quotas".into(),
                match q {
                    Some(q) => json!({ "maxDiskQuota": q }),
                    None => json!({}),
                },
            );
        }
        if !patch.is_empty() {
            let r = self
                .admin_call(vec![Call::new(
                    "x:Account/set",
                    json!({ "update": { id: Value::Object(patch) } }),
                )])
                .await?;
            set_outcome(&r[0], SetOp::Update, id)?;
        }
        parse_account(&self.get_account(id).await?).ok_or(EngineError::NotFound)
    }

    async fn set_password(&self, id: &str, password: &str) -> EngineResult<()> {
        // Patch only the Password credential's secret so app passwords and API
        // keys on the account are left untouched.
        let r = self
            .admin_call(vec![Call::new(
                "x:Account/get",
                json!({ "ids": [id], "properties": ["credentials"] }),
            )])
            .await?;
        let account = get_list(&r[0])
            .into_iter()
            .next()
            .ok_or(EngineError::NotFound)?;
        let creds = account.get("credentials");
        let existing = list_entries(creds)
            .into_iter()
            .find(|(_, c)| c.get("@type").and_then(Value::as_str) == Some("Password"))
            .map(|(k, _)| k);
        let patch = match existing {
            Some(k) => json!({ format!("credentials/{k}/secret"): password }),
            None => {
                json!({ format!("credentials/{}", next_list_key(creds)): { "@type": "Password", "secret": password } })
            }
        };
        let r = self
            .admin_call(vec![Call::new(
                "x:Account/set",
                json!({ "update": { id: patch } }),
            )])
            .await?;
        set_outcome(&r[0], SetOp::Update, id).map(|_| ())
    }

    async fn delete_account(&self, id: &str) -> EngineResult<()> {
        let r = self
            .admin_call(vec![Call::new("x:Account/set", json!({ "destroy": [id] }))])
            .await?;
        set_outcome(&r[0], SetOp::Destroy, id).map(|_| ())
    }

    async fn add_alias(&self, account_id: &str, local: &str, domain_id: &str) -> EngineResult<()> {
        let account = self.get_account(account_id).await?;
        let key = next_list_key(account.get("aliases"));
        let r = self
            .admin_call(vec![Call::new(
                "x:Account/set",
                json!({ "update": { account_id: {
                    format!("aliases/{key}"): { "name": local, "domainId": domain_id, "enable": true }
                }}}),
            )])
            .await?;
        set_outcome(&r[0], SetOp::Update, account_id).map(|_| ())
    }

    async fn remove_alias(&self, account_id: &str, alias_key: &str) -> EngineResult<()> {
        let r = self
            .admin_call(vec![Call::new(
                "x:Account/set",
                json!({ "update": { account_id: { format!("aliases/{alias_key}"): null } } }),
            )])
            .await?;
        set_outcome(&r[0], SetOp::Update, account_id).map(|_| ())
    }

    async fn queue(&self) -> EngineResult<Vec<EQueued>> {
        let r = self
            .admin_call(vec![
                Call::new("x:QueuedMessage/query", json!({ "limit": 500 })),
                Call::new(
                    "x:QueuedMessage/get",
                    json!({ "#ids": ids_ref(0, "x:QueuedMessage/query"), "properties": ["recipients", "nextRetry", "createdAt"] }),
                ),
            ])
            .await?;
        Ok(get_list(&r[1]).iter().filter_map(parse_queued).collect())
    }

    async fn reload_settings(&self) -> EngineResult<()> {
        self.create_action(json!({ "@type": "ReloadSettings" }))
            .await
            .map(|_| ())
    }

    async fn install_certificate(&self, cert_pem: &str, key_pem: &str) -> EngineResult<()> {
        let r = self
            .admin_call(vec![Call::new(
                "x:Certificate/set",
                json!({ "create": { "c": {
                    "certificate": { "@type": "Text", "value": cert_pem },
                    "privateKey": { "@type": "Text", "secret": key_pem }
                }}}),
            )])
            .await?;
        let created = set_outcome(&r[0], SetOp::Create, "c")?;
        let id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Protocol("no id".into()))?;
        // Look up the previous default so it can be retired after the switch.
        let previous = {
            let r = self
                .admin_call(vec![Call::new(
                    "x:SystemSettings/get",
                    json!({ "ids": null, "properties": ["defaultCertificateId"] }),
                )])
                .await?;
            get_list(&r[0]).into_iter().next().and_then(|v| {
                v.get("defaultCertificateId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        };
        self.update_singleton("x:SystemSettings", json!({ "defaultCertificateId": id }))
            .await?;
        self.create_action(json!({ "@type": "ReloadTlsCertificates" }))
            .await?;
        if let Some(prev) = previous.filter(|p| p != id) {
            let _ = self
                .admin_call(vec![Call::new(
                    "x:Certificate/set",
                    json!({ "destroy": [prev] }),
                )])
                .await;
        }
        Ok(())
    }

    async fn set_relay(&self, relay: Option<&RelayConfig>) -> EngineResult<()> {
        // Find an existing Yunori relay route.
        let r = self
            .admin_call(vec![
                Call::new("x:MtaRoute/query", json!({})),
                Call::new(
                    "x:MtaRoute/get",
                    json!({ "#ids": ids_ref(0, "x:MtaRoute/query"), "properties": ["name"] }),
                ),
            ])
            .await?;
        let existing = get_list(&r[1])
            .into_iter()
            .find(|v| v.get("name").and_then(Value::as_str) == Some(RELAY_ROUTE))
            .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string));

        match relay {
            Some(cfg) => {
                let route = json!({
                    "host": cfg.host,
                    "port": cfg.port,
                    "protocol": "smtp",
                    "implicitTls": cfg.implicit_tls,
                    "allowInvalidCerts": false,
                    "authUsername": cfg.username,
                    "authSecret": { "@type": "Value", "secret": cfg.password },
                    "description": "Yunori Mail outbound relay"
                });
                let call = match &existing {
                    Some(id) => json!({ "update": { id.clone(): route } }),
                    None => {
                        let mut create = route;
                        create["@type"] = json!("Relay");
                        create["name"] = json!(RELAY_ROUTE);
                        json!({ "create": { "r": create } })
                    }
                };
                let r = self
                    .admin_call(vec![Call::new("x:MtaRoute/set", call)])
                    .await?;
                match &existing {
                    Some(id) => set_outcome(&r[0], SetOp::Update, id)?,
                    None => set_outcome(&r[0], SetOp::Create, "r")?,
                };
                // Local recipients stay local; everything else leaves through the relay.
                self.update_singleton(
                    "x:MtaOutboundStrategy",
                    json!({ "route": {
                        "else": format!("'{RELAY_ROUTE}'"),
                        "match": object_list(vec![json!({ "if": "is_local_domain(rcpt_domain)", "then": "'local'" })])
                    }}),
                )
                .await?;
            }
            None => {
                self.update_singleton(
                    "x:MtaOutboundStrategy",
                    json!({ "route": {
                        "else": "'mx'",
                        "match": object_list(vec![json!({ "if": "is_local_domain(rcpt_domain)", "then": "'local'" })])
                    }}),
                )
                .await?;
                if let Some(id) = existing {
                    let _ = self
                        .admin_call(vec![Call::new(
                            "x:MtaRoute/set",
                            json!({ "destroy": [id] }),
                        )])
                        .await;
                }
            }
        }
        self.reload_settings().await
    }

    async fn user_verify(&self, creds: &UserCreds) -> EngineResult<()> {
        self.client
            .session(&Auth::basic(&creds.email, &creds.password))
            .await
            .map(|_| ())
    }

    async fn user_change_password(&self, creds: &UserCreds, next: &str) -> EngineResult<()> {
        let auth = Auth::basic(&creds.email, &creds.password);
        let id = self.singleton_id(&auth, "x:AccountPassword").await?;
        let r = self
            .client
            .call(
                &auth,
                vec![Call::new(
                    "x:AccountPassword/set",
                    json!({ "update": { id.clone(): { "currentSecret": creds.password.as_str(), "secret": next } } }),
                )],
            )
            .await?;
        set_outcome(&r[0], SetOp::Update, &id).map(|_| ())
    }

    async fn user_app_passwords(&self, creds: &UserCreds) -> EngineResult<Vec<EAppPassword>> {
        let auth = Auth::basic(&creds.email, &creds.password);
        let r = self
            .client
            .call(
                &auth,
                vec![
                    Call::new("x:AppPassword/query", json!({})),
                    Call::new(
                        "x:AppPassword/get",
                        json!({ "#ids": ids_ref(0, "x:AppPassword/query"), "properties": ["description", "createdAt"] }),
                    ),
                ],
            )
            .await?;
        Ok(get_list(&r[1])
            .into_iter()
            .filter_map(|v| {
                Some(EAppPassword {
                    id: v.get("id")?.as_str()?.to_string(),
                    description: v
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("App password")
                        .to_string(),
                    created_at: v
                        .get("createdAt")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect())
    }

    async fn user_create_app_password(
        &self,
        creds: &UserCreds,
        description: &str,
    ) -> EngineResult<(EAppPassword, String)> {
        let auth = Auth::basic(&creds.email, &creds.password);
        let r = self
            .client
            .call(
                &auth,
                vec![Call::new(
                    "x:AppPassword/set",
                    json!({ "create": { "p": { "description": description, "permissions": { "@type": "Inherit" } } } }),
                )],
            )
            .await?;
        let created = set_outcome(&r[0], SetOp::Create, "p")?;
        let id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Protocol("no id".into()))?;
        let secret = created
            .get("secret")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Protocol("no secret returned".into()))?;
        Ok((
            EAppPassword {
                id: id.to_string(),
                description: description.to_string(),
                created_at: None,
            },
            secret.to_string(),
        ))
    }

    async fn user_delete_app_password(&self, creds: &UserCreds, id: &str) -> EngineResult<()> {
        let auth = Auth::basic(&creds.email, &creds.password);
        let r = self
            .client
            .call(
                &auth,
                vec![Call::new("x:AppPassword/set", json!({ "destroy": [id] }))],
            )
            .await?;
        set_outcome(&r[0], SetOp::Destroy, id).map(|_| ())
    }
}

// ── Install-time operations (bootstrap & recovery mode) ─────────────────────

/// Talks to Stalwart while it runs in bootstrap or recovery mode, authenticated
/// as the pinned recovery admin. Used only by the installer.
pub struct StalwartProvisioner {
    client: JmapClient,
    auth: Auth,
}

pub struct BootstrapResult {
    pub admin_user: String,
    pub admin_secret: zeroize::Zeroizing<String>,
}

pub struct ListenerSpec {
    pub name: &'static str,
    pub protocol: &'static str,
    pub bind: String,
    pub use_tls: bool,
    pub implicit_tls: bool,
}

/// The Yunori listener set (docs/stalwart-integration.md). POP3 off; the HTTP
/// listener is loopback-only and plain — the Yunori edge terminates TLS.
pub fn yunori_listeners(http_port: u16) -> Vec<ListenerSpec> {
    vec![
        ListenerSpec {
            name: "smtp",
            protocol: "smtp",
            bind: "[::]:25".into(),
            use_tls: true,
            implicit_tls: false,
        },
        ListenerSpec {
            name: "submissions",
            protocol: "smtp",
            bind: "[::]:465".into(),
            use_tls: true,
            implicit_tls: true,
        },
        ListenerSpec {
            name: "submission",
            protocol: "smtp",
            bind: "[::]:587".into(),
            use_tls: true,
            implicit_tls: false,
        },
        ListenerSpec {
            name: "imaps",
            protocol: "imap",
            bind: "[::]:993".into(),
            use_tls: true,
            implicit_tls: true,
        },
        ListenerSpec {
            name: "sieve",
            protocol: "manageSieve",
            bind: "[::]:4190".into(),
            use_tls: true,
            implicit_tls: false,
        },
        ListenerSpec {
            name: "http-internal",
            protocol: "http",
            bind: format!("127.0.0.1:{http_port}"),
            use_tls: false,
            implicit_tls: false,
        },
    ]
}

impl StalwartProvisioner {
    pub fn new(base_url: &str, user: &str, pass: &str) -> Self {
        Self {
            client: JmapClient::new(base_url),
            auth: Auth::basic(user, pass),
        }
    }

    pub async fn wait_ready(&self, timeout: std::time::Duration) -> EngineResult<()> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.client.session(&self.auth).await {
                Ok(_) => return Ok(()),
                Err(EngineError::Unauthorized) => return Err(EngineError::Unauthorized),
                Err(e) if std::time::Instant::now() >= deadline => return Err(e),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
            }
        }
    }

    /// First-run provisioning (bootstrap mode). Returns the permanent admin.
    pub async fn bootstrap(
        &self,
        hostname: &str,
        domain: &str,
        data_path: &str,
    ) -> EngineResult<BootstrapResult> {
        let r = self
            .client
            .call(
                &self.auth,
                vec![Call::new("x:Bootstrap/get", json!({ "ids": null }))],
            )
            .await?;
        let id = get_list(&r[0])
            .into_iter()
            .next()
            .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
            .ok_or_else(|| {
                EngineError::Protocol(
                    "Stalwart is not in bootstrap mode (store already initialised?)".into(),
                )
            })?;
        let r = self
            .client
            .call(
                &self.auth,
                vec![Call::new(
                    "x:Bootstrap/set",
                    json!({ "update": { id.clone(): {
                        "defaultHostname": hostname,
                        "defaultDomain": domain,
                        "dataStore": { "@type": "RocksDb", "path": data_path },
                        "generateDkimKeys": true,
                        "requestTlsCertificate": false,
                        "dnsServer": { "@type": "Manual" }
                    }}}),
                )],
            )
            .await?;
        let out = set_outcome(&r[0], SetOp::Update, &id)?;
        let user = out.get("username").and_then(Value::as_str);
        let secret = out.get("secret").and_then(Value::as_str);
        match (user, secret) {
            (Some(u), Some(s)) => Ok(BootstrapResult {
                admin_user: u.to_string(),
                admin_secret: zeroize::Zeroizing::new(s.to_string()),
            }),
            _ => Err(EngineError::Protocol(
                "bootstrap did not return admin credentials".into(),
            )),
        }
    }

    /// Recovery mode: create the listener set (only if none exist) and HTTP trust.
    pub async fn configure(&self, hostname: &str, http_port: u16) -> EngineResult<()> {
        let r = self
            .client
            .call(
                &self.auth,
                vec![Call::new("x:NetworkListener/query", json!({}))],
            )
            .await?;
        let existing = r[0]
            .get("ids")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        if existing == 0 {
            let mut create = Map::new();
            for (i, l) in yunori_listeners(http_port).into_iter().enumerate() {
                create.insert(
                    format!("l{i}"),
                    json!({
                        "name": l.name,
                        "protocol": l.protocol,
                        "bind": set_of([l.bind]),
                        "useTls": l.use_tls,
                        "implicitTls": l.implicit_tls
                    }),
                );
            }
            let keys: Vec<String> = create.keys().cloned().collect();
            let r = self
                .client
                .call(
                    &self.auth,
                    vec![Call::new(
                        "x:NetworkListener/set",
                        json!({ "create": create }),
                    )],
                )
                .await?;
            for k in keys {
                set_outcome(&r[0], SetOp::Create, &k)?;
            }
        }

        // Trust the Yunori edge (loopback) for client IPs; advertise service hostnames.
        for (object, patch) in [
            (
                "x:Http",
                json!({ "clientIpSource": "xForwardedFor", "forwardedHops": 1 }),
            ),
            (
                "x:SystemSettings",
                json!({
                    "defaultHostname": hostname,
                    "proxyTrustedNetworks": set_of(["127.0.0.1/32", "::1/128"])
                }),
            ),
        ] {
            let r = self
                .client
                .call(
                    &self.auth,
                    vec![Call::new(format!("{object}/get"), json!({ "ids": null }))],
                )
                .await?;
            let id = get_list(&r[0])
                .into_iter()
                .next()
                .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
                .ok_or_else(|| EngineError::Protocol(format!("{object} not found")))?;
            let r = self
                .client
                .call(
                    &self.auth,
                    vec![Call::new(
                        format!("{object}/set"),
                        json!({ "update": { id.clone(): patch } }),
                    )],
                )
                .await?;
            set_outcome(&r[0], SetOp::Update, &id)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_account_with_aliases_and_quota() {
        let v = json!({
            "id": "u1", "@type": "User", "name": "ana", "domainId": "d1",
            "emailAddress": "ana@example.com", "description": "Ana Ito",
            "roles": {"@type": "User"}, "quotas": {"maxDiskQuota": 1073741824u64},
            "usedDiskQuota": 2048, "aliases": {"1": {"name": "hello", "domainId": "d1", "enable": true}, "0": {"name": "ana.ito", "domainId": "d1"}}
        });
        let a = parse_account(&v).unwrap();
        assert_eq!(a.address, "ana@example.com");
        assert_eq!(a.quota_bytes, Some(1073741824));
        assert_eq!(a.aliases.len(), 2);
        assert_eq!(a.aliases[0].key, "0");
        assert_eq!(a.aliases[0].local, "ana.ito");
        assert!(!a.is_admin);
    }

    #[test]
    fn groups_are_not_mailboxes() {
        assert!(parse_account(&json!({"id": "g", "@type": "Group", "name": "team"})).is_none());
    }

    #[test]
    fn next_key_after_sparse_list() {
        assert_eq!(next_list_key(Some(&json!({"0": {}, "4": {}}))), "5");
        assert_eq!(next_list_key(None), "0");
    }

    #[test]
    fn parses_queue_failures() {
        let q = parse_queued(&json!({
            "id": "q1",
            "recipients": {"bob@gmail.com": {"retryCount": 3, "status": {"@type": "TemporaryFailure", "errorType": "connectionError", "errorMessage": "Connection refused"}}}
        }))
        .unwrap();
        assert_eq!(q.recipients[0].state, RcptState::TemporaryFailure);
        assert_eq!(
            q.recipients[0].error_type.as_deref(),
            Some("connectionError")
        );
        assert_eq!(q.recipients[0].retry_count, 3);
    }

    #[test]
    fn listener_set_keeps_80_443_free() {
        let ls = yunori_listeners(18080);
        assert!(
            ls.iter()
                .all(|l| !l.bind.ends_with(":443") && !l.bind.ends_with(":80"))
        );
        assert!(ls.iter().any(|l| l.bind == "127.0.0.1:18080" && !l.use_tls));
        assert!(!ls.iter().any(|l| l.protocol == "pop3"));
    }
}
