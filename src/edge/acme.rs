//! ACME (Let's Encrypt) via HTTP-01. The challenge files are written to a
//! directory the edge's port-80 listener serves, so issuance works whether it
//! is driven by the installer or by the renewal job inside `serve`.

use crate::paths::Paths;
use crate::store::Db;
use anyhow::{Context, Result, anyhow, bail};
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt,
    NewAccount, NewOrder, OrderStatus,
};
use std::time::Duration;

fn directory_url() -> &'static str {
    if std::env::var("YUNORI_ACME_STAGING").is_ok_and(|v| v == "1") {
        LetsEncrypt::Staging.url()
    } else {
        LetsEncrypt::Production.url()
    }
}

async fn account(db: &Db, contact: &str) -> Result<Account> {
    if let Some(json) = db.get_secret("acme.account")? {
        let creds: AccountCredentials = serde_json::from_str(&json)?;
        return Ok(Account::from_credentials(creds).await?);
    }
    let contact_uri = format!("mailto:{contact}");
    let (acct, creds) = Account::create(
        &NewAccount {
            contact: &[&contact_uri],
            terms_of_service_agreed: true,
            only_return_existing: false,
        },
        directory_url(),
        None,
    )
    .await
    .context("creating the ACME account")?;
    db.put_secret("acme.account", &serde_json::to_string(&creds)?)?;
    Ok(acct)
}

/// Obtain a certificate for `names` and write it to the certs directory.
pub async fn issue(paths: &Paths, db: &Db, names: &[String], contact: &str) -> Result<()> {
    if names.is_empty() {
        bail!("no hostnames to certify");
    }
    let challenges = paths.acme_challenges_dir();
    std::fs::create_dir_all(&challenges)?;
    let acct = account(db, contact).await?;
    let identifiers: Vec<Identifier> = names.iter().map(|n| Identifier::Dns(n.clone())).collect();
    let mut order = acct
        .new_order(&NewOrder {
            identifiers: &identifiers,
        })
        .await
        .context("creating the order")?;

    let mut written = Vec::new();
    for authz in order.authorizations().await? {
        match authz.status {
            AuthorizationStatus::Valid => continue,
            AuthorizationStatus::Pending => {}
            other => bail!("authorization is {other:?}"),
        }
        let ch = authz
            .challenges
            .iter()
            .find(|c| c.r#type == ChallengeType::Http01)
            .ok_or_else(|| anyhow!("the CA offered no HTTP-01 challenge"))?;
        if !super::valid_token(&ch.token) {
            bail!("the CA sent a malformed challenge token");
        }
        let ka = order.key_authorization(ch);
        let file = challenges.join(&ch.token);
        std::fs::write(&file, ka.as_str())?;
        written.push(file);
        order.set_challenge_ready(&ch.url).await?;
    }

    let mut delay = Duration::from_millis(500);
    let mut status = OrderStatus::Pending;
    for _ in 0..12 {
        tokio::time::sleep(delay).await;
        status = order.refresh().await?.status;
        if matches!(
            status,
            OrderStatus::Ready | OrderStatus::Invalid | OrderStatus::Valid
        ) {
            break;
        }
        delay = (delay * 2).min(Duration::from_secs(10));
    }
    for f in &written {
        let _ = std::fs::remove_file(f);
    }
    if status == OrderStatus::Invalid {
        bail!(
            "the CA could not validate the challenge — check that port 80 is reachable and DNS points here"
        );
    }
    if status != OrderStatus::Ready && status != OrderStatus::Valid {
        bail!("the order did not become ready in time ({status:?})");
    }

    let key = rcgen::KeyPair::generate()?;
    let mut params = rcgen::CertificateParams::new(names.to_vec())?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    let csr = params.serialize_request(&key)?;
    order.finalize(csr.der()).await?;
    let chain = loop {
        match order.certificate().await? {
            Some(c) => break c,
            None => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    };
    super::tls::write_cert_files(paths, &chain, &key.serialize_pem())?;
    db.audit("system", "certificate.issue", Some(&names.join(", ")), None)?;
    Ok(())
}

/// Hostnames to certify: the mail host plus helper names that already resolve here.
pub async fn wanted_names(hostname: &str, domains: &[String], ipv4: Option<&str>) -> Vec<String> {
    use crate::dns::{DnsLookup, Resolver};
    let r = Resolver::public();
    let mut names = vec![hostname.to_string()];
    for d in domains {
        for prefix in ["autoconfig", "mta-sts"] {
            let n = format!("{prefix}.{d}");
            let points_here = match ipv4 {
                Some(ip) => r
                    .a(&n)
                    .await
                    .map(|a| a.iter().any(|x| x == ip))
                    .unwrap_or(false),
                None => false,
            };
            if points_here && !names.contains(&n) {
                names.push(n);
            }
        }
    }
    names
}
