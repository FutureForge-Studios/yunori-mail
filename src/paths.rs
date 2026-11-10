//! Filesystem layout of a Yunori Mail Server host.
//!
//! Every path hangs off a root prefix (`/` in production). Setting `YUNORI_ROOT`
//! relocates the whole tree, which is how development (`serve --dev`) and the
//! test-suite run without touching the real system.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let root = std::env::var_os("YUNORI_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Self { root }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn is_production(&self) -> bool {
        self.root == Path::new("/")
    }

    fn at(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    // ── yunori-mail ────────────────────────────────────────────────────
    pub fn etc_dir(&self) -> PathBuf {
        self.at("etc/yunori-mail")
    }
    pub fn master_key(&self) -> PathBuf {
        self.etc_dir().join("master.key")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.at("var/lib/yunori-mail")
    }
    pub fn db(&self) -> PathBuf {
        self.state_dir().join("yunori.db")
    }
    pub fn certs_dir(&self) -> PathBuf {
        self.state_dir().join("certs")
    }
    pub fn acme_challenges_dir(&self) -> PathBuf {
        self.state_dir().join("acme-challenges")
    }
    /// Where the engine writes an export / reads an import: owned by the
    /// `stalwart` user (0700), outside Yunori's own state directory, which the
    /// engine cannot enter. Exists only while a backup or restore runs.
    pub fn engine_io_dir(&self) -> PathBuf {
        self.at("var/lib/yunori-mail-engine-io")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.state_dir().join("backups")
    }
    pub fn yunori_bin(&self) -> PathBuf {
        self.at("usr/local/bin/yunori-mail")
    }

    // ── stalwart ───────────────────────────────────────────────────────
    pub fn stalwart_bin(&self) -> PathBuf {
        self.at("usr/local/bin/stalwart")
    }
    pub fn stalwart_bin_previous(&self) -> PathBuf {
        self.at("usr/local/bin/stalwart.previous")
    }
    pub fn stalwart_etc(&self) -> PathBuf {
        self.at("etc/stalwart")
    }
    pub fn stalwart_config(&self) -> PathBuf {
        self.stalwart_etc().join("config.json")
    }
    pub fn stalwart_data(&self) -> PathBuf {
        self.at("var/lib/stalwart")
    }
    pub fn stalwart_logs(&self) -> PathBuf {
        self.at("var/log/stalwart")
    }

    // ── system ─────────────────────────────────────────────────────────
    pub fn systemd_dir(&self) -> PathBuf {
        self.at("etc/systemd/system")
    }
    pub fn os_release(&self) -> PathBuf {
        self.at("etc/os-release")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_prefix_relocates_everything() {
        let p = Paths::with_root("/tmp/x");
        assert_eq!(
            p.db(),
            PathBuf::from("/tmp/x/var/lib/yunori-mail/yunori.db")
        );
        assert_eq!(
            p.stalwart_config(),
            PathBuf::from("/tmp/x/etc/stalwart/config.json")
        );
        assert!(!p.is_production());
        assert!(Paths::with_root("/").is_production());
    }
}
