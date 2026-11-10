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
mod csv;
mod dns;
mod dns_publish;
mod edge;
mod engine;
mod installer;
mod jobs;
mod model;
mod ops;
mod paths;
mod relay;
mod rules;
mod store;
mod updates;
mod util;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = cli::Cli::parse();
    // The service logs to the journal; terminal commands already print every
    // step in plain words, so they only add log lines for errors.
    let default_filter = if matches!(cli.command, cli::Command::Serve(_)) {
        "warn,yunori_mail=info"
    } else {
        "warn,yunori_mail=error"
    };
    tracing_subscriber::fmt()
        // Colour only on a terminal (never ANSI codes in the journal).
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
        )
        .with_target(false)
        .init();
    if let Err(e) = cli::run(cli).await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
