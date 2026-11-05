//! Secrets at rest: AES-256-GCM under a 32-byte master key kept in a 0600 file.
//!
//! Every sealed value is bound to its name as associated data, so a ciphertext
//! copied onto another row fails to open. Nothing in Yunori stores a secret in
//! plaintext — the opposite of Openship's `mail-state.json`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, anyhow};
use rand::RngCore;
use std::path::Path;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct Vault {
    cipher: Aes256Gcm,
}

impl Vault {
    /// Load the master key, creating it (0600, parent 0700) on first use.
    pub fn open_or_create(path: &Path) -> Result<Self> {
        let key: Zeroizing<Vec<u8>> = if path.exists() {
            let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let decoded = hex::decode(String::from_utf8_lossy(&raw).trim())
                .map_err(|_| anyhow!("master key at {} is not hex", path.display()))?;
            Zeroizing::new(decoded)
        } else {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
                set_mode(dir, 0o700);
            }
            let mut key = Zeroizing::new(vec![0u8; 32]);
            rand::rngs::OsRng.fill_bytes(&mut key);
            write_private(path, hex::encode(&*key).as_bytes())?;
            key
        };
        if key.len() != 32 {
            return Err(anyhow!("master key must be 32 bytes"));
        }
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&key).expect("32-byte key"),
        })
    }

    /// For tests.
    #[cfg(test)]
    pub fn from_key(key: [u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(&key).expect("32-byte key"),
        }
    }

    pub fn seal(&self, name: &str, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut nonce = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("encryption failed"))?;
        Ok((nonce.to_vec(), ct))
    }

    pub fn open(&self, name: &str, nonce: &[u8], ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if nonce.len() != 12 {
            return Err(anyhow!("bad nonce"));
        }
        let pt = self
            .cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("secret '{name}' could not be decrypted (wrong master key?)"))?;
        Ok(Zeroizing::new(pt))
    }
}

pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("writing {}", tmp.display()))?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    set_mode(path, 0o600);
    Ok(())
}

pub fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip_and_aad_binding() {
        let v = Vault::from_key([7u8; 32]);
        let (n, c) = v.seal("relay.password", b"hunter2").unwrap();
        assert_eq!(&*v.open("relay.password", &n, &c).unwrap(), b"hunter2");
        // Same ciphertext under a different name must not open.
        assert!(v.open("engine.admin", &n, &c).is_err());
    }

    #[test]
    fn creates_key_file_with_private_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("etc/master.key");
        let v1 = Vault::open_or_create(&path).unwrap();
        let (n, c) = v1.seal("x", b"y").unwrap();
        let v2 = Vault::open_or_create(&path).unwrap();
        assert_eq!(&*v2.open("x", &n, &c).unwrap(), b"y");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
