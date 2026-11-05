//! In-memory engine for `serve --dev --mock-engine` and tests. Behaves like
//! Stalwart at the level the console cares about (ids, validation, aliases,
//! zone files, queue) without any network.

use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct State {
    seq: u64,
    domains: Vec<EDomain>,
    accounts: Vec<(EAccount, String)>, // (account, password)
    queue: Vec<EQueued>,
    relay: Option<RelayConfig>,
    app_passwords: Vec<(String, EAppPassword)>, // (owner address, pw)
    cert_installed: bool,
}

pub struct MockEngine {
    state: Mutex<State>,
    hostname: String,
}

impl MockEngine {
    pub fn new(hostname: &str) -> Self {
        Self {
            state: Mutex::new(State::default()),
            hostname: hostname.to_string(),
        }
    }

    /// A believable demo dataset for console development.
    pub fn seeded(hostname: &str, domain: &str) -> Self {
        let e = Self::new(hostname);
        {
            let mut s = e.state.lock().unwrap();
            s.seq = 100;
            s.domains.push(EDomain {
                id: "d1".into(),
                name: domain.into(),
                enabled: true,
                catch_all: None,
                created_at: Some("2026-11-05T09:00:00Z".into()),
            });
            for (i, (local, name, used)) in [
                ("hello", Some("Front desk"), 52_428_800u64),
                ("ana", Some("Ana Ito"), 734_003_200),
                ("ren", Some("Ren Sato"), 12_582_912),
            ]
            .into_iter()
            .enumerate()
            {
                s.accounts.push((
                    EAccount {
                        id: format!("u{}", i + 1),
                        local: local.into(),
                        domain_id: "d1".into(),
                        address: format!("{local}@{domain}"),
                        display_name: name.map(str::to_string),
                        is_admin: false,
                        quota_bytes: Some(5 * 1024 * 1024 * 1024),
                        used_bytes: Some(used),
                        aliases: if local == "hello" {
                            vec![EAlias {
                                key: "0".into(),
                                local: "info".into(),
                                domain_id: "d1".into(),
                                enabled: true,
                            }]
                        } else {
                            vec![]
                        },
                        created_at: Some("2026-11-05T09:10:00Z".into()),
                    },
                    "demo-password-1234".into(),
                ));
            }
            s.queue.push(EQueued {
                id: "q1".into(),
                created_at: Some("2026-11-05T09:30:00Z".into()),
                next_retry: Some("2026-11-05T09:45:00Z".into()),
                recipients: vec![EQueuedRecipient {
                    address: "partner@outlook.example".into(),
                    state: RcptState::TemporaryFailure,
                    error_type: Some("unexpectedResponse".into()),
                    response: Some("451 4.7.650 The mail server [203.0.113.10] has been temporarily rate limited due to IP reputation".into()),
                    retry_count: 2,
                }],
            });
        }
        e
    }

    fn next_id(s: &mut State, prefix: &str) -> String {
        s.seq += 1;
        format!("{prefix}{}", s.seq)
    }

    #[cfg(test)]
    pub fn relay(&self) -> Option<RelayConfig> {
        self.state.lock().unwrap().relay.clone()
    }
}

fn valid_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
        && !s.starts_with('.')
        && !s.ends_with('.')
}

#[async_trait]
impl Engine for MockEngine {
    async fn info(&self) -> EngineInfo {
        EngineInfo {
            reachable: true,
            version: Some("1.0.3 (mock)".into()),
        }
    }

    async fn list_domains(&self) -> EngineResult<Vec<EDomain>> {
        let mut d = self.state.lock().unwrap().domains.clone();
        d.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(d)
    }

    async fn create_domain(&self, name: &str) -> EngineResult<EDomain> {
        let mut s = self.state.lock().unwrap();
        if s.domains.iter().any(|d| d.name == name) {
            return Err(EngineError::Rejected(
                "A domain with this name already exists.".into(),
            ));
        }
        let d = EDomain {
            id: Self::next_id(&mut s, "d"),
            name: name.into(),
            enabled: true,
            catch_all: None,
            created_at: Some(chrono::Utc::now().to_rfc3339()),
        };
        s.domains.push(d.clone());
        Ok(d)
    }

    async fn update_domain(
        &self,
        id: &str,
        catch_all: Option<Option<String>>,
        enabled: Option<bool>,
    ) -> EngineResult<EDomain> {
        let mut s = self.state.lock().unwrap();
        let d = s
            .domains
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or(EngineError::NotFound)?;
        if let Some(c) = catch_all {
            d.catch_all = c;
        }
        if let Some(e) = enabled {
            d.enabled = e;
        }
        Ok(d.clone())
    }

    async fn delete_domain(&self, id: &str) -> EngineResult<()> {
        let mut s = self.state.lock().unwrap();
        if s.accounts.iter().any(|(a, _)| a.domain_id == id) {
            return Err(EngineError::Rejected("Domain still has accounts.".into()));
        }
        let before = s.domains.len();
        s.domains.retain(|d| d.id != id);
        if s.domains.len() == before {
            Err(EngineError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn zone_file(&self, id: &str) -> EngineResult<String> {
        let s = self.state.lock().unwrap();
        let d = s
            .domains
            .iter()
            .find(|d| d.id == id)
            .ok_or(EngineError::NotFound)?;
        let n = &d.name;
        let h = &self.hostname;
        Ok(format!(
            "{n}. IN MX 10 {h}.\n\
             {n}. IN TXT \"v=spf1 mx ra=postmaster -all\"\n\
             v1-ed25519-20261105._domainkey.{n}. IN TXT \"v=DKIM1; k=ed25519; h=sha256; p=MCowBQYDK2VwAyEAy2vR8n4mJm8pV3dQ0gq1Yb3o3F9cZKQ7uXl2YkqJp1s=\"\n\
             v1-rsa-20261105._domainkey.{n}. IN TXT \"v=DKIM1; k=rsa; h=sha256; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAxMockKeyForDevelopmentOnly\"\n\
             _dmarc.{n}. IN TXT \"v=DMARC1; p=reject; rua=mailto:postmaster@{n}; ruf=mailto:postmaster@{n}\"\n\
             _mta-sts.{n}. IN TXT \"v=STSv1; id=20261105\"\n\
             _smtp._tls.{n}. IN TXT \"v=TLSRPTv1; rua=mailto:postmaster@{n}\"\n\
             _submissions._tcp.{n}. IN SRV 0 1 465 {h}.\n\
             _imaps._tcp.{n}. IN SRV 0 1 993 {h}.\n\
             _jmap._tcp.{n}. IN SRV 0 1 443 {h}.\n\
             autoconfig.{n}. IN CNAME {h}.\n\
             autodiscover.{n}. IN CNAME {h}.\n\
             mta-sts.{n}. IN CNAME {h}.\n"
        ))
    }

    async fn list_accounts(&self) -> EngineResult<Vec<EAccount>> {
        let mut a: Vec<EAccount> = self
            .state
            .lock()
            .unwrap()
            .accounts
            .iter()
            .map(|(a, _)| a.clone())
            .collect();
        a.sort_by(|x, y| x.address.cmp(&y.address));
        Ok(a)
    }

    async fn create_account(
        &self,
        local: &str,
        domain_id: &str,
        display_name: Option<&str>,
        quota_bytes: Option<u64>,
        password: &str,
    ) -> EngineResult<EAccount> {
        let mut s = self.state.lock().unwrap();
        if !valid_label(local) {
            return Err(EngineError::Rejected("Invalid local part.".into()));
        }
        if password.len() < 12 {
            return Err(EngineError::Rejected(
                "Password is too weak (secret)".into(),
            ));
        }
        let domain = s
            .domains
            .iter()
            .find(|d| d.id == domain_id)
            .ok_or(EngineError::NotFound)?
            .name
            .clone();
        let address = format!("{local}@{domain}");
        let taken = s.accounts.iter().any(|(a, _)| {
            a.address == address
                || a.aliases
                    .iter()
                    .any(|al| al.domain_id == domain_id && al.local == local)
        });
        if taken {
            return Err(EngineError::Rejected(
                "This address is already in use.".into(),
            ));
        }
        let a = EAccount {
            id: Self::next_id(&mut s, "u"),
            local: local.into(),
            domain_id: domain_id.into(),
            address,
            display_name: display_name.map(str::to_string),
            is_admin: false,
            quota_bytes,
            used_bytes: Some(0),
            aliases: vec![],
            created_at: Some(chrono::Utc::now().to_rfc3339()),
        };
        s.accounts.push((a.clone(), password.to_string()));
        Ok(a)
    }

    async fn update_account(
        &self,
        id: &str,
        display_name: Option<Option<String>>,
        quota_bytes: Option<Option<u64>>,
    ) -> EngineResult<EAccount> {
        let mut s = self.state.lock().unwrap();
        let (a, _) = s
            .accounts
            .iter_mut()
            .find(|(a, _)| a.id == id)
            .ok_or(EngineError::NotFound)?;
        if let Some(d) = display_name {
            a.display_name = d;
        }
        if let Some(q) = quota_bytes {
            a.quota_bytes = q;
        }
        Ok(a.clone())
    }

    async fn set_password(&self, id: &str, password: &str) -> EngineResult<()> {
        if password.len() < 12 {
            return Err(EngineError::Rejected(
                "Password is too weak (secret)".into(),
            ));
        }
        let mut s = self.state.lock().unwrap();
        let (_, pw) = s
            .accounts
            .iter_mut()
            .find(|(a, _)| a.id == id)
            .ok_or(EngineError::NotFound)?;
        *pw = password.to_string();
        Ok(())
    }

    async fn delete_account(&self, id: &str) -> EngineResult<()> {
        let mut s = self.state.lock().unwrap();
        let before = s.accounts.len();
        s.accounts.retain(|(a, _)| a.id != id);
        if s.accounts.len() == before {
            Err(EngineError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn add_alias(&self, account_id: &str, local: &str, domain_id: &str) -> EngineResult<()> {
        let mut s = self.state.lock().unwrap();
        if !valid_label(local) {
            return Err(EngineError::Rejected("Invalid local part.".into()));
        }
        let domain = s
            .domains
            .iter()
            .find(|d| d.id == domain_id)
            .ok_or(EngineError::NotFound)?
            .name
            .clone();
        let address = format!("{local}@{domain}");
        if s.accounts.iter().any(|(a, _)| a.address == address) {
            return Err(EngineError::Rejected(
                "This address belongs to a mailbox.".into(),
            ));
        }
        let (a, _) = s
            .accounts
            .iter_mut()
            .find(|(a, _)| a.id == account_id)
            .ok_or(EngineError::NotFound)?;
        let key = a
            .aliases
            .iter()
            .filter_map(|x| x.key.parse::<u64>().ok())
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        a.aliases.push(EAlias {
            key: key.to_string(),
            local: local.into(),
            domain_id: domain_id.into(),
            enabled: true,
        });
        Ok(())
    }

    async fn remove_alias(&self, account_id: &str, alias_key: &str) -> EngineResult<()> {
        let mut s = self.state.lock().unwrap();
        let (a, _) = s
            .accounts
            .iter_mut()
            .find(|(a, _)| a.id == account_id)
            .ok_or(EngineError::NotFound)?;
        let before = a.aliases.len();
        a.aliases.retain(|x| x.key != alias_key);
        if a.aliases.len() == before {
            Err(EngineError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn queue(&self) -> EngineResult<Vec<EQueued>> {
        Ok(self.state.lock().unwrap().queue.clone())
    }

    async fn reload_settings(&self) -> EngineResult<()> {
        Ok(())
    }

    async fn install_certificate(&self, cert_pem: &str, key_pem: &str) -> EngineResult<()> {
        if !cert_pem.contains("BEGIN CERTIFICATE") || !key_pem.contains("PRIVATE KEY") {
            return Err(EngineError::Rejected("invalid PEM".into()));
        }
        self.state.lock().unwrap().cert_installed = true;
        Ok(())
    }

    async fn set_relay(&self, relay: Option<&RelayConfig>) -> EngineResult<()> {
        self.state.lock().unwrap().relay = relay.cloned();
        Ok(())
    }

    async fn user_verify(&self, creds: &UserCreds) -> EngineResult<()> {
        let s = self.state.lock().unwrap();
        let ok = s
            .accounts
            .iter()
            .any(|(a, pw)| a.address == creds.email && *pw == *creds.password);
        if ok {
            Ok(())
        } else {
            Err(EngineError::Unauthorized)
        }
    }

    async fn user_change_password(&self, creds: &UserCreds, next: &str) -> EngineResult<()> {
        self.user_verify(creds).await?;
        if next.len() < 12 {
            return Err(EngineError::Rejected(
                "Password is too weak (secret)".into(),
            ));
        }
        let mut s = self.state.lock().unwrap();
        if let Some((_, pw)) = s
            .accounts
            .iter_mut()
            .find(|(a, _)| a.address == creds.email)
        {
            *pw = next.to_string();
        }
        Ok(())
    }

    async fn user_app_passwords(&self, creds: &UserCreds) -> EngineResult<Vec<EAppPassword>> {
        self.user_verify(creds).await?;
        let s = self.state.lock().unwrap();
        Ok(s.app_passwords
            .iter()
            .filter(|(o, _)| *o == creds.email)
            .map(|(_, p)| p.clone())
            .collect())
    }

    async fn user_create_app_password(
        &self,
        creds: &UserCreds,
        description: &str,
    ) -> EngineResult<(EAppPassword, String)> {
        self.user_verify(creds).await?;
        let mut s = self.state.lock().unwrap();
        let id = Self::next_id(&mut s, "p");
        let p = EAppPassword {
            id,
            description: description.into(),
            created_at: Some(chrono::Utc::now().to_rfc3339()),
        };
        s.app_passwords.push((creds.email.clone(), p.clone()));
        Ok((p, crate::util::generate_password(24)))
    }

    async fn user_delete_app_password(&self, creds: &UserCreds, id: &str) -> EngineResult<()> {
        self.user_verify(creds).await?;
        let mut s = self.state.lock().unwrap();
        let before = s.app_passwords.len();
        s.app_passwords
            .retain(|(o, p)| !(o == &creds.email && p.id == id));
        if s.app_passwords.len() == before {
            Err(EngineError::NotFound)
        } else {
            Ok(())
        }
    }
}
