//! Minimal JMAP client for Stalwart's management API (`POST /jmap/`,
//! capability `urn:stalwart:jmap`, methods `x:<Object>/get|set|query`).
//!
//! Wire encodings (see docs/stalwart-integration.md): sets are
//! `{"value": true}`, objectLists are `{"0": {...}}`, tagged unions carry
//! `"@type"`. Errors come back either as a method-level `["error", …]` or as
//! per-object `notCreated` / `notUpdated` / `notDestroyed` entries.

use super::EngineError;
use serde_json::{Value, json};
use std::time::Duration;
use zeroize::Zeroizing;

pub const USING: [&str; 2] = ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"];

#[derive(Clone)]
pub enum Auth {
    Basic {
        user: String,
        pass: Zeroizing<String>,
    },
}

impl Auth {
    pub fn basic(user: &str, pass: &str) -> Self {
        Auth::Basic {
            user: user.to_string(),
            pass: Zeroizing::new(pass.to_string()),
        }
    }
}

#[derive(Clone)]
pub struct JmapClient {
    http: reqwest::Client,
    base: String,
}

pub struct Call {
    pub method: String,
    pub args: Value,
}

impl Call {
    pub fn new(method: impl Into<String>, args: Value) -> Self {
        Self {
            method: method.into(),
            args,
        }
    }
}

impl JmapClient {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("http client");
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder, auth: &Auth) -> reqwest::RequestBuilder {
        match auth {
            Auth::Basic { user, pass } => req.basic_auth(user, Some(pass.as_str())),
        }
    }

    /// GET /jmap/session — used to verify a user's credentials without any admin rights.
    pub async fn session(&self, auth: &Auth) -> Result<Value, EngineError> {
        let req = self.apply_auth(self.http.get(format!("{}/jmap/session", self.base)), auth);
        let resp = req
            .send()
            .await
            .map_err(|e| EngineError::Unavailable(e.to_string()))?;
        match resp.status().as_u16() {
            200 => resp
                .json()
                .await
                .map_err(|e| EngineError::Protocol(e.to_string())),
            401 | 403 => Err(EngineError::Unauthorized),
            s => Err(EngineError::Protocol(format!("session returned HTTP {s}"))),
        }
    }

    /// Send method calls in one request; returns each response's arguments in order.
    /// Calls are tagged `c0`, `c1`, … so later calls can back-reference earlier results.
    pub async fn call(&self, auth: &Auth, calls: Vec<Call>) -> Result<Vec<Value>, EngineError> {
        let method_calls: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(i, c)| json!([c.method, c.args, format!("c{i}")]))
            .collect();
        let body = json!({ "using": USING, "methodCalls": method_calls });
        let req = self.apply_auth(
            self.http.post(format!("{}/jmap/", self.base)).json(&body),
            auth,
        );
        let resp = req
            .send()
            .await
            .map_err(|e| EngineError::Unavailable(e.to_string()))?;
        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            return Err(EngineError::Unauthorized);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| EngineError::Protocol(e.to_string()))?;
        if status >= 400 {
            let detail = value
                .get("detail")
                .or_else(|| value.get("description"))
                .and_then(Value::as_str)
                .unwrap_or("request rejected");
            return Err(EngineError::Rejected(format!("HTTP {status}: {detail}")));
        }
        parse_method_responses(value, calls.len())
    }
}

pub fn parse_method_responses(value: Value, expected: usize) -> Result<Vec<Value>, EngineError> {
    let responses = value
        .get("methodResponses")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::Protocol("missing methodResponses".into()))?;
    let mut out = Vec::with_capacity(expected);
    for r in responses {
        let arr = r
            .as_array()
            .ok_or_else(|| EngineError::Protocol("malformed response".into()))?;
        let name = arr.first().and_then(Value::as_str).unwrap_or_default();
        let args = arr.get(1).cloned().unwrap_or(Value::Null);
        if name == "error" {
            let kind = args.get("type").and_then(Value::as_str).unwrap_or("error");
            let desc = args
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            return Err(match kind {
                "forbidden" | "accountNotFound" => EngineError::Unauthorized,
                _ => EngineError::Rejected(if desc.is_empty() {
                    kind.to_string()
                } else {
                    format!("{kind}: {desc}")
                }),
            });
        }
        out.push(args);
    }
    if out.len() < expected {
        return Err(EngineError::Protocol(format!(
            "expected {expected} responses, got {}",
            out.len()
        )));
    }
    Ok(out)
}

/// Extract the result of a single create/update/destroy from a `/set` response.
pub fn set_outcome(resp: &Value, op: SetOp, key: &str) -> Result<Value, EngineError> {
    let (ok, not) = match op {
        SetOp::Create => ("created", "notCreated"),
        SetOp::Update => ("updated", "notUpdated"),
        SetOp::Destroy => ("destroyed", "notDestroyed"),
    };
    if let Some(err) = resp.get(not).and_then(|m| m.get(key)) {
        return Err(set_error(err));
    }
    match op {
        SetOp::Destroy => {
            let destroyed = resp
                .get(ok)
                .and_then(Value::as_array)
                .map(|a| a.iter().any(|v| v.as_str() == Some(key)))
                .unwrap_or(false);
            if destroyed {
                Ok(Value::Null)
            } else {
                Err(EngineError::Protocol("object was not destroyed".into()))
            }
        }
        _ => resp
            .get(ok)
            .and_then(|m| m.get(key))
            .cloned()
            .map(|v| if v.is_null() { json!({}) } else { v })
            .ok_or_else(|| EngineError::Protocol(format!("no {ok} entry for {key}"))),
    }
}

fn set_error(err: &Value) -> EngineError {
    let kind = err
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("invalidProperties");
    let desc = err.get("description").and_then(Value::as_str).unwrap_or("");
    let props = err
        .get("properties")
        .and_then(Value::as_array)
        .map(|p| {
            p.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    match kind {
        "notFound" => EngineError::NotFound,
        "forbidden" => EngineError::Unauthorized,
        _ => {
            let mut msg = if desc.is_empty() {
                kind.to_string()
            } else {
                desc.to_string()
            };
            if !props.is_empty() {
                msg.push_str(&format!(" ({props})"));
            }
            EngineError::Rejected(msg)
        }
    }
}

#[derive(Clone, Copy)]
pub enum SetOp {
    Create,
    Update,
    Destroy,
}

/// Encode a list as Stalwart's objectList map (`{"0": a, "1": b}`).
pub fn object_list(items: Vec<Value>) -> Value {
    Value::Object(
        items
            .into_iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
    )
}

/// Encode a set as `{"item": true}`.
pub fn set_of<I: IntoIterator<Item = S>, S: Into<String>>(items: I) -> Value {
    Value::Object(
        items
            .into_iter()
            .map(|s| (s.into(), Value::Bool(true)))
            .collect(),
    )
}

/// Back-reference to the `ids` of an earlier `/query` call.
pub fn ids_ref(call_index: usize, method: &str) -> Value {
    json!({ "resultOf": format!("c{call_index}"), "name": method, "path": "/ids" })
}

/// The `list` array of a `/get` response.
pub fn get_list(resp: &Value) -> Vec<Value> {
    resp.get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_lists_and_sets_like_stalwart() {
        assert_eq!(
            object_list(vec![json!({"a":1}), json!({"b":2})]),
            json!({"0":{"a":1},"1":{"b":2}})
        );
        assert_eq!(set_of(["127.0.0.1/32"]), json!({"127.0.0.1/32": true}));
    }

    #[test]
    fn method_error_maps_to_engine_error() {
        let v = json!({"methodResponses": [["error", {"type":"forbidden"}, "c0"]]});
        assert!(matches!(
            parse_method_responses(v, 1),
            Err(EngineError::Unauthorized)
        ));
        let v = json!({"methodResponses": [["error", {"type":"invalidArguments","description":"bad"}, "c0"]]});
        match parse_method_responses(v, 1) {
            Err(EngineError::Rejected(m)) => assert!(m.contains("bad")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn set_outcomes() {
        let resp = json!({
            "created": {"new": {"id": "abc"}},
            "notUpdated": {"x": {"type": "invalidProperties", "description": "Password too weak", "properties": ["secret"]}},
            "destroyed": ["d1"],
            "notDestroyed": {"d2": {"type": "notFound"}}
        });
        assert_eq!(
            set_outcome(&resp, SetOp::Create, "new").unwrap()["id"],
            "abc"
        );
        match set_outcome(&resp, SetOp::Update, "x") {
            Err(EngineError::Rejected(m)) => assert_eq!(m, "Password too weak (secret)"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(set_outcome(&resp, SetOp::Destroy, "d1").is_ok());
        assert!(matches!(
            set_outcome(&resp, SetOp::Destroy, "d2"),
            Err(EngineError::NotFound)
        ));
    }
}
