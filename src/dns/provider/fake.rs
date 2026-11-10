//! A small in-memory Cloudflare API (the four endpoints Yunori uses), served
//! on a loopback port for tests. No real Cloudflare account is involved.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct FakeState {
    token: String,
    zones: Vec<(String, String)>,
    /// zone id → records in Cloudflare's own JSON shape.
    records: HashMap<String, Vec<Value>>,
    read_only: bool,
    /// "METHOD /path" of every request, in order.
    requests: Vec<String>,
    seq: u64,
}

#[derive(Clone)]
pub struct FakeCloudflare {
    pub base: String,
    state: Arc<Mutex<FakeState>>,
}

type S = Arc<Mutex<FakeState>>;

fn ok(result: Value) -> Response {
    Json(json!({"success": true, "errors": [], "messages": [], "result": result})).into_response()
}

fn err(status: StatusCode, code: i64, message: &str) -> Response {
    (
        status,
        Json(json!({"success": false, "errors": [{"code": code, "message": message}], "messages": [], "result": null})),
    )
        .into_response()
}

fn authed(s: &FakeState, h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {}", s.token))
}

/// Store a record the way Cloudflare echoes it back.
fn shape(id: String, mut body: Value) -> Value {
    body["id"] = json!(id);
    if body["proxied"].is_null() {
        body["proxied"] = json!(false);
    }
    if body["type"] == "SRV" && body["data"].is_object() {
        let d = body["data"].clone();
        body["priority"] = d["priority"].clone();
        body["content"] = json!(format!(
            "{} {} {}",
            d["weight"],
            d["port"],
            d["target"].as_str().unwrap_or_default()
        ));
    }
    if body["type"] == "CAA" && body["data"].is_object() {
        let d = body["data"].clone();
        body["content"] = json!(format!(
            "{} {} \"{}\"",
            d["flags"],
            d["tag"].as_str().unwrap_or_default(),
            d["value"].as_str().unwrap_or_default()
        ));
    }
    body
}

async fn verify(State(s): State<S>, h: HeaderMap) -> Response {
    let s = s.lock().unwrap();
    if !authed(&s, &h) {
        return err(StatusCode::UNAUTHORIZED, 1000, "Invalid API Token");
    }
    ok(json!({"id": "tok", "status": "active"}))
}

async fn list_zones(
    State(s): State<S>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let s = s.lock().unwrap();
    if !authed(&s, &h) {
        return err(StatusCode::UNAUTHORIZED, 9109, "Invalid access token");
    }
    let name = q.get("name").cloned().unwrap_or_default();
    let found: Vec<Value> = s
        .zones
        .iter()
        .filter(|(_, n)| name.is_empty() || *n == name)
        .map(|(id, n)| json!({"id": id, "name": n, "status": "active"}))
        .collect();
    ok(json!(found))
}

async fn list(
    State(s): State<S>,
    h: HeaderMap,
    Path(zid): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let s = s.lock().unwrap();
    if !authed(&s, &h) {
        return err(StatusCode::UNAUTHORIZED, 9109, "Invalid access token");
    }
    let all = s.records.get(&zid).cloned().unwrap_or_default();
    let per: usize = q
        .get("per_page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let page: usize = q.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
    let total_pages = all.len().div_ceil(per).max(1);
    let rows: Vec<Value> = all
        .iter()
        .skip((page - 1) * per)
        .take(per)
        .cloned()
        .collect();
    Json(json!({
        "success": true, "errors": [], "messages": [], "result": rows,
        "result_info": {"page": page, "per_page": per, "count": rows.len(), "total_count": all.len(), "total_pages": total_pages}
    }))
    .into_response()
}

async fn create(
    State(s): State<S>,
    h: HeaderMap,
    Path(zid): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut s = s.lock().unwrap();
    if !authed(&s, &h) {
        return err(StatusCode::UNAUTHORIZED, 9109, "Invalid access token");
    }
    if s.read_only {
        return err(StatusCode::FORBIDDEN, 10000, "Authentication error");
    }
    // Cloudflare refuses a CNAME beside other records at the same name.
    let name = body["name"].as_str().unwrap_or_default().to_string();
    let clash = s.records.get(&zid).is_some_and(|rs| {
        rs.iter().any(|r| {
            r["name"] == name.as_str() && (r["type"] == "CNAME" || body["type"] == "CNAME")
        })
    });
    if clash {
        return err(
            StatusCode::BAD_REQUEST,
            81053,
            "An A, AAAA, or CNAME record with that host already exists.",
        );
    }
    s.seq += 1;
    let rec = shape(format!("r{}", s.seq), body);
    s.records.entry(zid).or_default().push(rec.clone());
    ok(rec)
}

async fn update(
    State(s): State<S>,
    h: HeaderMap,
    Path((zid, rid)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let mut s = s.lock().unwrap();
    if !authed(&s, &h) {
        return err(StatusCode::UNAUTHORIZED, 9109, "Invalid access token");
    }
    if s.read_only {
        return err(StatusCode::FORBIDDEN, 10000, "Authentication error");
    }
    let Some(r) = s
        .records
        .get_mut(&zid)
        .and_then(|rs| rs.iter_mut().find(|r| r["id"] == rid.as_str()))
    else {
        return err(StatusCode::NOT_FOUND, 81044, "Record does not exist.");
    };
    *r = shape(rid, body);
    let out = r.clone();
    ok(out)
}

async fn log_requests(
    State(s): State<S>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    s.lock()
        .unwrap()
        .requests
        .push(format!("{} {}", req.method(), req.uri().path()));
    next.run(req).await
}

async fn other(method: Method, uri: Uri) -> Response {
    err(
        StatusCode::METHOD_NOT_ALLOWED,
        10405,
        &format!("{method} {} is not faked", uri.path()),
    )
}

impl FakeCloudflare {
    pub async fn start(token: &str, zones: &[(&str, &str)]) -> Self {
        let state: S = Arc::new(Mutex::new(FakeState {
            token: token.into(),
            zones: zones
                .iter()
                .map(|(i, n)| (i.to_string(), n.to_string()))
                .collect(),
            ..Default::default()
        }));
        let app = Router::new()
            .route("/client/v4/user/tokens/verify", get(verify))
            .route("/client/v4/zones", get(list_zones))
            .route("/client/v4/zones/{zid}/dns_records", get(list).post(create))
            .route("/client/v4/zones/{zid}/dns_records/{rid}", patch(update))
            .fallback(other)
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                log_requests,
            ))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            base: format!("http://{addr}/client/v4"),
            state,
        }
    }

    /// Add a record as if someone had created it at Cloudflare.
    pub fn seed(&self, zone_id: &str, record: Value) {
        let mut s = self.state.lock().unwrap();
        s.seq += 1;
        let rec = shape(format!("r{}", s.seq), record);
        s.records.entry(zone_id.into()).or_default().push(rec);
    }

    pub fn records(&self, zone_id: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .records
            .get(zone_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn set_read_only(&self, v: bool) {
        self.state.lock().unwrap().read_only = v;
    }
}

/// A loopback URL nothing listens on.
pub async fn unused_base() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    format!("http://{addr}/client/v4")
}
