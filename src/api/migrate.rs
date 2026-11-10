//! `/portal/migrate*`: "Move your mail here" for the signed-in mailbox user.
//!
//! The old server's password arrives with each request and lives only in
//! memory (for a test, the request; for an import, the job). The import
//! writes as the user, through a temporary app password made with their own
//! credentials (see `crate::migrate`).

use super::portal::{creds_from, map_user_err};
use super::{ApiError, ApiResult, AppState};
use crate::migrate::{self, DestAuth, JobRequest, NetPolicy, SourceSpec, StartError};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRequest {
    host: String,
    port: u16,
    username: String,
    password: String,
    /// Source folder ids to copy (from the test); absent = the default selection.
    #[serde(default)]
    folders: Option<Vec<String>>,
}

impl SourceRequest {
    fn spec(&self) -> SourceSpec {
        SourceSpec {
            host: self.host.clone(),
            port: self.port,
            username: self.username.trim().to_string(),
            password: Zeroizing::new(self.password.clone()),
        }
    }
}

impl Drop for SourceRequest {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.password);
    }
}

fn start_error(e: StartError) -> ApiError {
    match e {
        StartError::Input(i) => ApiError::bad_request(i.message).field(i.field),
        StartError::Busy => ApiError::conflict("An import into this mailbox is already running."),
        StartError::ServerBusy => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Several imports are running on this server right now. Try again in a few minutes.",
        ),
        StartError::Source(m) => {
            ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "source_failed", m)
        }
        StartError::Engine(e) => map_user_err(e),
        StartError::Internal(e) => e.into(),
    }
}

fn throttle(app: &AppState, key: String, max: usize, window: Duration, msg: &str) -> ApiResult<()> {
    if app.throttled(&key, max, window) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            msg,
        ));
    }
    Ok(())
}

/// `POST /portal/migrate/test`: sign in to the old server and list its
/// folders with message counts and where each would go.
pub async fn test(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SourceRequest>,
) -> ApiResult<Json<Value>> {
    let creds = creds_from(&app, &headers)?;
    throttle(
        &app,
        format!("migrate-test:{}", creds.email),
        20,
        Duration::from_secs(900),
        "That's a lot of connection tests. Wait 15 minutes and try again.",
    )?;
    let folders = migrate::inspect(req.spec(), NetPolicy::default())
        .await
        .map_err(start_error)?;
    Ok(Json(json!({ "folders": folders })))
}

/// `POST /portal/migrate`: start copying in the background.
pub async fn start(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SourceRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let creds = creds_from(&app, &headers)?;
    if let Some(f) = &req.folders {
        if f.is_empty() {
            return Err(ApiError::bad_request("Choose at least one folder.").field("folders"));
        }
        if f.len() > 2000 || f.iter().any(|x| x.len() > 1000) {
            return Err(ApiError::bad_request("That folder list is too long.").field("folders"));
        }
    }
    if migrate::running(&creds.email).is_some() {
        return Err(start_error(StartError::Busy));
    }
    throttle(
        &app,
        format!("migrate-start:{}", creds.email),
        6,
        Duration::from_secs(3600),
        "Imports were started several times in the last hour. Wait a while and try again.",
    )?;
    let job = migrate::start(
        &app,
        JobRequest {
            account: creds.email.clone(),
            actor: creds.email.clone(),
            source: req.spec(),
            folders: req.folders.clone(),
            dry_run: false,
        },
        DestAuth::TemporaryAppPassword(creds),
        NetPolicy::default(),
    )
    .await
    .map_err(start_error)?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "job": job.progress() }))))
}

/// `GET /portal/migrate`: the running import, else the last one (or null).
pub async fn status(State(app): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let creds = creds_from(&app, &headers)?;
    let job = migrate::status(&app.db, &creds.email)?;
    Ok(Json(json!({ "job": job })))
}

/// `POST /portal/migrate/cancel`: stop after the message being copied.
pub async fn cancel(State(app): State<AppState>, headers: HeaderMap) -> ApiResult<StatusCode> {
    let creds = creds_from(&app, &headers)?;
    let job = migrate::running(&creds.email)
        .ok_or_else(|| ApiError::conflict("No import is running."))?;
    job.cancel();
    app.db.audit(
        &creds.email,
        "portal.migrate_cancel",
        Some(&creds.email),
        None,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn call(
        router: &axum::Router,
        cookie: Option<&str>,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value, Option<String>) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("x-yunori-request", "1");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let set = res
            .headers()
            .get(header::SET_COOKIE)
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            set,
        )
    }

    #[tokio::test]
    async fn portal_import_endpoints_validate_before_connecting() {
        let router = super::super::router(App::for_tests());
        let (s, _, _) = call(&router, None, "GET", "/portal/migrate", None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, _, cookie) = call(
            &router,
            None,
            "POST",
            "/portal/login",
            Some(json!({"email": "ana@example.test", "password": "demo-password-1234"})),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let cookie = cookie.unwrap();
        let c = Some(cookie.as_str());

        let (s, v, _) = call(&router, c, "GET", "/portal/migrate", None).await;
        assert_eq!((s, v), (StatusCode::OK, json!({ "job": null })));

        let src = |host: &str, port: u16| json!({"host": host, "port": port, "username": "me@example.com", "password": "pw"});
        let test = "/portal/migrate/test";
        let (s, v, _) = call(&router, c, "POST", test, Some(src("imap.example.com", 25))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"]["field"], "port");
        let (s, v, _) = call(&router, c, "POST", test, Some(src("127.0.0.1", 993))).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(v["error"]["code"], "source_failed");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("private or local"),
            "{v}"
        );
        let mut body = src("imap.example.com", 993);
        body["folders"] = json!([]);
        let (s, v, _) = call(&router, c, "POST", "/portal/migrate", Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"]["field"], "folders");
        let (s, _, _) = call(&router, c, "POST", "/portal/migrate/cancel", None).await;
        assert_eq!(s, StatusCode::CONFLICT);
    }
}
