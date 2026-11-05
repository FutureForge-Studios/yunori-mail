//! Shared application state for `serve` (API, edge, jobs).

use crate::engine::mock::MockEngine;
use crate::engine::stalwart::StalwartEngine;
use crate::engine::{Engine, EngineError};
use crate::model::{SendingMode, ServerConfig};
use crate::paths::Paths;
use crate::store::Db;
use crate::store::vault::Vault;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use zeroize::Zeroizing;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_CLIENT_ORIGIN: &str = "https://app.yunori.email";
pub const ENGINE_HTTP_PORT: u16 = 18080;

pub struct PortalSession {
    pub email: String,
    pub password: Zeroizing<String>,
    pub expires: Instant,
}

pub struct App {
    pub paths: Paths,
    pub db: Db,
    pub dev: bool,
    engine: RwLock<Option<Arc<dyn Engine>>>,
    mock: Option<Arc<MockEngine>>,
    /// Self-service sessions keep the user's credentials in memory only (never on disk),
    /// for at most 30 minutes — Stalwart is the authority on their password.
    pub portal_sessions: Mutex<HashMap<String, PortalSession>>,
    pub throttle: Mutex<HashMap<String, Vec<Instant>>>,
}

impl App {
    pub fn open(paths: Paths, dev: bool, mock_engine: bool) -> Result<Arc<Self>> {
        let vault =
            Vault::open_or_create(&paths.master_key()).context("opening the secrets vault")?;
        let db = Db::open(&paths.db(), vault)?;
        let mock = if mock_engine {
            let cfg = Self::ensure_dev_config(&db)?;
            Some(Arc::new(MockEngine::seeded(
                &cfg.hostname,
                &cfg.primary_domain,
            )))
        } else {
            None
        };
        let app = Arc::new(Self {
            paths,
            db,
            dev,
            engine: RwLock::new(None),
            mock,
            portal_sessions: Mutex::new(HashMap::new()),
            throttle: Mutex::new(HashMap::new()),
        });
        app.reload_engine()?;
        Ok(app)
    }

    /// For tests: in-memory DB + mock engine.
    #[cfg(test)]
    pub fn for_tests() -> Arc<Self> {
        let db = Db::open_memory().unwrap();
        let cfg = Self::ensure_dev_config(&db).unwrap();
        let mock = Arc::new(MockEngine::seeded(&cfg.hostname, &cfg.primary_domain));
        Arc::new(Self {
            paths: Paths::with_root(std::env::temp_dir().join("yunori-test-root")),
            db,
            dev: true,
            engine: RwLock::new(Some(mock.clone() as Arc<dyn Engine>)),
            mock: Some(mock),
            portal_sessions: Mutex::new(HashMap::new()),
            throttle: Mutex::new(HashMap::new()),
        })
    }

    fn ensure_dev_config(db: &Db) -> Result<ServerConfig> {
        if let Some(cfg) = db.get_json::<ServerConfig>(ServerConfig::KEY)? {
            return Ok(cfg);
        }
        let cfg = ServerConfig {
            hostname: "mail.example.test".into(),
            primary_domain: "example.test".into(),
            admin_email: "owner@elsewhere.example".into(),
            client_origin: DEFAULT_CLIENT_ORIGIN.into(),
            engine_url: format!("http://127.0.0.1:{ENGINE_HTTP_PORT}"),
            sending_mode: SendingMode::Direct,
            public_ipv4: Some("203.0.113.10".into()),
            public_ipv6: None,
            stalwart_version: Some("1.0.3".into()),
            install_complete: true,
        };
        db.set_json(ServerConfig::KEY, &cfg)?;
        Ok(cfg)
    }

    pub fn config(&self) -> Option<ServerConfig> {
        self.db
            .get_json::<ServerConfig>(ServerConfig::KEY)
            .ok()
            .flatten()
    }

    pub fn save_config(&self, cfg: &ServerConfig) -> Result<()> {
        self.db.set_json(ServerConfig::KEY, cfg)
    }

    /// (Re)build the engine client from config + sealed admin credentials.
    pub fn reload_engine(&self) -> Result<()> {
        let engine: Option<Arc<dyn Engine>> = if let Some(m) = &self.mock {
            Some(m.clone() as Arc<dyn Engine>)
        } else {
            match (self.config(), self.db.get_secret("engine.admin")?) {
                (Some(cfg), Some(creds)) => {
                    let (user, pass) = creds.split_once(':').unwrap_or((creds.as_str(), ""));
                    Some(Arc::new(StalwartEngine::new(&cfg.engine_url, user, pass))
                        as Arc<dyn Engine>)
                }
                _ => None,
            }
        };
        *self.engine.write().unwrap() = engine;
        Ok(())
    }

    pub fn engine(&self) -> Result<Arc<dyn Engine>, EngineError> {
        self.engine
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| EngineError::Unavailable("the mail engine is not installed yet".into()))
    }

    #[cfg(test)]
    pub fn mock(&self) -> Option<Arc<MockEngine>> {
        self.mock.clone()
    }

    /// Sliding-window throttle: true if the key exceeded `max` hits in `window`.
    pub fn throttled(&self, key: &str, max: usize, window: std::time::Duration) -> bool {
        let mut t = self.throttle.lock().unwrap();
        let now = Instant::now();
        let hits = t.entry(key.to_string()).or_default();
        hits.retain(|h| now.duration_since(*h) < window);
        if hits.len() >= max {
            return true;
        }
        hits.push(now);
        false
    }
}
