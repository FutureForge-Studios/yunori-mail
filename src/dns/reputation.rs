//! IP reputation (DNS blocklists) and hosting-provider guidance.

use super::DnsLookup;
use crate::model::{Check, Status};
use std::net::IpAddr;

pub struct Blocklist {
    pub zone: &'static str,
    pub label: &'static str,
    pub delist_url: &'static str,
}

pub const BLOCKLISTS: &[Blocklist] = &[
    Blocklist {
        zone: "zen.spamhaus.org",
        label: "Spamhaus ZEN",
        delist_url: "https://check.spamhaus.org/",
    },
    Blocklist {
        zone: "b.barracudacentral.org",
        label: "Barracuda",
        delist_url: "https://www.barracudacentral.org/rbl/removal-request",
    },
    Blocklist {
        zone: "bl.spamcop.net",
        label: "SpamCop",
        delist_url: "https://www.spamcop.net/bl.shtml",
    },
];

/// `1.2.3.4` → `4.3.2.1`; IPv6 → nibble form.
pub fn reverse_ip(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.{}", o[3], o[2], o[1], o[0])
        }
        IpAddr::V6(v6) => v6
            .octets()
            .iter()
            .rev()
            .flat_map(|b| [b & 0x0f, b >> 4])
            .map(|n| format!("{n:x}"))
            .collect::<Vec<_>>()
            .join("."),
    }
}

/// Query each blocklist through the given (local) resolver.
pub async fn check_blocklists(ip: &str, dns: &dyn DnsLookup) -> Vec<Check> {
    let Ok(addr) = ip.parse::<IpAddr>() else {
        return vec![];
    };
    let rev = reverse_ip(addr);
    let mut out = Vec::new();
    for bl in BLOCKLISTS {
        let id = format!("dnsbl:{}", bl.zone);
        let label = format!("{} blocklist", bl.label);
        let check = match dns.a(&format!("{rev}.{}", bl.zone)).await {
            Err(e) => Check::new(
                &id,
                &label,
                Status::Unknown,
                format!("Lookup failed ({e})."),
            ),
            Ok(answers) if answers.is_empty() => {
                Check::new(&id, &label, Status::Pass, format!("{ip} is not listed."))
            }
            Ok(answers) => {
                // 127.255.255.x are error/refusal codes (e.g. Spamhaus via public resolvers), not listings.
                if answers.iter().all(|a| a.starts_with("127.255.255.")) {
                    Check::new(&id, &label, Status::Unknown,
                        "The blocklist refused the query (usually because it came through a public resolver).")
                        .remedy("Yunori queries blocklists through the server's local resolver (unbound); make sure it is running.")
                } else {
                    Check::new(&id, &label, Status::Fail, format!("{ip} is listed ({}).", answers.join(", ")))
                        .remedy("Request removal at the link, and switch sending to a relay until the listing clears.")
                        .help(bl.delist_url)
                }
            }
        };
        out.push(check);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Hetzner,
    Vultr,
    Linode,
    DigitalOcean,
    Aws,
    Gcp,
    Azure,
    Oracle,
    Scaleway,
    Ovh,
    Contabo,
    Unknown,
}

/// Best-effort provider detection from the default reverse-DNS name.
pub fn detect_provider(ptr_names: &[String]) -> Provider {
    let joined = ptr_names.join(" ").to_ascii_lowercase();
    let has = |s: &str| joined.contains(s);
    if has("your-server.de") || has("hetzner") || has("clients.your-server") {
        Provider::Hetzner
    } else if has("vultrusercontent.com") || has("vultr.com") {
        Provider::Vultr
    } else if has("linodeusercontent.com") || has("members.linode.com") || has("akamai") {
        Provider::Linode
    } else if has("amazonaws.com") {
        Provider::Aws
    } else if has("googleusercontent.com") {
        Provider::Gcp
    } else if has("cloudapp.azure.com") || has("cloudapp.net") {
        Provider::Azure
    } else if has("oraclecloud.com") || has("oraclevcn.com") {
        Provider::Oracle
    } else if has("scw.cloud") || has("scaleway") {
        Provider::Scaleway
    } else if has("ovh.net") || has("vps.ovh") || has("ovh.ca") || has("ip-") && has(".eu") {
        Provider::Ovh
    } else if has("contaboserver.net") || has("contabo") {
        Provider::Contabo
    } else if has("digitalocean") {
        Provider::DigitalOcean
    } else {
        Provider::Unknown
    }
}

pub struct ProviderGuidance {
    pub name: &'static str,
    pub port25: &'static str,
    pub ptr: &'static str,
    pub url: Option<&'static str>,
}

/// Plan §8.4 — kept in sync with the researched table.
pub fn guidance(p: Provider) -> ProviderGuidance {
    match p {
        Provider::Hetzner => ProviderGuidance {
            name: "Hetzner Cloud",
            port25: "Ports 25 and 465 are blocked on new accounts. After your first paid invoice (about a month) you can file a limit request; approval is case by case.",
            ptr: "Set reverse DNS in the Hetzner Cloud console: Server → Networking → Reverse DNS.",
            url: Some("https://docs.hetzner.com/cloud/servers/faq/"),
        },
        Provider::Vultr => ProviderGuidance {
            name: "Vultr",
            port25: "Port 25 is blocked by default. Open a support ticket describing your use, SPF/DKIM setup and expected volume.",
            ptr: "Set reverse DNS under Server → Settings → IPv4/IPv6.",
            url: Some("https://docs.vultr.com/support/products/compute/why-is-smtp-blocked"),
        },
        Provider::Linode => ProviderGuidance {
            name: "Linode (Akamai)",
            port25: "Ports 25, 465 and 587 are blocked on accounts created since 2019. Set reverse DNS first, then open a support ticket.",
            ptr: "Set reverse DNS under Linode → Network → IP Addresses → Edit RDNS.",
            url: Some("https://www.linode.com/community/questions/19082/"),
        },
        Provider::DigitalOcean => ProviderGuidance {
            name: "DigitalOcean",
            port25: "SMTP ports are blocked and DigitalOcean discourages self-hosted mail. Use relay mode for sending.",
            ptr: "Reverse DNS follows the Droplet's name: rename the Droplet to your mail hostname.",
            url: Some("https://docs.digitalocean.com/support/why-is-smtp-blocked/"),
        },
        Provider::Aws => ProviderGuidance {
            name: "AWS EC2",
            port25: "Port 25 is throttled. Submit the per-region removal request form (can take up to 48 hours and can be refused).",
            ptr: "Use an Elastic IP and set its reverse DNS (EC2 → Elastic IPs → Update reverse DNS).",
            url: Some("https://repost.aws/knowledge-center/ec2-port-25-throttle"),
        },
        Provider::Gcp => ProviderGuidance {
            name: "Google Cloud",
            port25: "Outbound port 25 to external destinations is blocked. Use relay mode for sending.",
            ptr: "Set a PTR record on the VM's external IP (VM → Network interface → Public DNS PTR record).",
            url: Some("https://docs.cloud.google.com/compute/docs/tutorials/sending-mail"),
        },
        Provider::Azure => ProviderGuidance {
            name: "Microsoft Azure",
            port25: "Outbound port 25 is blocked except on enterprise agreements. Use relay mode for sending.",
            ptr: "Set the reverse FQDN on the public IP resource.",
            url: Some(
                "https://learn.microsoft.com/en-us/azure/virtual-network/troubleshoot-outbound-smtp-connectivity",
            ),
        },
        Provider::Oracle => ProviderGuidance {
            name: "Oracle Cloud",
            port25: "Port 25 is blocked for tenancies created after June 2021. File a service-limit request per region.",
            ptr: "Reverse DNS is set by Oracle support — open a ticket.",
            url: Some(
                "https://docs.oracle.com/en-us/iaas/Content/Network/Concepts/reverse_dns.htm",
            ),
        },
        Provider::Scaleway => ProviderGuidance {
            name: "Scaleway",
            port25: "SMTP ports are closed until you complete identity verification, then you can open them in the security group.",
            ptr: "Set reverse DNS on the flexible IP in the console.",
            url: Some(
                "https://scaleway.com/en/docs/instances/how-to/send-emails-from-your-instance",
            ),
        },
        Provider::Ovh => ProviderGuidance {
            name: "OVHcloud",
            port25: "Port 25 is usually open, with outbound anti-spam monitoring. Yunori measured it for you above.",
            ptr: "Set reverse DNS in the OVHcloud control panel under IP → Reverse.",
            url: None,
        },
        Provider::Contabo => ProviderGuidance {
            name: "Contabo",
            port25: "Policies vary; Yunori measured port 25 for you above.",
            ptr: "Set reverse DNS in the Contabo customer panel under Reverse DNS management.",
            url: None,
        },
        Provider::Unknown => ProviderGuidance {
            name: "your hosting provider",
            port25: "Many providers block outbound port 25 on new servers; ask their support how to get it opened.",
            ptr: "Reverse DNS (PTR) is set in your hosting provider's control panel, not at your DNS provider.",
            url: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::fake::FakeDns;

    #[test]
    fn reverses_ips() {
        assert_eq!(reverse_ip("203.0.113.10".parse().unwrap()), "10.113.0.203");
        let v6 = reverse_ip("2001:db8::1".parse().unwrap());
        assert!(v6.starts_with("1.0.0.0"));
        assert!(v6.ends_with("8.b.d.0.1.0.0.2"));
    }

    #[test]
    fn detects_providers_from_ptr() {
        assert_eq!(
            detect_provider(&["static.10.113.0.203.clients.your-server.de".into()]),
            Provider::Hetzner
        );
        assert_eq!(
            detect_provider(&["ec2-203-0-113-10.compute-1.amazonaws.com".into()]),
            Provider::Aws
        );
        assert_eq!(
            detect_provider(&["mail.example.com".into()]),
            Provider::Unknown
        );
    }

    #[tokio::test]
    async fn blocklist_outcomes() {
        let mut dns = FakeDns::default();
        dns.a.insert(
            "10.113.0.203.zen.spamhaus.org".into(),
            vec!["127.255.255.254".into()],
        );
        dns.a.insert(
            "10.113.0.203.bl.spamcop.net".into(),
            vec!["127.0.0.2".into()],
        );
        let out = check_blocklists("203.0.113.10", &dns).await;
        let st = |z: &str| {
            out.iter()
                .find(|c| c.id == format!("dnsbl:{z}"))
                .unwrap()
                .status
        };
        assert_eq!(st("zen.spamhaus.org"), Status::Unknown);
        assert_eq!(st("b.barracudacentral.org"), Status::Pass);
        assert_eq!(st("bl.spamcop.net"), Status::Fail);
    }
}
