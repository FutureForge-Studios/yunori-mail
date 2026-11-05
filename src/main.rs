//! Yunori Mail Server — `yunori-mail`.
//!
//! One binary: the installer, the control plane (console API + jobs), the
//! HTTPS edge and the embedded console. The mail engine is Stalwart, run as a
//! separate pinned process and driven through its JMAP management API.

mod api;
mod app;
mod auth;
mod backup;
mod checks;
mod cli;
mod dns;
mod edge;
mod engine;
mod installer;
mod jobs;
mod model;
mod ops;
mod paths;
mod relay;
mod store;
mod util;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,yunori_mail=info")),
        )
        .with_target(false)
        .init();
    let cli = cli::Cli::parse();
    if let Err(e) = cli::run(cli).await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
