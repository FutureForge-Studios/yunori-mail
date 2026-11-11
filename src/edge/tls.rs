//! Certificates on disk, the hot-reloading rustls resolver, a self-signed
//! fallback (until ACME succeeds), and the served-certificate probe that
//! compares what Stalwart actually presents on 465/587/993 with the file on
//! disk (Openship's most useful health check).

use crate::paths::Paths;
use crate::store::vault::write_private;
use anyhow::{Context, Result, anyhow};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub fn cert_path(paths: &Paths) -> PathBuf {
    paths.certs_dir().join("fullchain.pem")
}
pub fn key_path(paths: &Paths) -> PathBuf {
    paths.certs_dir().join("privkey.pem")
}

pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn parse_certs(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut pem.as_bytes()).collect::<std::result::Result<_, _>>()?;
    if certs.is_empty() {
        return Err(anyhow!("no certificate in PEM"));
    }
    Ok(certs)
}

pub fn parse_key(pem: &str) -> Result<PrivateKeyDer<'static>> {
    rustls_pemfile::private_key(&mut pem.as_bytes())?
        .ok_or_else(|| anyhow!("no private key in PEM"))
}

pub fn certified_key(cert_pem: &str, key_pem: &str) -> Result<Arc<CertifiedKey>> {
    let certs = parse_certs(cert_pem)?;
    let key = parse_key(key_pem)?;
    let signing = provider()
        .key_provider
        .load_private_key(key)
        .map_err(|e| anyhow!("unsupported key: {e}"))?;
    Ok(Arc::new(CertifiedKey::new(certs, signing)))
}

/// Self-signed certificate for `hostnames` (ECDSA P-256), used until ACME succeeds.
pub fn self_signed(hostnames: &[String]) -> Result<(String, String)> {
    let key = rcgen::KeyPair::generate()?;
    let mut params = rcgen::CertificateParams::new(hostnames.to_vec())?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        hostnames.first().cloned().unwrap_or_default(),
    );
    let cert = params.self_signed(&key)?;
    Ok((cert.pem(), key.serialize_pem()))
}

pub fn write_cert_files(paths: &Paths, cert_pem: &str, key_pem: &str) -> Result<()> {
    std::fs::create_dir_all(paths.certs_dir())?;
    crate::store::vault::set_mode(&paths.certs_dir(), 0o700);
    write_private(&key_path(paths), key_pem.as_bytes())?;
    write_private(&cert_path(paths), cert_pem.as_bytes())?;
    Ok(())
}

pub fn read_cert_files(paths: &Paths) -> Option<(String, String)> {
    let c = std::fs::read_to_string(cert_path(paths)).ok()?;
    let k = std::fs::read_to_string(key_path(paths)).ok()?;
    Some((c, k))
}

#[derive(Debug, Clone)]
pub struct CertInfo {
    pub hostnames: Vec<String>,
    pub issuer: Option<String>,
    pub not_after: Option<String>,
    pub days_left: i64,
    pub self_signed: bool,
}

pub fn cert_info_from_pem(pem: &str) -> Option<CertInfo> {
    let der = parse_certs(pem).ok()?.into_iter().next()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    let mut hostnames: Vec<String> = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|san| {
            san.value
                .general_names
                .iter()
                .filter_map(|n| match n {
                    x509_parser::extensions::GeneralName::DNSName(d) => Some(d.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if hostnames.is_empty()
        && let Some(cn) = cert
            .subject()
            .iter_common_name()
            .next()
            .and_then(|c| c.as_str().ok())
    {
        hostnames.push(cn.to_string());
    }
    let not_after = cert.validity().not_after.timestamp();
    let now = chrono::Utc::now().timestamp();
    let issuer = cert.issuer().to_string();
    Some(CertInfo {
        hostnames,
        self_signed: cert.issuer() == cert.subject(),
        issuer: Some(issuer),
        not_after: chrono::DateTime::from_timestamp(not_after, 0).map(|d| d.to_rfc3339()),
        days_left: (not_after - now) / 86_400,
    })
}

pub fn disk_certificate_info(paths: &Paths) -> Option<CertInfo> {
    cert_info_from_pem(&std::fs::read_to_string(cert_path(paths)).ok()?)
}

pub fn disk_fingerprint(paths: &Paths) -> Option<String> {
    let pem = std::fs::read_to_string(cert_path(paths)).ok()?;
    parse_certs(&pem)
        .ok()?
        .into_iter()
        .next()
        .map(|c| crate::util::sha256_hex(c.as_ref()))
}

/// rustls resolver whose certificate can be swapped at runtime.
#[derive(Debug)]
pub struct HotCert {
    current: RwLock<Arc<CertifiedKey>>,
}

impl HotCert {
    pub fn new(initial: Arc<CertifiedKey>) -> Arc<Self> {
        Arc::new(Self {
            current: RwLock::new(initial),
        })
    }
    pub fn set(&self, ck: Arc<CertifiedKey>) {
        *self.current.write().unwrap() = ck;
    }
}

impl ResolvesServerCert for HotCert {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current.read().unwrap().clone())
    }
}

/// Load the disk certificate, creating a self-signed one if none exists yet.
pub fn load_or_self_signed(paths: &Paths, names: &[String]) -> Result<Arc<CertifiedKey>> {
    if let Some((c, k)) = read_cert_files(paths)
        && let Ok(ck) = certified_key(&c, &k)
    {
        return Ok(ck);
    }
    let (c, k) = self_signed(names)?;
    write_cert_files(paths, &c, &k).context("writing self-signed certificate")?;
    certified_key(&c, &k)
}

// ── served-certificate probe ────────────────────────────────────────────

#[derive(Debug)]
struct AcceptAny;

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

async fn starttls_smtp(stream: &mut TcpStream) -> Result<()> {
    let (r, mut w) = stream.split();
    let mut reader = BufReader::new(r);
    let mut line = String::new();
    // Banner (possibly multi-line).
    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        if line.len() < 4 || line.as_bytes()[3] == b' ' {
            break;
        }
    }
    w.write_all(b"EHLO yunori-probe.invalid\r\n").await?;
    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        if line.len() < 4 || line.as_bytes()[3] == b' ' {
            break;
        }
    }
    w.write_all(b"STARTTLS\r\n").await?;
    line.clear();
    reader.read_line(&mut line).await?;
    if !line.starts_with("220") {
        return Err(anyhow!("STARTTLS refused: {}", line.trim()));
    }
    Ok(())
}

/// TLS to the engine's own listeners on loopback (alert email submission):
/// the certificate is not verified, because the connection never leaves the
/// host and the engine may still serve a self-signed certificate.
pub fn loopback_connector() -> Result<tokio_rustls::TlsConnector> {
    let config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny))
        .with_no_client_auth();
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

/// SHA-256 of the leaf certificate a local port presents.
pub async fn served_fingerprint(port: u16, sni: &str, starttls: bool) -> Result<String> {
    let config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny))
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let fut = async {
        let mut tcp = TcpStream::connect(("127.0.0.1", port)).await?;
        if starttls {
            starttls_smtp(&mut tcp).await?;
        }
        let name = ServerName::try_from(sni.to_string())?;
        let tls = connector.connect(name, tcp).await?;
        let (_, conn) = tls.get_ref();
        let leaf = conn
            .peer_certificates()
            .and_then(|c| c.first().cloned())
            .ok_or_else(|| anyhow!("no certificate"))?;
        Ok::<_, anyhow::Error>(crate::util::sha256_hex(leaf.as_ref()))
    };
    tokio::time::timeout(Duration::from_secs(8), fut)
        .await
        .map_err(|_| anyhow!("timed out"))?
}

/// Does every mail port serve the certificate on disk? Returns per-port detail.
pub async fn verify_served(paths: &Paths, hostname: &str) -> (bool, Vec<String>) {
    let Some(disk) = disk_fingerprint(paths) else {
        return (false, vec!["no certificate on disk".into()]);
    };
    let mut ok = true;
    let mut detail = Vec::new();
    for (port, starttls) in [(465u16, false), (993, false), (587, true)] {
        match served_fingerprint(port, hostname, starttls).await {
            Ok(fp) if fp == disk => detail.push(format!("{port}: current")),
            Ok(_) => {
                ok = false;
                detail.push(format!("{port}: serving an older certificate"));
            }
            Err(e) => {
                ok = false;
                detail.push(format!("{port}: {e}"));
            }
        }
    }
    (ok, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_signed_roundtrip_and_info() {
        let (c, k) = self_signed(&["mail.example.test".into()]).unwrap();
        assert!(certified_key(&c, &k).is_ok());
        let info = cert_info_from_pem(&c).unwrap();
        assert_eq!(info.hostnames, vec!["mail.example.test".to_string()]);
        assert!(info.self_signed);
        assert!(info.days_left > 300);
    }

    #[test]
    fn disk_files_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(dir.path());
        let ck = load_or_self_signed(&paths, &["mail.example.test".into()]).unwrap();
        assert!(!ck.cert.is_empty());
        assert!(disk_fingerprint(&paths).is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(key_path(&paths))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
