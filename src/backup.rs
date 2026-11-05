//! Encrypted, chunked local backups (plan §7.7).
//!
//! Format: `YUNORI-BACKUP-1\n` + 8-byte random nonce prefix, then chunks of
//! `u32 len || AES-256-GCM(chunk)`; nonce = prefix || u32 index, AAD marks the
//! final chunk so truncation is detected. The key is 32 random bytes shown to
//! the operator once as a recovery key (hex) and kept in the vault.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Result, anyhow, bail};
use rand::RngCore;
use std::io::{Read, Write};

const MAGIC: &[u8] = b"YUNORI-BACKUP-1\n";
const CHUNK: usize = 4 * 1024 * 1024;

pub fn new_key() -> String {
    let mut k = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut k);
    hex::encode(k)
}

fn cipher(key_hex: &str) -> Result<Aes256Gcm> {
    let key = hex::decode(key_hex.trim())
        .map_err(|_| anyhow!("the backup key must be 64 hex characters"))?;
    if key.len() != 32 {
        bail!("the backup key must be 64 hex characters");
    }
    Ok(Aes256Gcm::new_from_slice(&key).expect("32 bytes"))
}

fn nonce(prefix: &[u8; 8], index: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(prefix);
    n[8..].copy_from_slice(&index.to_be_bytes());
    n
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = r.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

pub fn encrypt(key_hex: &str, mut input: impl Read, mut out: impl Write) -> Result<()> {
    let c = cipher(key_hex)?;
    let mut prefix = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut prefix);
    out.write_all(MAGIC)?;
    out.write_all(&prefix)?;
    let mut buf = vec![0u8; CHUNK];
    let mut next = vec![0u8; CHUNK];
    let mut n = read_full(&mut input, &mut buf)?;
    let mut index: u32 = 0;
    loop {
        let m = if n == CHUNK {
            read_full(&mut input, &mut next)?
        } else {
            0
        };
        let last = m == 0;
        let ct = c
            .encrypt(
                Nonce::from_slice(&nonce(&prefix, index)),
                Payload {
                    msg: &buf[..n],
                    aad: if last { b"last" } else { b"more" },
                },
            )
            .map_err(|_| anyhow!("encryption failed"))?;
        out.write_all(&(ct.len() as u32).to_be_bytes())?;
        out.write_all(&ct)?;
        if last {
            break;
        }
        std::mem::swap(&mut buf, &mut next);
        n = m;
        index += 1;
    }
    out.flush()?;
    Ok(())
}

pub fn decrypt(key_hex: &str, mut input: impl Read, mut out: impl Write) -> Result<()> {
    let c = cipher(key_hex)?;
    let mut magic = [0u8; 16];
    input.read_exact(&mut magic)?;
    if magic != MAGIC {
        bail!("not a Yunori backup file");
    }
    let mut prefix = [0u8; 8];
    input.read_exact(&mut prefix)?;
    let mut index: u32 = 0;
    loop {
        let mut len = [0u8; 4];
        if read_full(&mut input, &mut len)? < 4 {
            bail!("the backup is truncated");
        }
        let len = u32::from_be_bytes(len) as usize;
        if len > CHUNK + 16 {
            bail!("corrupt backup chunk");
        }
        let mut ct = vec![0u8; len];
        input.read_exact(&mut ct)?;
        let n = nonce(&prefix, index);
        if let Ok(pt) = c.decrypt(
            Nonce::from_slice(&n),
            Payload {
                msg: &ct,
                aad: b"last",
            },
        ) {
            out.write_all(&pt)?;
            break;
        }
        let pt = c
            .decrypt(
                Nonce::from_slice(&n),
                Payload {
                    msg: &ct,
                    aad: b"more",
                },
            )
            .map_err(|_| anyhow!("wrong backup key, or the file is corrupted"))?;
        out.write_all(&pt)?;
        index += 1;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_multi_chunk_and_detects_truncation() {
        let key = new_key();
        let data: Vec<u8> = (0..(CHUNK * 2 + 123)).map(|i| (i % 251) as u8).collect();
        let mut enc = Vec::new();
        encrypt(&key, &data[..], &mut enc).unwrap();
        let mut dec = Vec::new();
        decrypt(&key, &enc[..], &mut dec).unwrap();
        assert_eq!(dec, data);

        // Dropping the final chunk must not decrypt silently.
        let cut = &enc[..enc.len() - 200];
        assert!(decrypt(&key, cut, &mut Vec::new()).is_err());
        // Wrong key fails.
        assert!(decrypt(&new_key(), &enc[..], &mut Vec::new()).is_err());
    }

    #[test]
    fn empty_input() {
        let key = new_key();
        let mut enc = Vec::new();
        encrypt(&key, &b""[..], &mut enc).unwrap();
        let mut dec = Vec::new();
        decrypt(&key, &enc[..], &mut dec).unwrap();
        assert!(dec.is_empty());
    }
}
