//! Stalwart adapter: outgoing-queue management and login-abuse controls.
//!
//! Wire facts, all verified against a live 1.0.3 engine (see
//! docs/stalwart-integration.md, "Queue" and "Blocked and allowed IPs"):
//!
//! - **Retry now** = update the message's `nextRetry` to a time in the past
//!   (Stalwart's own web admin uses `2000-01-01T00:00:00Z`). The queue picks it
//!   up within a second or two and bumps `retryCount`.
//! - **Cancel** = `x:QueuedMessage/set destroy`. No bounce is sent.
//! - **Pause/resume** = `x:Action` `PauseMtaQueue` / `ResumeMtaQueue`. The state
//!   is in memory only: an engine restart resumes the queue, and nothing reports
//!   whether it is paused.
//! - **Unblock** = `x:BlockedIp/set destroy`, then `x:Action ReloadBlockedIps`
//!   (without the reload the ban stays in force).
//! - **Allowlist** changes take effect after `x:Action ReloadSettings`
//!   (`ReloadBlockedIps` alone does not reload `x:AllowedIp`).

use super::jmap::{Call, SetOp, get_list, ids_ref, set_outcome};
use super::stalwart::{SINGLETON, StalwartEngine};
use super::*;
use serde_json::{Map, Value, json};

/// "Due long ago": the queue delivers it on its next pass.
const RETRY_NOW: &str = "2000-01-01T00:00:00Z";

const BAN_PROPS: &[&str] = &["address", "reason", "createdAt", "expiresAt"];

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

pub(super) fn parse_blocked(v: &Value) -> Option<EBlockedIp> {
    Some(EBlockedIp {
        id: s(v, "id")?,
        address: s(v, "address")?,
        reason: s(v, "reason").unwrap_or_else(|| "manual".into()),
        created_at: s(v, "createdAt"),
        expires_at: s(v, "expiresAt"),
    })
}

pub(super) fn parse_allowed(v: &Value) -> Option<EAllowedIp> {
    Some(EAllowedIp {
        id: s(v, "id")?,
        address: s(v, "address")?,
        reason: s(v, "reason"),
        created_at: s(v, "createdAt"),
        expires_at: s(v, "expiresAt"),
    })
}

/// `x:Rate` is `{count, period}` with the period in milliseconds.
pub(super) fn parse_rate(v: Option<&Value>) -> Option<BanRate> {
    let v = v?;
    Some(BanRate {
        count: v.get("count")?.as_u64()?,
        period_ms: v.get("period")?.as_u64()?,
    })
}

impl StalwartEngine {
    pub(super) async fn op_queue_retry(&self, ids: &[String]) -> EngineResult<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let update: Map<String, Value> = ids
            .iter()
            .map(|id| (id.clone(), json!({ "nextRetry": RETRY_NOW })))
            .collect();
        let r = self
            .queue_set_while_locked(json!({ "update": update }), ids)
            .await?;
        let mut retried = 0;
        for id in ids {
            match set_outcome(&r[0], SetOp::Update, id) {
                Ok(_) => retried += 1,
                // Delivered (or expired) since the list was read.
                Err(EngineError::NotFound) if ids.len() > 1 => {}
                Err(e) => return Err(e),
            }
        }
        Ok(retried)
    }

    pub(super) async fn op_queue_cancel(&self, id: &str) -> EngineResult<()> {
        let r = self
            .queue_set_while_locked(json!({ "destroy": [id] }), &[id.to_string()])
            .await?;
        set_outcome(&r[0], SetOp::Destroy, id).map(|_| ())
    }

    /// `x:QueuedMessage/set`, waiting briefly while the engine holds a message
    /// for a delivery attempt in progress: it refuses changes to a message it
    /// is delivering (`forbidden`; seen live right after a message is queued).
    async fn queue_set_while_locked(
        &self,
        args: Value,
        ids: &[String],
    ) -> EngineResult<Vec<Value>> {
        let mut attempt = 0;
        loop {
            let r = self
                .admin_call(vec![Call::new("x:QueuedMessage/set", args.clone())])
                .await?;
            let locked = ["notUpdated", "notDestroyed"].iter().any(|k| {
                r[0].get(*k).and_then(Value::as_object).is_some_and(|m| {
                    ids.iter().any(|id| {
                        m.get(id)
                            .and_then(|e| e.get("type"))
                            .and_then(Value::as_str)
                            == Some("forbidden")
                    })
                })
            });
            attempt += 1;
            if !locked || attempt >= 4 {
                if locked {
                    return Err(EngineError::Rejected(
                        "The engine is delivering this message right now. Try again in a few seconds.".into(),
                    ));
                }
                return Ok(r);
            }
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }
    }

    pub(super) async fn op_queue_set_paused(&self, paused: bool) -> EngineResult<()> {
        let action = if paused {
            "PauseMtaQueue"
        } else {
            "ResumeMtaQueue"
        };
        self.create_action(json!({ "@type": action }))
            .await
            .map(|_| ())
    }

    pub(super) async fn op_blocked_ips(&self) -> EngineResult<Vec<EBlockedIp>> {
        let r = self
            .admin_call(vec![
                Call::new("x:BlockedIp/query", json!({ "limit": 1000 })),
                Call::new(
                    "x:BlockedIp/get",
                    json!({ "#ids": ids_ref(0, "x:BlockedIp/query"), "properties": BAN_PROPS }),
                ),
            ])
            .await?;
        Ok(get_list(&r[1]).iter().filter_map(parse_blocked).collect())
    }

    pub(super) async fn op_unblock_ip(&self, id: &str) -> EngineResult<()> {
        let r = self
            .admin_call(vec![Call::new(
                "x:BlockedIp/set",
                json!({ "destroy": [id] }),
            )])
            .await?;
        set_outcome(&r[0], SetOp::Destroy, id)?;
        self.create_action(json!({ "@type": "ReloadBlockedIps" }))
            .await
            .map(|_| ())
    }

    pub(super) async fn op_allowed_ips(&self) -> EngineResult<Vec<EAllowedIp>> {
        let r = self
            .admin_call(vec![
                Call::new("x:AllowedIp/query", json!({ "limit": 1000 })),
                Call::new(
                    "x:AllowedIp/get",
                    json!({ "#ids": ids_ref(0, "x:AllowedIp/query"), "properties": BAN_PROPS }),
                ),
            ])
            .await?;
        Ok(get_list(&r[1]).iter().filter_map(parse_allowed).collect())
    }

    async fn reload_access_lists(&self) -> EngineResult<()> {
        self.create_action(json!({ "@type": "ReloadSettings" }))
            .await?;
        self.create_action(json!({ "@type": "ReloadBlockedIps" }))
            .await
            .map(|_| ())
    }

    pub(super) async fn op_allow_ip(
        &self,
        address: &str,
        reason: Option<&str>,
    ) -> EngineResult<EAllowedIp> {
        let mut obj = json!({ "address": address });
        if let Some(r) = reason.filter(|r| !r.is_empty()) {
            obj["reason"] = json!(r);
        }
        let r = self
            .admin_call(vec![Call::new(
                "x:AllowedIp/set",
                json!({ "create": { "a": obj } }),
            )])
            .await?;
        let created = set_outcome(&r[0], SetOp::Create, "a")?;
        let id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Protocol("no id".into()))?
            .to_string();
        let r = self
            .admin_call(vec![Call::new(
                "x:AllowedIp/get",
                json!({ "ids": [id], "properties": BAN_PROPS }),
            )])
            .await?;
        let entry = get_list(&r[0])
            .iter()
            .filter_map(parse_allowed)
            .next()
            .unwrap_or(EAllowedIp {
                id,
                address: address.to_string(),
                reason: reason.map(str::to_string),
                created_at: None,
                expires_at: None,
            });
        self.reload_access_lists().await?;
        Ok(entry)
    }

    pub(super) async fn op_remove_allowed_ip(&self, id: &str) -> EngineResult<()> {
        let r = self
            .admin_call(vec![Call::new(
                "x:AllowedIp/set",
                json!({ "destroy": [id] }),
            )])
            .await?;
        set_outcome(&r[0], SetOp::Destroy, id)?;
        self.reload_access_lists().await
    }

    pub(super) async fn op_auth_ban_policy(&self) -> EngineResult<EAuthBanPolicy> {
        let r = self
            .admin_call(vec![Call::new(
                "x:Security/get",
                json!({ "ids": [SINGLETON], "properties": ["authBanRate", "authBanPeriod"] }),
            )])
            .await?;
        let v = get_list(&r[0])
            .into_iter()
            .next()
            .ok_or(EngineError::NotFound)?;
        Ok(EAuthBanPolicy {
            rate: parse_rate(v.get("authBanRate")),
            ban_for_ms: v.get("authBanPeriod").and_then(Value::as_u64),
        })
    }

    pub(super) async fn op_set_auth_ban_policy(&self, p: &EAuthBanPolicy) -> EngineResult<()> {
        let rate = p
            .rate
            .map(|r| json!({ "count": r.count, "period": r.period_ms }));
        self.update_singleton(
            "x:Security",
            json!({ "authBanRate": rate, "authBanPeriod": p.ban_for_ms }),
        )
        .await?;
        self.create_action(json!({ "@type": "ReloadSettings" }))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocked_and_allowed_entries() {
        // Shapes as returned by a live 1.0.3 engine.
        let b = parse_blocked(&json!({
            "address": "10.77.0.1", "reason": "authFailure",
            "createdAt": "2026-11-10T21:52:43Z", "expiresAt": null, "id": "jn93lcduahqa"
        }))
        .unwrap();
        assert_eq!(b.reason, "authFailure");
        assert_eq!(b.expires_at, None);
        let a = parse_allowed(&json!({
            "address": "10.77.0.0/24", "reason": "office", "createdAt": "2026-11-10T21:53:23Z",
            "expiresAt": null, "id": "jn93npesajaa"
        }))
        .unwrap();
        assert_eq!(a.address, "10.77.0.0/24");
        assert_eq!(a.reason.as_deref(), Some("office"));
    }

    #[test]
    fn parses_rates() {
        assert_eq!(
            parse_rate(Some(&json!({"count": 100, "period": 86400000}))),
            Some(BanRate {
                count: 100,
                period_ms: 86_400_000
            })
        );
        assert_eq!(parse_rate(Some(&Value::Null)), None);
        assert_eq!(parse_rate(None), None);
    }
}
