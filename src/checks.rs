//! Preflight and host checks (plan §7.3). Every result is a `Check` with a
//! plain-language detail and, where useful, a remedy. Nothing here changes
//! the system.

use crate::dns::{self, DnsLookup, Resolver};
use crate::model::{Check, Status};
use crate::paths::Paths;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

pub const MAIL_PORTS: &[(u16, &str)] = &[
    (25, "SMTP"),
    (80, "HTTP (certificates)"),
    (443, "HTTPS (console, JMAP)"),
    (465, "SMTP submission (TLS)"),
    (587, "SMTP submission"),
    (993, "IMAP"),
    (4190, "ManageSieve"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct OsInfo {
    pub id: String,
    pub version_id: String,
    pub pretty: String,
}

pub fn parse_os_release(text: &str) -> OsInfo {
    let mut info = OsInfo {
        id: String::new(),
        version_id: String::new(),
        pretty: String::new(),
    };
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "ID" => info.id = v,
                "VERSION_ID" => info.version_id = v,
                "PRETTY_NAME" => info.pretty = v,
                _ => {}
            }
        }
    }
    info
}

pub fn os_supported(os: &OsInfo) -> bool {
    matches!(
        (os.id.as_str(), os.version_id.as_str()),
        ("debian", "12")
            | ("debian", "13")
            | ("ubuntu", "22.04")
            | ("ubuntu", "24.04")
            | ("ubuntu", "26.04")
    )
}

pub fn check_os(paths: &Paths) -> Check {
    match std::fs::read_to_string(paths.os_release()) {
        Err(_) => Check::new(
            "os",
            "Operating system",
            Status::Fail,
            "Could not read /etc/os-release.",
        )
        .remedy("Yunori Mail Server supports Debian 12/13 and Ubuntu 22.04/24.04/26.04."),
        Ok(text) => {
            let os = parse_os_release(&text);
            let name = if os.pretty.is_empty() {
                format!("{} {}", os.id, os.version_id)
            } else {
                os.pretty.clone()
            };
            if os_supported(&os) {
                Check::new(
                    "os",
                    "Operating system",
                    Status::Pass,
                    format!("{name} is supported."),
                )
            } else {
                Check::new(
                    "os",
                    "Operating system",
                    Status::Fail,
                    format!("{name} is not supported."),
                )
                .remedy("Use a fresh Debian 12/13 or Ubuntu 22.04/24.04/26.04 server.")
            }
        }
    }
}

/// x86-64-v2 requires these CPU flags (Stalwart 1.0 builds need it).
const X86_64_V2_FLAGS: &[&str] = &["cx16", "lahf_lm", "popcnt", "sse4_1", "sse4_2", "ssse3"];

pub fn cpu_check_from(arch: &str, cpuinfo: &str) -> Check {
    match arch {
        "aarch64" => Check::new("cpu", "CPU", Status::Pass, "64-bit ARM (aarch64)."),
        "x86_64" => {
            let flags: Vec<&str> = cpuinfo
                .lines()
                .find(|l| l.starts_with("flags"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, f)| f.split_whitespace().collect())
                .unwrap_or_default();
            let missing: Vec<&&str> = X86_64_V2_FLAGS
                .iter()
                .filter(|f| !flags.contains(f))
                .collect();
            if missing.is_empty() {
                Check::new(
                    "cpu",
                    "CPU",
                    Status::Pass,
                    "x86-64 with the v2 instruction set.",
                )
            } else {
                Check::new("cpu", "CPU", Status::Fail,
                    format!("This CPU lacks x86-64-v2 features ({}) that the mail engine needs.",
                        missing.iter().map(|s| **s).collect::<Vec<_>>().join(", ")))
                    .remedy("On Proxmox/KVM, change the VM CPU type from kvm64 to \"host\" or x86-64-v2-AES.")
            }
        }
        other => Check::new(
            "cpu",
            "CPU",
            Status::Fail,
            format!("Unsupported architecture {other}."),
        )
        .remedy("Use an x86-64 or ARM64 server."),
    }
}

pub fn check_cpu() -> Check {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    cpu_check_from(std::env::consts::ARCH, &cpuinfo)
}

pub fn mem_total_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo(&text, "MemTotal")
}

pub fn parse_meminfo(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
}

pub fn check_memory() -> Check {
    match mem_total_bytes() {
        None => Check::new(
            "memory",
            "Memory",
            Status::Unknown,
            "Could not read /proc/meminfo.",
        ),
        Some(b) => {
            let gb = b as f64 / 1_073_741_824.0;
            if gb < 0.9 {
                Check::new(
                    "memory",
                    "Memory",
                    Status::Fail,
                    format!("{gb:.1} GB RAM — at least 1 GB is needed."),
                )
                .remedy("Resize the server to 1 GB RAM or more.")
            } else if gb < 1.9 {
                Check::new(
                    "memory",
                    "Memory",
                    Status::Pass,
                    format!(
                        "{gb:.1} GB RAM — fine for a small team; optional antivirus needs 2 GB+."
                    ),
                )
            } else {
                Check::new("memory", "Memory", Status::Pass, format!("{gb:.1} GB RAM."))
            }
        }
    }
}

/// (total, free) bytes of the filesystem holding `path`, via `df -Pk`.
pub fn disk_space(path: &std::path::Path) -> Option<(u64, u64)> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    let out = Command::new("df").arg("-Pk").arg(&p).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    let total = cols.get(1)?.parse::<u64>().ok()? * 1024;
    let avail = cols.get(3)?.parse::<u64>().ok()? * 1024;
    Some((total, avail))
}

pub fn check_disk(paths: &Paths) -> Check {
    match disk_space(&paths.stalwart_data()) {
        None => Check::new(
            "disk",
            "Disk space",
            Status::Unknown,
            "Could not measure free disk space.",
        ),
        Some((_, free)) => {
            let gb = free as f64 / 1_073_741_824.0;
            if gb < 5.0 {
                Check::new(
                    "disk",
                    "Disk space",
                    Status::Fail,
                    format!("{gb:.1} GB free — at least 10 GB is recommended."),
                )
                .remedy("Free up space or attach a larger disk.")
            } else if gb < 10.0 {
                Check::new(
                    "disk",
                    "Disk space",
                    Status::Warn,
                    format!("{gb:.1} GB free — mail grows; 10 GB+ is recommended."),
                )
            } else {
                Check::new(
                    "disk",
                    "Disk space",
                    Status::Pass,
                    format!("{gb:.0} GB free."),
                )
            }
        }
    }
}

fn port_owner(port: u16) -> Option<String> {
    let out = Command::new("ss")
        .args(["-H", "-ltnp", &format!("sport = :{port}")])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?;
    // users:(("nginx",pid=123,fd=6))
    line.split("users:((\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .map(str::to_string)
}

/// Ports we will need. `ours` = process names that may legitimately hold them (re-runs).
pub fn check_ports(ours: &[&str]) -> Vec<Check> {
    MAIL_PORTS
        .iter()
        .map(|(port, label)| {
            let id = format!("port:{port}");
            let name = format!("Port {port} ({label})");
            match TcpListener::bind(("0.0.0.0", *port)) {
                Ok(_) => Check::new(&id, &name, Status::Pass, "Free."),
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => match port_owner(*port) {
                    Some(owner) if ours.contains(&owner.as_str()) => Check::new(
                        &id,
                        &name,
                        Status::Pass,
                        format!("In use by {owner} (Yunori)."),
                    ),
                    Some(owner) => {
                        Check::new(&id, &name, Status::Fail, format!("In use by {owner}.")).remedy(
                            format!("Stop and disable {owner}, or use a server dedicated to mail."),
                        )
                    }
                    None => Check::new(&id, &name, Status::Fail, "In use by another program.")
                        .remedy("Find it with `ss -ltnp` and stop it."),
                },
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Check::new(
                    &id,
                    &name,
                    Status::Unknown,
                    "Run as root to check privileged ports.",
                ),
                Err(e) => Check::new(&id, &name, Status::Unknown, e.to_string()),
            }
        })
        .collect()
}

pub fn check_existing_mta() -> Check {
    let active: Vec<&str> = ["postfix", "exim4", "sendmail", "opensmtpd"]
        .into_iter()
        .filter(|svc| {
            Command::new("systemctl")
                .args(["is-active", "--quiet", svc])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .collect();
    if active.is_empty() {
        Check::new(
            "mta",
            "Other mail software",
            Status::Pass,
            "No other mail server is running.",
        )
    } else {
        Check::new(
            "mta",
            "Other mail software",
            Status::Fail,
            format!("{} is running.", active.join(", ")),
        )
        .remedy(format!("Run: systemctl disable --now {}", active.join(" ")))
    }
}

pub fn check_clock() -> Check {
    let out = Command::new("timedatectl")
        .args(["show", "-p", "NTPSynchronized", "--value"])
        .output();
    match out {
        Ok(o) if String::from_utf8_lossy(&o.stdout).trim() == "yes" => Check::new(
            "clock",
            "Clock",
            Status::Pass,
            "Synchronised with network time.",
        ),
        Ok(_) => Check::new(
            "clock",
            "Clock",
            Status::Warn,
            "The clock is not synchronised.",
        )
        .remedy(
            "Run: timedatectl set-ntp true (signatures and certificates depend on correct time).",
        ),
        Err(_) => Check::new(
            "clock",
            "Clock",
            Status::Unknown,
            "Could not query timedatectl.",
        ),
    }
}

pub fn check_systemd() -> Check {
    if std::path::Path::new("/run/systemd/system").exists() {
        Check::new(
            "systemd",
            "Service manager",
            Status::Pass,
            "systemd is running.",
        )
    } else {
        Check::new(
            "systemd",
            "Service manager",
            Status::Fail,
            "systemd is not running (container without init?).",
        )
        .remedy("Install on a full VM or bare-metal server, not inside a container.")
    }
}

/// Try a plain TCP connect to well-known MX hosts on port 25. Connect only — never sends mail.
pub fn check_outbound_25() -> Check {
    let targets = [
        "gmail-smtp-in.l.google.com:25",
        "mx1.mail.yahoo.com:25",
        "smtp.mail.me.com:25",
    ];
    // smtp.mail.me.com accepts on 25 too; any one success proves the port is open.
    let mut last_err = String::from("no targets resolved");
    for t in targets {
        let addrs: Vec<SocketAddr> = match t.to_socket_addrs() {
            Ok(a) => a.filter(|a| a.is_ipv4()).collect(),
            Err(e) => {
                last_err = e.to_string();
                continue;
            }
        };
        for addr in addrs.into_iter().take(2) {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(6)) {
                Ok(_) => {
                    return Check::new(
                        "port25",
                        "Outbound port 25",
                        Status::Pass,
                        "This server can reach other mail servers on port 25.",
                    );
                }
                Err(e) => last_err = e.to_string(),
            }
        }
    }
    Check::new("port25", "Outbound port 25", Status::Warn,
        format!("Could not reach other mail servers on port 25 ({last_err}). Your provider probably blocks it."))
        .remedy("Receiving still works. Choose relay mode for sending, or ask your provider to open port 25.")
}

fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_documentation()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80)
        }
    }
}

/// Public addresses: a probe endpoint if configured, then OpenDNS's resolver trick, then interfaces.
pub async fn public_ips() -> (Option<String>, Option<String>) {
    let mut v4 = None;
    let mut v6 = None;

    if let Ok(base) = std::env::var("YUNORI_PROBE_URL")
        && let Ok(resp) = reqwest::Client::new()
            .get(format!("{base}/ip"))
            .timeout(Duration::from_secs(5))
            .send()
            .await
        && let Ok(text) = resp.text().await
        && let Ok(ip) = text.trim().parse::<IpAddr>()
    {
        if ip.is_ipv4() {
            v4 = Some(ip.to_string())
        } else {
            v6 = Some(ip.to_string())
        }
    }

    if v4.is_none() {
        let opendns = Resolver::at("208.67.222.222".parse().unwrap());
        if let Ok(ips) = opendns.a("myip.opendns.com").await {
            v4 = ips.into_iter().next();
        }
    }

    if let Ok(out) = Command::new("ip")
        .args(["-o", "addr", "show", "scope", "global"])
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if let (Some(fam), Some(addr)) = (cols.get(2), cols.get(3)) {
                let ip_str = addr.split('/').next().unwrap_or("");
                if let Ok(ip) = ip_str.parse::<IpAddr>()
                    && is_public(&ip)
                {
                    match *fam {
                        "inet" if v4.is_none() => v4 = Some(ip.to_string()),
                        "inet6" if v6.is_none() => v6 = Some(ip.to_string()),
                        _ => {}
                    }
                }
            }
        }
    }
    (v4, v6)
}

/// Host record gate: does `hostname` resolve to this server on public DNS?
pub async fn check_host_record(
    hostname: &str,
    ipv4: Option<&str>,
    dns_impl: &dyn DnsLookup,
) -> Check {
    let label = format!("{hostname} points here");
    match dns_impl.a(hostname).await {
        Err(e) => Check::new(
            "host_record",
            &label,
            Status::Unknown,
            format!("Public DNS lookup failed ({e})."),
        ),
        Ok(ips) if ips.is_empty() => Check::new(
            "host_record",
            &label,
            Status::Fail,
            format!("{hostname} has no A record yet."),
        )
        .remedy(match ipv4 {
            Some(ip) => format!("At your DNS provider, add: {hostname}  A  {ip}"),
            None => format!(
                "At your DNS provider, add an A record for {hostname} pointing at this server."
            ),
        }),
        Ok(ips) => match ipv4 {
            Some(ip) if ips.iter().any(|a| a == ip) => Check::new(
                "host_record",
                &label,
                Status::Pass,
                format!("{hostname} → {ip}."),
            ),
            Some(ip) => Check::new(
                "host_record",
                &label,
                Status::Fail,
                format!(
                    "{hostname} points at {}, not this server ({ip}).",
                    ips.join(", ")
                ),
            )
            .remedy(format!("Change the A record of {hostname} to {ip}.")),
            None => Check::new(
                "host_record",
                &label,
                Status::Warn,
                format!(
                    "{hostname} → {} (could not determine this server's IP to compare).",
                    ips.join(", ")
                ),
            ),
        },
    }
}

/// Full preflight (installer step 1 and `GET /preflight`).
pub async fn preflight(paths: &Paths, hostname: Option<&str>) -> Vec<Check> {
    let mut out = vec![
        check_os(paths),
        check_systemd(),
        check_cpu(),
        check_memory(),
        check_disk(paths),
    ];
    out.extend(check_ports(&["stalwart", "yunori-mail"]));
    out.push(check_existing_mta());
    out.push(check_clock());
    out.push(
        tokio::task::spawn_blocking(check_outbound_25)
            .await
            .unwrap_or_else(|_| {
                Check::new(
                    "port25",
                    "Outbound port 25",
                    Status::Unknown,
                    "Check did not complete.",
                )
            }),
    );
    let (v4, v6) = public_ips().await;
    out.push(match (&v4, &v6) {
        (None, None) => Check::new(
            "public_ip",
            "Public address",
            Status::Fail,
            "This server has no public IP address.",
        )
        .remedy("A mail server needs a static public IPv4 address."),
        _ => Check::new(
            "public_ip",
            "Public address",
            Status::Pass,
            format!(
                "{}{}",
                v4.as_deref()
                    .map(|v| format!("IPv4 {v}"))
                    .unwrap_or_default(),
                v6.as_deref()
                    .map(|v| format!("{}IPv6 {v}", if v4.is_some() { ", " } else { "" }))
                    .unwrap_or_default()
            ),
        ),
    });
    let public = Resolver::public();
    if let Some(ip) = &v4 {
        let local = Resolver::system();
        out.extend(dns::reputation::check_blocklists(ip, &local).await);
        let ptr_names = public.ptr(ip.parse().unwrap()).await.unwrap_or_default();
        let provider = dns::reputation::detect_provider(&ptr_names);
        let g = dns::reputation::guidance(provider);
        let mut c = Check::new(
            "provider",
            "Hosting provider",
            Status::Info,
            format!("Detected {}. {} {}", g.name, g.port25, g.ptr),
        );
        if let Some(url) = g.url {
            c = c.help(url);
        }
        out.push(c);
    }
    if let Some(h) = hostname {
        out.push(check_host_record(h, v4.as_deref(), &public).await);
    }
    out
}

pub fn has_blocking_failure(checks: &[Check]) -> bool {
    const BLOCKING: &[&str] = &["os", "systemd", "cpu", "memory", "disk", "mta", "public_ip"];
    checks.iter().any(|c| {
        c.status == Status::Fail && (BLOCKING.contains(&c.id.as_str()) || c.id.starts_with("port:"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_parsing() {
        let os = parse_os_release(
            "PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nID=debian\nVERSION_ID=\"12\"\n",
        );
        assert_eq!(os.id, "debian");
        assert!(os_supported(&os));
        assert!(!os_supported(&parse_os_release(
            "ID=centos\nVERSION_ID=\"7\""
        )));
    }

    #[test]
    fn cpu_v2_detection() {
        let good = "flags\t\t: fpu cx16 lahf_lm popcnt sse4_1 sse4_2 ssse3 avx";
        assert_eq!(cpu_check_from("x86_64", good).status, Status::Pass);
        let kvm64 = "flags\t\t: fpu cx16 sse2";
        let c = cpu_check_from("x86_64", kvm64);
        assert_eq!(c.status, Status::Fail);
        assert!(c.remedy.unwrap().contains("kvm64"));
        assert_eq!(cpu_check_from("aarch64", "").status, Status::Pass);
    }

    #[test]
    fn meminfo_parse() {
        assert_eq!(
            parse_meminfo("MemTotal:        2048000 kB\n", "MemTotal"),
            Some(2048000 * 1024)
        );
    }

    #[test]
    fn public_ip_filter() {
        assert!(!is_public(&"10.0.0.1".parse().unwrap()));
        assert!(!is_public(&"100.64.1.1".parse().unwrap()));
        assert!(is_public(&"203.0.114.1".parse().unwrap()));
        assert!(!is_public(&"fd00::1".parse().unwrap()));
        assert!(is_public(&"2a01:4f8::1".parse().unwrap()));
    }

    #[tokio::test]
    async fn host_record_gate() {
        let mut d = crate::dns::fake::FakeDns::default();
        let c = check_host_record("mail.example.com", Some("203.0.113.10"), &d).await;
        assert_eq!(c.status, Status::Fail);
        assert!(c.remedy.unwrap().contains("203.0.113.10"));
        d.a.insert("mail.example.com".into(), vec!["203.0.113.10".into()]);
        assert_eq!(
            check_host_record("mail.example.com", Some("203.0.113.10"), &d)
                .await
                .status,
            Status::Pass
        );
    }

    #[test]
    fn blocking_failures() {
        let soft = vec![Check::new("port25", "x", Status::Fail, "")];
        assert!(!has_blocking_failure(&soft));
        let hard = vec![Check::new("port:25", "x", Status::Fail, "")];
        assert!(has_blocking_failure(&hard));
    }
}
